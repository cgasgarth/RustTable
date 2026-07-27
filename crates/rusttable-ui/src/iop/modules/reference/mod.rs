//! Registry-backed darkroom operation rows and explicit unavailable states.

use super::{DarkroomModuleError, DarkroomModulesViewModel};

mod registry;

/// Whether a visible operation row can currently dispatch to the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomModuleAvailability {
    Supported,
    PartiallySupported {
        reason: String,
        deferred_responsibilities: Vec<String>,
    },
    Deprecated {
        reason: String,
    },
    DeprecatedUnavailable {
        reason: String,
    },
    Unsupported {
        reason: String,
    },
}

impl DarkroomModuleAvailability {
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        matches!(
            self,
            Self::Supported | Self::PartiallySupported { .. } | Self::Deprecated { .. }
        )
    }

    #[must_use]
    pub const fn is_fully_supported(&self) -> bool {
        matches!(self, Self::Supported | Self::Deprecated { .. })
    }

    #[must_use]
    pub const fn is_partial(&self) -> bool {
        matches!(self, Self::PartiallySupported { .. })
    }

    #[must_use]
    pub const fn is_deprecated(&self) -> bool {
        matches!(
            self,
            Self::Deprecated { .. } | Self::DeprecatedUnavailable { .. }
        )
    }

    #[must_use]
    pub const fn is_unsupported(&self) -> bool {
        matches!(
            self,
            Self::DeprecatedUnavailable { .. } | Self::Unsupported { .. }
        )
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Supported => None,
            Self::PartiallySupported { reason, .. }
            | Self::Deprecated { reason }
            | Self::DeprecatedUnavailable { reason }
            | Self::Unsupported { reason } => Some(reason),
        }
    }

    #[must_use]
    pub fn deferred_responsibilities(&self) -> &[String] {
        match self {
            Self::PartiallySupported {
                deferred_responsibilities,
                ..
            } => deferred_responsibilities,
            Self::Supported
            | Self::Deprecated { .. }
            | Self::DeprecatedUnavailable { .. }
            | Self::Unsupported { .. } => &[],
        }
    }
}

/// Provides the next Darktable operation rows while their backend is explicit.
///
/// # Errors
///
/// Returns a typed module error if the registry projection is invalid.
pub fn reference_modules() -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    registry::modules_from_registry()
}
