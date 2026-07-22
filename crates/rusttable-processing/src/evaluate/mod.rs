use crate::operations::{
    OperationExecutionError,
    colorreconstruction::ColorReconstructionPlan,
    highlights::{HighlightsInputClass, HighlightsPlan},
};
use crate::{
    CompiledPipeline, FiniteF32, LinearRgb, OperationMaskSet, PipelineStepIndex,
    PreparedCpuOperation, ProcessingOperation, ProcessingOperationKind, RasterDimensions,
    RgbChannel, TerminalOutputFrame, WorkingFrameDescriptor, WorkingRgbImage,
};
use rusttable_core::OperationId;
use std::fmt;
mod arithmetic;
mod basicadj;
mod basicadj_runtime;
mod frame;
mod lab_boundary;
mod liquify;
mod mask;
mod output;
mod spots;
pub(super) use arithmetic::{apply_channels, apply_reconstruction, blend};
pub use basicadj::BasicAdjPlanSet;
pub use frame::{
    DistortionBorderMode, DistortionInterpolation, DistortionPlan, DistortionSamplingPolicy,
    EvaluatedFrame, FrameBoundaryMode, FrameBoundaryOptions, FrameBoundaryPlan,
    evaluate_graph_at_frame_boundaries, evaluate_graph_at_frame_boundaries_with_masks,
    graph_has_discrete_geometry, graph_has_frame_geometry,
};
pub(crate) use frame::{
    evaluate_graph_at_frame_boundaries_with_plans,
    evaluate_graph_at_frame_boundaries_with_plans_and_masks,
};
use lab_boundary::{apply_defringe, apply_relight, apply_shadhi};
use mask::{apply_mask_blend, validate_operation_mask};
pub use output::EvaluationOutput;
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluationError {
    InvalidExposureScale {
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
    TerminalOutputRequiresTypedPublication {
        encoding: rusttable_color::ColorEncoding,
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
    match evaluate_output(pipeline, input)? {
        EvaluationOutput::Working(output) => Ok(output),
        EvaluationOutput::Terminal(output) => {
            Err(EvaluationError::TerminalOutputRequiresTypedPublication {
                encoding: output.descriptor().encoding(),
            })
        }
    }
}

/// Evaluates a pipeline while preserving a terminal colorout as a typed
/// publication frame.
///
/// # Errors
///
/// Returns the first operation or terminal-publication error encountered
/// while evaluating the graph.
pub fn evaluate_output(
    pipeline: &CompiledPipeline,
    input: &WorkingRgbImage,
) -> Result<EvaluationOutput, EvaluationError> {
    let (output, frame, terminal) = evaluate_steps_with_frame(
        pipeline.steps().map(|step| (step.index(), step.prepared())),
        input.pixel_slice(),
        input.dimensions(),
        0,
        input.frame(),
        None,
    )?;
    Ok(terminal.map_or_else(
        || {
            EvaluationOutput::Working(WorkingRgbImage::from_validated_parts_with_frame(
                input.dimensions(),
                output,
                frame,
            ))
        },
        EvaluationOutput::Terminal,
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
    evaluate_steps_with_plans(steps, input, dimensions, pixel_index_offset, None)
}
pub(crate) fn evaluate_steps_with_plans<'a, I>(
    steps: I,
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
    basicadj_plans: Option<&BasicAdjPlanSet>,
) -> Result<Vec<LinearRgb>, EvaluationError>
where
    I: IntoIterator<Item = (PipelineStepIndex, &'a PreparedCpuOperation)>,
{
    let (pixels, _, terminal) = evaluate_steps_with_frame(
        steps,
        input,
        dimensions,
        pixel_index_offset,
        WorkingFrameDescriptor::srgb(),
        basicadj_plans,
    )?;
    if let Some(output) = terminal {
        return Err(EvaluationError::TerminalOutputRequiresTypedPublication {
            encoding: output.descriptor().encoding(),
        });
    }
    Ok(pixels)
}
pub(crate) fn evaluate_steps_with_frame<'a, I>(
    steps: I,
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
    frame: WorkingFrameDescriptor,
    basicadj_plans: Option<&BasicAdjPlanSet>,
) -> Result<
    (
        Vec<LinearRgb>,
        WorkingFrameDescriptor,
        Option<TerminalOutputFrame>,
    ),
    EvaluationError,
>
where
    I: IntoIterator<Item = (PipelineStepIndex, &'a PreparedCpuOperation)>,
{
    evaluate_steps_with_frame_and_masks(
        steps,
        input,
        dimensions,
        pixel_index_offset,
        frame,
        basicadj_plans,
        None,
    )
}

pub(crate) fn evaluate_steps_with_frame_and_masks<'a, I>(
    steps: I,
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
    mut frame: WorkingFrameDescriptor,
    basicadj_plans: Option<&BasicAdjPlanSet>,
    masks: Option<&OperationMaskSet>,
) -> Result<
    (
        Vec<LinearRgb>,
        WorkingFrameDescriptor,
        Option<TerminalOutputFrame>,
    ),
    EvaluationError,
>
where
    I: IntoIterator<Item = (PipelineStepIndex, &'a PreparedCpuOperation)>,
{
    let mut output = input.to_vec();
    let mut terminal = None;
    for (step_index, operation) in steps {
        apply_operation_with_profile(
            step_index,
            operation.operation(),
            &mut output,
            dimensions,
            pixel_index_offset,
            basicadj_plans,
            &mut frame,
            &mut terminal,
            masks,
        )?;
    }
    Ok((output, frame, terminal))
}
/// Resolves every automatic basicadj node against the full preceding image,
/// then executes the graph once to establish the next node's analysis input.
/// The returned set is reusable by every tile of that snapshot.
///
/// # Errors
///
/// Returns the first graph-operation or automatic-analysis failure.
pub use basicadj_runtime::prepare_basicadj_plans;
pub(crate) fn execute_prepared_operation(
    operation: &PreparedCpuOperation,
    step_index: PipelineStepIndex,
    pixels: &mut [LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
) -> Result<(), EvaluationError> {
    apply_operation_with_plans(
        step_index,
        operation.operation(),
        pixels,
        dimensions,
        pixel_index_offset,
        None,
    )
}
#[allow(clippy::too_many_lines)]
fn apply_operation_with_plans(
    step_index: PipelineStepIndex,
    operation: &ProcessingOperation,
    pixels: &mut [LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
    basicadj_plans: Option<&BasicAdjPlanSet>,
) -> Result<(), EvaluationError> {
    let mut frame = WorkingFrameDescriptor::srgb();
    apply_operation_with_profile(
        step_index,
        operation,
        pixels,
        dimensions,
        pixel_index_offset,
        basicadj_plans,
        &mut frame,
        &mut None,
        None,
    )
}
#[allow(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    reason = "the operation dispatcher keeps typed graph semantics centralized"
)]
pub(crate) fn apply_operation_with_profile(
    step_index: PipelineStepIndex,
    operation: &ProcessingOperation,
    pixels: &mut [LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
    basicadj_plans: Option<&BasicAdjPlanSet>,
    frame: &mut WorkingFrameDescriptor,
    terminal: &mut Option<TerminalOutputFrame>,
    masks: Option<&OperationMaskSet>,
) -> Result<(), EvaluationError> {
    let operation_id = operation.operation_id();
    let opacity = operation.opacity().get();
    if !operation.is_enabled() || opacity.to_bits() == 0.0f32.to_bits() {
        return Ok(());
    }
    let mask = masks.and_then(|set| set.mask_for(operation_id));
    if let Some(mask) = mask {
        validate_operation_mask(mask, pixels.len(), dimensions, step_index, operation_id)?;
    }
    let before_mask = mask.map(|_| pixels.to_vec());
    let result = match operation.kind() {
        ProcessingOperationKind::BasicAdj { config } => {
            let plan = basicadj_plans
                .and_then(|plans| plans.plan(operation_id))
                .cloned()
                .map_or_else(
                    || crate::operations::basicadj::BasicAdjPlan::new(*config),
                    Ok,
                )
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let candidate = plan
                .execute(pixels, pixel_index_offset)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::Exposure { stops, black } => {
            let white = (-stops.get()).exp2();
            let scale = 1.0 / (white - black.get());
            let scale =
                FiniteF32::new(scale).map_err(|_| EvaluationError::InvalidExposureScale {
                    step_index,
                    operation_id,
                })?;
            apply_channels(
                pixels,
                step_index,
                operation_id,
                opacity,
                pixel_index_offset,
                |_, value| (value - black.get()) * scale.get(),
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
        ProcessingOperationKind::Invert { config } => {
            let plan = crate::operations::invert::InvertPlan::new(*config, dimensions);
            let candidate = plan
                .execute(pixels)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::Dither { config } => {
            let plan = crate::operations::dither::DitherPlan::new(*config, dimensions);
            let candidate = plan
                .execute(pixels)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::Grain { config } => {
            let plan = crate::operations::grain::GrainPlan::new(*config, dimensions)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let candidate = plan
                .execute_window(pixels, pixel_index_offset)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::Censorize { config } => {
            let plan =
                crate::operations::censorize::CensorizePlan::new(*config, dimensions, 1.0, 1.0)
                    .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let rgba = pixels
                .iter()
                .copied()
                .map(|pixel| {
                    crate::operations::censorize::CensorizePixel::new(
                        pixel.red().get(),
                        pixel.green().get(),
                        pixel.blue().get(),
                        1.0,
                    )
                })
                .collect::<Vec<_>>();
            let candidate = plan
                .execute(&rgba, || false)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let candidate = candidate
                .into_iter()
                .enumerate()
                .map(|(index, pixel)| {
                    let channels = pixel.channels();
                    Ok(LinearRgb::new(
                        FiniteF32::new(channels[0]).map_err(|_| {
                            OperationExecutionError::NonFiniteResult {
                                pixel: index,
                                channel: RgbChannel::Red,
                            }
                        })?,
                        FiniteF32::new(channels[1]).map_err(|_| {
                            OperationExecutionError::NonFiniteResult {
                                pixel: index,
                                channel: RgbChannel::Green,
                            }
                        })?,
                        FiniteF32::new(channels[2]).map_err(|_| {
                            OperationExecutionError::NonFiniteResult {
                                pixel: index,
                                channel: RgbChannel::Blue,
                            }
                        })?,
                    ))
                })
                .collect::<Result<Vec<_>, OperationExecutionError>>()
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::Defringe { config } => {
            let candidate = apply_defringe(*config, pixels, dimensions, frame.encoding(), opacity)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::Clahe { config } => {
            let plan = crate::operations::clahe::ClahePlan::new(*config, dimensions, 1.0, 1.0)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let rgba = pixels
                .iter()
                .copied()
                .map(|pixel| {
                    crate::operations::clahe::ClahePixel::new(
                        pixel.red().get(),
                        pixel.green().get(),
                        pixel.blue().get(),
                        1.0,
                    )
                })
                .collect::<Vec<_>>();
            let candidate = plan
                .execute(&rgba, || false)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let candidate = candidate
                .into_iter()
                .enumerate()
                .map(|(index, pixel)| {
                    let channels = pixel.channels();
                    Ok(LinearRgb::new(
                        FiniteF32::new(channels[0]).map_err(|_| {
                            OperationExecutionError::NonFiniteResult {
                                pixel: index,
                                channel: RgbChannel::Red,
                            }
                        })?,
                        FiniteF32::new(channels[1]).map_err(|_| {
                            OperationExecutionError::NonFiniteResult {
                                pixel: index,
                                channel: RgbChannel::Green,
                            }
                        })?,
                        FiniteF32::new(channels[2]).map_err(|_| {
                            OperationExecutionError::NonFiniteResult {
                                pixel: index,
                                channel: RgbChannel::Blue,
                            }
                        })?,
                    ))
                })
                .collect::<Result<Vec<_>, OperationExecutionError>>()
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
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
        ProcessingOperationKind::Bloom { config } => {
            let plan = crate::operations::bloom::BloomPlan::new(*config, dimensions)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            let candidate = plan
                .execute(pixels, dimensions)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::Soften { config } => {
            let plan = crate::operations::soften::SoftenPlan::new(*config, dimensions)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            let candidate = plan
                .execute(pixels, dimensions)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::Relight { config } => {
            let candidate =
                apply_relight(*config, pixels, dimensions, frame.encoding(), opacity)
                    .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::Shadhi { config } => {
            let candidate = apply_shadhi(*config, pixels, dimensions, frame.encoding(), opacity)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::Vignette { config } => {
            let seed = u64::try_from(operation_id.get() & u128::from(u64::MAX))
                .expect("masked operation ID fits")
                ^ u64::try_from(operation_id.get() >> 64).expect("shifted operation ID fits");
            let plan = crate::operations::vignette::VignettePlan::new(*config, dimensions)
                .map_err(|error| operation_error(step_index, operation_id, error))?
                .with_seed(seed);
            let candidate = plan
                .execute_window(pixels, pixel_index_offset)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::GraduatedNd { config } => {
            let plan = crate::operations::graduatednd::GraduatedNdPlan::new(*config, dimensions)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            let candidate = plan
                .execute_window(pixels, pixel_index_offset)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
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
            )?;
            *frame = plan.output_frame();
            Ok(())
        }
        ProcessingOperationKind::Primaries { config } => {
            let plan = crate::operations::primaries::PrimariesPlan::new(*config, frame.primaries())
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
            if terminal.is_some() {
                return Err(EvaluationError::TerminalOutputRequiresTypedPublication {
                    encoding: rusttable_color::ColorEncoding::Unspecified,
                });
            }
            let plan = crate::operations::colorout::ColorOutPlan::new_with_working_frame(
                config.clone(),
                *frame,
            )
            .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let execution = plan
                .execute(pixels)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            pixels.copy_from_slice(execution.pixels());
            *terminal = Some(execution.terminal_output().clone());
            Ok(())
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
        ProcessingOperationKind::MaskManager { .. }
        | ProcessingOperationKind::RasterFile { .. } => Ok(()),
        ProcessingOperationKind::Retouch { config } => {
            let plan = crate::RetouchPlan::new(config.config(), dimensions).map_err(|error| {
                EvaluationError::OperationExecution {
                    step_index,
                    operation_id,
                    reason: error.to_string(),
                }
            })?;
            plan.execute_linear_rgb(pixels, || false, |_| {})
                .map(|_| ())
                .map_err(|error| EvaluationError::OperationExecution {
                    step_index,
                    operation_id,
                    reason: error.to_string(),
                })
        }
        ProcessingOperationKind::Spots { parameters } => {
            spots::apply_spots(step_index, operation_id, parameters, pixels, dimensions)
        }
        ProcessingOperationKind::Liquify { config } => liquify::apply_liquify(
            step_index,
            operation_id,
            config,
            pixels,
            dimensions,
            opacity,
        ),
        ProcessingOperationKind::Crop { .. }
        | ProcessingOperationKind::Flip { .. }
        | ProcessingOperationKind::RotatePixels { .. }
        | ProcessingOperationKind::ScalePixels { .. }
        | ProcessingOperationKind::FinalScale { .. }
        | ProcessingOperationKind::EnlargeCanvas { .. }
        | ProcessingOperationKind::Perspective { .. }
        | ProcessingOperationKind::Clipping { .. }
        | ProcessingOperationKind::LensCorrection { .. } => Err(operation_error(
            step_index,
            operation_id,
            OperationExecutionError::GeometryRequiresFrameBoundary,
        )),
    };
    result?;
    if let (Some(mask), Some(before)) = (mask, before_mask.as_deref()) {
        apply_mask_blend(
            pixels,
            before,
            mask,
            step_index,
            operation_id,
            pixel_index_offset,
        )?;
    }
    Ok(())
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
impl fmt::Display for EvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExposureScale {
                step_index,
                operation_id,
            } => write!(
                formatter,
                "operation {operation_id} at pipeline step {} has an invalid exposure scale",
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
            Self::TerminalOutputRequiresTypedPublication { encoding } => write!(
                formatter,
                "terminal colorout output {encoding:?} requires typed publication"
            ),
        }
    }
}

impl std::error::Error for EvaluationError {}
#[cfg(test)]
mod tests;
