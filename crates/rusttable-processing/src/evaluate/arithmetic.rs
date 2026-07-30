use crate::{
    BlendArithmeticStage, EvaluationError, FiniteF32, LinearRgb, PipelineStepIndex, RgbChannel,
};
use rusttable_core::OperationId;

pub(crate) fn apply_reconstruction(
    pixels: &mut [LinearRgb],
    candidates: &[LinearRgb],
    opacity: f32,
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    pixel_index_offset: usize,
) -> Result<(), EvaluationError> {
    for (local_index, (pixel, candidate)) in pixels.iter_mut().zip(candidates).enumerate() {
        let pixel_index = pixel_index_offset + local_index;
        if opacity.to_bits() == 1.0f32.to_bits() {
            *pixel = *candidate;
        } else {
            let current = *pixel;
            *pixel = LinearRgb::new(
                blend(
                    current.red().get(),
                    candidate.red().get(),
                    opacity,
                    step_index,
                    operation_id,
                    pixel_index,
                    RgbChannel::Red,
                )?,
                blend(
                    current.green().get(),
                    candidate.green().get(),
                    opacity,
                    step_index,
                    operation_id,
                    pixel_index,
                    RgbChannel::Green,
                )?,
                blend(
                    current.blue().get(),
                    candidate.blue().get(),
                    opacity,
                    step_index,
                    operation_id,
                    pixel_index,
                    RgbChannel::Blue,
                )?,
            );
        }
    }
    Ok(())
}

pub(crate) fn apply_channels<F>(
    pixels: &mut [LinearRgb],
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    opacity: f32,
    pixel_index_offset: usize,
    transform: F,
) -> Result<(), EvaluationError>
where
    F: Fn(RgbChannel, f32) -> f32,
{
    for (local_pixel_index, pixel) in pixels.iter_mut().enumerate() {
        let pixel_index = pixel_index_offset + local_pixel_index;
        let current = *pixel;
        let red_candidate = checked_channel(
            transform(RgbChannel::Red, current.red().get()),
            step_index,
            operation_id,
            pixel_index,
            RgbChannel::Red,
        )?;
        let green_candidate = checked_channel(
            transform(RgbChannel::Green, current.green().get()),
            step_index,
            operation_id,
            pixel_index,
            RgbChannel::Green,
        )?;
        let blue_candidate = checked_channel(
            transform(RgbChannel::Blue, current.blue().get()),
            step_index,
            operation_id,
            pixel_index,
            RgbChannel::Blue,
        )?;
        if opacity.to_bits() == 1.0f32.to_bits() {
            *pixel = LinearRgb::new(red_candidate, green_candidate, blue_candidate);
        } else {
            *pixel = LinearRgb::new(
                blend(
                    current.red().get(),
                    red_candidate.get(),
                    opacity,
                    step_index,
                    operation_id,
                    pixel_index,
                    RgbChannel::Red,
                )?,
                blend(
                    current.green().get(),
                    green_candidate.get(),
                    opacity,
                    step_index,
                    operation_id,
                    pixel_index,
                    RgbChannel::Green,
                )?,
                blend(
                    current.blue().get(),
                    blue_candidate.get(),
                    opacity,
                    step_index,
                    operation_id,
                    pixel_index,
                    RgbChannel::Blue,
                )?,
            );
        }
    }
    Ok(())
}

pub(crate) fn blend(
    current: f32,
    candidate: f32,
    opacity: f32,
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    pixel_index: usize,
    channel: RgbChannel,
) -> Result<FiniteF32, EvaluationError> {
    let delta =
        FiniteF32::new(candidate - current).map_err(|_| EvaluationError::NonFiniteBlendResult {
            step_index,
            operation_id,
            pixel_index,
            channel,
            stage: BlendArithmeticStage::Delta,
        })?;
    let weighted_delta = FiniteF32::new(delta.get() * opacity).map_err(|_| {
        EvaluationError::NonFiniteBlendResult {
            step_index,
            operation_id,
            pixel_index,
            channel,
            stage: BlendArithmeticStage::WeightedDelta,
        }
    })?;
    FiniteF32::new(current + weighted_delta.get()).map_err(|_| {
        EvaluationError::NonFiniteBlendResult {
            step_index,
            operation_id,
            pixel_index,
            channel,
            stage: BlendArithmeticStage::Output,
        }
    })
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
