//! GTK-independent editor state for Darktable `src/iop/bloom.c` parameters.

use rusttable_processing::operations::bloom::{
    BloomConfig, BloomParameterError, BloomParametersV1,
};

/// One source Bloom parameter exposed by the native editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BloomParameter {
    Threshold,
    Size,
    Strength,
}

impl BloomParameter {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Threshold => "threshold",
            Self::Size => "size",
            Self::Strength => "strength",
        }
    }
}

/// Validated, GTK-independent state for the three ordinary Bloom controls.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomEditorState {
    parameters: BloomParametersV1,
}

impl BloomEditorState {
    /// Creates editor state from persisted version-one parameters.
    ///
    /// # Errors
    ///
    /// Returns the processing contract's parameter error for a non-finite or
    /// out-of-range value.
    pub fn new(parameters: BloomParametersV1) -> Result<Self, BloomParameterError> {
        BloomConfig::try_from(parameters)?;
        Ok(Self { parameters })
    }

    #[must_use]
    pub const fn parameters(self) -> BloomParametersV1 {
        self.parameters
    }

    #[must_use]
    pub const fn threshold(self) -> f32 {
        self.parameters.threshold
    }

    #[must_use]
    pub const fn size(self) -> f32 {
        self.parameters.size
    }

    #[must_use]
    pub const fn strength(self) -> f32 {
        self.parameters.strength
    }

    #[must_use]
    pub const fn value(self, parameter: BloomParameter) -> f32 {
        match parameter {
            BloomParameter::Threshold => self.threshold(),
            BloomParameter::Size => self.size(),
            BloomParameter::Strength => self.strength(),
        }
    }

    /// Replaces one parameter while preserving the other two exactly.
    ///
    /// # Errors
    ///
    /// Returns the processing contract's parameter error for a non-finite or
    /// out-of-range value, leaving this state unchanged.
    pub fn set(
        &mut self,
        parameter: BloomParameter,
        value: f32,
    ) -> Result<bool, BloomParameterError> {
        let mut candidate = self.parameters;
        match parameter {
            BloomParameter::Threshold => candidate.threshold = value,
            BloomParameter::Size => candidate.size = value,
            BloomParameter::Strength => candidate.strength = value,
        }
        BloomConfig::try_from(candidate)?;
        let changed = candidate != self.parameters;
        self.parameters = candidate;
        Ok(changed)
    }

    /// Restores the source defaults and reports whether state changed.
    pub fn reset(&mut self) -> bool {
        let defaults = BloomParametersV1::defaults();
        let changed = self.parameters != defaults;
        self.parameters = defaults;
        changed
    }
}

impl Default for BloomEditorState {
    fn default() -> Self {
        Self {
            parameters: BloomParametersV1::defaults(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_preserve_native_values_and_payload_layout() {
        let editor = BloomEditorState::default();
        assert_eq!(editor.size().to_bits(), 20.0_f32.to_bits());
        assert_eq!(editor.threshold().to_bits(), 90.0_f32.to_bits());
        assert_eq!(editor.strength().to_bits(), 25.0_f32.to_bits());
        assert_eq!(
            editor.parameters().to_bytes(),
            BloomParametersV1::new(20.0, 90.0, 25.0).to_bytes()
        );
    }

    #[test]
    fn one_editor_change_preserves_the_other_parameters() {
        let mut editor = BloomEditorState::default();
        assert!(editor.set(BloomParameter::Threshold, 42.0).unwrap());
        assert_eq!(
            editor.parameters(),
            BloomParametersV1::new(20.0, 42.0, 25.0)
        );
        assert!(!editor.set(BloomParameter::Threshold, 42.0).unwrap());

        assert!(editor.set(BloomParameter::Size, 64.0).unwrap());
        assert_eq!(
            editor.parameters(),
            BloomParametersV1::new(64.0, 42.0, 25.0)
        );

        assert!(editor.set(BloomParameter::Strength, 11.0).unwrap());
        assert_eq!(
            editor.parameters(),
            BloomParametersV1::new(64.0, 42.0, 11.0)
        );
    }

    #[test]
    fn invalid_edits_are_rejected_without_mutation() {
        let mut editor = BloomEditorState::default();
        let before = editor;
        assert!(editor.set(BloomParameter::Size, f32::NAN).is_err());
        assert_eq!(editor, before);
        assert!(editor.set(BloomParameter::Threshold, -1.0).is_err());
        assert_eq!(editor, before);
        assert!(editor.set(BloomParameter::Strength, 101.0).is_err());
        assert_eq!(editor, before);
    }

    #[test]
    fn reset_restores_all_source_defaults_once() {
        let mut editor = BloomEditorState::new(BloomParametersV1::new(1.0, 2.0, 3.0)).unwrap();
        assert!(editor.reset());
        assert_eq!(editor, BloomEditorState::default());
        assert!(!editor.reset());
    }
}
