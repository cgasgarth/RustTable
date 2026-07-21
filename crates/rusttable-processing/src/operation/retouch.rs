use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};
use rusttable_core::{Operation, ParameterName, ParameterValue};

const ALLOWED: [&str; 3] = ["scales", "tile_edge", "memory_budget"];

pub(crate) fn compile_retouch(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    if let Some((parameter, _)) = operation
        .parameters()
        .find(|(name, _)| !ALLOWED.contains(&name.as_str()))
    {
        return Err(OperationCompileError::UnexpectedParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: parameter.clone(),
        });
    }
    let scales = integer(operation, "scales", 0)?;
    let tile_edge = integer(operation, "tile_edge", 256)?;
    let memory_budget = integer(operation, "memory_budget", 512 * 1024 * 1024)?;
    let scales = u8::try_from(scales).map_err(|_| invalid(operation, "scales must fit u8"))?;
    let tile_edge =
        u32::try_from(tile_edge).map_err(|_| invalid(operation, "tile_edge must fit u32"))?;
    let memory_budget = u64::try_from(memory_budget)
        .map_err(|_| invalid(operation, "memory_budget must be nonnegative"))?;
    if tile_edge == 0 || memory_budget == 0 {
        return Err(invalid(
            operation,
            "retouch resource values must be nonzero",
        ));
    }
    let opacity = super::compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::Retouch {
            config: crate::operations::retouch::RetouchParameters::new(scales)
                .with_tile_edge(tile_edge)
                .with_memory_budget(memory_budget),
        },
    })
}

fn integer(operation: &Operation, name: &str, default: i64) -> Result<i64, OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static parameter");
    match operation.parameter(&parameter) {
        None => Ok(default),
        Some(ParameterValue::Integer(value)) => Ok(*value),
        Some(_) => Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        }),
    }
}

fn invalid(operation: &Operation, reason: &str) -> OperationCompileError {
    OperationCompileError::InvalidParameters {
        operation_id: operation.id(),
        key: operation.key().clone(),
        reason: reason.to_owned(),
    }
}
