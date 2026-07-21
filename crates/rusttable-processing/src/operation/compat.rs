use rusttable_core::Operation;

use crate::operations::{
    dither::{DitherConfig, DitherMethod},
    invert::InvertConfig,
};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const INVERT_PARAMETERS: [&str; 4] = ["red", "green", "blue", "four"];
const DITHER_PARAMETERS: [&str; 8] = [
    "method", "palette", "radius", "range0", "range1", "range2", "range3", "damping",
];

pub(crate) fn compile_invert(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &INVERT_PARAMETERS)?;
    let color = [
        super::parameter_f32(operation, "red", 1.0)?,
        super::parameter_f32(operation, "green", 1.0)?,
        super::parameter_f32(operation, "blue", 1.0)?,
        super::parameter_f32(operation, "four", 1.0)?,
    ];
    let config = InvertConfig::new(color, [1.0; 4])
        .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Invert { config },
    })
}

pub(crate) fn compile_dither(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &DITHER_PARAMETERS)?;
    let method_id = super::parameter_integer(operation, "method", 5.0)?;
    let method = DitherMethod::from_id(u32::try_from(method_id).map_err(|_| {
        super::invalid_parameters(operation, "method must be a known non-negative integer")
    })?)
    .map_err(|error| super::invalid_parameters(operation, error))?;
    let damping = super::parameter_f32(operation, "damping", -200.0)?;
    let id = operation.id().get();
    let seed = u64::try_from(id & u128::from(u64::MAX)).expect("masked operation ID fits")
        ^ u64::try_from(id >> 64).expect("shifted operation ID fits");
    let config = DitherConfig::new(method, damping)
        .map_err(|error| super::invalid_parameters(operation, error))?
        .with_seed(seed);
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Dither { config },
    })
}
