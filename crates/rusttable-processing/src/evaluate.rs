use std::fmt;

use crate::{
    CompiledPipeline, FiniteF32, LinearRgb, PipelineStep, PipelineStepIndex,
    ProcessingOperationKind, RgbChannel, WorkingRgbImage,
};
use rusttable_core::OperationId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationError {
    NonFiniteExposureMultiplier {
        step_index: PipelineStepIndex,
        operation_id: OperationId,
    },
    NonFiniteChannelResult {
        step_index: PipelineStepIndex,
        operation_id: OperationId,
        pixel_index: usize,
        channel: RgbChannel,
    },
}

/// Evaluates a compiled pipeline into a new linear-light sRGB image.
///
/// The input image and compiled pipeline remain unchanged. Working values are
/// kept linear-light and are not clipped, quantized, or labeled scene-referred.
///
/// # Errors
///
/// Returns deterministic step, pixel, and channel context when an arithmetic
/// result leaves the finite working-image domain.
pub fn evaluate(
    pipeline: &CompiledPipeline,
    input: &WorkingRgbImage,
) -> Result<WorkingRgbImage, EvaluationError> {
    let mut output = input.pixel_slice().to_vec();
    for step in pipeline.active_steps() {
        apply_step(step, &mut output)?;
    }
    Ok(WorkingRgbImage::from_validated_parts(
        input.dimensions(),
        output,
    ))
}

fn apply_step(step: &PipelineStep, pixels: &mut [LinearRgb]) -> Result<(), EvaluationError> {
    let step_index = step.index();
    let operation_id = step.operation().operation_id();
    match step.operation().kind() {
        ProcessingOperationKind::Exposure { stops } => {
            let multiplier = FiniteF32::new(stops.get().exp2()).map_err(|_| {
                EvaluationError::NonFiniteExposureMultiplier {
                    step_index,
                    operation_id,
                }
            })?;
            apply_channels(pixels, step_index, operation_id, |value| {
                value * multiplier.get()
            })
        }
        ProcessingOperationKind::LinearOffset { value } => {
            apply_channels(pixels, step_index, operation_id, |sample| {
                sample + value.get()
            })
        }
    }
}

fn apply_channels<F>(
    pixels: &mut [LinearRgb],
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    transform: F,
) -> Result<(), EvaluationError>
where
    F: Fn(f32) -> f32,
{
    for (pixel_index, pixel) in pixels.iter_mut().enumerate() {
        let red = checked_channel(
            transform(pixel.red().get()),
            step_index,
            operation_id,
            pixel_index,
            RgbChannel::Red,
        )?;
        let green = checked_channel(
            transform(pixel.green().get()),
            step_index,
            operation_id,
            pixel_index,
            RgbChannel::Green,
        )?;
        let blue = checked_channel(
            transform(pixel.blue().get()),
            step_index,
            operation_id,
            pixel_index,
            RgbChannel::Blue,
        )?;
        *pixel = LinearRgb::new(red, green, blue);
    }
    Ok(())
}

fn checked_channel(
    value: f32,
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    pixel_index: usize,
    channel: RgbChannel,
) -> Result<FiniteF32, EvaluationError> {
    FiniteF32::new(value).map_err(|_| EvaluationError::NonFiniteChannelResult {
        step_index,
        operation_id,
        pixel_index,
        channel,
    })
}

impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteExposureMultiplier {
                step_index,
                operation_id,
            } => write!(
                formatter,
                "operation {operation_id} at pipeline step {} has a non-finite exposure multiplier",
                step_index.get()
            ),
            Self::NonFiniteChannelResult {
                step_index,
                operation_id,
                pixel_index,
                channel,
            } => write!(
                formatter,
                "operation {operation_id} at pipeline step {} produced a non-finite {channel:?} value at pixel {pixel_index}",
                step_index.get()
            ),
        }
    }
}

impl std::error::Error for EvaluationError {}

#[cfg(test)]
mod tests {
    use rusttable_core::{
        Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName,
        ParameterValue, PhotoId, Revision,
    };

    use super::*;

    #[test]
    fn reuses_single_output_slice_across_steps() {
        let operations = [
            operation(1, "rusttable.linear_offset", "value", 0.25),
            operation(2, "rusttable.exposure", "stops", 1.0),
        ];
        let edit = Edit::new(
            EditId::new(1).expect("nonzero edit ID"),
            PhotoId::new(2).expect("nonzero photo ID"),
            Revision::ZERO,
            operations,
        )
        .expect("valid edit");
        let pipeline = CompiledPipeline::compile(&edit).expect("valid pipeline");
        let mut pixels = vec![LinearRgb::new(
            FiniteF32::new(0.5).expect("finite"),
            FiniteF32::new(0.5).expect("finite"),
            FiniteF32::new(0.5).expect("finite"),
        )];
        let pointer = pixels.as_ptr();
        let capacity = pixels.capacity();

        for step in pipeline.active_steps() {
            apply_step(step, &mut pixels).expect("finite operation");
        }

        assert_eq!(pixels.as_ptr(), pointer);
        assert_eq!(pixels.capacity(), capacity);
    }

    fn operation(id: u128, key: &str, parameter: &str, value: f64) -> Operation {
        Operation::new(
            OperationId::new(id).expect("nonzero operation ID"),
            OperationKey::new(key).expect("valid operation key"),
            true,
            [(
                ParameterName::new(parameter).expect("valid parameter name"),
                ParameterValue::Scalar(FiniteF64::new(value).expect("finite")),
            )],
        )
        .expect("valid operation")
    }
}
