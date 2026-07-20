//! Typed state for Darktable's exposure image-operation module.
//!
//! The bounds and defaults mirror Darktable/src/iop/exposure.c. This module
//! describes module state and user actions only; pixel-pipe execution is a
//! separate migration slice.

/// Minimum persisted exposure parameter, in EV.
pub const EXPOSURE_EV_MINIMUM: f64 = -18.0;
/// Maximum persisted exposure parameter, in EV.
pub const EXPOSURE_EV_MAXIMUM: f64 = 18.0;
/// Minimum manual-mode exposure control range, in EV.
pub const EXPOSURE_EV_SOFT_MINIMUM: f64 = -3.0;
/// Maximum manual-mode exposure control range, in EV.
pub const EXPOSURE_EV_SOFT_MAXIMUM: f64 = 4.0;
/// Minimum persisted black-level parameter.
pub const BLACK_LEVEL_MINIMUM: f64 = -1.0;
/// Maximum persisted black-level parameter.
pub const BLACK_LEVEL_MAXIMUM: f64 = 1.0;
/// Minimum black-level control range used by Darktable's manual UI.
pub const BLACK_LEVEL_SOFT_MINIMUM: f64 = -0.1;
/// Maximum black-level control range used by Darktable's manual UI.
pub const BLACK_LEVEL_SOFT_MAXIMUM: f64 = 0.1;
/// Default exposure value, in EV.
pub const DEFAULT_EXPOSURE_EV: f64 = 0.0;
/// Default black-level value.
pub const DEFAULT_BLACK_LEVEL: f64 = 0.0;

/// Exposure module operating mode from Darktable's parameter schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExposureMode {
    /// Directly expose the image using the manual controls.
    #[default]
    Manual,
    /// Compute exposure from the source histogram.
    Automatic,
}

/// Complete typed state for one Exposure IOP instance.
#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the independent booleans mirror Darktable's Exposure parameter schema"
)]
pub struct ExposureModuleState {
    enabled: bool,
    expanded: bool,
    mode: ExposureMode,
    exposure_ev: f64,
    black_level: f64,
    compensate_exposure_bias: bool,
    compensate_highlight_preservation: bool,
}

impl Default for ExposureModuleState {
    fn default() -> Self {
        Self {
            enabled: true,
            expanded: true,
            mode: ExposureMode::Manual,
            exposure_ev: DEFAULT_EXPOSURE_EV,
            black_level: DEFAULT_BLACK_LEVEL,
            compensate_exposure_bias: false,
            compensate_highlight_preservation: true,
        }
    }
}

impl ExposureModuleState {
    /// Returns whether the module participates in processing.
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }

    /// Returns whether the module panel is expanded.
    #[must_use]
    pub const fn expanded(self) -> bool {
        self.expanded
    }

    /// Returns the selected operating mode.
    #[must_use]
    pub const fn mode(self) -> ExposureMode {
        self.mode
    }

    /// Returns the exposure adjustment in EV.
    #[must_use]
    pub const fn exposure_ev(self) -> f64 {
        self.exposure_ev
    }

    /// Returns the black-level correction.
    #[must_use]
    pub const fn black_level(self) -> f64 {
        self.black_level
    }

    /// Returns whether camera exposure bias compensation is enabled.
    #[must_use]
    pub const fn compensate_exposure_bias(self) -> bool {
        self.compensate_exposure_bias
    }

    /// Returns whether highlight-preservation compensation is enabled.
    #[must_use]
    pub const fn compensate_highlight_preservation(self) -> bool {
        self.compensate_highlight_preservation
    }

    /// Applies one explicit user action to the module state.
    ///
    /// # Errors
    ///
    /// Returns an error when a numeric action is non-finite or outside the
    /// persisted Darktable parameter bounds. The state is unchanged on error.
    pub fn apply(&mut self, action: ExposureAction) -> Result<(), ExposureActionError> {
        match action {
            ExposureAction::SetEnabled(value) => self.enabled = value,
            ExposureAction::SetExpanded(value) => self.expanded = value,
            ExposureAction::SetMode(value) => self.mode = value,
            ExposureAction::SetExposureEv(value) => {
                validate_range(value, EXPOSURE_EV_MINIMUM, EXPOSURE_EV_MAXIMUM, |value| {
                    ExposureActionError::ExposureEvOutOfRange { value }
                })?;
                self.exposure_ev = value;
            }
            ExposureAction::SetBlackLevel(value) => {
                validate_range(value, BLACK_LEVEL_MINIMUM, BLACK_LEVEL_MAXIMUM, |value| {
                    ExposureActionError::BlackLevelOutOfRange { value }
                })?;
                self.black_level = value;
            }
            ExposureAction::SetCompensateExposureBias(value) => {
                self.compensate_exposure_bias = value;
            }
            ExposureAction::SetCompensateHighlightPreservation(value) => {
                self.compensate_highlight_preservation = value;
            }
            ExposureAction::Reset => *self = Self::default(),
        }
        Ok(())
    }
}

/// Explicit user actions supported by the Exposure module panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExposureAction {
    /// Enable or disable the module.
    SetEnabled(bool),
    /// Expand or collapse the module panel.
    SetExpanded(bool),
    /// Select manual or automatic exposure mode.
    SetMode(ExposureMode),
    /// Set the exposure adjustment in EV.
    SetExposureEv(f64),
    /// Set the black-level correction.
    SetBlackLevel(f64),
    /// Enable or disable camera exposure bias compensation.
    SetCompensateExposureBias(bool),
    /// Enable or disable highlight-preservation compensation.
    SetCompensateHighlightPreservation(bool),
    /// Restore the Darktable Exposure defaults.
    Reset,
}

/// Rejected numeric action values for the Exposure module.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExposureActionError {
    /// The exposure EV was non-finite or outside [-18, 18].
    ExposureEvOutOfRange { value: f64 },
    /// The black level was non-finite or outside [-1, 1].
    BlackLevelOutOfRange { value: f64 },
}

impl std::fmt::Display for ExposureActionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExposureEvOutOfRange { value } => {
                write!(formatter, "exposure EV {value} is outside [-18, 18]")
            }
            Self::BlackLevelOutOfRange { value } => {
                write!(formatter, "black level {value} is outside [-1, 1]")
            }
        }
    }
}

impl std::error::Error for ExposureActionError {}

fn validate_range(
    value: f64,
    minimum: f64,
    maximum: f64,
    error: impl FnOnce(f64) -> ExposureActionError,
) -> Result<(), ExposureActionError> {
    if value.is_finite() && (minimum..=maximum).contains(&value) {
        Ok(())
    } else {
        Err(error(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_darktable_exposure_schema() {
        let state = ExposureModuleState::default();

        assert!(state.enabled());
        assert!(state.expanded());
        assert_eq!(state.mode(), ExposureMode::Manual);
        assert_close(state.exposure_ev(), 0.0);
        assert_close(state.black_level(), 0.0);
        assert!(!state.compensate_exposure_bias());
        assert!(state.compensate_highlight_preservation());
    }

    #[test]
    fn actions_update_each_independent_parameter() {
        let mut state = ExposureModuleState::default();
        for action in [
            ExposureAction::SetEnabled(false),
            ExposureAction::SetExpanded(false),
            ExposureAction::SetMode(ExposureMode::Automatic),
            ExposureAction::SetExposureEv(2.5),
            ExposureAction::SetBlackLevel(-0.05),
            ExposureAction::SetCompensateExposureBias(true),
            ExposureAction::SetCompensateHighlightPreservation(false),
        ] {
            state.apply(action).expect("action is in range");
        }

        assert!(!state.enabled());
        assert!(!state.expanded());
        assert_eq!(state.mode(), ExposureMode::Automatic);
        assert_close(state.exposure_ev(), 2.5);
        assert_close(state.black_level(), -0.05);
        assert!(state.compensate_exposure_bias());
        assert!(!state.compensate_highlight_preservation());
    }

    #[test]
    fn numeric_actions_reject_non_finite_and_out_of_range_values() {
        let mut state = ExposureModuleState::default();

        assert!(matches!(
            state.apply(ExposureAction::SetExposureEv(f64::NAN)),
            Err(ExposureActionError::ExposureEvOutOfRange { .. })
        ));
        assert!(matches!(
            state.apply(ExposureAction::SetExposureEv(18.1)),
            Err(ExposureActionError::ExposureEvOutOfRange { .. })
        ));
        assert!(matches!(
            state.apply(ExposureAction::SetBlackLevel(f64::INFINITY)),
            Err(ExposureActionError::BlackLevelOutOfRange { .. })
        ));
        assert!(matches!(
            state.apply(ExposureAction::SetBlackLevel(-1.1)),
            Err(ExposureActionError::BlackLevelOutOfRange { .. })
        ));
        assert_eq!(state, ExposureModuleState::default());
    }

    #[test]
    fn reset_restores_all_defaults() {
        let mut state = ExposureModuleState::default();
        state
            .apply(ExposureAction::SetExposureEv(-4.0))
            .expect("value is in range");
        state
            .apply(ExposureAction::SetBlackLevel(0.5))
            .expect("value is in range");
        state
            .apply(ExposureAction::SetEnabled(false))
            .expect("boolean action cannot fail");

        state
            .apply(ExposureAction::Reset)
            .expect("reset cannot fail");

        assert_eq!(state, ExposureModuleState::default());
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }
}
