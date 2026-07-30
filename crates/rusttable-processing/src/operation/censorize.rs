use rusttable_core::Operation;

use crate::operations::censorize::CensorizeConfig;
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const PARAMETERS: [&str; 4] = ["radius_1", "pixelate", "radius_2", "noise"];

pub(crate) fn compile_censorize(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &PARAMETERS)?;
    let config = CensorizeConfig::new(
        super::parameter_f32(operation, "radius_1", 0.0)?,
        super::parameter_f32(operation, "pixelate", 0.0)?,
        super::parameter_f32(operation, "radius_2", 0.0)?,
        super::parameter_f32(operation, "noise", 0.0)?,
    )
    .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Censorize { config },
    })
}
