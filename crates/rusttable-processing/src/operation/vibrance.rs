use rusttable_core::Operation;

use crate::operations::vibrance::{VIBRANCE_DEFAULT_AMOUNT, VibranceConfig};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const VIBRANCE_PARAMETERS: [&str; 1] = ["amount"];

pub(crate) fn compile_vibrance(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &VIBRANCE_PARAMETERS)?;
    let config = VibranceConfig::new(super::parameter_f32(
        operation,
        "amount",
        f64::from(VIBRANCE_DEFAULT_AMOUNT),
    )?)
    .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Vibrance { config },
    })
}
