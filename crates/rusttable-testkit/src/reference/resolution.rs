use std::env;
use std::fmt;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::identity::{CliReference, DEFAULT_FLAGS, ReferenceIdentity};

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceIdentityDocument {
    pub schema_version: u32,
    pub version: String,
    pub commit: String,
    pub source_path: PathBuf,
    pub executable_path: PathBuf,
    pub data_dir: PathBuf,
    pub executable_sha256: String,
    pub data_dir_sha256: String,
    pub opencl_bundle_sha256: String,
    pub target: String,
    pub architecture: String,
    pub build_options_hash: String,
    pub compiler: String,
    pub native_library_identity: String,
    pub normalized_log_ruleset: u32,
    #[serde(default)]
    pub required_flags: Vec<String>,
    pub cli: CliReference,
}

impl ReferenceIdentityDocument {
    /// Reads and validates the single checked-in reference identity document.
    ///
    /// # Errors
    ///
    /// Returns an error when the document cannot be read, parsed, or validated.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ReferenceProbeError> {
        let path = path.as_ref();
        let source = fs::read_to_string(path).map_err(|error| ReferenceProbeError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        let document: Self = toml::from_str(&source).map_err(|error| ReferenceProbeError::Pin {
            message: error.to_string(),
        })?;
        document.validate()?;
        Ok(document)
    }

    fn validate(&self) -> Result<(), ReferenceProbeError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(ReferenceProbeError::Pin {
                message: format!("unsupported identity schema {}", self.schema_version),
            });
        }
        if self.version.split('.').count() != 3
            || self.version.split('.').any(|part| {
                part.is_empty() || !part.chars().all(|character| character.is_ascii_digit())
            })
        {
            return Err(ReferenceProbeError::Pin {
                message: "reference version must be an exact semantic version".to_owned(),
            });
        }
        if self.commit.len() != 40 || !self.commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ReferenceProbeError::Pin {
                message: "reference commit must be a 40-character hexadecimal SHA".to_owned(),
            });
        }
        for (label, value) in [
            ("executable", &self.executable_sha256),
            ("data directory", &self.data_dir_sha256),
            ("OpenCL bundle", &self.opencl_bundle_sha256),
        ] {
            if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(ReferenceProbeError::Pin {
                    message: format!("{label} SHA-256 must be a 64-character hexadecimal value"),
                });
            }
        }
        if self.source_path.is_absolute()
            || self.executable_path.is_absolute()
            || self.data_dir.is_absolute()
        {
            return Err(ReferenceProbeError::Pin {
                message: "checked-in reference paths must be relative local aliases".to_owned(),
            });
        }
        if self.target.is_empty()
            || self.architecture.is_empty()
            || self.build_options_hash.is_empty()
            || self.compiler.is_empty()
            || self.native_library_identity.is_empty()
            || self.cli.name != "darktable-cli"
            || self.cli.reference_hash.is_empty()
        {
            return Err(ReferenceProbeError::Pin {
                message: "reference identity evidence is incomplete".to_owned(),
            });
        }
        if self.normalized_log_ruleset != 1 {
            return Err(ReferenceProbeError::Pin {
                message: "unsupported normalized log ruleset".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceIdentityOverrides {
    pub source_path: Option<PathBuf>,
    pub executable_path: Option<PathBuf>,
    pub data_dir: Option<PathBuf>,
}

impl ReferenceIdentityOverrides {
    fn resolve(
        &self,
        document: &Path,
        identity: &ReferenceIdentityDocument,
    ) -> Result<Paths, ReferenceProbeError> {
        let count = [
            self.source_path.is_some(),
            self.executable_path.is_some(),
            self.data_dir.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();
        if count != 0 && count != 3 {
            return Err(ReferenceProbeError::AmbiguousOverride {
                message: "source, executable, and data overrides must be supplied together"
                    .to_owned(),
            });
        }
        let base = document.parent().unwrap_or_else(|| Path::new("."));
        Ok(Paths {
            source: self
                .source_path
                .clone()
                .unwrap_or_else(|| base.join(&identity.source_path)),
            executable: self
                .executable_path
                .clone()
                .unwrap_or_else(|| base.join(&identity.executable_path)),
            data: self
                .data_dir
                .clone()
                .unwrap_or_else(|| base.join(&identity.data_dir)),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Paths {
    source: PathBuf,
    executable: PathBuf,
    data: PathBuf,
}

/// Resolves local paths and proves that all four reference surfaces agree.
///
/// # Errors
///
/// Returns an error when any selected asset is missing, dirty, mismatched, or
/// does not implement the pinned CLI contract.
pub fn resolve_reference(
    document_path: impl AsRef<Path>,
    overrides: &ReferenceIdentityOverrides,
) -> Result<ReferenceIdentity, ReferenceProbeError> {
    let document_path = document_path.as_ref();
    let document = ReferenceIdentityDocument::from_file(document_path)?;
    let paths = overrides.resolve(document_path, &document)?;
    if !paths.source.is_dir() {
        return Err(ReferenceProbeError::MissingSource { path: paths.source });
    }
    if !paths.executable.is_file() {
        return Err(ReferenceProbeError::MissingExecutable {
            path: paths.executable,
        });
    }
    if !paths.data.is_dir() {
        return Err(ReferenceProbeError::MissingDataDirectory { path: paths.data });
    }
    let commit = source_commit(&paths.source)?;
    if commit != document.commit {
        return Err(ReferenceProbeError::SourceMismatch {
            expected: document.commit,
            actual: commit,
        });
    }
    let executable_hash = hash_file(&paths.executable)?;
    if executable_hash != document.executable_sha256 {
        return Err(ReferenceProbeError::ExecutableMismatch {
            expected: document.executable_sha256,
            actual: executable_hash,
        });
    }
    let data_hash = hash_directory(&paths.data)?;
    if data_hash != document.data_dir_sha256 {
        return Err(ReferenceProbeError::DataMismatch {
            expected: document.data_dir_sha256,
            actual: data_hash,
        });
    }
    let opencl_path = paths.data.join("kernels");
    let opencl_hash = if opencl_path.is_dir() {
        hash_directory(&opencl_path)?
    } else {
        hash_bytes(&[])
    };
    if opencl_hash != document.opencl_bundle_sha256 {
        return Err(ReferenceProbeError::OpenclMismatch {
            expected: document.opencl_bundle_sha256,
            actual: opencl_hash,
        });
    }
    validate_target(&document.target, &document.architecture)?;
    probe_cli(&paths.executable, &document, &paths.data)?;
    let required_flags = flags(if document.cli.core_flags.is_empty() {
        &document.required_flags
    } else {
        &document.cli.core_flags
    });
    Ok(ReferenceIdentity {
        source_dir: paths.source,
        executable: paths.executable,
        version: document.version,
        commit: document.commit,
        data_dir: paths.data,
        executable_sha256: executable_hash.clone(),
        data_dir_sha256: data_hash.clone(),
        opencl_bundle_sha256: opencl_hash,
        target: document.target.clone(),
        architecture: document.architecture,
        build_options_hash: document.build_options_hash.clone(),
        compiler: document.compiler,
        native_library_identity: document.native_library_identity,
        cli: document.cli,
        required_flags,
        normalized_log_ruleset: document.normalized_log_ruleset,
        executable_hash: executable_hash.clone(),
        data_bundle_hash: data_hash.clone(),
        target_triple: document.target.clone(),
        c_abi_model: document.target.clone(),
        build_option_hash: document.build_options_hash.clone(),
    })
}

/// Re-checks the pinned inputs after a run so the reference remains read-only.
///
/// # Errors
///
/// Returns an error when the source, executable, or data tree changed.
pub fn verify_reference_unchanged(identity: &ReferenceIdentity) -> Result<(), ReferenceProbeError> {
    if identity.source_dir.as_os_str().is_empty() {
        return Ok(());
    }
    let commit = source_commit(&identity.source_dir)?;
    if commit != identity.commit {
        return Err(ReferenceProbeError::SourceMismatch {
            expected: identity.commit.clone(),
            actual: commit,
        });
    }
    let executable = hash_file(&identity.executable)?;
    if executable != identity.executable_sha256 {
        return Err(ReferenceProbeError::ExecutableMismatch {
            expected: identity.executable_sha256.clone(),
            actual: executable,
        });
    }
    let data = hash_directory(&identity.data_dir)?;
    if data != identity.data_dir_sha256 {
        return Err(ReferenceProbeError::DataMismatch {
            expected: identity.data_dir_sha256.clone(),
            actual: data,
        });
    }
    Ok(())
}

pub(crate) fn flags(configured: &[String]) -> Vec<String> {
    if configured.is_empty() {
        DEFAULT_FLAGS
            .iter()
            .map(|flag| (*flag).to_owned())
            .collect()
    } else {
        configured.to_owned()
    }
}

fn probe_cli(
    executable: &Path,
    document: &ReferenceIdentityDocument,
    data_dir: &Path,
) -> Result<(), ReferenceProbeError> {
    let mut command = Command::new(executable);
    command
        .env_clear()
        .env("PATH", default_path())
        .env("HOME", env::temp_dir())
        .env("RUSTTABLE_REFERENCE_DATA", data_dir)
        .args([&document.cli.version_flag]);
    let version = command
        .output()
        .map_err(|error| ReferenceProbeError::Spawn {
            executable: executable.to_path_buf(),
            message: error.to_string(),
        })?;
    let version_text = String::from_utf8_lossy(&version.stdout);
    if !version.status.success()
        || !version_text
            .split_whitespace()
            .any(|word| word == document.version)
    {
        return Err(ReferenceProbeError::CliIdentityMismatch {
            expected: document.version.clone(),
            actual: version_text.trim().to_owned(),
        });
    }
    let help = Command::new(executable)
        .env_clear()
        .env("PATH", default_path())
        .env("HOME", env::temp_dir())
        .env("RUSTTABLE_REFERENCE_DATA", data_dir)
        .arg(&document.cli.help_flag)
        .output()
        .map_err(|error| ReferenceProbeError::Spawn {
            executable: executable.to_path_buf(),
            message: error.to_string(),
        })?;
    let help_text = String::from_utf8_lossy(&help.stdout);
    let required = if document.cli.required_flags.is_empty() {
        vec![
            "--width".to_owned(),
            "--height".to_owned(),
            document.cli.core_prefix.clone(),
        ]
    } else {
        document.cli.required_flags.clone()
    };
    if !help.status.success()
        || required
            .iter()
            .any(|flag| !help_text.split_whitespace().any(|word| word == flag))
    {
        let missing = required
            .iter()
            .find(|flag| !help_text.split_whitespace().any(|word| word == *flag))
            .cloned()
            .unwrap_or_else(|| document.cli.help_flag.clone());
        return Err(ReferenceProbeError::UnsupportedFlag { flag: missing });
    }
    Ok(())
}

fn source_commit(path: &Path) -> Result<String, ReferenceProbeError> {
    let status = Command::new("git")
        .args(["-C", &path.display().to_string(), "status", "--porcelain"])
        .output()
        .map_err(|error| ReferenceProbeError::SourceIo(error.to_string()))?;
    if !status.status.success() {
        return Err(ReferenceProbeError::SourceIo(
            "source is not a Git worktree".to_owned(),
        ));
    }
    if !status.stdout.is_empty() {
        return Err(ReferenceProbeError::SourceMismatch {
            expected: "clean pinned worktree".to_owned(),
            actual: String::from_utf8_lossy(&status.stdout).trim().to_owned(),
        });
    }
    let output = Command::new("git")
        .args(["-C", &path.display().to_string(), "rev-parse", "HEAD"])
        .output()
        .map_err(|error| ReferenceProbeError::SourceIo(error.to_string()))?;
    if !output.status.success() {
        return Err(ReferenceProbeError::SourceIo(
            "source has no HEAD".to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn hash_file(path: &Path) -> Result<String, ReferenceProbeError> {
    fs::read(path)
        .map(|bytes| hash_bytes(&bytes))
        .map_err(|error| ReferenceProbeError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

fn hash_directory(path: &Path) -> Result<String, ReferenceProbeError> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| ReferenceProbeError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?
        .map(|entry| entry.map(|value| value.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ReferenceProbeError::Io {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    entries.sort();
    let mut bytes = Vec::new();
    for entry in entries {
        if entry.is_symlink() {
            return Err(ReferenceProbeError::DataMismatch {
                expected: "no symlinks".to_owned(),
                actual: entry.display().to_string(),
            });
        }
        bytes.extend_from_slice(
            entry
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_bytes(),
        );
        if entry.is_dir() {
            bytes.extend_from_slice(hash_directory(&entry)?.as_bytes());
        } else {
            bytes.extend_from_slice(&fs::read(&entry).map_err(|error| {
                ReferenceProbeError::Io {
                    path: entry.clone(),
                    message: error.to_string(),
                }
            })?);
        }
    }
    Ok(hash_bytes(&bytes))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(result, "{byte:02x}");
    }
    result
}

fn validate_target(target: &str, architecture: &str) -> Result<(), ReferenceProbeError> {
    if architecture != env::consts::ARCH || !target.contains(env::consts::ARCH) {
        return Err(ReferenceProbeError::TargetMismatch {
            expected: format!("{target} / {architecture}"),
            actual: format!("{} / {}", expected_target(), env::consts::ARCH),
        });
    }
    Ok(())
}

fn expected_target() -> &'static str {
    if cfg!(target_os = "macos") {
        "aarch64-apple-darwin"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

fn default_path() -> &'static str {
    if cfg!(windows) {
        r"C:\Windows\System32;C:\Windows"
    } else {
        "/usr/bin:/bin"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferenceProbeError {
    Io {
        path: PathBuf,
        message: String,
    },
    SourceIo(String),
    Pin {
        message: String,
    },
    MissingSource {
        path: PathBuf,
    },
    MissingExecutable {
        path: PathBuf,
    },
    MissingDataDirectory {
        path: PathBuf,
    },
    AmbiguousOverride {
        message: String,
    },
    SourceMismatch {
        expected: String,
        actual: String,
    },
    ExecutableMismatch {
        expected: String,
        actual: String,
    },
    DataMismatch {
        expected: String,
        actual: String,
    },
    OpenclMismatch {
        expected: String,
        actual: String,
    },
    TargetMismatch {
        expected: String,
        actual: String,
    },
    CliIdentityMismatch {
        expected: String,
        actual: String,
    },
    Spawn {
        executable: PathBuf,
        message: String,
    },
    UnsupportedFlag {
        flag: String,
    },
    Executable {
        path: PathBuf,
        message: String,
    },
    ProbeExit {
        argument: String,
        code: Option<i32>,
        stderr: String,
    },
    IdentityMismatch {
        expected_version: String,
        expected_commit: String,
        actual: String,
    },
    Isolation {
        message: String,
    },
}

impl fmt::Display for ReferenceProbeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, message } => {
                write!(formatter, "reference I/O at {}: {message}", path.display())
            }
            Self::SourceIo(message) => write!(formatter, "reference source I/O: {message}"),
            Self::Pin { message } => write!(formatter, "invalid reference identity: {message}"),
            Self::MissingSource { path } => {
                write!(formatter, "reference source is missing: {}", path.display())
            }
            Self::MissingExecutable { path } => write!(
                formatter,
                "reference executable is missing: {}",
                path.display()
            ),
            Self::MissingDataDirectory { path } => write!(
                formatter,
                "reference data directory is missing: {}",
                path.display()
            ),
            Self::AmbiguousOverride { message } => {
                write!(formatter, "ambiguous reference override: {message}")
            }
            Self::SourceMismatch { expected, actual } => write!(
                formatter,
                "reference source mismatch: expected {expected}, got {actual}"
            ),
            Self::ExecutableMismatch { expected, actual } => write!(
                formatter,
                "reference executable mismatch: expected {expected}, got {actual}"
            ),
            Self::DataMismatch { expected, actual } => write!(
                formatter,
                "reference data mismatch: expected {expected}, got {actual}"
            ),
            Self::OpenclMismatch { expected, actual } => write!(
                formatter,
                "reference OpenCL bundle mismatch: expected {expected}, got {actual}"
            ),
            Self::TargetMismatch { expected, actual } => write!(
                formatter,
                "reference target mismatch: expected {expected}, got {actual}"
            ),
            Self::CliIdentityMismatch { expected, actual } => write!(
                formatter,
                "reference CLI mismatch: expected {expected}, got {actual}"
            ),
            Self::Spawn {
                executable,
                message,
            } => write!(
                formatter,
                "cannot execute reference {}: {message}",
                executable.display()
            ),
            Self::UnsupportedFlag { flag } => write!(
                formatter,
                "reference CLI does not support required flag {flag}"
            ),
            Self::Executable { path, message } => write!(
                formatter,
                "invalid reference executable {}: {message}",
                path.display()
            ),
            Self::ProbeExit {
                argument,
                code,
                stderr,
            } => write!(formatter, "reference {argument} exited {code:?}: {stderr}"),
            Self::IdentityMismatch {
                expected_version,
                expected_commit,
                actual,
            } => write!(
                formatter,
                "reference identity mismatch; expected {expected_version} {expected_commit}, got {actual}"
            ),
            Self::Isolation { message } => formatter.write_str(message),
        }
    }
}

impl std::error::Error for ReferenceProbeError {}
