use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::compression;
use super::manifest::{FixtureEntry, FixtureManifest, ManifestError};
use super::privacy::{PrivacyFinding, PrivacyReport, PrivacyScanner};

const HEX: &[u8; 16] = b"0123456789abcdef";

pub struct FixtureRepository {
    root: PathBuf,
    manifest: FixtureManifest,
    scanner: PrivacyScanner,
}

impl FixtureRepository {
    /// Opens a manifest against a repository root without reading fixture data.
    ///
    /// # Errors
    ///
    /// Returns a manifest error when the schema or its invariants are invalid.
    pub fn new(root: impl Into<PathBuf>, manifest: FixtureManifest) -> Result<Self, ManifestError> {
        manifest.validate()?;
        Ok(Self {
            root: root.into(),
            manifest,
            scanner: PrivacyScanner::default(),
        })
    }

    #[must_use]
    pub fn manifest(&self) -> &FixtureManifest {
        &self.manifest
    }

    /// Returns entries in stable ID order for list-style tooling.
    #[must_use]
    pub fn list(&self) -> Vec<&FixtureEntry> {
        let mut entries = self.manifest.fixtures.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.id.cmp(&right.id));
        entries
    }

    /// Verifies registration, bounded sizes, decompression metadata, checksums,
    /// duplicate content, and privacy fields in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns a typed error for any missing, unregistered, oversized,
    /// non-canonical, duplicate, privacy-violating, or checksum-drifted fixture.
    pub fn verify(&self) -> Result<VerificationReport, VerificationError> {
        let registered = self.registered_paths();
        self.reject_unregistered_files(&registered)?;
        let mut verified = Vec::new();
        let mut hashes = BTreeMap::new();
        let mut total = 0u64;
        for entry in self.list() {
            let (fixture, next_total) = self.verify_entry(entry, &mut hashes, total)?;
            total = next_total;
            verified.push(fixture);
        }
        Ok(VerificationReport {
            fixtures: verified,
            total_bytes: total,
        })
    }

    /// Produces the deterministic, value-free metadata report without requiring
    /// the fixture set to be privacy-clean.
    ///
    /// # Errors
    ///
    /// Returns an error when a registered fixture cannot be resolved, read, or
    /// safely decompressed for scanning.
    pub fn scrub_report(&self) -> Result<ScrubReport, VerificationError> {
        let mut fixtures = Vec::new();
        for entry in self.list() {
            let path = entry.canonical_path(&self.root)?;
            let bytes = fs::read(&path).map_err(|error| VerificationError::Io {
                path: path.clone(),
                message: error.to_string(),
            })?;
            let scan_bytes = compression::decompressed_bytes(
                entry.compression,
                &bytes,
                self.manifest.limits.max_decompressed_bytes,
            )
            .map_err(|source| VerificationError::Compression {
                id: entry.id.clone(),
                source,
            })?;
            fixtures.push(ScrubbedFixture {
                id: entry.id.clone(),
                report: self.scanner.scan(Path::new(&entry.path), &scan_bytes),
            });
        }
        Ok(ScrubReport { fixtures })
    }

    fn registered_paths(&self) -> BTreeMap<PathBuf, String> {
        let mut paths = BTreeMap::new();
        for entry in &self.manifest.fixtures {
            let relative = portable_path(&entry.path);
            paths.insert(relative, entry.id.clone());
        }
        paths
    }

    fn verify_entry(
        &self,
        entry: &FixtureEntry,
        hashes: &mut BTreeMap<String, String>,
        total: u64,
    ) -> Result<(VerifiedFixture, u64), VerificationError> {
        let path = entry.canonical_path(&self.root)?;
        let metadata = fs::metadata(&path).map_err(|error| VerificationError::Io {
            path: path.clone(),
            message: error.to_string(),
        })?;
        if !metadata.is_file() {
            return Err(VerificationError::NotAFile {
                id: entry.id.clone(),
            });
        }
        let actual_size = metadata.len();
        if actual_size > self.manifest.limits.max_file_bytes {
            return Err(VerificationError::FileTooLarge {
                id: entry.id.clone(),
                limit: self.manifest.limits.max_file_bytes,
                actual: actual_size,
            });
        }
        if actual_size != entry.size {
            return Err(VerificationError::SizeDrift {
                id: entry.id.clone(),
                expected: entry.size,
                actual: actual_size,
            });
        }
        let next_total = total
            .checked_add(actual_size)
            .ok_or(VerificationError::Overflow)?;
        if next_total > self.manifest.limits.max_total_bytes {
            return Err(VerificationError::TotalSizeLimit {
                limit: self.manifest.limits.max_total_bytes,
                actual: next_total,
            });
        }
        let bytes = fs::read(&path).map_err(|error| VerificationError::Io {
            path: path.clone(),
            message: error.to_string(),
        })?;
        let actual_size_after_read =
            u64::try_from(bytes.len()).map_err(|_| VerificationError::Overflow)?;
        if actual_size_after_read != actual_size {
            return Err(VerificationError::SizeDrift {
                id: entry.id.clone(),
                expected: entry.size,
                actual: actual_size_after_read,
            });
        }
        let decompression = compression::preflight(
            entry.compression,
            &bytes,
            self.manifest.limits.max_decompressed_bytes,
            self.manifest.limits.max_compression_ratio,
        )
        .map_err(|source| compression_error(&entry.id, source))?;
        let checksum = sha256_hex(&bytes);
        if !checksum.eq_ignore_ascii_case(&entry.sha256) {
            return Err(VerificationError::ChecksumDrift {
                id: entry.id.clone(),
                expected: entry.sha256.clone(),
                actual: checksum,
            });
        }
        if let Some(first_id) = hashes.insert(checksum.clone(), entry.id.clone()) {
            return Err(VerificationError::DuplicateContent {
                first_id,
                duplicate_id: entry.id.clone(),
                sha256: checksum,
            });
        }
        let scan_bytes = compression::decompressed_bytes(
            entry.compression,
            &bytes,
            self.manifest.limits.max_decompressed_bytes,
        )
        .map_err(|source| compression_error(&entry.id, source))?;
        let privacy = self.scanner.scan(Path::new(&entry.path), &scan_bytes);
        let findings = without_allowed_fields(&privacy, &entry.allow_privacy_fields);
        if !findings.is_empty() {
            return Err(VerificationError::PrivacyLeak {
                id: entry.id.clone(),
                findings,
            });
        }
        Ok((
            VerifiedFixture {
                id: entry.id.clone(),
                path,
                size: actual_size,
                sha256: checksum,
                decompressed_size: decompression.output_size(),
            },
            next_total,
        ))
    }

    fn reject_unregistered_files(
        &self,
        registered: &BTreeMap<PathBuf, String>,
    ) -> Result<(), VerificationError> {
        for root in self.manifest.normalized_roots()? {
            let governed = self.root.join(root);
            collect_governed_files(&self.root, &governed, registered)?;
        }
        Ok(())
    }
}

fn collect_governed_files(
    repository_root: &Path,
    directory: &Path,
    registered: &BTreeMap<PathBuf, String>,
) -> Result<(), VerificationError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| VerificationError::Io {
            path: directory.to_path_buf(),
            message: error.to_string(),
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| VerificationError::Io {
            path: directory.to_path_buf(),
            message: error.to_string(),
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            return Err(VerificationError::HiddenFile { path: path.clone() });
        }
        let file_type = entry.file_type().map_err(|error| VerificationError::Io {
            path: path.clone(),
            message: error.to_string(),
        })?;
        if file_type.is_symlink() {
            return Err(VerificationError::SymlinkFile { path });
        }
        if file_type.is_dir() {
            collect_governed_files(repository_root, &path, registered)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(repository_root)
                .map(Path::to_path_buf)
                .map_err(|_| VerificationError::UnregisteredFile { path: path.clone() })?;
            if !registered.contains_key(&relative) {
                return Err(VerificationError::UnregisteredFile { path });
            }
        }
    }
    Ok(())
}

fn portable_path(path: &str) -> PathBuf {
    PathBuf::from(path.replace('\\', "/"))
}

fn without_allowed_fields(report: &PrivacyReport, allowed: &[String]) -> Vec<PrivacyFinding> {
    report
        .findings()
        .iter()
        .filter(|finding| !allowed.iter().any(|field| field == finding.field()))
        .cloned()
        .collect()
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedFixture {
    id: String,
    path: PathBuf,
    size: u64,
    sha256: String,
    decompressed_size: u64,
}

impl VerifiedFixture {
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    #[must_use]
    pub const fn decompressed_size(&self) -> u64 {
        self.decompressed_size
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationReport {
    fixtures: Vec<VerifiedFixture>,
    total_bytes: u64,
}

impl VerificationReport {
    #[must_use]
    pub fn fixtures(&self) -> &[VerifiedFixture] {
        &self.fixtures
    }

    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrubbedFixture {
    pub id: String,
    pub report: PrivacyReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScrubReport {
    pub fixtures: Vec<ScrubbedFixture>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationError {
    Manifest(ManifestError),
    Io {
        path: PathBuf,
        message: String,
    },
    NotAFile {
        id: String,
    },
    FileTooLarge {
        id: String,
        limit: u64,
        actual: u64,
    },
    SizeDrift {
        id: String,
        expected: u64,
        actual: u64,
    },
    TotalSizeLimit {
        limit: u64,
        actual: u64,
    },
    ChecksumDrift {
        id: String,
        expected: String,
        actual: String,
    },
    DuplicateContent {
        first_id: String,
        duplicate_id: String,
        sha256: String,
    },
    HiddenFile {
        path: PathBuf,
    },
    UnregisteredFile {
        path: PathBuf,
    },
    SymlinkFile {
        path: PathBuf,
    },
    Compression {
        id: String,
        source: super::compression::CompressionError,
    },
    DecompressedSizeLimit {
        id: String,
        limit: u64,
        actual: u64,
    },
    CompressionRatioLimit {
        id: String,
        limit: u64,
        compressed: u64,
        decompressed: u64,
    },
    PrivacyLeak {
        id: String,
        findings: Vec<PrivacyFinding>,
    },
    Overflow,
}

impl fmt::Display for VerificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(error) => error.fmt(formatter),
            Self::Io { path, message } => write!(
                formatter,
                "fixture path {} could not be read: {message}",
                path.display()
            ),
            Self::NotAFile { id } => write!(formatter, "fixture {id} is not a regular file"),
            Self::FileTooLarge { id, limit, actual } => write!(
                formatter,
                "fixture {id} is {actual} bytes, beyond limit {limit}"
            ),
            Self::SizeDrift {
                id,
                expected,
                actual,
            } => write!(
                formatter,
                "fixture {id} declares {expected} bytes but has {actual}"
            ),
            Self::TotalSizeLimit { limit, actual } => write!(
                formatter,
                "fixture set is {actual} bytes, beyond total limit {limit}"
            ),
            Self::ChecksumDrift { id, .. } => write!(formatter, "fixture {id} checksum drifted"),
            Self::DuplicateContent {
                first_id,
                duplicate_id,
                ..
            } => write!(
                formatter,
                "fixtures {first_id} and {duplicate_id} contain duplicate content"
            ),
            Self::HiddenFile { path } => write!(
                formatter,
                "hidden governed fixture file {} is not allowed",
                path.display()
            ),
            Self::UnregisteredFile { path } => write!(
                formatter,
                "governed fixture file {} is not registered",
                path.display()
            ),
            Self::SymlinkFile { path } => write!(
                formatter,
                "symlink fixture file {} is not allowed",
                path.display()
            ),
            Self::Compression { id, source } => {
                write!(formatter, "fixture {id} compression check failed: {source}")
            }
            Self::DecompressedSizeLimit { id, limit, actual } => write!(
                formatter,
                "fixture {id} decompresses to {actual} bytes, beyond limit {limit}"
            ),
            Self::CompressionRatioLimit {
                id,
                limit,
                compressed,
                decompressed,
            } => write!(
                formatter,
                "fixture {id} compression ratio {decompressed}/{compressed} exceeds limit {limit}"
            ),
            Self::PrivacyLeak { id, findings } => {
                write!(formatter, "fixture {id} has privacy fields: {findings:?}")
            }
            Self::Overflow => formatter.write_str("fixture verification arithmetic overflowed"),
        }
    }
}

impl std::error::Error for VerificationError {}

impl From<ManifestError> for VerificationError {
    fn from(error: ManifestError) -> Self {
        Self::Manifest(error)
    }
}

fn compression_error(id: &str, source: super::compression::CompressionError) -> VerificationError {
    match source {
        super::compression::CompressionError::OutputLimit { limit, actual } => {
            VerificationError::DecompressedSizeLimit {
                id: id.to_owned(),
                limit,
                actual,
            }
        }
        super::compression::CompressionError::RatioLimit {
            limit,
            compressed,
            decompressed,
        } => VerificationError::CompressionRatioLimit {
            id: id.to_owned(),
            limit,
            compressed,
            decompressed,
        },
        source => VerificationError::Compression {
            id: id.to_owned(),
            source,
        },
    }
}
