use std::fmt;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceRequest {
    pub source_fixture_id: String,
    pub source_path: PathBuf,
    pub xmp_path: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub output_format: OutputFormat,
    pub output_profile: ColorProfile,
    pub dimensions: Dimensions,
    pub timeout_ms: u64,
    #[serde(default)]
    pub execution_mode: ExecutionMode,
}

impl ReferenceRequest {
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Jpeg,
    Png,
    Tiff,
}

impl OutputFormat {
    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Tiff => "tif",
        }
    }

    pub(crate) const fn cli_name(self) -> &'static str {
        match self {
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Tiff => "tiff",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ColorProfile {
    Srgb,
    DisplayP3,
    AdobeRgb,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ExecutionMode {
    #[default]
    Cpu,
    Gpu,
}

impl ColorProfile {
    pub(crate) const fn cli_name(self) -> &'static str {
        match self {
            Self::Srgb => "srgb",
            Self::DisplayP3 => "display-p3",
            Self::AdobeRgb => "adobe-rgb",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceLimits {
    pub max_stdout_bytes: u64,
    pub max_stderr_bytes: u64,
    pub max_output_bytes: u64,
}

impl Default for ReferenceLimits {
    fn default() -> Self {
        Self {
            max_stdout_bytes: 64 * 1024,
            max_stderr_bytes: 64 * 1024,
            max_output_bytes: 256 * 1024 * 1024,
        }
    }
}

pub type ResourceLimits = ReferenceLimits;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceReceipt {
    pub source_fixture_id: String,
    pub xmp_path: Option<PathBuf>,
    pub config_path: Option<PathBuf>,
    pub output_format: OutputFormat,
    pub output_profile: ColorProfile,
    pub dimensions: Dimensions,
    pub timeout_ms: u64,
    pub status: ReferenceStatus,
    pub stdout_hash: String,
    pub stderr_hash: String,
    pub output_hash: String,
    pub reference_identity: ReferenceIdentityReceipt,
    pub normalized_log_ruleset: u32,
    pub execution_mode: ExecutionMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReferenceIdentityReceipt {
    pub version: String,
    pub commit: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStatus {
    pub code: Option<i32>,
    pub success: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceStatus {
    Completed(ExitStatus),
    Failed(ExitStatus),
    TimedOut,
    Cancelled,
}

impl ReferenceStatus {
    #[must_use]
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Completed(ExitStatus { success: true, .. }))
    }
}

#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}
