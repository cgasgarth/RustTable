use rusttable_core::Operation;

use crate::operations::rgblevels::{
    RgbLevelsAutoscale, RgbLevelsConfig, RgbLevelsParametersV1, RgbLevelsPreserveColors,
};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const PARAMETERS: [&str; 3] = ["autoscale", "preserve_colors", "levels"];

pub fn compile_rgblevels(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &PARAMETERS)?;
    let defaults = RgbLevelsParametersV1::defaults();
    let autoscale = RgbLevelsAutoscale::try_from(super::parameter_integer(
        operation,
        "autoscale",
        f64::from(i32::from(defaults.autoscale)),
    )?)
    .map_err(|error| super::invalid_parameters(operation, error))?;
    let preserve_colors = RgbLevelsPreserveColors::try_from(super::parameter_integer(
        operation,
        "preserve_colors",
        f64::from(i32::from(defaults.preserve_colors)),
    )?)
    .map_err(|error| super::invalid_parameters(operation, error))?;
    let default_levels: [f32; 9] = defaults
        .levels
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .try_into()
        .expect("native RGB Levels has nine level values");
    let flat = super::parameter_f32_array(operation, "levels", default_levels)?;
    let levels = [
        [flat[0], flat[1], flat[2]],
        [flat[3], flat[4], flat[5]],
        [flat[6], flat[7], flat[8]],
    ];
    let config = RgbLevelsConfig::new(RgbLevelsParametersV1::new(
        autoscale,
        preserve_colors,
        levels,
    ))
    .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::RgbLevels { config },
    })
}
