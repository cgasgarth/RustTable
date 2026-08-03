use rusttable_core::{Operation, ParameterName, ParameterValue};

use crate::operations::channelmixer::{
    CHANNEL_MIXER_OUTPUT_COUNT, ChannelMixerAlgorithm, ChannelMixerConfig,
};
use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};

const CHANNEL_MIXER_PARAMETERS: [&str; 4] = ["red", "green", "blue", "algorithm_version"];

pub fn compile_channelmixer(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    super::reject_unexpected(operation, &CHANNEL_MIXER_PARAMETERS)?;
    let defaults = ChannelMixerConfig::defaults();
    let red = parameter_array(operation, "red", defaults.red())?;
    let green = parameter_array(operation, "green", defaults.green())?;
    let blue = parameter_array(operation, "blue", defaults.blue())?;
    let algorithm_version = parameter_algorithm(operation)?;
    let config = ChannelMixerConfig::new(red, green, blue, algorithm_version)
        .map_err(|error| super::invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: super::compile_opacity(operation)?,
        kind: ProcessingOperationKind::ChannelMixer { config },
    })
}

fn parameter_array(
    operation: &Operation,
    name: &'static str,
    default: [f32; CHANNEL_MIXER_OUTPUT_COUNT],
) -> Result<[f32; CHANNEL_MIXER_OUTPUT_COUNT], OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    let Some(value) = operation.parameter(&parameter) else {
        return Ok(default);
    };
    let ParameterValue::Text(value) = value else {
        return Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        });
    };
    let text = value.as_str().trim();
    let Some(text) = text
        .strip_prefix('[')
        .and_then(|text| text.strip_suffix(']'))
    else {
        return Err(super::invalid_parameters(
            operation,
            format!("{name} must be a seven-element vector"),
        ));
    };
    let values = text
        .split(',')
        .map(str::trim)
        .map(|value| {
            value.parse::<f64>().map_err(|_| {
                super::invalid_parameters(operation, format!("{name} contains a non-numeric value"))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.len() != CHANNEL_MIXER_OUTPUT_COUNT {
        return Err(super::invalid_parameters(
            operation,
            format!("{name} must have exactly {CHANNEL_MIXER_OUTPUT_COUNT} values"),
        ));
    }
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "Native Channel Mixer parameters are committed as f32."
            )]
            let value = value as f32;
            if !value.is_finite() {
                return Err(super::invalid_parameters(
                    operation,
                    format!("{name}[{index}] must be finite"),
                ));
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, _>>()?
        .try_into()
        .map_err(|_| super::invalid_parameters(operation, format!("{name} has an invalid shape")))
}

fn parameter_algorithm(
    operation: &Operation,
) -> Result<ChannelMixerAlgorithm, OperationCompileError> {
    let parameter = ParameterName::new("algorithm_version").expect("static processing parameter");
    let value = match operation.parameter(&parameter) {
        None => 1,
        Some(ParameterValue::Integer(value)) => i32::try_from(*value).map_err(|_| {
            super::invalid_parameters(operation, "algorithm_version is not a 32-bit integer")
        })?,
        Some(_) => {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            });
        }
    };
    ChannelMixerAlgorithm::try_from(value)
        .map_err(|error| super::invalid_parameters(operation, error))
}
