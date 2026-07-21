//! GTK-independent darkroom control models.
//!
//! The GTK shell consumes these values as a stable snapshot.  Mutations are
//! revision guarded so an old widget callback cannot overwrite a newer edit.

use std::fmt;

use rusttable_core::Revision;

use super::{PresentationText, PresentationTextError};

const MAX_CONTROL_ID_BYTES: usize = 64;

/// Stable identifier for a darkroom control.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ControlId(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlIdError {
    Empty,
    TooLong { byte_length: usize },
    InvalidCharacter { byte_index: usize, value: char },
}

impl fmt::Display for ControlIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("control id is empty"),
            Self::TooLong { byte_length } => {
                write!(formatter, "control id is too long ({byte_length} bytes)")
            }
            Self::InvalidCharacter { byte_index, value } => {
                write!(
                    formatter,
                    "control id has invalid character {value:?} at byte {byte_index}"
                )
            }
        }
    }
}

impl std::error::Error for ControlIdError {}

impl ControlId {
    /// Creates a bounded id suitable for a GTK widget name and callback key.
    ///
    /// # Errors
    ///
    /// Returns an error when the id is empty, too long, or contains a character
    /// that cannot safely identify a GTK control.
    pub fn new(value: impl Into<String>) -> Result<Self, ControlIdError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ControlIdError::Empty);
        }
        if value.len() > MAX_CONTROL_ID_BYTES {
            return Err(ControlIdError::TooLong {
                byte_length: value.len(),
            });
        }
        if let Some((byte_index, value)) = value
            .char_indices()
            .find(|(_, value)| !(value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.')))
        {
            return Err(ControlIdError::InvalidCharacter { byte_index, value });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ControlId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The native GTK4 control shape represented by a darkroom control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarkroomControlKind {
    Slider,
    Choice,
    Toggle,
    Text,
}

/// Typed value carried by one GTK control.
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomControlValue {
    Slider(f64),
    Choice(usize),
    Toggle(bool),
    Text(String),
}

impl DarkroomControlValue {
    #[must_use]
    pub const fn kind(&self) -> DarkroomControlKind {
        match self {
            Self::Slider(_) => DarkroomControlKind::Slider,
            Self::Choice(_) => DarkroomControlKind::Choice,
            Self::Toggle(_) => DarkroomControlKind::Toggle,
            Self::Text(_) => DarkroomControlKind::Text,
        }
    }
}

/// A validated value and range for a Darktable-style slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SliderSpec {
    minimum: f64,
    maximum: f64,
    step: f64,
    value: f64,
    default: f64,
}

impl SliderSpec {
    #[must_use]
    pub const fn minimum(self) -> f64 {
        self.minimum
    }

    #[must_use]
    pub const fn maximum(self) -> f64 {
        self.maximum
    }

    #[must_use]
    pub const fn step(self) -> f64 {
        self.step
    }

    #[must_use]
    pub const fn value(self) -> f64 {
        self.value
    }

    #[must_use]
    pub const fn default_value(self) -> f64 {
        self.default
    }
}

/// A typed control with a stable id and a display-safe label.
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomControlViewModel {
    id: ControlId,
    label: PresentationText,
    kind: DarkroomControlKind,
    value: DarkroomControlValue,
    default: DarkroomControlValue,
    slider: Option<SliderSpec>,
    choices: Vec<PresentationText>,
    text_max_bytes: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlValidationError {
    InvalidId(ControlIdError),
    InvalidLabel(PresentationTextError),
    SliderNonFinite,
    SliderRangeInverted,
    SliderStepNotPositive,
    SliderValueOutOfRange {
        value: f64,
        minimum: f64,
        maximum: f64,
    },
    ChoiceListEmpty,
    ChoiceIndexOutOfRange {
        index: usize,
        choices: usize,
    },
    TextTooLong {
        byte_length: usize,
        maximum: usize,
    },
    ControlValueTypeMismatch {
        expected: DarkroomControlKind,
        actual: DarkroomControlKind,
    },
    DuplicateControlId(ControlId),
}

impl fmt::Display for ControlValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId(error) => error.fmt(formatter),
            Self::InvalidLabel(error) => write!(formatter, "invalid darkroom label: {error:?}"),
            Self::SliderNonFinite => formatter.write_str("slider values must be finite"),
            Self::SliderRangeInverted => formatter.write_str("slider minimum exceeds maximum"),
            Self::SliderStepNotPositive => formatter.write_str("slider step must be positive"),
            Self::SliderValueOutOfRange {
                value,
                minimum,
                maximum,
            } => write!(
                formatter,
                "slider value {value} is outside [{minimum}, {maximum}]"
            ),
            Self::ChoiceListEmpty => {
                formatter.write_str("choice control needs at least one option")
            }
            Self::ChoiceIndexOutOfRange { index, choices } => {
                write!(
                    formatter,
                    "choice index {index} is outside {choices} options"
                )
            }
            Self::TextTooLong {
                byte_length,
                maximum,
            } => write!(
                formatter,
                "text is {byte_length} bytes, maximum is {maximum}"
            ),
            Self::ControlValueTypeMismatch { expected, actual } => {
                write!(
                    formatter,
                    "control expects {expected:?}, received {actual:?}"
                )
            }
            Self::DuplicateControlId(id) => write!(formatter, "duplicate control id {id}"),
        }
    }
}

impl std::error::Error for ControlValidationError {}

impl DarkroomControlViewModel {
    /// Builds a slider whose value and default are checked against its bounds.
    ///
    /// # Errors
    ///
    /// Returns an error when the id, label, or any slider value is invalid.
    pub fn slider(
        id: impl Into<String>,
        label: impl Into<String>,
        minimum: f64,
        maximum: f64,
        step: f64,
        value: f64,
        default: f64,
    ) -> Result<Self, ControlValidationError> {
        let id = ControlId::new(id).map_err(ControlValidationError::InvalidId)?;
        let label = PresentationText::new(label).map_err(ControlValidationError::InvalidLabel)?;
        validate_slider(minimum, maximum, step, value)?;
        validate_slider(minimum, maximum, step, default)?;
        Ok(Self {
            id,
            label,
            kind: DarkroomControlKind::Slider,
            value: DarkroomControlValue::Slider(value),
            default: DarkroomControlValue::Slider(default),
            slider: Some(SliderSpec {
                minimum,
                maximum,
                step,
                value,
                default,
            }),
            choices: Vec::new(),
            text_max_bytes: None,
        })
    }

    /// Builds a choice control.  Choice order is preserved for GTK4 `DropDown`.
    ///
    /// # Errors
    ///
    /// Returns an error when the id, label, choices, or selected index is invalid.
    pub fn choice<I, S>(
        id: impl Into<String>,
        label: impl Into<String>,
        choices: I,
        selected: usize,
    ) -> Result<Self, ControlValidationError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let id = ControlId::new(id).map_err(ControlValidationError::InvalidId)?;
        let label = PresentationText::new(label).map_err(ControlValidationError::InvalidLabel)?;
        let choices = choices
            .into_iter()
            .map(|choice| {
                PresentationText::new(choice.into()).map_err(ControlValidationError::InvalidLabel)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if choices.is_empty() {
            return Err(ControlValidationError::ChoiceListEmpty);
        }
        if selected >= choices.len() {
            return Err(ControlValidationError::ChoiceIndexOutOfRange {
                index: selected,
                choices: choices.len(),
            });
        }
        Ok(Self {
            id,
            label,
            kind: DarkroomControlKind::Choice,
            value: DarkroomControlValue::Choice(selected),
            default: DarkroomControlValue::Choice(selected),
            slider: None,
            choices,
            text_max_bytes: None,
        })
    }

    /// Builds a boolean control backed by a GTK4 `Switch`.
    ///
    /// # Errors
    ///
    /// Returns an error when the id or label is invalid.
    pub fn toggle(
        id: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        default: bool,
    ) -> Result<Self, ControlValidationError> {
        let id = ControlId::new(id).map_err(ControlValidationError::InvalidId)?;
        let label = PresentationText::new(label).map_err(ControlValidationError::InvalidLabel)?;
        Ok(Self {
            id,
            label,
            kind: DarkroomControlKind::Toggle,
            value: DarkroomControlValue::Toggle(active),
            default: DarkroomControlValue::Toggle(default),
            slider: None,
            choices: Vec::new(),
            text_max_bytes: None,
        })
    }

    /// Builds a bounded text control for registry parameters such as color profiles.
    ///
    /// # Errors
    ///
    /// Returns an error when the id, label, or either text value is invalid.
    pub fn text(
        id: impl Into<String>,
        label: impl Into<String>,
        value: impl Into<String>,
        default: impl Into<String>,
        maximum_bytes: usize,
    ) -> Result<Self, ControlValidationError> {
        let id = ControlId::new(id).map_err(ControlValidationError::InvalidId)?;
        let label = PresentationText::new(label).map_err(ControlValidationError::InvalidLabel)?;
        let value = value.into();
        let default = default.into();
        validate_text(&value, maximum_bytes)?;
        validate_text(&default, maximum_bytes)?;
        Ok(Self {
            id,
            label,
            kind: DarkroomControlKind::Text,
            value: DarkroomControlValue::Text(value),
            default: DarkroomControlValue::Text(default),
            slider: None,
            choices: Vec::new(),
            text_max_bytes: Some(maximum_bytes),
        })
    }

    #[must_use]
    pub fn id(&self) -> &ControlId {
        &self.id
    }

    #[must_use]
    pub fn label(&self) -> &PresentationText {
        &self.label
    }

    #[must_use]
    pub const fn kind(&self) -> DarkroomControlKind {
        self.kind
    }

    #[must_use]
    pub fn value(&self) -> DarkroomControlValue {
        self.value.clone()
    }

    #[must_use]
    pub fn default_value(&self) -> DarkroomControlValue {
        self.default.clone()
    }

    #[must_use]
    pub const fn slider_spec(&self) -> Option<SliderSpec> {
        self.slider
    }

    #[must_use]
    pub fn choices(&self) -> impl ExactSizeIterator<Item = &PresentationText> {
        self.choices.iter()
    }

    /// Sets a value after checking its typed shape and range.
    ///
    /// # Errors
    ///
    /// Returns an error when the value has the wrong type or is outside its
    /// control's validated range.
    ///
    /// # Panics
    ///
    /// Panics only if an internally inconsistent slider omits its slider metadata.
    pub fn set_value(
        &mut self,
        value: DarkroomControlValue,
    ) -> Result<bool, ControlValidationError> {
        if value.kind() != self.kind {
            return Err(ControlValidationError::ControlValueTypeMismatch {
                expected: self.kind,
                actual: value.kind(),
            });
        }
        if let DarkroomControlValue::Slider(value) = &value {
            let slider = self.slider.expect("slider controls carry slider metadata");
            validate_slider(slider.minimum, slider.maximum, slider.step, *value)?;
        }
        if let DarkroomControlValue::Choice(index) = &value
            && *index >= self.choices.len()
        {
            return Err(ControlValidationError::ChoiceIndexOutOfRange {
                index: *index,
                choices: self.choices.len(),
            });
        }
        if let DarkroomControlValue::Text(value) = &value {
            validate_text(
                value,
                self.text_max_bytes
                    .expect("text controls carry a byte bound"),
            )?;
        }
        let changed = self.value != value;
        self.value = value;
        if let Some(slider) = &mut self.slider {
            slider.value = match &self.value {
                DarkroomControlValue::Slider(value) => *value,
                _ => slider.value,
            };
        }
        Ok(changed)
    }

    /// Restores the control's construction-time default.
    pub fn reset(&mut self) -> bool {
        let changed = self.value != self.default;
        self.value = self.default.clone();
        if let Some(slider) = &mut self.slider {
            slider.value = slider.default;
        }
        changed
    }
}

fn validate_slider(
    minimum: f64,
    maximum: f64,
    step: f64,
    value: f64,
) -> Result<(), ControlValidationError> {
    if !minimum.is_finite() || !maximum.is_finite() || !step.is_finite() || !value.is_finite() {
        return Err(ControlValidationError::SliderNonFinite);
    }
    if minimum > maximum {
        return Err(ControlValidationError::SliderRangeInverted);
    }
    if step <= 0.0 {
        return Err(ControlValidationError::SliderStepNotPositive);
    }
    if !(minimum..=maximum).contains(&value) {
        return Err(ControlValidationError::SliderValueOutOfRange {
            value,
            minimum,
            maximum,
        });
    }
    Ok(())
}

fn validate_text(value: &str, maximum: usize) -> Result<(), ControlValidationError> {
    if value.len() > maximum {
        return Err(ControlValidationError::TextTooLong {
            byte_length: value.len(),
            maximum,
        });
    }
    Ok(())
}

/// Why a revision-guarded control operation was rejected.
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomControlError {
    UnknownControl(ControlId),
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    Validation(ControlValidationError),
    RevisionOverflow,
}

/// Last-known status of a control snapshot.
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomControlsStatus {
    Ready,
    Stale {
        expected: Revision,
        actual: Revision,
    },
    Error(DarkroomControlError),
}

/// Revisioned, typed controls for one darkroom module.
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomControlsViewModel {
    revision: Revision,
    controls: Vec<DarkroomControlViewModel>,
    status: DarkroomControlsStatus,
}

impl DarkroomControlsViewModel {
    /// Builds a control snapshot while rejecting duplicate ids.
    ///
    /// # Errors
    ///
    /// Returns an error when two controls share an id.
    pub fn new(
        revision: Revision,
        controls: Vec<DarkroomControlViewModel>,
    ) -> Result<Self, ControlValidationError> {
        for (index, control) in controls.iter().enumerate() {
            if controls[..index]
                .iter()
                .any(|previous| previous.id() == control.id())
            {
                return Err(ControlValidationError::DuplicateControlId(
                    control.id().clone(),
                ));
            }
        }
        Ok(Self {
            revision,
            controls,
            status: DarkroomControlsStatus::Ready,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn status(&self) -> &DarkroomControlsStatus {
        &self.status
    }

    #[must_use = "iterate over controls in deterministic insertion order"]
    pub fn controls(&self) -> impl ExactSizeIterator<Item = &DarkroomControlViewModel> {
        self.controls.iter()
    }

    #[must_use]
    pub fn control(&self, id: &str) -> Option<&DarkroomControlViewModel> {
        self.controls
            .iter()
            .find(|control| control.id().as_str() == id)
    }

    /// Applies a typed edit only when the caller still owns this revision.
    ///
    /// # Errors
    ///
    /// Returns an error when the revision is stale, the id is unknown, or the
    /// value fails validation.
    pub fn set_value(
        &mut self,
        expected_revision: Revision,
        id: &str,
        value: DarkroomControlValue,
    ) -> Result<Revision, DarkroomControlError> {
        self.check_revision(expected_revision)?;
        let Some(index) = self
            .controls
            .iter()
            .position(|control| control.id().as_str() == id)
        else {
            let error = DarkroomControlError::UnknownControl(unknown_control_id(id));
            return Err(self.record_error(error));
        };
        if let Err(error) = self.controls[index].set_value(value) {
            return Err(self.record_error(DarkroomControlError::Validation(error)));
        }
        self.advance_revision()
    }

    /// Resets one control to its original default.
    ///
    /// # Errors
    ///
    /// Returns an error when the revision is stale, the id is unknown, or the
    /// revision counter cannot advance.
    pub fn reset_control(
        &mut self,
        expected_revision: Revision,
        id: &str,
    ) -> Result<Revision, DarkroomControlError> {
        self.check_revision(expected_revision)?;
        let Some(index) = self
            .controls
            .iter()
            .position(|control| control.id().as_str() == id)
        else {
            let error = DarkroomControlError::UnknownControl(unknown_control_id(id));
            return Err(self.record_error(error));
        };
        self.controls[index].reset();
        self.advance_revision()
    }

    /// Resets all controls and returns the new snapshot revision.
    ///
    /// # Errors
    ///
    /// Returns an error when the revision is stale or cannot advance.
    pub fn reset_all(
        &mut self,
        expected_revision: Revision,
    ) -> Result<Revision, DarkroomControlError> {
        self.check_revision(expected_revision)?;
        for control in &mut self.controls {
            control.reset();
        }
        self.advance_revision()
    }

    /// Replaces a snapshot after the controller has reconciled a stale edit.
    ///
    /// # Errors
    ///
    /// Returns an error when the replacement contains duplicate or invalid controls.
    pub fn replace_snapshot(
        &mut self,
        revision: Revision,
        controls: Vec<DarkroomControlViewModel>,
    ) -> Result<(), ControlValidationError> {
        let replacement = Self::new(revision, controls)?;
        *self = replacement;
        Ok(())
    }

    fn check_revision(&mut self, expected: Revision) -> Result<(), DarkroomControlError> {
        if expected != self.revision {
            let error = DarkroomControlError::StaleRevision {
                expected,
                actual: self.revision,
            };
            self.status = DarkroomControlsStatus::Stale {
                expected,
                actual: self.revision,
            };
            return Err(error);
        }
        Ok(())
    }

    fn record_error(&mut self, error: DarkroomControlError) -> DarkroomControlError {
        self.status = DarkroomControlsStatus::Error(error.clone());
        error
    }

    fn advance_revision(&mut self) -> Result<Revision, DarkroomControlError> {
        let next = self
            .revision
            .checked_increment()
            .map_err(|_| self.record_error(DarkroomControlError::RevisionOverflow))?;
        self.revision = next;
        self.status = DarkroomControlsStatus::Ready;
        Ok(next)
    }
}

fn unknown_control_id(id: &str) -> ControlId {
    ControlId::new(id).unwrap_or_else(|_| ControlId(format!("invalid-{id}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slider() -> DarkroomControlViewModel {
        DarkroomControlViewModel::slider("exposure", "Exposure", -2.0, 2.0, 0.01, 0.5, 0.0)
            .expect("valid slider")
    }

    fn controls() -> DarkroomControlsViewModel {
        DarkroomControlsViewModel::new(
            Revision::from_u64(4),
            vec![
                slider(),
                DarkroomControlViewModel::choice("method", "Method", ["balanced", "preserve"], 0)
                    .expect("valid choice"),
                DarkroomControlViewModel::toggle("enabled", "Enabled", true, true)
                    .expect("valid toggle"),
            ],
        )
        .expect("unique controls")
    }

    #[test]
    fn typed_controls_preserve_order_and_reset_defaults() {
        let mut model = controls();
        assert_eq!(
            model
                .controls()
                .map(|control| control.id().as_str())
                .collect::<Vec<_>>(),
            ["exposure", "method", "enabled"]
        );
        let next = model
            .set_value(
                Revision::from_u64(4),
                "exposure",
                DarkroomControlValue::Slider(1.25),
            )
            .expect("fresh slider edit");
        assert_eq!(next, Revision::from_u64(5));
        assert_eq!(
            model.control("exposure").expect("exposure").value(),
            DarkroomControlValue::Slider(1.25)
        );
        model
            .reset_control(next, "exposure")
            .expect("reset succeeds");
        assert_eq!(
            model.control("exposure").expect("exposure").value(),
            DarkroomControlValue::Slider(0.0)
        );
    }

    #[test]
    fn invalid_values_and_wrong_types_are_errors_without_mutation() {
        let mut model = controls();
        let error = model
            .set_value(
                Revision::from_u64(4),
                "exposure",
                DarkroomControlValue::Slider(f64::NAN),
            )
            .expect_err("NaN is not a GTK slider value");
        assert!(matches!(
            error,
            DarkroomControlError::Validation(ControlValidationError::SliderNonFinite)
        ));
        assert!(matches!(model.status(), DarkroomControlsStatus::Error(_)));
        assert_eq!(model.revision(), Revision::from_u64(4));
        let error = model
            .set_value(
                Revision::from_u64(4),
                "method",
                DarkroomControlValue::Toggle(false),
            )
            .expect_err("choice cannot receive a toggle");
        assert!(matches!(
            error,
            DarkroomControlError::Validation(
                ControlValidationError::ControlValueTypeMismatch { .. }
            )
        ));
    }

    #[test]
    fn stale_revision_is_explicit_and_does_not_apply_a_late_callback() {
        let mut model = controls();
        model
            .set_value(
                Revision::from_u64(4),
                "enabled",
                DarkroomControlValue::Toggle(false),
            )
            .expect("first callback");
        let error = model
            .set_value(
                Revision::from_u64(4),
                "exposure",
                DarkroomControlValue::Slider(1.0),
            )
            .expect_err("old callback is stale");
        assert_eq!(
            error,
            DarkroomControlError::StaleRevision {
                expected: Revision::from_u64(4),
                actual: Revision::from_u64(5),
            }
        );
        assert_eq!(
            model.control("exposure").expect("exposure").value(),
            DarkroomControlValue::Slider(0.5)
        );
    }

    #[test]
    fn constructors_reject_bad_ranges_and_duplicate_ids() {
        assert!(matches!(
            DarkroomControlViewModel::slider("x", "X", 1.0, 0.0, 0.1, 0.5, 0.5),
            Err(ControlValidationError::SliderRangeInverted)
        ));
        let duplicate = DarkroomControlsViewModel::new(Revision::ZERO, vec![slider(), slider()]);
        assert!(matches!(
            duplicate,
            Err(ControlValidationError::DuplicateControlId(_))
        ));
    }

    #[test]
    fn text_controls_keep_registry_values_typed_and_bounded() {
        let mut control = DarkroomControlViewModel::text(
            "profile",
            "Profile",
            "builtin:srgb",
            "builtin:srgb",
            32,
        )
        .expect("valid text control");
        assert_eq!(control.kind(), DarkroomControlKind::Text);
        control
            .set_value(DarkroomControlValue::Text("builtin:display".to_owned()))
            .expect("text edit");
        assert_eq!(
            control.value(),
            DarkroomControlValue::Text("builtin:display".to_owned())
        );
        assert!(matches!(
            control.set_value(DarkroomControlValue::Text("x".repeat(33))),
            Err(ControlValidationError::TextTooLong { .. })
        ));
    }
}
