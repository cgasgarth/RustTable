use rusttable_core::{Operation, ParameterName, ParameterValue};

use crate::ScalarNarrowingError;
use crate::{FiniteF32, OperationCompileError, ProcessingOperation, ProcessingOperationKind};

pub fn compile_scalar<F>(
    operation: &Operation,
    required_name: &str,
    build: F,
) -> Result<ProcessingOperation, OperationCompileError>
where
    F: FnOnce(FiniteF32) -> ProcessingOperationKind,
{
    let required = ParameterName::new(required_name).expect("processing schema names are valid");
    if operation.parameter(&required).is_none() {
        return Err(OperationCompileError::MissingParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: required,
        });
    }
    if let Some((unexpected, _)) = operation
        .parameters()
        .find(|(name, _)| name.as_str() != required_name)
    {
        return Err(OperationCompileError::UnexpectedParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: unexpected.clone(),
        });
    }
    let value = compile_scalar_parameter(operation, required_name)?;
    let opacity = super::compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: build(value),
    })
}

pub fn compile_scalar_parameter(
    operation: &Operation,
    parameter_name: &str,
) -> Result<FiniteF32, OperationCompileError> {
    let parameter = ParameterName::new(parameter_name).expect("processing schema names are valid");
    if operation.parameter(&parameter).is_none() {
        return Err(OperationCompileError::MissingParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        });
    }
    let value = match operation.parameter(&parameter) {
        Some(ParameterValue::Scalar(value)) => *value,
        Some(_) => {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            });
        }
        None => unreachable!("required parameter was checked above"),
    };
    match FiniteF32::try_from(value) {
        Ok(value) => Ok(value),
        Err(ScalarNarrowingError::Overflow) => {
            Err(OperationCompileError::ScalarNarrowingOverflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            })
        }
        Err(ScalarNarrowingError::Underflow) => {
            Err(OperationCompileError::ScalarNarrowingUnderflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            })
        }
    }
}

pub fn parameter_f64(
    operation: &Operation,
    name: &'static str,
    default: f64,
) -> Result<f64, OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    match operation.parameter(&parameter) {
        None => Ok(default),
        #[expect(
            clippy::cast_precision_loss,
            reason = "integer compatibility values are widened to the canonical f64 form"
        )]
        Some(ParameterValue::Integer(value)) => Ok(*value as f64),
        Some(ParameterValue::Scalar(value)) => Ok(value.get()),
        Some(_) => Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        }),
    }
}

pub fn parameter_integer(
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
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the validated compatibility integer is narrowed at the native boundary"
    )]
    Ok(value as i32)
}

pub fn parameter_f32_array<const N: usize>(
    operation: &Operation,
    name: &'static str,
    default: [f32; N],
) -> Result<[f32; N], OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    let Some(value) = operation.parameter(&parameter) else {
        return Ok(default);
    };
    let ParameterValue::Text(value) = value else {
        return Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        });
    };
    let text = value.as_str().trim();
    let Some(text) = text
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
    else {
        return Err(super::invalid_parameters(
            operation,
            format!("{name} must be a {N}-element vector"),
        ));
    };
    let values = text
        .split(',')
        .map(str::trim)
        .map(|value| {
            value.parse::<f64>().map_err(|_| {
                super::invalid_parameters(operation, format!("{name} contains a non-numeric value"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != N {
        return Err(super::invalid_parameters(
            operation,
            format!("{name} must have exactly {N} values"),
        ));
    }
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let value = rusttable_core::FiniteF64::new(value)
                .map_err(|_| {
                    super::invalid_parameters(operation, format!("{name}[{index}] must be finite"))
                })
                .and_then(|value| {
                    FiniteF32::try_from(value).map_err(|_| {
                        super::invalid_parameters(
                            operation,
                            format!("{name}[{index}] does not fit f32"),
                        )
                    })
                })?;
            Ok(value.get())
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| super::invalid_parameters(operation, format!("{name} has an invalid shape")))
}

pub fn parameter_bool_default(
    operation: &Operation,
    name: &'static str,
    default: bool,
) -> Result<bool, OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    match operation.parameter(&parameter) {
        None => Ok(default),
        Some(ParameterValue::Bool(value)) => Ok(*value),
        Some(_) => Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        }),
    }
}

pub fn parameter_u32(
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
