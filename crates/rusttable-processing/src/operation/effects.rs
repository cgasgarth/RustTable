use rusttable_core::Operation;

use super::error::compile_opacity;
use super::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};
use crate::operations::{bloom::BloomConfig, soften::SoftenConfig};

const BLOOM_PARAMETERS: [&str; 3] = ["size", "threshold", "strength"];
const SOFTEN_PARAMETERS: [&str; 4] = ["size", "saturation", "brightness", "amount"];

pub(crate) fn compile_bloom(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &BLOOM_PARAMETERS)?;
    let config = BloomConfig::new(
        super::parameter_f32(operation, "size", 20.0)?,
        super::parameter_f32(operation, "threshold", 90.0)?,
        super::parameter_f32(operation, "strength", 25.0)?,
    )
    .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::Bloom { config },
    })
}

pub(crate) fn compile_soften(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &SOFTEN_PARAMETERS)?;
    let config = SoftenConfig::new(
        super::parameter_f32(operation, "size", 50.0)?,
        super::parameter_f32(operation, "saturation", 100.0)?,
        super::parameter_f32(operation, "brightness", 0.33)?,
        super::parameter_f32(operation, "amount", 50.0)?,
    )
    .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::Soften { config },
    })
}
