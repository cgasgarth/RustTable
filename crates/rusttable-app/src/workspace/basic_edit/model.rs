use std::fmt;

use super::parse::{checked_value, parse_operation};

use rusttable_core::{
    Edit, EditRevisionError, FiniteF64, Operation, OperationBuildError, OperationId,
    OperationOpacity, ParameterValue,
};

const EXPOSURE_KEY: &str = "rusttable.exposure";
const EXPOSURE_PARAMETER: &str = "stops";
const RGB_GAIN_KEY: &str = "rusttable.rgb_gain";
const RGB_RED_PARAMETER: &str = "red";
const RGB_GREEN_PARAMETER: &str = "green";
const RGB_BLUE_PARAMETER: &str = "blue";

const EXPOSURE_MIN: f64 = -5.0;
const EXPOSURE_MAX: f64 = 5.0;
const RGB_GAIN_MIN: f64 = 0.0;
const RGB_GAIN_MAX: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicEditOperation {
    Exposure,
    RgbGain,
}

impl BasicEditOperation {
    const fn key(self) -> &'static str {
        match self {
            Self::Exposure => EXPOSURE_KEY,
            Self::RgbGain => RGB_GAIN_KEY,
        }
    }
}

impl fmt::Display for BasicEditOperation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.key())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicEditParameter {
    ExposureStops,
    RgbRed,
    RgbGreen,
    RgbBlue,
}

impl BasicEditParameter {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::ExposureStops => EXPOSURE_PARAMETER,
            Self::RgbRed => RGB_RED_PARAMETER,
            Self::RgbGreen => RGB_GREEN_PARAMETER,
            Self::RgbBlue => RGB_BLUE_PARAMETER,
        }
    }
}

impl fmt::Display for BasicEditParameter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterValueType {
    Boolean,
    Integer,
    Scalar,
    Text,
}

impl ParameterValueType {
    pub(super) const fn of(value: &ParameterValue) -> Self {
        match value {
            ParameterValue::Bool(_) => Self::Boolean,
            ParameterValue::Integer(_) => Self::Integer,
            ParameterValue::Scalar(_) => Self::Scalar,
            ParameterValue::Text(_) => Self::Text,
        }
    }
}

impl fmt::Display for ParameterValueType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Boolean => "boolean",
            Self::Integer => "integer",
            Self::Scalar => "scalar",
            Self::Text => "text",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicEditValue {
    ExposureStops,
    RgbRed,
    RgbGreen,
    RgbBlue,
}

impl BasicEditValue {
    pub(super) const fn range(self) -> (f64, f64) {
        match self {
            Self::ExposureStops => (EXPOSURE_MIN, EXPOSURE_MAX),
            Self::RgbRed | Self::RgbGreen | Self::RgbBlue => (RGB_GAIN_MIN, RGB_GAIN_MAX),
        }
    }
}

impl fmt::Display for BasicEditValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::ExposureStops => "exposure stops",
            Self::RgbRed => "RGB red gain",
            Self::RgbGreen => "RGB green gain",
            Self::RgbBlue => "RGB blue gain",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicEditValueError {
    NonFinite {
        value: BasicEditValue,
    },
    OutOfRange {
        value: BasicEditValue,
        actual: FiniteF64,
        minimum: FiniteF64,
        maximum: FiniteF64,
    },
}

impl fmt::Display for BasicEditValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite { value } => write!(formatter, "{value} must be finite"),
            Self::OutOfRange {
                value,
                actual,
                minimum,
                maximum,
            } => write!(
                formatter,
                "{value} {} is outside the inclusive range {}..={}",
                actual.get(),
                minimum.get(),
                maximum.get(),
            ),
        }
    }
}

impl std::error::Error for BasicEditValueError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasicEditDraftError {
    MissingOperation {
        operation: BasicEditOperation,
    },
    DuplicateOperation {
        operation: BasicEditOperation,
    },
    MissingParameter {
        operation: BasicEditOperation,
        parameter: BasicEditParameter,
    },
    WrongParameterType {
        operation: BasicEditOperation,
        parameter: BasicEditParameter,
        actual: ParameterValueType,
    },
    NonFinite {
        operation: BasicEditOperation,
        parameter: BasicEditParameter,
    },
    OutOfRange {
        operation: BasicEditOperation,
        parameter: BasicEditParameter,
        actual: FiniteF64,
        minimum: FiniteF64,
        maximum: FiniteF64,
    },
    NonUnitOpacity {
        operation: BasicEditOperation,
        operation_id: OperationId,
        actual: OperationOpacity,
    },
}

impl fmt::Display for BasicEditDraftError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingOperation { operation } => {
                write!(formatter, "missing {operation} operation")
            }
            Self::DuplicateOperation { operation } => {
                write!(formatter, "duplicate {operation} operation")
            }
            Self::MissingParameter {
                operation,
                parameter,
            } => write!(formatter, "{operation} is missing {parameter} parameter"),
            Self::WrongParameterType {
                operation,
                parameter,
                actual,
            } => write!(
                formatter,
                "{operation} parameter {parameter} has type {actual}; expected scalar"
            ),
            Self::NonFinite {
                operation,
                parameter,
            } => write!(
                formatter,
                "{operation} parameter {parameter} must be finite"
            ),
            Self::OutOfRange {
                operation,
                parameter,
                actual,
                minimum,
                maximum,
            } => write!(
                formatter,
                "{operation} parameter {parameter} {} is outside {}..={}",
                actual.get(),
                minimum.get(),
                maximum.get(),
            ),
            Self::NonUnitOpacity {
                operation,
                operation_id,
                actual,
            } => write!(
                formatter,
                "{operation} operation {operation_id} has non-unit opacity {actual}"
            ),
        }
    }
}

impl std::error::Error for BasicEditDraftError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasicEditDraftReplacementError {
    Revision(EditRevisionError),
    InvalidOperation {
        operation_id: OperationId,
        source: OperationBuildError,
    },
}

impl fmt::Display for BasicEditDraftReplacementError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Revision(source) => source.fmt(formatter),
            Self::InvalidOperation {
                operation_id,
                source,
            } => write!(
                formatter,
                "cannot replace operation {operation_id}: {source}"
            ),
        }
    }
}

impl std::error::Error for BasicEditDraftReplacementError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Revision(source) => Some(source),
            Self::InvalidOperation { source, .. } => Some(source),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OperationState {
    pub(super) id: OperationId,
    pub(super) enabled: bool,
    pub(super) opacity: OperationOpacity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicEditDraft {
    edit: Edit,
    exposure: OperationState,
    rgb_gain: OperationState,
    pub(super) exposure_stops: FiniteF64,
    pub(super) rgb_red: FiniteF64,
    pub(super) rgb_green: FiniteF64,
    pub(super) rgb_blue: FiniteF64,
}

impl BasicEditDraft {
    /// Parses the two required scalar operations from an existing edit.
    ///
    /// The edit is retained as the replacement source so every operation and parameter that
    /// this draft does not own remains byte-for-byte equivalent at the model level.
    ///
    /// # Errors
    ///
    /// Returns a typed error when either required operation is missing, duplicated,
    /// malformed, non-finite, or outside its supported range.
    pub fn from_edit(edit: &Edit) -> Result<Self, BasicEditDraftError> {
        let mut exposure = None;
        let mut rgb_gain = None;
        for operation in edit.operations() {
            match operation.key().as_str() {
                EXPOSURE_KEY => {
                    if exposure.is_some() {
                        return Err(BasicEditDraftError::DuplicateOperation {
                            operation: BasicEditOperation::Exposure,
                        });
                    }
                    exposure = Some(parse_operation(operation, BasicEditOperation::Exposure)?);
                }
                RGB_GAIN_KEY => {
                    if rgb_gain.is_some() {
                        return Err(BasicEditDraftError::DuplicateOperation {
                            operation: BasicEditOperation::RgbGain,
                        });
                    }
                    rgb_gain = Some(parse_operation(operation, BasicEditOperation::RgbGain)?);
                }
                _ => {}
            }
        }

        let exposure = exposure.ok_or(BasicEditDraftError::MissingOperation {
            operation: BasicEditOperation::Exposure,
        })?;
        let rgb_gain = rgb_gain.ok_or(BasicEditDraftError::MissingOperation {
            operation: BasicEditOperation::RgbGain,
        })?;

        Ok(Self {
            edit: edit.clone(),
            exposure: exposure.state,
            rgb_gain: rgb_gain.state,
            exposure_stops: exposure.values[0],
            rgb_red: rgb_gain.values[0],
            rgb_green: rgb_gain.values[1],
            rgb_blue: rgb_gain.values[2],
        })
    }

    #[must_use]
    pub const fn edit_id(&self) -> rusttable_core::EditId {
        self.edit.id()
    }

    #[must_use]
    pub const fn photo_id(&self) -> rusttable_core::PhotoId {
        self.edit.photo_id()
    }

    #[must_use]
    pub const fn base_photo_revision(&self) -> rusttable_core::Revision {
        self.edit.base_photo_revision()
    }

    #[must_use]
    pub const fn edit_revision(&self) -> rusttable_core::Revision {
        self.edit.revision()
    }

    #[must_use]
    pub const fn exposure_operation_id(&self) -> OperationId {
        self.exposure.id
    }

    #[must_use]
    pub const fn rgb_gain_operation_id(&self) -> OperationId {
        self.rgb_gain.id
    }

    #[must_use]
    pub const fn exposure_enabled(&self) -> bool {
        self.exposure.enabled
    }

    #[must_use]
    pub const fn rgb_gain_enabled(&self) -> bool {
        self.rgb_gain.enabled
    }

    #[must_use]
    pub const fn exposure_opacity(&self) -> OperationOpacity {
        self.exposure.opacity
    }

    #[must_use]
    pub const fn rgb_gain_opacity(&self) -> OperationOpacity {
        self.rgb_gain.opacity
    }

    #[must_use]
    pub const fn exposure_stops(&self) -> f64 {
        self.exposure_stops.get()
    }

    #[must_use]
    pub const fn rgb_red(&self) -> f64 {
        self.rgb_red.get()
    }

    #[must_use]
    pub const fn rgb_green(&self) -> f64 {
        self.rgb_green.get()
    }

    #[must_use]
    pub const fn rgb_blue(&self) -> f64 {
        self.rgb_blue.get()
    }

    /// Updates exposure stops after enforcing the supported `-5..=5` range.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is non-finite or outside the supported range.
    pub fn set_exposure_stops(&mut self, value: f64) -> Result<(), BasicEditValueError> {
        self.exposure_stops = checked_value(BasicEditValue::ExposureStops, value)?;
        Ok(())
    }

    /// Updates the red gain after enforcing the supported `0..=2` range.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is non-finite or outside the supported range.
    pub fn set_rgb_red(&mut self, value: f64) -> Result<(), BasicEditValueError> {
        self.rgb_red = checked_value(BasicEditValue::RgbRed, value)?;
        Ok(())
    }

    /// Updates the green gain after enforcing the supported `0..=2` range.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is non-finite or outside the supported range.
    pub fn set_rgb_green(&mut self, value: f64) -> Result<(), BasicEditValueError> {
        self.rgb_green = checked_value(BasicEditValue::RgbGreen, value)?;
        Ok(())
    }

    /// Updates the blue gain after enforcing the supported `0..=2` range.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is non-finite or outside the supported range.
    pub fn set_rgb_blue(&mut self, value: f64) -> Result<(), BasicEditValueError> {
        self.rgb_blue = checked_value(BasicEditValue::RgbBlue, value)?;
        Ok(())
    }

    /// Returns the next edit revision with only the four owned scalar values replaced.
    ///
    /// # Errors
    ///
    /// Returns an error when the revised operation set cannot form a valid next edit revision.
    pub fn replacement_edit(&self) -> Result<Edit, BasicEditDraftReplacementError> {
        let operations = self
            .edit
            .operations()
            .map(|operation| self.replacement_operation(operation))
            .collect::<Result<Vec<_>, _>>()
            .map_err(
                |(operation_id, source)| BasicEditDraftReplacementError::InvalidOperation {
                    operation_id,
                    source,
                },
            )?;
        self.edit
            .revised(operations)
            .map_err(BasicEditDraftReplacementError::Revision)
    }

    fn replacement_operation(
        &self,
        operation: &Operation,
    ) -> Result<Operation, (OperationId, OperationBuildError)> {
        let replacement = if operation.id() == self.exposure.id {
            Some((EXPOSURE_PARAMETER, self.exposure_stops))
        } else if operation.id() == self.rgb_gain.id {
            None
        } else {
            return Ok(operation.clone());
        };

        let parameters = operation.parameters().map(|(name, value)| {
            let value = if replacement.is_some_and(|(parameter, _)| name.as_str() == parameter) {
                ParameterValue::Scalar(self.exposure_stops)
            } else if operation.id() == self.rgb_gain.id {
                match name.as_str() {
                    RGB_RED_PARAMETER => ParameterValue::Scalar(self.rgb_red),
                    RGB_GREEN_PARAMETER => ParameterValue::Scalar(self.rgb_green),
                    RGB_BLUE_PARAMETER => ParameterValue::Scalar(self.rgb_blue),
                    _ => value.clone(),
                }
            } else {
                value.clone()
            };
            (name.clone(), value)
        });
        Operation::new_with_opacity(
            operation.id(),
            operation.key().clone(),
            operation.is_enabled(),
            operation.opacity(),
            parameters,
        )
        .map_err(|source| (operation.id(), source))
    }
}
