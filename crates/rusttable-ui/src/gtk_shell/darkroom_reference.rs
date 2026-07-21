//! Reference darkroom operation rows for controls not yet backed by processing.

use rusttable_core::Revision;

use super::{
    DarkroomModuleError, DarkroomModuleSide, DarkroomModuleViewModel, DarkroomModulesViewModel,
};
use crate::presentation::darkroom_controls::DarkroomControlError;
use crate::presentation::darkroom_controls::DarkroomControlViewModel;

/// Whether a visible operation row can currently dispatch to the backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomModuleAvailability {
    Supported,
    Unsupported { reason: String },
}

impl DarkroomModuleAvailability {
    #[must_use]
    pub const fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }
}

fn unsupported(
    id: &str,
    title: &str,
    controls: Vec<DarkroomControlViewModel>,
) -> Result<DarkroomModuleViewModel, DarkroomModuleError> {
    DarkroomModuleViewModel::new(
        id,
        title,
        DarkroomModuleSide::Right,
        false,
        false,
        false,
        Revision::from_u64(0),
        controls,
    )
    .map(|module| {
        module.with_availability(DarkroomModuleAvailability::Unsupported {
            reason: "processing backend is not connected yet".to_owned(),
        })
    })
    .map_err(|error| DarkroomModuleError::Control(DarkroomControlError::Validation(error)))
}

/// Provides the next Darktable operation rows while their backend is explicit.
pub fn reference_modules() -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    let temperature = unsupported(
        "temperature",
        "Temperature",
        vec![
            DarkroomControlViewModel::slider(
                "temperature-kelvin",
                "Temperature",
                2000.0,
                20000.0,
                1.0,
                6500.0,
                6500.0,
            )
            .map_err(|error| {
                DarkroomModuleError::Control(DarkroomControlError::Validation(error))
            })?,
            DarkroomControlViewModel::slider("temperature-tint", "Tint", -1.0, 1.0, 0.01, 0.0, 0.0)
                .map_err(|error| {
                    DarkroomModuleError::Control(DarkroomControlError::Validation(error))
                })?,
        ],
    )?;
    let crop = unsupported(
        "crop",
        "Crop",
        vec![
            DarkroomControlViewModel::toggle("crop-locked", "Lock aspect ratio", false, false)
                .map_err(|error| {
                    DarkroomModuleError::Control(DarkroomControlError::Validation(error))
                })?,
            DarkroomControlViewModel::choice(
                "crop-orientation",
                "Orientation",
                ["portrait", "landscape", "free"],
                2,
            )
            .map_err(|error| {
                DarkroomModuleError::Control(DarkroomControlError::Validation(error))
            })?,
        ],
    )?;
    DarkroomModulesViewModel::new(vec![temperature, crop])
}
