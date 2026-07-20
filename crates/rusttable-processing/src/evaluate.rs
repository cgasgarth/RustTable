use std::fmt;

use crate::operations::{
    OperationExecutionError,
    colorreconstruction::ColorReconstructionPlan,
    highlights::{HighlightsInputClass, HighlightsPlan},
};
use crate::{
    CompiledPipeline, FiniteF32, LinearRgb, PipelineStepIndex, PreparedCpuOperation,
    ProcessingOperation, ProcessingOperationKind, RasterDimensions, RgbChannel, WorkingRgbImage,
};
use rusttable_core::OperationId;

#[derive(Debug, Clone, PartialEq, Eq)]
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
    NonFiniteBlendResult {
        step_index: PipelineStepIndex,
        operation_id: OperationId,
        pixel_index: usize,
        channel: RgbChannel,
        stage: BlendArithmeticStage,
    },
    OperationExecution {
        step_index: PipelineStepIndex,
        operation_id: OperationId,
        reason: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendArithmeticStage {
    Delta,
    WeightedDelta,
    Output,
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
    let output = evaluate_steps(
        pipeline.steps().map(|step| (step.index(), step.prepared())),
        input.pixel_slice(),
        input.dimensions(),
        0,
    )?;
    Ok(WorkingRgbImage::from_validated_parts(
        input.dimensions(),
        output,
    ))
}

pub(crate) fn evaluate_steps<'a, I>(
    steps: I,
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
) -> Result<Vec<LinearRgb>, EvaluationError>
where
    I: IntoIterator<Item = (PipelineStepIndex, &'a PreparedCpuOperation)>,
{
    let mut output = input.to_vec();
    for (step_index, operation) in steps {
        operation.execute(step_index, &mut output, dimensions, pixel_index_offset)?;
    }
    Ok(output)
}

pub(crate) fn execute_prepared_operation(
    operation: &PreparedCpuOperation,
    step_index: PipelineStepIndex,
    pixels: &mut [LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
) -> Result<(), EvaluationError> {
    apply_operation(
        step_index,
        operation.operation(),
        pixels,
        dimensions,
        pixel_index_offset,
    )
}

#[allow(clippy::too_many_lines)]
fn apply_operation(
    step_index: PipelineStepIndex,
    operation: &ProcessingOperation,
    pixels: &mut [LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
) -> Result<(), EvaluationError> {
    let operation_id = operation.operation_id();
    let opacity = operation.opacity().get();
    if !operation.is_enabled() || opacity.to_bits() == 0.0f32.to_bits() {
        return Ok(());
    }
    match operation.kind() {
        ProcessingOperationKind::Exposure { stops } => {
            let multiplier = FiniteF32::new(stops.get().exp2()).map_err(|_| {
                EvaluationError::NonFiniteExposureMultiplier {
                    step_index,
                    operation_id,
                }
            })?;
            apply_channels(
                pixels,
                step_index,
                operation_id,
                opacity,
                pixel_index_offset,
                |_, value| value * multiplier.get(),
            )
        }
        ProcessingOperationKind::LinearOffset { value } => apply_channels(
            pixels,
            step_index,
            operation_id,
            opacity,
            pixel_index_offset,
            |_, sample| sample + value.get(),
        ),
        ProcessingOperationKind::RgbGain { red, green, blue } => apply_channels(
            pixels,
            step_index,
            operation_id,
            opacity,
            pixel_index_offset,
            |channel, value| {
                let gain = match channel {
                    RgbChannel::Red => red,
                    RgbChannel::Green => green,
                    RgbChannel::Blue => blue,
                };
                value * gain.get()
            },
        ),
        ProcessingOperationKind::Temperature { config } => {
            let multipliers = config.multipliers();
            apply_channels(
                pixels,
                step_index,
                operation_id,
                opacity,
                pixel_index_offset,
                |channel, value| {
                    let multiplier = match channel {
                        RgbChannel::Red => multipliers.red(),
                        RgbChannel::Green => multipliers.green(),
                        RgbChannel::Blue => multipliers.blue(),
                    };
                    value * multiplier.get()
                },
            )
        }
        ProcessingOperationKind::Highlights { config } => {
            let plan = HighlightsPlan::new(
                *config,
                dimensions,
                HighlightsInputClass::Rgb,
                crate::operations::ReconstructionBudget::default(),
            )
            .map_err(|error| operation_error(step_index, operation_id, error))?;
            let execution = plan
                .execute(pixels)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                execution.pixels(),
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::ColorReconstruction { config } => {
            let plan = ColorReconstructionPlan::new(
                *config,
                dimensions,
                crate::operations::ReconstructionBudget::default(),
            )
            .map_err(|error| operation_error(step_index, operation_id, error))?;
            let execution = plan
                .execute(pixels)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                execution.pixels(),
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::ColorIn { config } => {
            let plan = crate::operations::colorin::ColorInPlan::new(config.clone())
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let execution = plan
                .execute(pixels)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                execution.pixels(),
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::Primaries { config } => {
            let plan = crate::operations::primaries::PrimariesPlan::new(
                *config,
                rusttable_color::Primaries::srgb(),
            )
            .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let execution = plan
                .execute(pixels)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                execution.pixels(),
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::ColorOut { config } => {
            let plan = crate::operations::colorout::ColorOutPlan::new(config.clone())
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let execution = plan
                .execute(pixels)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                execution.pixels(),
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::ColorCorrection { config } => {
            let plan = crate::operations::colorcorrection::ColorCorrectionPlan::new(*config)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let execution = plan
                .execute(pixels)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                execution.pixels(),
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::Crop { .. }
        | ProcessingOperationKind::Flip { .. }
        | ProcessingOperationKind::RotatePixels { .. }
        | ProcessingOperationKind::ScalePixels { .. } => Err(operation_error(
            step_index,
            operation_id,
            OperationExecutionError::GeometryRequiresFrameBoundary,
        )),
    }
}

fn operation_error(
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    error: OperationExecutionError,
) -> EvaluationError {
    EvaluationError::OperationExecution {
        step_index,
        operation_id,
        reason: error.to_string(),
    }
}

fn operation_plan_error<E: fmt::Display>(
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    error: E,
) -> EvaluationError {
    EvaluationError::OperationExecution {
        step_index,
        operation_id,
        reason: error.to_string(),
    }
}

fn apply_reconstruction(
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

fn apply_channels<F>(
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
        if opacity.to_bits() == 1.0f32.to_bits() {
            let red = checked_channel(
                transform(RgbChannel::Red, current.red().get()),
                step_index,
                operation_id,
                pixel_index,
                RgbChannel::Red,
            )?;
            let green = checked_channel(
                transform(RgbChannel::Green, current.green().get()),
                step_index,
                operation_id,
                pixel_index,
                RgbChannel::Green,
            )?;
            let blue = checked_channel(
                transform(RgbChannel::Blue, current.blue().get()),
                step_index,
                operation_id,
                pixel_index,
                RgbChannel::Blue,
            )?;
            *pixel = LinearRgb::new(red, green, blue);
            continue;
        }
        let red_candidate = checked_channel(
            transform(RgbChannel::Red, current.red().get()),
            step_index,
            operation_id,
            pixel_index,
            RgbChannel::Red,
        )?;
        let red = blend(
            current.red().get(),
            red_candidate.get(),
            opacity,
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
        let green = blend(
            current.green().get(),
            green_candidate.get(),
            opacity,
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
        let blue = blend(
            current.blue().get(),
            blue_candidate.get(),
            opacity,
            step_index,
            operation_id,
            pixel_index,
            RgbChannel::Blue,
        )?;
        *pixel = LinearRgb::new(red, green, blue);
    }
    Ok(())
}

fn blend(
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
            Self::NonFiniteBlendResult {
                step_index,
                operation_id,
                pixel_index,
                channel,
                stage,
            } => write!(
                formatter,
                "operation {operation_id} at pipeline step {} produced a non-finite {stage:?} blend value for {channel:?} at pixel {pixel_index}",
                step_index.get()
            ),
            Self::OperationExecution {
                step_index,
                operation_id,
                reason,
            } => write!(
                formatter,
                "operation {operation_id} at pipeline step {} failed during reconstruction: {reason}",
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
            apply_operation(
                step.index(),
                step.operation(),
                &mut pixels,
                RasterDimensions::new(1, 1).expect("test dimensions"),
                0,
            )
            .expect("finite operation");
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
