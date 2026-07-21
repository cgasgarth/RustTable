use rusttable_core::Operation;

use crate::operations::{
    relight::RelightConfig,
    shadhi::{ShadhiConfig, ShadhiParametersV5},
};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const RELIGHT_PARAMETERS: [&str; 3] = ["ev", "center", "width"];
const SHADHI_PARAMETERS: [&str; 12] = [
    "order",
    "radius",
    "shadows",
    "whitepoint",
    "highlights",
    "reserved2",
    "compress",
    "shadows_ccorrect",
    "highlights_ccorrect",
    "flags",
    "low_approximation",
    "shadhi_algo",
];

pub(crate) fn compile_relight(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &RELIGHT_PARAMETERS)?;
    let config = RelightConfig::new(
        super::parameter_f32(operation, "ev", 0.33)?,
        super::parameter_f32(operation, "center", 0.0)?,
        super::parameter_f32(operation, "width", 4.0)?,
    )
    .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Relight { config },
    })
}

pub(crate) fn compile_shadhi(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &SHADHI_PARAMETERS)?;
    let order = small_u32(operation, "order", 0)?;
    let flags = small_u32(operation, "flags", 127)?;
    let algorithm = small_u32(operation, "shadhi_algo", 1)?;
    let parameters = ShadhiParametersV5 {
        order,
        radius: super::parameter_f32(operation, "radius", 100.0)?,
        shadows: super::parameter_f32(operation, "shadows", 50.0)?,
        whitepoint: super::parameter_f32(operation, "whitepoint", 0.0)?,
        highlights: super::parameter_f32(operation, "highlights", -50.0)?,
        reserved2: super::parameter_f32(operation, "reserved2", 0.0)?,
        compress: super::parameter_f32(operation, "compress", 50.0)?,
        shadows_ccorrect: super::parameter_f32(operation, "shadows_ccorrect", 100.0)?,
        highlights_ccorrect: super::parameter_f32(operation, "highlights_ccorrect", 50.0)?,
        flags,
        low_approximation: super::parameter_f32(operation, "low_approximation", 0.000_001)?,
        shadhi_algo: algorithm,
    };
    let config = ShadhiConfig::new(parameters)
        .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Shadhi { config },
    })
}

fn small_u32(
    operation: &Operation,
    name: &'static str,
    default: i32,
) -> Result<u32, OperationCompileError> {
    let value = super::parameter_integer(operation, name, f64::from(default))?;
    u32::try_from(value)
        .map_err(|_| super::invalid_parameters(operation, format!("{name} must be non-negative")))
}
