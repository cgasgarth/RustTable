use rusttable_core::Operation;

use crate::operations::levels::{LevelsConfig, LevelsMode, LevelsParametersV2};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const PARAMETERS: [&str; 5] = ["mode", "black", "gray", "white", "levels"];

pub fn compile_levels(operation: &Operation) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &PARAMETERS)?;
    let defaults = LevelsParametersV2::defaults();
    let mode = LevelsMode::from_raw(super::parameter_integer(
        operation,
        "mode",
        f64::from(defaults.mode.raw()),
    )?)
    .map_err(|error| super::invalid_parameters(operation, error))?;
    let parameters = LevelsParametersV2::new(
        mode,
        super::parameter_f32(operation, "black", f64::from(defaults.black))?,
        super::parameter_f32(operation, "gray", f64::from(defaults.gray))?,
        super::parameter_f32(operation, "white", f64::from(defaults.white))?,
        super::parameter_f32_array(operation, "levels", defaults.levels)?,
    );
    let config = LevelsConfig::new(parameters)
        .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Levels { config },
    })
}
