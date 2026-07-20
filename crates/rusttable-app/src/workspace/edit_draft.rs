use std::fmt;

use rusttable_core::{
    Edit, EditRevisionError, FiniteF64, Operation, OperationBuildError, OperationId,
    OperationOpacity, ParameterName, ParameterValue,
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
    const fn name(self) -> &'static str {
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
    const fn of(value: &ParameterValue) -> Self {
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
    const fn range(self) -> (f64, f64) {
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
struct OperationState {
    id: OperationId,
    enabled: bool,
    opacity: OperationOpacity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicEditDraft {
    edit: Edit,
    exposure: OperationState,
    rgb_gain: OperationState,
    exposure_stops: FiniteF64,
    rgb_red: FiniteF64,
    rgb_green: FiniteF64,
    rgb_blue: FiniteF64,
}

impl BasicEditDraft {
    /// Parses the two required scalar operations from an existing edit.
    ///
    /// The edit is retained as the replacement source so every operation and parameter that
    /// this draft does not own remains byte-for-byte equivalent at the model level.
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

    pub fn with_exposure_stops(mut self, value: f64) -> Result<Self, BasicEditValueError> {
        self.exposure_stops = checked_value(BasicEditValue::ExposureStops, value)?;
        Ok(self)
    }

    pub fn with_rgb_red(mut self, value: f64) -> Result<Self, BasicEditValueError> {
        self.rgb_red = checked_value(BasicEditValue::RgbRed, value)?;
        Ok(self)
    }

    pub fn with_rgb_green(mut self, value: f64) -> Result<Self, BasicEditValueError> {
        self.rgb_green = checked_value(BasicEditValue::RgbGreen, value)?;
        Ok(self)
    }

    pub fn with_rgb_blue(mut self, value: f64) -> Result<Self, BasicEditValueError> {
        self.rgb_blue = checked_value(BasicEditValue::RgbBlue, value)?;
        Ok(self)
    }

    /// Returns the next edit revision with only the four owned scalar values replaced.
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

#[derive(Debug, Clone, Copy)]
struct ParsedOperation {
    state: OperationState,
    values: [FiniteF64; 3],
}

fn parse_operation(
    operation: &Operation,
    kind: BasicEditOperation,
) -> Result<ParsedOperation, BasicEditDraftError> {
    if operation.opacity() != OperationOpacity::ONE {
        return Err(BasicEditDraftError::NonUnitOpacity {
            operation: kind,
            operation_id: operation.id(),
            actual: operation.opacity(),
        });
    }

    let values = match kind {
        BasicEditOperation::Exposure => [
            parse_scalar(operation, kind, BasicEditParameter::ExposureStops)?,
            FiniteF64::new(0.0).expect("zero is finite"),
            FiniteF64::new(0.0).expect("zero is finite"),
        ],
        BasicEditOperation::RgbGain => [
            parse_scalar(operation, kind, BasicEditParameter::RgbRed)?,
            parse_scalar(operation, kind, BasicEditParameter::RgbGreen)?,
            parse_scalar(operation, kind, BasicEditParameter::RgbBlue)?,
        ],
    };

    Ok(ParsedOperation {
        state: OperationState {
            id: operation.id(),
            enabled: operation.is_enabled(),
            opacity: operation.opacity(),
        },
        values,
    })
}

fn parse_scalar(
    operation: &Operation,
    kind: BasicEditOperation,
    parameter: BasicEditParameter,
) -> Result<FiniteF64, BasicEditDraftError> {
    let name = ParameterName::new(parameter.name()).expect("static parameter name is valid");
    let value = operation
        .parameter(&name)
        .ok_or(BasicEditDraftError::MissingParameter {
            operation: kind,
            parameter,
        })?;
    let scalar = match value {
        ParameterValue::Scalar(value) => *value,
        _ => {
            return Err(BasicEditDraftError::WrongParameterType {
                operation: kind,
                parameter,
                actual: ParameterValueType::of(value),
            });
        }
    };
    let scalar = FiniteF64::new(scalar.get()).map_err(|_| BasicEditDraftError::NonFinite {
        operation: kind,
        parameter,
    })?;
    let value_kind = match parameter {
        BasicEditParameter::ExposureStops => BasicEditValue::ExposureStops,
        BasicEditParameter::RgbRed => BasicEditValue::RgbRed,
        BasicEditParameter::RgbGreen => BasicEditValue::RgbGreen,
        BasicEditParameter::RgbBlue => BasicEditValue::RgbBlue,
    };
    let (minimum, maximum) = value_kind.range();
    if !(minimum..=maximum).contains(&scalar.get()) {
        return Err(BasicEditDraftError::OutOfRange {
            operation: kind,
            parameter,
            actual: scalar,
            minimum: FiniteF64::new(minimum).expect("range minimum is finite"),
            maximum: FiniteF64::new(maximum).expect("range maximum is finite"),
        });
    }
    Ok(scalar)
}

fn checked_value(value: BasicEditValue, actual: f64) -> Result<FiniteF64, BasicEditValueError> {
    let actual = FiniteF64::new(actual).map_err(|_| BasicEditValueError::NonFinite { value })?;
    let (minimum, maximum) = value.range();
    if !(minimum..=maximum).contains(&actual.get()) {
        return Err(BasicEditValueError::OutOfRange {
            value,
            actual,
            minimum: FiniteF64::new(minimum).expect("range minimum is finite"),
            maximum: FiniteF64::new(maximum).expect("range maximum is finite"),
        });
    }
    Ok(actual)
}

#[cfg(test)]
mod tests {
    use rusttable_core::{
        Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity,
        ParameterName, ParameterText, ParameterValue, PhotoId, Revision,
    };

    use super::{
        BasicEditDraft, BasicEditDraftError, BasicEditOperation, BasicEditParameter,
        BasicEditValue, BasicEditValueError, ParameterValueType,
    };

    fn id(value: u128) -> OperationId {
        OperationId::new(value).expect("nonzero operation ID")
    }

    fn parameter(name: &str) -> ParameterName {
        ParameterName::new(name).expect("valid parameter name")
    }

    fn scalar(value: f64) -> ParameterValue {
        ParameterValue::Scalar(FiniteF64::new(value).expect("finite test scalar"))
    }

    fn operation(
        operation_id: u128,
        key: &str,
        enabled: bool,
        opacity: f64,
        parameters: impl IntoIterator<Item = (&'static str, ParameterValue)>,
    ) -> Operation {
        Operation::new_with_opacity(
            id(operation_id),
            OperationKey::new(key).expect("valid operation key"),
            enabled,
            OperationOpacity::new(opacity).expect("valid opacity"),
            parameters
                .into_iter()
                .map(|(name, value)| (parameter(name), value)),
        )
        .expect("valid operation")
    }

    fn valid_edit(exposure: f64, red: f64, green: f64, blue: f64) -> Edit {
        Edit::from_parts(
            EditId::new(41).expect("edit ID"),
            PhotoId::new(42).expect("photo ID"),
            Revision::from_u64(7),
            Revision::from_u64(3),
            [
                operation(
                    10,
                    "rusttable.exposure",
                    false,
                    1.0,
                    [
                        ("stops", scalar(exposure)),
                        (
                            "note",
                            ParameterValue::Text(ParameterText::new("keep").expect("text")),
                        ),
                    ],
                ),
                operation(
                    20,
                    "rusttable.rgb_gain",
                    true,
                    1.0,
                    [
                        ("red", scalar(red)),
                        ("green", scalar(green)),
                        ("blue", scalar(blue)),
                    ],
                ),
                operation(
                    30,
                    "rusttable.tone",
                    true,
                    0.75,
                    [
                        ("strength", scalar(0.25)),
                        ("label", ParameterValue::Bool(true)),
                    ],
                ),
            ],
        )
        .expect("valid edit")
    }

    #[test]
    fn parses_defaults_and_non_default_values_with_identity_and_state() {
        let edit = valid_edit(0.0, 1.0, 1.0, 1.0);
        let draft = BasicEditDraft::from_edit(&edit).expect("valid draft");

        assert_eq!(draft.edit_id(), edit.id());
        assert_eq!(draft.photo_id(), edit.photo_id());
        assert_eq!(draft.base_photo_revision(), Revision::from_u64(7));
        assert_eq!(draft.edit_revision(), Revision::from_u64(3));
        assert_eq!(draft.exposure_operation_id(), id(10));
        assert_eq!(draft.rgb_gain_operation_id(), id(20));
        assert!(!draft.exposure_enabled());
        assert!(draft.rgb_gain_enabled());
        assert_eq!(draft.exposure_opacity(), OperationOpacity::ONE);
        assert_eq!(draft.rgb_gain_opacity(), OperationOpacity::ONE);
        assert_float_eq(draft.exposure_stops(), 0.0);
        assert_float_eq(draft.rgb_red(), 1.0);
        assert_float_eq(draft.rgb_green(), 1.0);
        assert_float_eq(draft.rgb_blue(), 1.0);

        let edit = valid_edit(-2.5, 0.25, 1.5, 2.0);
        let draft = BasicEditDraft::from_edit(&edit).expect("valid non-default draft");
        assert_float_eq(draft.exposure_stops(), -2.5);
        assert_float_eq(draft.rgb_red(), 0.25);
        assert_float_eq(draft.rgb_green(), 1.5);
        assert_float_eq(draft.rgb_blue(), 2.0);
    }

    #[test]
    fn replacement_methods_accept_inclusive_boundaries_and_replace_values() {
        let draft = BasicEditDraft::from_edit(&valid_edit(0.0, 1.0, 1.0, 1.0))
            .unwrap()
            .with_exposure_stops(-5.0)
            .unwrap()
            .with_rgb_red(0.0)
            .unwrap()
            .with_rgb_green(2.0)
            .unwrap()
            .with_rgb_blue(0.0)
            .unwrap();

        assert_float_eq(draft.exposure_stops(), -5.0);
        assert_float_eq(draft.rgb_red(), 0.0);
        assert_float_eq(draft.rgb_green(), 2.0);
        assert_float_eq(draft.rgb_blue(), 0.0);
    }

    #[test]
    fn replacement_methods_reject_non_finite_and_out_of_range_values() {
        let draft = BasicEditDraft::from_edit(&valid_edit(0.0, 1.0, 1.0, 1.0)).unwrap();
        assert_eq!(
            draft.clone().with_exposure_stops(f64::NAN),
            Err(BasicEditValueError::NonFinite {
                value: BasicEditValue::ExposureStops
            })
        );
        assert!(matches!(
            draft.clone().with_exposure_stops(-5.1),
            Err(BasicEditValueError::OutOfRange { .. })
        ));
        assert!(matches!(
            draft.clone().with_rgb_red(2.1),
            Err(BasicEditValueError::OutOfRange {
                value: BasicEditValue::RgbRed,
                ..
            })
        ));
        assert_float_eq(draft.exposure_stops(), 0.0);
    }

    #[test]
    fn rejects_missing_and_duplicate_instances() {
        let edit = valid_edit(0.0, 1.0, 1.0, 1.0);
        let without_exposure = Edit::from_parts(
            edit.id(),
            edit.photo_id(),
            edit.base_photo_revision(),
            edit.revision(),
            edit.operations().skip(1).cloned().collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            BasicEditDraft::from_edit(&without_exposure),
            Err(BasicEditDraftError::MissingOperation {
                operation: BasicEditOperation::Exposure
            })
        );

        let duplicate = Edit::from_parts(
            edit.id(),
            edit.photo_id(),
            edit.base_photo_revision(),
            edit.revision(),
            edit.operations()
                .cloned()
                .chain(std::iter::once(operation(
                    40,
                    "rusttable.exposure",
                    true,
                    1.0,
                    [("stops", scalar(1.0))],
                )))
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            BasicEditDraft::from_edit(&duplicate),
            Err(BasicEditDraftError::DuplicateOperation {
                operation: BasicEditOperation::Exposure
            })
        );
    }

    #[test]
    fn rejects_missing_parameters_wrong_types_and_non_unit_opacity() {
        let edit = valid_edit(0.0, 1.0, 1.0, 1.0);
        let missing = operation(10, "rusttable.exposure", true, 1.0, []);
        let edit = replace_first(&edit, &missing);
        assert_eq!(
            BasicEditDraft::from_edit(&edit),
            Err(BasicEditDraftError::MissingParameter {
                operation: BasicEditOperation::Exposure,
                parameter: BasicEditParameter::ExposureStops
            })
        );

        let wrong_type = operation(
            10,
            "rusttable.exposure",
            true,
            1.0,
            [("stops", ParameterValue::Bool(true))],
        );
        let source = valid_edit(0.0, 1.0, 1.0, 1.0);
        let edit = replace_first(&source, &wrong_type);
        assert_eq!(
            BasicEditDraft::from_edit(&edit),
            Err(BasicEditDraftError::WrongParameterType {
                operation: BasicEditOperation::Exposure,
                parameter: BasicEditParameter::ExposureStops,
                actual: ParameterValueType::Boolean
            })
        );

        let non_unit = operation(
            10,
            "rusttable.exposure",
            true,
            0.5,
            [("stops", scalar(0.0))],
        );
        let source = valid_edit(0.0, 1.0, 1.0, 1.0);
        let edit = replace_first(&source, &non_unit);
        assert!(matches!(
            BasicEditDraft::from_edit(&edit),
            Err(BasicEditDraftError::NonUnitOpacity {
                operation: BasicEditOperation::Exposure,
                ..
            })
        ));
    }

    #[test]
    fn rejects_source_values_outside_the_typed_ranges() {
        let source = valid_edit(0.0, 1.0, 1.0, 1.0);
        let replacement = operation(
            10,
            "rusttable.exposure",
            true,
            1.0,
            [("stops", scalar(5.01))],
        );
        let edit = replace_first(&source, &replacement);
        assert!(matches!(
            BasicEditDraft::from_edit(&edit),
            Err(BasicEditDraftError::OutOfRange {
                operation: BasicEditOperation::Exposure,
                parameter: BasicEditParameter::ExposureStops,
                ..
            })
        ));

        let source = valid_edit(0.0, 1.0, 1.0, 1.0);
        let replacement = operation(
            20,
            "rusttable.rgb_gain",
            true,
            1.0,
            [
                ("red", scalar(1.0)),
                ("green", scalar(-0.01)),
                ("blue", scalar(1.0)),
            ],
        );
        let edit = replace_second(&source, &replacement);
        assert!(matches!(
            BasicEditDraft::from_edit(&edit),
            Err(BasicEditDraftError::OutOfRange {
                operation: BasicEditOperation::RgbGain,
                parameter: BasicEditParameter::RgbGreen,
                ..
            })
        ));
    }

    #[test]
    fn replacement_preserves_unrelated_operations_order_and_parameters() {
        let original = valid_edit(0.0, 1.0, 1.0, 1.0);
        let unrelated = original.operations().nth(2).unwrap().clone();
        let draft = BasicEditDraft::from_edit(&original)
            .unwrap()
            .with_exposure_stops(2.0)
            .unwrap()
            .with_rgb_red(0.5)
            .unwrap()
            .with_rgb_green(1.25)
            .unwrap()
            .with_rgb_blue(1.75)
            .unwrap();
        let revised = draft.replacement_edit().expect("revision succeeds");

        assert_eq!(
            revised.operations().map(Operation::id).collect::<Vec<_>>(),
            vec![id(10), id(20), id(30)]
        );
        assert_eq!(revised.operations().nth(2), Some(&unrelated));
        let exposure = revised.operations().next().unwrap();
        assert_eq!(
            exposure.parameter(&parameter("note")),
            Some(&ParameterValue::Text(ParameterText::new("keep").unwrap()))
        );
        assert!(!exposure.is_enabled());
        assert_eq!(exposure.opacity(), OperationOpacity::ONE);
        assert_eq!(exposure.parameter(&parameter("stops")), Some(&scalar(2.0)));
        let rgb = revised.operations().nth(1).unwrap();
        assert!(rgb.is_enabled());
        assert_eq!(rgb.opacity(), OperationOpacity::ONE);
        assert_eq!(rgb.parameter(&parameter("red")), Some(&scalar(0.5)));
        assert_eq!(rgb.parameter(&parameter("green")), Some(&scalar(1.25)));
        assert_eq!(rgb.parameter(&parameter("blue")), Some(&scalar(1.75)));
    }

    #[test]
    fn replacement_preserves_identity_and_advances_only_edit_revision() {
        let original = valid_edit(0.0, 1.0, 1.0, 1.0);
        let draft = BasicEditDraft::from_edit(&original).unwrap();
        let revised = draft.replacement_edit().expect("revision succeeds");

        assert_eq!(revised.id(), original.id());
        assert_eq!(revised.photo_id(), original.photo_id());
        assert_eq!(
            revised.base_photo_revision(),
            original.base_photo_revision()
        );
        assert_eq!(revised.revision(), Revision::from_u64(4));
        assert_eq!(original.revision(), Revision::from_u64(3));
    }

    fn assert_float_eq(actual: f64, expected: f64) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }

    fn replace_first(edit: &Edit, operation: &Operation) -> Edit {
        replace_at(edit, 0, operation)
    }

    fn replace_second(edit: &Edit, operation: &Operation) -> Edit {
        replace_at(edit, 1, operation)
    }

    fn replace_at(edit: &Edit, index: usize, replacement: &Operation) -> Edit {
        Edit::from_parts(
            edit.id(),
            edit.photo_id(),
            edit.base_photo_revision(),
            edit.revision(),
            edit.operations()
                .enumerate()
                .map(|(current, operation)| {
                    if current == index {
                        replacement.clone()
                    } else {
                        operation.clone()
                    }
                })
                .collect::<Vec<_>>(),
        )
        .expect("valid replacement edit")
    }
}
