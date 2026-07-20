use std::path::{Path, PathBuf};

use rusttable_catalog::{SourcePath, SourcePathError};

const PREFIX: &str = "reference-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceSourceError {
    UnsupportedPathEncoding,
    InvalidEncoding,
    InvalidSourcePath,
}

/// Encodes an absolute reference-in-place source without exposing path components.
///
/// # Errors
///
/// Returns a typed error for an unrepresentable path or oversized source key.
pub fn encode_reference_source(
    path: &Path,
    sha256: [u8; 32],
) -> Result<SourcePath, ReferenceSourceError> {
    let path = path
        .to_str()
        .ok_or(ReferenceSourceError::UnsupportedPathEncoding)?;
    let value = format!(
        "{PREFIX}/{}/{path_hex}",
        hex(&sha256),
        path_hex = hex(path.as_bytes())
    );
    SourcePath::new(&value).map_err(|_: SourcePathError| ReferenceSourceError::InvalidSourcePath)
}

/// Decodes one versioned privacy-safe source key into its physical reference.
///
/// # Errors
///
/// Returns a typed error for malformed or unsupported encoding.
pub fn decode_reference_source(source: &SourcePath) -> Result<PathBuf, ReferenceSourceError> {
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
    let bytes = decode_hex(encoded)?;
    let path = String::from_utf8(bytes).map_err(|_| ReferenceSourceError::InvalidEncoding)?;
    Ok(PathBuf::from(path))
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
    use super::{decode_reference_source, encode_reference_source};

    #[test]
    fn reference_source_round_trips_without_exposing_the_path_as_components() {
        let path = std::path::Path::new("/private/photos/one.png");
        let source = encode_reference_source(path, [7; 32]).expect("encoded source");

        assert_eq!(decode_reference_source(&source).unwrap(), path);
        assert!(!source.as_str().contains("private"));
        assert!(!source.as_str().contains("one.png"));
    }
}
