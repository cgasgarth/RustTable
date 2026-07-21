use rusttable_core::Operation;

use crate::operations::basicadj::{BasicAdjConfig, BasicAdjParametersV2};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const BASICADJ_PARAMETERS: [&str; 11] = [
    "black_point",
    "exposure",
    "hlcompr",
    "hlcomprthresh",
    "contrast",
    "preserve_colors",
    "middle_grey",
    "brightness",
    "saturation",
    "vibrance",
    "clip",
];

pub(crate) fn compile_basicadj(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &BASICADJ_PARAMETERS)?;
    let parameters = BasicAdjParametersV2 {
        black_point: super::parameter_f32(operation, "black_point", 0.0)?,
        exposure: super::parameter_f32(operation, "exposure", 0.0)?,
        hlcompr: super::parameter_f32(operation, "hlcompr", 0.0)?,
        hlcomprthresh: super::parameter_f32(operation, "hlcomprthresh", 0.0)?,
        contrast: super::parameter_f32(operation, "contrast", 0.0)?,
        preserve_colors: super::parameter_integer(operation, "preserve_colors", 1.0)?,
        middle_grey: super::parameter_f32(operation, "middle_grey", 18.42)?,
        brightness: super::parameter_f32(operation, "brightness", 0.0)?,
        saturation: super::parameter_f32(operation, "saturation", 0.0)?,
        vibrance: super::parameter_f32(operation, "vibrance", 0.0)?,
        clip: super::parameter_f32(operation, "clip", 0.0)?,
    };
    let config = BasicAdjConfig::new(parameters)
        .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::BasicAdj { config },
    })
}
