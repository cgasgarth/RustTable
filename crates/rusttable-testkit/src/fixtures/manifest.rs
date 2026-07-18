use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Compression {
    #[default]
    None,
    Gzip,
    Zip,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PrivacyClass {
    Synthetic,
    Scrubbed,
    External,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FixtureManifest {
    pub version: u32,
    #[serde(default = "default_governed_roots")]
    pub governed_roots: Vec<String>,
    #[serde(default)]
    pub limits: FixtureManifestLimits,
    #[serde(default)]
    pub fixtures: Vec<FixtureEntry>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FixtureManifestLimits {
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: u64,
    #[serde(default = "default_max_total_bytes")]
    pub max_total_bytes: u64,
    #[serde(default = "default_max_decompressed_bytes")]
    pub max_decompressed_bytes: u64,
    #[serde(default = "default_max_compression_ratio")]
    pub max_compression_ratio: u64,
}

impl Default for FixtureManifestLimits {
    fn default() -> Self {
        Self {
            max_file_bytes: default_max_file_bytes(),
            max_total_bytes: default_max_total_bytes(),
            max_decompressed_bytes: default_max_decompressed_bytes(),
            max_compression_ratio: default_max_compression_ratio(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FixtureEntry {
    pub id: String,
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub media_type: String,
    #[serde(default)]
    pub compression: Compression,
    pub privacy: PrivacyClass,
    #[serde(default)]
    pub consumers: Vec<String>,
    #[serde(default)]
    pub consuming_issue_ranges: Vec<String>,
    #[serde(default)]
    pub expected: FixtureExpectation,
    #[serde(default)]
    pub allow_privacy_fields: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Default)]
#[serde(deny_unknown_fields)]
pub struct FixtureExpectation {
    pub dimensions: Option<FixtureDimensions>,
    pub orientation: Option<String>,
    #[serde(default)]
    pub metadata: Vec<String>,
    pub output_sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub struct FixtureDimensions {
    pub width: u32,
    pub height: u32,
}

impl FixtureManifest {
    /// Parses and validates the stable TOML fixture manifest schema.
    ///
    /// # Errors
    ///
    /// Returns a typed error when TOML, schema, path, checksum, or limit
    /// validation fails.
    pub fn parse(source: &str) -> Result<Self, ManifestError> {
        let manifest: Self = toml::from_str(source).map_err(|error| ManifestError::Parse {
            message: error.to_string(),
        })?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validates IDs, paths, checksums, limits, and deterministic ordering rules.
    ///
    /// # Errors
    ///
    /// Returns a typed error when any manifest invariant is violated.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.version != 1 {
            return Err(ManifestError::UnsupportedVersion {
                version: self.version,
            });
        }
        if self.governed_roots.is_empty() {
            return Err(ManifestError::NoGovernedRoots);
        }
        let roots = self
            .governed_roots
            .iter()
            .map(|path| normalize_relative_path(path, "governed root"))
            .collect::<Result<Vec<_>, _>>()?;
        validate_limits(self.limits)?;
        let mut ids = Vec::new();
        let mut paths = Vec::new();
        for entry in &self.fixtures {
            if entry.id.is_empty() || entry.id.contains('/') || entry.id.contains('\\') {
                return Err(ManifestError::InvalidId {
                    id: entry.id.clone(),
                });
            }
            if ids.iter().any(|id| id == &entry.id) {
                return Err(ManifestError::DuplicateId {
                    id: entry.id.clone(),
                });
            }
            ids.push(entry.id.clone());
            let path = normalize_relative_path(&entry.path, &entry.id)?;
            if paths.iter().any(|registered| registered == &path) {
                return Err(ManifestError::DuplicatePath { path });
            }
            if !roots.iter().any(|root| path.starts_with(root)) {
                return Err(ManifestError::OutsideGovernedRoot { path });
            }
            paths.push(path);
            if entry.size > self.limits.max_file_bytes {
                return Err(ManifestError::EntrySizeLimit {
                    id: entry.id.clone(),
                    size: entry.size,
                    limit: self.limits.max_file_bytes,
                });
            }
            validate_checksum(&entry.id, &entry.sha256)?;
            if entry.media_type.trim().is_empty() {
                return Err(ManifestError::MissingMediaType {
                    id: entry.id.clone(),
                });
            }
            if entry.allow_privacy_fields.iter().any(String::is_empty) {
                return Err(ManifestError::EmptyPrivacyField {
                    id: entry.id.clone(),
                });
            }
            if let Some(dimensions) = entry.expected.dimensions
                && (dimensions.width == 0 || dimensions.height == 0)
            {
                return Err(ManifestError::InvalidDimensions {
                    id: entry.id.clone(),
                });
            }
            if let Some(checksum) = &entry.expected.output_sha256 {
                validate_checksum(&entry.id, checksum)?;
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn fixture(&self, id: &str) -> Option<&FixtureEntry> {
        self.fixtures.iter().find(|entry| entry.id == id)
    }

    pub(crate) fn normalized_roots(&self) -> Result<Vec<PathBuf>, ManifestError> {
        self.governed_roots
            .iter()
            .map(|path| normalize_relative_path(path, "governed root"))
            .collect()
    }
}

impl FixtureEntry {
    /// Resolves a manifest path through canonical root and candidate paths.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the path is not portable, cannot be
    /// canonicalized, or resolves outside the root.
    pub fn canonical_path(&self, root: &Path) -> Result<PathBuf, ManifestError> {
        let relative = normalize_relative_path(&self.path, &self.id)?;
        let canonical_root = fs::canonicalize(root).map_err(|error| ManifestError::Io {
            path: root.to_path_buf(),
            message: error.to_string(),
        })?;
        let candidate = canonical_root.join(relative);
        let canonical = fs::canonicalize(&candidate).map_err(|error| ManifestError::Io {
            path: candidate.clone(),
            message: error.to_string(),
        })?;
        if !canonical.starts_with(&canonical_root) {
            return Err(ManifestError::SymlinkEscape {
                id: self.id.clone(),
            });
        }
        Ok(canonical)
    }
}

fn normalize_relative_path(value: &str, subject: &str) -> Result<PathBuf, ManifestError> {
    if value.trim().is_empty() {
        return Err(ManifestError::EmptyPath {
            subject: subject.to_owned(),
        });
    }
    let portable = value.replace('\\', "/");
    let path = Path::new(&portable);
    let windows_drive = portable.len() >= 3
        && portable.as_bytes()[0].is_ascii_alphabetic()
        && portable.as_bytes()[1] == b':'
        && portable.as_bytes()[2] == b'/';
    if path.is_absolute() || portable.starts_with('/') || windows_drive {
        return Err(ManifestError::AbsolutePath {
            path: value.to_owned(),
        });
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ManifestError::PathTraversal {
                    path: value.to_owned(),
                });
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(ManifestError::EmptyPath {
            subject: subject.to_owned(),
        });
    }
    Ok(normalized)
}

fn validate_checksum(id: &str, checksum: &str) -> Result<(), ManifestError> {
    if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ManifestError::InvalidChecksum { id: id.to_owned() });
    }
    Ok(())
}

fn validate_limits(limits: FixtureManifestLimits) -> Result<(), ManifestError> {
    if limits.max_file_bytes == 0
        || limits.max_total_bytes == 0
        || limits.max_decompressed_bytes == 0
        || limits.max_compression_ratio == 0
    {
        return Err(ManifestError::ZeroLimit);
    }
    Ok(())
}

fn default_governed_roots() -> Vec<String> {
    vec!["fixtures".to_owned()]
}

const fn default_max_file_bytes() -> u64 {
    64 * 1024 * 1024
}

const fn default_max_total_bytes() -> u64 {
    512 * 1024 * 1024
}

const fn default_max_decompressed_bytes() -> u64 {
    64 * 1024 * 1024
}

const fn default_max_compression_ratio() -> u64 {
    100
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestError {
    Parse { message: String },
    UnsupportedVersion { version: u32 },
    NoGovernedRoots,
    ZeroLimit,
    InvalidId { id: String },
    DuplicateId { id: String },
    DuplicatePath { path: PathBuf },
    OutsideGovernedRoot { path: PathBuf },
    EntrySizeLimit { id: String, size: u64, limit: u64 },
    InvalidChecksum { id: String },
    MissingMediaType { id: String },
    EmptyPrivacyField { id: String },
    InvalidDimensions { id: String },
    EmptyPath { subject: String },
    AbsolutePath { path: String },
    PathTraversal { path: String },
    Io { path: PathBuf, message: String },
    SymlinkEscape { id: String },
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse { message } => {
                write!(formatter, "fixture manifest parse failed: {message}")
            }
            Self::UnsupportedVersion { version } => {
                write!(formatter, "unsupported fixture manifest version {version}")
            }
            Self::NoGovernedRoots => formatter.write_str("fixture manifest has no governed roots"),
            Self::ZeroLimit => formatter.write_str("fixture manifest limits must be nonzero"),
            Self::InvalidId { id } => write!(formatter, "invalid fixture ID {id}"),
            Self::DuplicateId { id } => write!(formatter, "duplicate fixture ID {id}"),
            Self::DuplicatePath { path } => {
                write!(formatter, "duplicate fixture path {}", path.display())
            }
            Self::OutsideGovernedRoot { path } => write!(
                formatter,
                "fixture path {} is outside governed roots",
                path.display()
            ),
            Self::EntrySizeLimit { id, size, limit } => write!(
                formatter,
                "fixture {id} declares {size} bytes beyond limit {limit}"
            ),
            Self::InvalidChecksum { id } => {
                write!(formatter, "fixture {id} has an invalid SHA-256 checksum")
            }
            Self::MissingMediaType { id } => write!(formatter, "fixture {id} has no media type"),
            Self::EmptyPrivacyField { id } => {
                write!(formatter, "fixture {id} has an empty allowed privacy field")
            }
            Self::InvalidDimensions { id } => {
                write!(formatter, "fixture {id} has zero expected dimensions")
            }
            Self::EmptyPath { subject } => write!(formatter, "{subject} path is empty"),
            Self::AbsolutePath { path } => {
                write!(formatter, "absolute fixture path {path} is not allowed")
            }
            Self::PathTraversal { path } => write!(
                formatter,
                "path traversal is not allowed in fixture path {path}"
            ),
            Self::Io { path, message } => write!(
                formatter,
                "fixture path {} could not be resolved: {message}",
                path.display()
            ),
            Self::SymlinkEscape { id } => write!(
                formatter,
                "fixture {id} resolves outside the repository root"
            ),
        }
    }
}

impl std::error::Error for ManifestError {}
