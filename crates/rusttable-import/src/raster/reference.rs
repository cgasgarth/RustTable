use std::path::{Component, Path, PathBuf};

use rusttable_catalog::{SourcePath, SourcePathError};
use sha2::{Digest, Sha256};

const PREFIX: &str = "reference-v1";
const PATH_IDENTITY_DOMAIN: &[u8] = b"rusttable-reference-path-v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceSourceError {
    UnsupportedPathEncoding,
    InvalidEncoding,
    InvalidSourcePath,
    PathEscapesRoot,
}

/// Encodes a canonical reference-in-place source without exposing path components.
///
/// `path_identity` must be the value returned by [`reference_path_identity`].
///
/// # Errors
///
/// Returns a typed error for an unrepresentable path or oversized source key.
pub fn encode_reference_source(
    path: &Path,
    path_identity: [u8; 32],
) -> Result<SourcePath, ReferenceSourceError> {
    let path = normalize_reference_path(path)?;
    let path = path
        .to_str()
        .ok_or(ReferenceSourceError::UnsupportedPathEncoding)?;
    let value = format!(
        "{PREFIX}/{}/{path_hex}",
        hex(&path_identity),
        path_hex = hex(path.as_bytes())
    );
    SourcePath::new(&value).map_err(|_: SourcePathError| ReferenceSourceError::InvalidSourcePath)
}

/// Hashes the exact accepted path bytes without resolving or canonicalizing it.
///
/// # Errors
///
/// Returns a typed error when the path cannot be represented by the reference format.
pub fn reference_path_identity(path: &Path) -> Result<[u8; 32], ReferenceSourceError> {
    let path = normalize_reference_path(path)?;
    let path = path
        .to_str()
        .ok_or(ReferenceSourceError::UnsupportedPathEncoding)?;
    let mut hasher = Sha256::new();
    hasher.update(PATH_IDENTITY_DOMAIN);
    hasher.update(path.as_bytes());
    Ok(hasher.finalize().into())
}

/// Returns the lexical canonical form used for a reference source.
///
/// The policy is deliberately filesystem-independent: `.` components are
/// removed, `..` components are resolved without following symlinks, and case
/// is preserved. The snapshot reader separately rejects symlinks, so a path
/// alias cannot silently merge two physical files. Preserving case also keeps
/// identity deterministic on case-insensitive filesystems; callers that need
/// a case-folded policy must normalize before entering this API.
///
/// # Errors
///
/// Returns a typed error for an unrepresentable path or a relative path that
/// escapes above its logical root.
pub fn normalize_reference_path(path: &Path) -> Result<PathBuf, ReferenceSourceError> {
    if path.as_os_str().is_empty() {
        return Err(ReferenceSourceError::InvalidSourcePath);
    }
    let mut normalized = PathBuf::new();
    let mut component_count = 0_usize;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::Normal(value) => {
                normalized.push(value);
                component_count = component_count.saturating_add(1);
            }
            Component::ParentDir => {
                if component_count == 0 {
                    if path.is_absolute() {
                        continue;
                    }
                    return Err(ReferenceSourceError::PathEscapesRoot);
                }
                normalized.pop();
                component_count = component_count.saturating_sub(1);
            }
        }
    }
    if normalized.as_os_str().is_empty() || (path.is_relative() && component_count == 0) {
        return Err(ReferenceSourceError::InvalidSourcePath);
    }
    Ok(normalized)
}

/// Decodes one versioned privacy-safe source key into its physical reference.
///
/// # Errors
///
/// Returns a typed error for malformed or unsupported encoding.
pub fn decode_reference_source(source: &SourcePath) -> Result<PathBuf, ReferenceSourceError> {
    Ok(parse_reference_source(source)?.1)
}

/// Returns the canonical path identity embedded in one reference source key.
///
/// # Errors
///
/// Returns a typed error when the versioned source key is malformed or unsupported.
pub fn reference_source_identity(source: &SourcePath) -> Result<[u8; 32], ReferenceSourceError> {
    Ok(parse_reference_source(source)?.0)
}

fn parse_reference_source(
    source: &SourcePath,
) -> Result<([u8; 32], PathBuf), ReferenceSourceError> {
    let mut components = source.as_str().split('/');
    if components.next() != Some(PREFIX) {
        return Err(ReferenceSourceError::InvalidEncoding);
    }
    let hash = components
        .next()
        .ok_or(ReferenceSourceError::InvalidEncoding)?;
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ReferenceSourceError::InvalidEncoding);
    }
    let encoded = components
        .next()
        .ok_or(ReferenceSourceError::InvalidEncoding)?;
    if components.next().is_some() {
        return Err(ReferenceSourceError::InvalidEncoding);
    }
    let identity = decode_hex(hash)?
        .try_into()
        .map_err(|_: Vec<u8>| ReferenceSourceError::InvalidEncoding)?;
    let bytes = decode_hex(encoded)?;
    let path = String::from_utf8(bytes).map_err(|_| ReferenceSourceError::InvalidEncoding)?;
    Ok((identity, PathBuf::from(path)))
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn decode_hex(encoded: &str) -> Result<Vec<u8>, ReferenceSourceError> {
    if !encoded.len().is_multiple_of(2) {
        return Err(ReferenceSourceError::InvalidEncoding);
    }
    let (pairs, remainder) = encoded.as_bytes().as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(ReferenceSourceError::InvalidEncoding);
    }
    pairs
        .iter()
        .map(|pair| {
            let high = digit(pair[0])?;
            let low = digit(pair[1])?;
            Ok((high << 4) | low)
        })
        .collect()
}

fn digit(byte: u8) -> Result<u8, ReferenceSourceError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(ReferenceSourceError::InvalidEncoding),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        decode_reference_source, encode_reference_source, normalize_reference_path,
        reference_path_identity, reference_source_identity,
    };

    #[test]
    fn reference_source_round_trips_without_exposing_the_path_as_components() {
        let path = std::path::Path::new("/private/photos/one.png");
        let source = encode_reference_source(path, [7; 32]).expect("encoded source");

        assert_eq!(decode_reference_source(&source).unwrap(), path);
        assert_eq!(reference_source_identity(&source).unwrap(), [7; 32]);
        assert!(!source.as_str().contains("private"));
        assert!(!source.as_str().contains("one.png"));
        assert_eq!(reference_path_identity(path), reference_path_identity(path));
        assert_eq!(
            reference_path_identity(path),
            reference_path_identity(std::path::Path::new("/private/photos/../photos/one.png"))
        );
    }

    #[test]
    fn normalization_is_lexical_and_does_not_follow_symlinks_or_fold_case() {
        assert_eq!(
            normalize_reference_path(std::path::Path::new("/photos/./roll/../IMG.JPG")).unwrap(),
            std::path::PathBuf::from("/photos/IMG.JPG")
        );
        assert_ne!(
            reference_path_identity(std::path::Path::new("/photos/IMG.JPG")),
            reference_path_identity(std::path::Path::new("/photos/img.jpg"))
        );
        assert_eq!(
            decode_reference_source(
                &encode_reference_source(
                    std::path::Path::new("/photos/./roll/../IMG.JPG"),
                    [7; 32]
                )
                .unwrap()
            )
            .unwrap(),
            std::path::PathBuf::from("/photos/IMG.JPG")
        );
    }

    #[test]
    fn relative_parent_escape_is_rejected() {
        assert_eq!(
            normalize_reference_path(std::path::Path::new("../outside/image.jpg")),
            Err(super::ReferenceSourceError::PathEscapesRoot)
        );
    }
}
