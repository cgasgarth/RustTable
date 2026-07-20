//! Display-ready projection for the Darktable Exposure module.

use rusttable_processing::{ExposureAction, ExposureMode, ExposureModuleState};

/// GTK-independent state used to render one Exposure module panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExposurePanelViewModel {
    state: ExposureModuleState,
}

impl ExposurePanelViewModel {
    /// Projects the processing-domain state without changing any parameters.
    #[must_use]
    pub const fn from_state(state: ExposureModuleState) -> Self {
        Self { state }
    }

    /// Returns the current processing-domain state.
    #[must_use]
    pub const fn state(self) -> ExposureModuleState {
        self.state
    }

    /// Returns the module's enabled presentation state.
    #[must_use]
    pub const fn enabled(self) -> bool {
        self.state.enabled()
    }

    /// Returns the expander's presentation state.
    #[must_use]
    pub const fn expanded(self) -> bool {
        self.state.expanded()
    }

    /// Returns the active mode.
    #[must_use]
    pub const fn mode(self) -> ExposureMode {
        self.state.mode()
    }

    /// Returns the exposure adjustment in EV.
    #[must_use]
    pub const fn exposure_ev(self) -> f64 {
        self.state.exposure_ev()
    }

    /// Returns the black-level correction.
    #[must_use]
    pub const fn black_level(self) -> f64 {
        self.state.black_level()
    }

    /// Returns whether camera exposure bias compensation is enabled.
    #[must_use]
    pub const fn compensate_exposure_bias(self) -> bool {
        self.state.compensate_exposure_bias()
    }

    /// Returns whether highlight-preservation compensation is enabled.
    #[must_use]
    pub const fn compensate_highlight_preservation(self) -> bool {
        self.state.compensate_highlight_preservation()
    }

    /// Returns a stable label for the selected mode.
    #[must_use]
    pub const fn mode_label(self) -> &'static str {
        match self.mode() {
            ExposureMode::Manual => "manual",
            ExposureMode::Automatic => "automatic",
        }
    }

    /// Returns a compact EV label matching Darktable's control formatting.
    #[must_use]
    pub fn exposure_label(self) -> String {
        format!("{:.3} EV", self.exposure_ev())
    }

    /// Returns a compact black-level label for accessibility and tests.
    #[must_use]
    pub fn black_level_label(self) -> String {
        format!("{:.4}", self.black_level())
    }

    /// Applies a user action and returns the next display projection.
    ///
    /// # Errors
    ///
    /// Returns the domain error when the action contains an invalid numeric
    /// value.
    pub fn apply(
        self,
        action: ExposureAction,
    ) -> Result<Self, rusttable_processing::ExposureActionError> {
        let mut state = self.state;
        state.apply(action)?;
        Ok(Self::from_state(state))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_preserves_domain_state_and_labels() {
        let state = ExposureModuleState::default();
        let view = ExposurePanelViewModel::from_state(state);

        assert_eq!(view.state(), state);
        assert_eq!(view.mode_label(), "manual");
        assert_eq!(view.exposure_label(), "0.000 EV");
        assert_eq!(view.black_level_label(), "0.0000");
    }

    #[test]
    fn projection_applies_explicit_action_without_rgb_gain_mapping() {
        let view = ExposurePanelViewModel::from_state(ExposureModuleState::default());
        let updated = view
            .apply(ExposureAction::SetBlackLevel(-0.025))
            .expect("value is in range");

        assert_close(updated.black_level(), -0.025);
        assert_close(updated.exposure_ev(), 0.0);
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }
}
