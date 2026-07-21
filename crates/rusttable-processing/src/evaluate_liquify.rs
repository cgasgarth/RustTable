use crate::operations::OperationExecutionError;
use crate::{FiniteF32, LinearRgb, PipelineStepIndex, RasterDimensions, RgbChannel};
use rusttable_core::OperationId;

use super::{EvaluationError, operation_error, operation_plan_error};

pub(super) fn apply_liquify(
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    config: &crate::operations::liquify::LiquifyConfig,
    pixels: &mut [LinearRgb],
    dimensions: RasterDimensions,
    opacity: f32,
) -> Result<(), EvaluationError> {
    let plan = crate::operations::liquify::LiquifyPlan::new(config.clone(), dimensions)
        .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
    let source = pixels.to_vec();
    let execution = plan
        .execute(&source, || false)
        .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
    if opacity.to_bits() == 1.0f32.to_bits() {
        pixels.copy_from_slice(execution.pixels());
        return Ok(());
    }
    for (pixel, warped) in pixels.iter_mut().zip(execution.pixels()) {
        let red = pixel.red().get() + (warped.red().get() - pixel.red().get()) * opacity;
        let green = pixel.green().get() + (warped.green().get() - pixel.green().get()) * opacity;
        let blue = pixel.blue().get() + (warped.blue().get() - pixel.blue().get()) * opacity;
        *pixel = LinearRgb::new(
            finite_channel(red, RgbChannel::Red, step_index, operation_id)?,
            finite_channel(green, RgbChannel::Green, step_index, operation_id)?,
            finite_channel(blue, RgbChannel::Blue, step_index, operation_id)?,
        );
    }
    Ok(())
}

fn finite_channel(
    value: f32,
    channel: RgbChannel,
    step_index: PipelineStepIndex,
    operation_id: OperationId,
) -> Result<FiniteF32, EvaluationError> {
    FiniteF32::new(value).map_err(|_| {
        operation_error(
            step_index,
            operation_id,
            OperationExecutionError::NonFiniteResult { pixel: 0, channel },
        )
    })
}
