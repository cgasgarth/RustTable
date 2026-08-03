use rusttable_core::{Operation, ParameterName, ParameterValue};

use crate::operations::colorcontrast::{
    COLOR_CONTRAST_DEFAULT_A_OFFSET, COLOR_CONTRAST_DEFAULT_A_STEEPNESS,
    COLOR_CONTRAST_DEFAULT_B_OFFSET, COLOR_CONTRAST_DEFAULT_B_STEEPNESS,
    COLOR_CONTRAST_DEFAULT_UNBOUND, ColorContrastConfig,
};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const COLOR_CONTRAST_PARAMETERS: [&str; 5] = [
    "a_steepness",
    "a_offset",
    "b_steepness",
    "b_offset",
    "unbound",
];

pub fn compile_colorcontrast(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &COLOR_CONTRAST_PARAMETERS)?;
    let config = ColorContrastConfig::new(
        super::parameter_f32(
            operation,
            "a_steepness",
            f64::from(COLOR_CONTRAST_DEFAULT_A_STEEPNESS),
        )?,
        super::parameter_f32(
            operation,
            "a_offset",
            f64::from(COLOR_CONTRAST_DEFAULT_A_OFFSET),
        )?,
        super::parameter_f32(
            operation,
            "b_steepness",
            f64::from(COLOR_CONTRAST_DEFAULT_B_STEEPNESS),
        )?,
        super::parameter_f32(
            operation,
            "b_offset",
            f64::from(COLOR_CONTRAST_DEFAULT_B_OFFSET),
        )?,
        parameter_i32(operation, "unbound", COLOR_CONTRAST_DEFAULT_UNBOUND)?,
    )
    .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::ColorContrast { config },
    })
}

fn parameter_i32(
    operation: &Operation,
    name: &'static str,
    default: i32,
) -> Result<i32, OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    match operation.parameter(&parameter) {
        None => Ok(default),
        Some(ParameterValue::Integer(value)) => i32::try_from(*value).map_err(|_| {
            super::invalid_parameters(operation, format!("{name} must fit a native 32-bit int"))
        }),
        Some(_) => Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        }),
    }
}
