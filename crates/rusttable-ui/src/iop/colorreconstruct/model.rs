//! GTK-independent editor state for Darktable
//! `src/iop/colorreconstruction.c::{gui_changed,commit_params,gui_update}`.

use rusttable_processing::operations::colorreconstruction::{
    ColorReconstructionParameterError, ColorReconstructionPrecedence, ColorReconstructionV3,
};

/// One scalar Color Reconstruction parameter exposed by the native editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorReconstructionParameter {
    Threshold,
    Spatial,
    Range,
    Hue,
}

impl ColorReconstructionParameter {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Threshold => "threshold",
            Self::Spatial => "spatial",
            Self::Range => "range",
            Self::Hue => "hue",
        }
    }
}

/// Validated GTK-independent state for the five native parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionEditorState {
    parameters: ColorReconstructionV3,
}

impl ColorReconstructionEditorState {
    /// Exact version-three native defaults.
    pub const DEFAULT_PARAMETERS: ColorReconstructionV3 = ColorReconstructionV3 {
        threshold: 100.0,
        spatial: 400.0,
        range: 10.0,
        hue: 0.66,
        precedence: 0,
    };

    /// Creates editor state from persisted version-three parameters.
    ///
    /// # Errors
    ///
    /// Returns the processing contract's error for a non-finite or unknown value.
    pub fn new(
        parameters: ColorReconstructionV3,
    ) -> Result<Self, ColorReconstructionParameterError> {
        parameters.config()?;
        Ok(Self { parameters })
    }

    #[must_use]
    pub const fn parameters(self) -> ColorReconstructionV3 {
        self.parameters
    }

    #[must_use]
    pub const fn threshold(self) -> f32 {
        self.parameters.threshold
    }

    #[must_use]
    pub const fn spatial(self) -> f32 {
        self.parameters.spatial
    }

    #[must_use]
    pub const fn range(self) -> f32 {
        self.parameters.range
    }

    #[must_use]
    pub const fn hue(self) -> f32 {
        self.parameters.hue
    }

    /// Returns the validated precedence. Construction proves this conversion.
    ///
    /// # Panics
    ///
    /// Panics if the editor state contains an unvalidated precedence identifier.
    #[must_use]
    pub fn precedence(self) -> ColorReconstructionPrecedence {
        ColorReconstructionPrecedence::from_id(self.parameters.precedence)
            .expect("validated editor precedence")
    }

    /// Mirrors `gui_changed`: hue is shown only for hue precedence.
    #[must_use]
    pub fn hue_control_visible(self) -> bool {
        self.precedence() == ColorReconstructionPrecedence::Hue
    }

    #[must_use]
    pub const fn value(self, parameter: ColorReconstructionParameter) -> f32 {
        match parameter {
            ColorReconstructionParameter::Threshold => self.threshold(),
            ColorReconstructionParameter::Spatial => self.spatial(),
            ColorReconstructionParameter::Range => self.range(),
            ColorReconstructionParameter::Hue => self.hue(),
        }
    }

    /// Replaces one scalar while preserving every other parameter exactly.
    ///
    /// # Errors
    ///
    /// Returns the processing contract's error for an invalid candidate and
    /// leaves this state unchanged.
    pub fn set(
        &mut self,
        parameter: ColorReconstructionParameter,
        value: f32,
    ) -> Result<bool, ColorReconstructionParameterError> {
        let mut candidate = self.parameters;
        match parameter {
            ColorReconstructionParameter::Threshold => candidate.threshold = value,
            ColorReconstructionParameter::Spatial => candidate.spatial = value,
            ColorReconstructionParameter::Range => candidate.range = value,
            ColorReconstructionParameter::Hue => candidate.hue = value,
        }
        Self::replace_if_valid(&mut self.parameters, candidate)
    }

    /// Replaces the source enum and reports whether persisted state changed.
    ///
    /// # Errors
    ///
    /// Returns the processing contract's error if the complete candidate is no
    /// longer valid and leaves this state unchanged.
    pub fn set_precedence(
        &mut self,
        precedence: ColorReconstructionPrecedence,
    ) -> Result<bool, ColorReconstructionParameterError> {
        let mut candidate = self.parameters;
        candidate.precedence = precedence.id();
        Self::replace_if_valid(&mut self.parameters, candidate)
    }

    fn replace_if_valid(
        current: &mut ColorReconstructionV3,
        candidate: ColorReconstructionV3,
    ) -> Result<bool, ColorReconstructionParameterError> {
        candidate.config()?;
        let changed = candidate != *current;
        *current = candidate;
        Ok(changed)
    }

    /// Restores all source defaults and reports whether state changed.
    pub fn reset(&mut self) -> bool {
        let changed = self.parameters != Self::DEFAULT_PARAMETERS;
        self.parameters = Self::DEFAULT_PARAMETERS;
        changed
    }
}

impl Default for ColorReconstructionEditorState {
    fn default() -> Self {
        Self {
            parameters: Self::DEFAULT_PARAMETERS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_native_values_and_enum_identity() {
        let editor = ColorReconstructionEditorState::default();
        assert_eq!(editor.threshold().to_bits(), 100.0_f32.to_bits());
        assert_eq!(editor.spatial().to_bits(), 400.0_f32.to_bits());
        assert_eq!(editor.range().to_bits(), 10.0_f32.to_bits());
        assert_eq!(editor.hue().to_bits(), 0.66_f32.to_bits());
        assert_eq!(editor.precedence(), ColorReconstructionPrecedence::None);
        assert!(!editor.hue_control_visible());
        assert_eq!(
            editor.parameters(),
            ColorReconstructionEditorState::DEFAULT_PARAMETERS
        );
    }

    #[test]
    fn one_scalar_change_preserves_every_other_parameter() {
        let mut editor = ColorReconstructionEditorState::default();
        assert!(
            editor
                .set(ColorReconstructionParameter::Threshold, 125.0)
                .unwrap()
        );
        assert_eq!(
            editor.parameters(),
            ColorReconstructionV3 {
                threshold: 125.0,
                ..ColorReconstructionEditorState::DEFAULT_PARAMETERS
            }
        );
        assert!(
            !editor
                .set(ColorReconstructionParameter::Threshold, 125.0)
                .unwrap()
        );

        assert!(
            editor
                .set(ColorReconstructionParameter::Spatial, 1.0)
                .unwrap()
        );
        assert!(
            editor
                .set(ColorReconstructionParameter::Range, 50.0)
                .unwrap()
        );
        assert!(editor.set(ColorReconstructionParameter::Hue, 1.0).unwrap());
        assert_eq!(editor.threshold().to_bits(), 125.0_f32.to_bits());
        assert_eq!(editor.spatial().to_bits(), 1.0_f32.to_bits());
        assert_eq!(editor.range().to_bits(), 50.0_f32.to_bits());
        assert_eq!(editor.hue().to_bits(), 1.0_f32.to_bits());
        assert_eq!(editor.precedence(), ColorReconstructionPrecedence::None);
    }

    #[test]
    fn precedence_state_transitions_preserve_scalars() {
        let mut editor = ColorReconstructionEditorState::default();
        assert!(
            editor
                .set_precedence(ColorReconstructionPrecedence::Hue)
                .unwrap()
        );
        assert_eq!(editor.precedence(), ColorReconstructionPrecedence::Hue);
        assert!(editor.hue_control_visible());
        assert_eq!(editor.hue().to_bits(), 0.66_f32.to_bits());
        assert!(
            !editor
                .set_precedence(ColorReconstructionPrecedence::Hue)
                .unwrap()
        );
        assert!(
            editor
                .set_precedence(ColorReconstructionPrecedence::Chroma)
                .unwrap()
        );
        assert_eq!(editor.precedence(), ColorReconstructionPrecedence::Chroma);
        assert!(!editor.hue_control_visible());
    }

    #[test]
    fn invalid_edits_are_rejected_without_mutation() {
        let invalid = ColorReconstructionV3 {
            precedence: 99,
            ..ColorReconstructionEditorState::DEFAULT_PARAMETERS
        };
        assert!(ColorReconstructionEditorState::new(invalid).is_err());

        let mut editor = ColorReconstructionEditorState::default();
        let before = editor;
        assert!(
            editor
                .set(ColorReconstructionParameter::Threshold, f32::NAN)
                .is_err()
        );
        assert_eq!(editor, before);
        assert!(
            editor
                .set(ColorReconstructionParameter::Spatial, 1000.1)
                .unwrap()
        );
        assert_eq!(editor.spatial().to_bits(), 1000.1_f32.to_bits());
        let before = editor;
        assert!(
            editor
                .set(ColorReconstructionParameter::Range, f32::INFINITY)
                .is_err()
        );
        assert_eq!(editor, before);
        assert!(
            editor
                .set(ColorReconstructionParameter::Hue, f32::NAN)
                .is_err()
        );
        assert_eq!(editor, before);
    }

    #[test]
    fn reset_restores_all_source_defaults_once() {
        let mut editor = ColorReconstructionEditorState::new(ColorReconstructionV3 {
            threshold: 75.0,
            spatial: 25.0,
            range: 30.0,
            hue: 0.25,
            precedence: ColorReconstructionPrecedence::Hue.id(),
        })
        .unwrap();
        assert!(editor.reset());
        assert_eq!(editor, ColorReconstructionEditorState::default());
        assert!(!editor.reset());
    }
}
