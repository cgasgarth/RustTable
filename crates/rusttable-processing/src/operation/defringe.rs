use rusttable_core::Operation;

use crate::operations::defringe::{DefringeConfig, DefringeMode};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const DEFRINGE_PARAMETERS: [&str; 3] = ["radius", "threshold", "mode"];

pub(crate) fn compile_defringe(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &DEFRINGE_PARAMETERS)?;
    let radius = super::parameter_f32(operation, "radius", 4.0)?;
    let threshold = super::parameter_f32(operation, "threshold", 20.0)?;
    let mode_id = super::parameter_integer(operation, "mode", 0.0)?;
    let mode = u32::try_from(mode_id)
        .ok()
        .and_then(|id| DefringeMode::try_from(id).ok())
        .ok_or_else(|| super::invalid_parameters(operation, "mode must be 0, 1, or 2"))?;
    let config = DefringeConfig::new(radius, threshold, mode)
        .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Defringe { config },
    })
}
