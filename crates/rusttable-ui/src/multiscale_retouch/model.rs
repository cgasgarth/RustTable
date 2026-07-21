//! Display-safe multiscale-retouch state and its application-service port.

#![allow(clippy::missing_errors_doc)]

use std::fmt;

pub const MULTISCALE_RETOUCH_FOCUS_ORDER: [&str; 9] = [
    "multiscale-retouch-band",
    "multiscale-retouch-source",
    "multiscale-retouch-target",
    "multiscale-retouch-strength",
    "multiscale-retouch-preview",
    "multiscale-retouch-cancel",
    "multiscale-retouch-progress",
    "multiscale-retouch-status",
    "multiscale-retouch-refresh",
];

pub const MULTISCALE_RETOUCH_MAX_STRENGTH: u8 = 100;
pub const MULTISCALE_RETOUCH_BANDS: [u8; 5] = [1, 2, 3, 4, 5];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiscaleBand {
    Original,
    Band(u8),
    Residual,
}

impl MultiscaleBand {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Original => "Original image",
            Self::Band(1) => "Band 1",
            Self::Band(2) => "Band 2",
            Self::Band(3) => "Band 3",
            Self::Band(4) => "Band 4",
            Self::Band(5) => "Band 5",
            Self::Band(_) => "Unsupported band",
            Self::Residual => "Residual",
        }
    }

    #[must_use]
    pub const fn all() -> [Self; 7] {
        [
            Self::Original,
            Self::Band(1),
            Self::Band(2),
            Self::Band(3),
            Self::Band(4),
            Self::Band(5),
            Self::Residual,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MultiscaleSourceTarget {
    #[default]
    Source,
    Target,
}

impl MultiscaleSourceTarget {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Source => "Source",
            Self::Target => "Target",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiscaleCapability {
    Available,
    Unavailable { reason: String },
}

impl MultiscaleCapability {
    #[must_use]
    pub const fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MultiscaleProgress {
    completed: u32,
    total: u32,
}

impl MultiscaleProgress {
    #[must_use]
    pub const fn new(completed: u32, total: u32) -> Option<Self> {
        if total == 0 || completed > total {
            None
        } else {
            Some(Self { completed, total })
        }
    }

    #[must_use]
    pub const fn completed(self) -> u32 {
        self.completed
    }

    #[must_use]
    pub const fn total(self) -> u32 {
        self.total
    }

    #[must_use]
    pub fn fraction(self) -> f64 {
        f64::from(self.completed) / f64::from(self.total)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiscaleRetouchStatus {
    Unavailable,
    Ready,
    Running { job: u64 },
    Cancelling { job: u64 },
    Completed { job: u64 },
    Cancelled { job: u64 },
    Failed { message: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultiscaleRetouchSnapshot {
    pub(crate) generation: u64,
    pub(crate) capability: MultiscaleCapability,
    pub(crate) band: MultiscaleBand,
    pub(crate) source: MultiscaleSourceTarget,
    pub(crate) target: MultiscaleSourceTarget,
    pub(crate) strength: u8,
    pub(crate) progress: Option<MultiscaleProgress>,
    pub(crate) status: MultiscaleRetouchStatus,
}

impl MultiscaleRetouchSnapshot {
    #[must_use]
    pub fn unavailable(generation: u64, reason: impl Into<String>) -> Self {
        Self {
            generation,
            capability: MultiscaleCapability::Unavailable {
                reason: reason.into(),
            },
            band: MultiscaleBand::Original,
            source: MultiscaleSourceTarget::Source,
            target: MultiscaleSourceTarget::Target,
            strength: 50,
            progress: None,
            status: MultiscaleRetouchStatus::Unavailable,
        }
    }

    #[must_use]
    pub fn available(generation: u64) -> Self {
        Self {
            generation,
            capability: MultiscaleCapability::Available,
            band: MultiscaleBand::Original,
            source: MultiscaleSourceTarget::Source,
            target: MultiscaleSourceTarget::Target,
            strength: 50,
            progress: None,
            status: MultiscaleRetouchStatus::Ready,
        }
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn capability(&self) -> &MultiscaleCapability {
        &self.capability
    }

    #[must_use]
    pub const fn band(&self) -> MultiscaleBand {
        self.band
    }

    #[must_use]
    pub const fn source(&self) -> MultiscaleSourceTarget {
        self.source
    }

    #[must_use]
    pub const fn target(&self) -> MultiscaleSourceTarget {
        self.target
    }

    #[must_use]
    pub const fn strength(&self) -> u8 {
        self.strength
    }

    #[must_use]
    pub const fn progress(&self) -> Option<MultiscaleProgress> {
        self.progress
    }

    #[must_use]
    pub const fn status(&self) -> &MultiscaleRetouchStatus {
        &self.status
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MultiscaleRetouchRequest {
    band: MultiscaleBand,
    source: MultiscaleSourceTarget,
    target: MultiscaleSourceTarget,
    strength: u8,
}

impl MultiscaleRetouchRequest {
    #[must_use]
    pub const fn new(
        band: MultiscaleBand,
        source: MultiscaleSourceTarget,
        target: MultiscaleSourceTarget,
        strength: u8,
    ) -> Self {
        Self {
            band,
            source,
            target,
            strength,
        }
    }

    #[must_use]
    pub const fn band(&self) -> MultiscaleBand {
        self.band
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiscaleRetouchAction {
    SetBand(MultiscaleBand),
    SetSource(MultiscaleSourceTarget),
    SetTarget(MultiscaleSourceTarget),
    SetStrength(u8),
    Preview,
    Cancel,
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultiscaleRetouchServiceError {
    BackendUnavailable,
    InvalidControl,
    NoActiveJob,
    StaleGeneration { expected: u64, actual: u64 },
    Failed(String),
}

impl fmt::Display for MultiscaleRetouchServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BackendUnavailable => formatter.write_str(
                "multiscale-retouch service is unavailable; no processing or edit was performed",
            ),
            Self::InvalidControl => formatter.write_str("multiscale-retouch control is invalid"),
            Self::NoActiveJob => formatter.write_str("no multiscale-retouch job is active"),
            Self::StaleGeneration { expected, actual } => {
                write!(
                    formatter,
                    "stale multiscale-retouch generation: expected {expected}, got {actual}"
                )
            }
            Self::Failed(message) => {
                write!(formatter, "multiscale-retouch service failed: {message}")
            }
        }
    }
}

impl std::error::Error for MultiscaleRetouchServiceError {}

#[derive(Debug, Clone, PartialEq)]
pub enum MultiscaleRetouchServiceEvent {
    Progress {
        generation: u64,
        job: u64,
        progress: MultiscaleProgress,
    },
    Completed {
        generation: u64,
        job: u64,
    },
    Cancelled {
        generation: u64,
        job: u64,
    },
    Failed {
        generation: u64,
        job: u64,
        message: String,
    },
}

pub trait MultiscaleRetouchServicePort {
    fn snapshot(
        &mut self,
        generation: u64,
    ) -> Result<MultiscaleRetouchSnapshot, MultiscaleRetouchServiceError>;

    fn update(
        &mut self,
        generation: u64,
        action: &MultiscaleRetouchAction,
    ) -> Result<MultiscaleRetouchSnapshot, MultiscaleRetouchServiceError>;

    fn start(
        &mut self,
        generation: u64,
        request: &MultiscaleRetouchRequest,
    ) -> Result<u64, MultiscaleRetouchServiceError>;

    fn cancel(&mut self, generation: u64, job: u64) -> Result<(), MultiscaleRetouchServiceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_rejects_unbounded_values() {
        assert!(MultiscaleProgress::new(0, 0).is_none());
        assert!(MultiscaleProgress::new(2, 1).is_none());
        assert!(
            (MultiscaleProgress::new(1, 4).expect("progress").fraction() - 0.25).abs()
                < f64::EPSILON
        );
    }

    #[test]
    fn band_order_is_stable_and_includes_lifecycle_endpoints() {
        assert_eq!(MultiscaleBand::all()[0], MultiscaleBand::Original);
        assert_eq!(
            MultiscaleBand::all().last(),
            Some(&MultiscaleBand::Residual)
        );
    }
}
