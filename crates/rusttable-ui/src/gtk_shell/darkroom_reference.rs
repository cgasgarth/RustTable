//! Registry-backed darkroom operation rows and explicit unavailable states.

use super::{DarkroomModuleError, DarkroomModulesViewModel};

#[path = "darkroom_registry_modules.rs"]
mod registry_modules;

/// Whether a visible operation row can currently dispatch to the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomModuleAvailability {
    Supported,
    Deprecated { reason: String },
    Unsupported { reason: String },
}

impl DarkroomModuleAvailability {
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        matches!(self, Self::Supported | Self::Deprecated { .. })
    }

    #[must_use]
    pub const fn is_deprecated(&self) -> bool {
        matches!(self, Self::Deprecated { .. })
    }
}

/// Provides the next Darktable operation rows while their backend is explicit.
pub fn reference_modules() -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    registry_modules::modules_from_registry()
}
