use rusttable_core::Operation;

use crate::operations::clahe::ClaheConfig;
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const PARAMETERS: [&str; 2] = ["radius", "slope"];

pub(crate) fn compile_clahe(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &PARAMETERS)?;
    let config = ClaheConfig::new(
        super::parameter_f64(operation, "radius", 64.0)?,
        super::parameter_f64(operation, "slope", 1.25)?,
    )
    .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Clahe { config },
    })
}
