use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};
use rusttable_core::Operation;

pub(crate) fn compile_mask_manager(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    if operation.parameters().next().is_some() {
        return Err(OperationCompileError::InvalidParameters {
            operation_id: operation.id(),
            key: operation.key().clone(),
            reason: "mask_manager historical bytes are carried by the immutable stack boundary"
                .to_owned(),
        });
    }
    let opacity = super::compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::MaskManager {
            config: crate::operations::mask_manager::MaskManagerParameters::new(0),
        },
    })
}
