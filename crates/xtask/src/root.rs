use std::env;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use crate::process::{
    EnvironmentProfile, ProcessError, ProcessLimits, ProcessRequest, ProcessRunner,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRoot(PathBuf);

impl RepositoryRoot {
    /// Resolves the workspace root returned by Cargo metadata.
    pub fn discover(runner: &ProcessRunner) -> Result<Self, RootError> {
        let current = env::current_dir().map_err(|source| RootError::CurrentDirectory {
            message: source.to_string(),
        })?;
        let request =
            ProcessRequest::new("cargo", ["metadata", "--no-deps", "--format-version", "1"])
                .profile(EnvironmentProfile::RustTool)
                .current_dir(current)
                .limits(ProcessLimits {
                    max_stdout_bytes: 4 * 1024 * 1024,
                    max_stderr_bytes: 256 * 1024,
                    timeout: std::time::Duration::from_secs(4),
                })
                .cancellation(Arc::new(AtomicBool::new(false)));
        let result = runner.run(request).map_err(RootError::MetadataProcess)?;
        if !result.receipt.success() {
            return Err(RootError::MetadataFailed {
                status: result.receipt.status.clone(),
                stderr: String::from_utf8_lossy(&result.stderr).trim().to_owned(),
            });
        }
        let metadata: CargoMetadata =
            serde_json::from_slice(&result.stdout).map_err(|source| RootError::MetadataParse {
                message: source.to_string(),
            })?;
        let root = PathBuf::from(metadata.workspace_root);
        if !root.join("Cargo.toml").is_file() {
            return Err(RootError::MissingManifest(root));
        }
        Ok(Self(root))
    }

    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.0
    }

    #[must_use]
    pub fn join(&self, path: impl AsRef<std::path::Path>) -> PathBuf {
        self.0.join(path)
    }
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    workspace_root: String,
}

#[derive(Debug)]
pub enum RootError {
    CurrentDirectory { message: String },
    MetadataProcess(ProcessError),
    MetadataFailed { status: String, stderr: String },
    MetadataParse { message: String },
    MissingManifest(PathBuf),
}

impl fmt::Display for RootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CurrentDirectory { message } => {
                write!(formatter, "cannot inspect cwd: {message}")
            }
            Self::MetadataProcess(error) => {
                write!(formatter, "cargo metadata failed to run: {error}")
            }
            Self::MetadataFailed { status, stderr } => {
                write!(formatter, "cargo metadata failed ({status}): {stderr}")
            }
            Self::MetadataParse { message } => {
                write!(formatter, "cargo metadata returned invalid JSON: {message}")
            }
            Self::MissingManifest(path) => write!(
                formatter,
                "Cargo metadata root has no Cargo.toml: {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for RootError {}
