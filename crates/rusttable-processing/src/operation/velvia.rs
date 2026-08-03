use rusttable_core::Operation;

use crate::operations::velvia::{VELVIA_DEFAULT_BIAS, VELVIA_DEFAULT_STRENGTH, VelviaConfig};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const VELVIA_PARAMETERS: [&str; 2] = ["strength", "bias"];

pub fn compile_velvia(operation: &Operation) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &VELVIA_PARAMETERS)?;
    let config = VelviaConfig::new(
        super::parameter_f32(operation, "strength", f64::from(VELVIA_DEFAULT_STRENGTH))?,
        super::parameter_f32(operation, "bias", f64::from(VELVIA_DEFAULT_BIAS))?,
    )
    .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Velvia { config },
    })
}
