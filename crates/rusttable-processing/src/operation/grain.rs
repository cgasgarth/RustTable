use rusttable_core::{Operation, ParameterName, ParameterValue};

use crate::operations::grain::{GrainChannel, GrainConfig, GrainParametersV2};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const GRAIN_PARAMETERS: [&str; 4] = ["channel", "scale", "strength", "midtones_bias"];

pub(crate) fn compile_grain(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &GRAIN_PARAMETERS)?;
    let channel = channel(operation)?;
    let parameters = GrainParametersV2::new(
        channel,
        super::parameter_f32(operation, "scale", 1600.0 / 213.2)?,
        super::parameter_f32(operation, "strength", 25.0)?,
        super::parameter_f32(operation, "midtones_bias", 100.0)?,
    );
    let id = operation.id().get();
    let seed = u64::try_from(id & u128::from(u64::MAX)).expect("masked operation ID fits")
        ^ u64::try_from(id >> 64).expect("shifted operation ID fits");
    let config = GrainConfig::new(parameters)
        .map_err(|error| super::invalid_parameters(operation, error))?
        .with_seed(seed);
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::Grain { config },
    })
}

fn channel(operation: &Operation) -> Result<GrainChannel, OperationCompileError> {
    let name = ParameterName::new("channel").expect("static processing parameter");
    let id = match operation.parameter(&name) {
        Some(ParameterValue::Text(value)) => match value.as_str() {
            "hue" => 0,
            "saturation" => 1,
            "lightness" => 2,
            "rgb" => 3,
            _ => {
                return Err(super::invalid_parameters(
                    operation,
                    "grain channel is unknown",
                ));
            }
        },
        Some(_) | None => super::parameter_integer(operation, "channel", 2.0)?,
    };
    GrainChannel::from_id(
        u32::try_from(id).map_err(|_| {
            super::invalid_parameters(operation, "grain channel must be non-negative")
        })?,
    )
    .map_err(|error| super::invalid_parameters(operation, error))
}
