use rusttable_core::{FiniteF64, Operation, OperationOpacity, ParameterName, ParameterValue};

use super::model::OperationState;
use super::{
    BasicEditDraftError, BasicEditOperation, BasicEditParameter, BasicEditValue,
    BasicEditValueError, ParameterValueType,
};

#[derive(Debug, Clone, Copy)]
pub(super) struct ParsedOperation {
    pub(super) state: OperationState,
    pub(super) values: [FiniteF64; 3],
}

pub(super) fn parse_operation(
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

pub(super) fn checked_value(
    value: BasicEditValue,
    actual: f64,
) -> Result<FiniteF64, BasicEditValueError> {
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
