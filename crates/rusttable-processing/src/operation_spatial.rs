use rusttable_core::{Operation, ParameterName, ParameterValue};

use crate::operations::{
    graduatednd::{GraduatedNdConfig, GraduatedNdParametersV1},
    vignette::{VignetteConfig, VignetteDither, VignetteParametersV4},
};
use crate::{FiniteF32, OperationCompileError, ProcessingOperation, ProcessingOperationKind};

use super::{compile_opacity, invalid_parameters, parameter_f32, parameter_integer};

const VIGNETTE_PARAMETERS: [&str; 11] = [
    "scale",
    "falloff_scale",
    "brightness",
    "saturation",
    "center_x",
    "center_y",
    "autoratio",
    "whratio",
    "shape",
    "dithering",
    "unbound",
];
const GRADUATED_ND_PARAMETERS: [&str; 6] = [
    "density",
    "hardness",
    "rotation",
    "offset",
    "hue",
    "saturation",
];

pub(crate) fn compile_vignette(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &VIGNETTE_PARAMETERS)?;
    let parameters = VignetteParametersV4::new(
        parameter_f32(operation, "scale", 80.0)?,
        parameter_f32(operation, "falloff_scale", 50.0)?,
        parameter_f32(operation, "brightness", -0.5)?,
        parameter_f32(operation, "saturation", -0.5)?,
        [
            parameter_f32(operation, "center_x", 0.0)?,
            parameter_f32(operation, "center_y", 0.0)?,
        ],
        parameter_bool(operation, "autoratio", false)?,
        parameter_f32(operation, "whratio", 1.0)?,
        parameter_f32(operation, "shape", 1.0)?,
        VignetteDither::from_id(
            u32::try_from(parameter_integer(operation, "dithering", 0.0)?)
                .map_err(|_| invalid_parameters(operation, "dithering must be non-negative"))?,
        )
        .map_err(|error| invalid_parameters(operation, error))?,
        parameter_bool(operation, "unbound", true)?,
    );
    let config =
        VignetteConfig::new(parameters).map_err(|error| invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::Vignette { config },
    })
}

pub(crate) fn compile_graduatednd(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &GRADUATED_ND_PARAMETERS)?;
    let parameters = GraduatedNdParametersV1::new(
        parameter_f32(operation, "density", 1.0)?,
        parameter_f32(operation, "hardness", 0.0)?,
        parameter_f32(operation, "rotation", 0.0)?,
        parameter_f32(operation, "offset", 50.0)?,
        parameter_f32(operation, "hue", 0.0)?,
        parameter_f32(operation, "saturation", 0.0)?,
    );
    let config =
        GraduatedNdConfig::new(parameters).map_err(|error| invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::GraduatedNd { config },
    })
}

fn parameter_bool(
    operation: &Operation,
    name: &'static str,
    default: bool,
) -> Result<bool, OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    match operation.parameter(&parameter) {
        None => Ok(default),
        Some(ParameterValue::Bool(value)) => Ok(*value),
        Some(ParameterValue::Integer(value)) if *value == i64::from(default) => Ok(default),
        Some(_) => Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        }),
    }
}

fn reject_unexpected(operation: &Operation, allowed: &[&str]) -> Result<(), OperationCompileError> {
    if let Some((parameter, _)) = operation
        .parameters()
        .find(|(name, _)| !allowed.contains(&name.as_str()))
    {
        return Err(OperationCompileError::UnexpectedParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: parameter.clone(),
        });
    }
    Ok(())
}

#[allow(dead_code)]
fn _finite(value: f32) -> FiniteF32 {
    FiniteF32::new(value).expect("compiler values are finite")
}
