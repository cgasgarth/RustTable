use rusttable_core::{Operation, ParameterName, ParameterValue};

use crate::OperationCompileError;

pub(crate) fn parameter_integer(
    operation: &Operation,
    name: &'static str,
    default: f64,
) -> Result<i32, OperationCompileError> {
    let value = super::parameter_f32(operation, name, default)?;
    if !value.is_finite()
        || value.fract() != 0.0
        || value < f32::from(i16::MIN)
        || value > f32::from(i16::MAX)
    {
        return Err(super::invalid_parameters(
            operation,
            format!("{name} must be an exact small integer"),
        ));
    }
    #[allow(clippy::cast_possible_truncation, reason = "range checked above")]
    Ok(value as i32)
}

pub(crate) fn parameter_u32(
    operation: &Operation,
    name: &'static str,
    default: i64,
) -> Result<u32, OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    let value = match operation.parameter(&parameter) {
        None => default,
        Some(ParameterValue::Integer(value)) => *value,
        Some(_) => {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            });
        }
    };
    u32::try_from(value)
        .map_err(|_| super::invalid_parameters(operation, format!("{name} must be a u32")))
}
