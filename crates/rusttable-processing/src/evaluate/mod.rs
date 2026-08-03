use crate::operations::{
    OperationExecutionError,
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
pub use arithmetic::{apply_channels, apply_reconstruction, blend};
pub use basicadj::BasicAdjPlanSet;
pub use frame::{
    DistortionBorderMode, DistortionInterpolation, DistortionPlan, DistortionSamplingPolicy,
    EvaluatedFrame, FrameBoundaryMode, FrameBoundaryOptions, FrameBoundaryPlan,
    evaluate_graph_at_frame_boundaries, evaluate_graph_at_frame_boundaries_with_masks,
    graph_has_discrete_geometry, graph_has_frame_geometry,
};
pub use frame::{
    evaluate_graph_at_frame_boundaries_with_plans,
    evaluate_graph_at_frame_boundaries_with_plans_and_masks,
};
pub use lab_boundary::{
    ShadhiBilateralBoundaryError, ShadhiBilateralEvaluationError, evaluate_bilateral_shadhi_with,
    evaluate_bilateral_shadhi_with_cancellation,
};
use lab_boundary::{
    apply_bloom_with_cancellation, apply_colorcontrast, apply_colorcorrection,
    apply_colormapping_with_cancellation, apply_colorreconstruction_with_cancellation,
    apply_colortransfer_with_cancellation, apply_colorzones, apply_defringe,
    apply_levels_with_cancellation, apply_relight_with_cancellation,
    apply_shadhi_with_cancellation, apply_vibrance,
};
use mask::{OperationMaskRoute, apply_mask_blend, validate_operation_mask};
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
    Cancelled {
        step_index: PipelineStepIndex,
        operation_id: OperationId,
    },
    TerminalOutputRequiresTypedPublication {
        encoding: rusttable_color::ColorEncoding,
    },
}

impl EvaluationError {
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled { .. })
    }
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
pub fn evaluate_steps<'a, I>(
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
pub fn evaluate_steps_with_plans<'a, I>(
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
pub fn evaluate_steps_with_frame<'a, I>(
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

pub fn evaluate_steps_with_frame_and_masks<'a, I>(
    steps: I,
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
    frame: WorkingFrameDescriptor,
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
    evaluate_steps_with_frame_and_masks_with_cancellation(
        steps,
        input,
        dimensions,
        pixel_index_offset,
        frame,
        basicadj_plans,
        masks,
        || false,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "the canonical graph boundary carries explicit frame, mask, plan, and cancellation evidence"
)]
pub fn evaluate_steps_with_frame_and_masks_with_cancellation<'a, I, C>(
    steps: I,
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
    mut frame: WorkingFrameDescriptor,
    basicadj_plans: Option<&BasicAdjPlanSet>,
    masks: Option<&OperationMaskSet>,
    cancelled: C,
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
    C: Fn() -> bool,
{
    let mut output = input.to_vec();
    let mut terminal = None;
    for (step_index, operation) in steps {
        apply_operation_with_profile_with_cancellation(
            step_index,
            operation.operation(),
            &mut output,
            dimensions,
            pixel_index_offset,
            basicadj_plans,
            &mut frame,
            &mut terminal,
            masks,
            &cancelled,
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
pub use basicadj_runtime::{prepare_basicadj_plans, prepare_basicadj_plans_with_cancellation};
pub fn execute_prepared_operation(
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
#[expect(
    clippy::too_many_arguments,
    reason = "the operation dispatcher keeps typed graph semantics centralized"
)]
pub fn apply_operation_with_profile(
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
    apply_operation_with_profile_with_cancellation(
        step_index,
        operation,
        pixels,
        dimensions,
        pixel_index_offset,
        basicadj_plans,
        frame,
        terminal,
        masks,
        || false,
    )
}

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "The operation dispatcher keeps typed graph semantics and cancellation routing centralized."
)]
pub fn apply_operation_with_profile_with_cancellation<C: Fn() -> bool>(
    step_index: PipelineStepIndex,
    operation: &ProcessingOperation,
    pixels: &mut [LinearRgb],
    dimensions: RasterDimensions,
    pixel_index_offset: usize,
    basicadj_plans: Option<&BasicAdjPlanSet>,
    frame: &mut WorkingFrameDescriptor,
    terminal: &mut Option<TerminalOutputFrame>,
    masks: Option<&OperationMaskSet>,
    cancelled: C,
) -> Result<(), EvaluationError> {
    let operation_id = operation.operation_id();
    if cancelled() {
        return Err(EvaluationError::Cancelled {
            step_index,
            operation_id,
        });
    }
    let opacity = operation.opacity().get();
    if !operation.is_enabled() || opacity.to_bits() == 0.0f32.to_bits() {
        return Ok(());
    }
    let mask = masks.and_then(|set| set.mask_for(operation_id));
    if let Some(mask) = mask {
        validate_operation_mask(mask, pixels.len(), dimensions, step_index, operation_id)?;
    }
    let mask_route = OperationMaskRoute::new(operation.kind(), mask);
    if matches!(operation.kind(), ProcessingOperationKind::BasicAdj { .. }) {
        if opacity.to_bits() != 1.0_f32.to_bits() {
            return Err(operation_plan_error(
                step_index,
                operation_id,
                OperationExecutionError::UnsupportedCapability(
                    "basicadj opacity/blend semantics are not yet source-faithful",
                ),
            ));
        }
        if mask.is_some() {
            return Err(operation_plan_error(
                step_index,
                operation_id,
                OperationExecutionError::UnsupportedCapability(
                    "basicadj mask semantics are not yet source-faithful",
                ),
            ));
        }
    }
    let before_mask = mask_route.working_rgb_blend().map(|_| pixels.to_vec());
    let result = match operation.kind() {
        ProcessingOperationKind::Agx { config } => {
            require_unblended_tonal_route(
                step_index,
                operation_id,
                opacity,
                mask.is_some(),
                "AgX",
            )?;
            let profile = crate::operations::agx::resolve_builtin_working_profile(*frame)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let plan =
                crate::operations::agx::AgxPlan::new_with_profile(*config, dimensions, profile)
                    .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let rgba = pixels
                .iter()
                .copied()
                .map(|pixel| {
                    crate::operations::agx::AgxPixel::new(
                        pixel.red().get(),
                        pixel.green().get(),
                        pixel.blue().get(),
                        1.0,
                    )
                })
                .collect::<Vec<_>>();
            let candidate =
                plan.execute_with_cancel(&rgba, &cancelled)
                    .map_err(|error| match error {
                        crate::operations::agx::AgxExecutionError::Cancelled => {
                            EvaluationError::Cancelled {
                                step_index,
                                operation_id,
                            }
                        }
                        error => operation_plan_error(step_index, operation_id, error),
                    })?;
            let candidate = linear_rgb_from_rgba(
                candidate
                    .into_iter()
                    .map(crate::operations::agx::AgxPixel::channels),
                step_index,
                operation_id,
                pixel_index_offset,
            )?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::Levels { config } => {
            require_unblended_tonal_route(
                step_index,
                operation_id,
                opacity,
                mask.is_some(),
                "Levels",
            )?;
            let candidate = apply_levels_with_cancellation(
                *config,
                pixels,
                dimensions,
                frame.encoding(),
                &cancelled,
            )
            .map_err(|error| {
                if error.is_cancelled() {
                    EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    }
                } else {
                    operation_plan_error(step_index, operation_id, error)
                }
            })?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::ColorTransfer { parameters } => {
            require_unblended_tonal_route(
                step_index,
                operation_id,
                opacity,
                mask.is_some(),
                "Color Transfer",
            )?;
            let candidate = apply_colortransfer_with_cancellation(
                parameters,
                pixels,
                dimensions,
                frame.encoding(),
                &cancelled,
            )
            .map_err(|error| {
                if error.is_cancelled() {
                    EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    }
                } else {
                    operation_plan_error(step_index, operation_id, error)
                }
            })?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::ColorMapping { config } => {
            require_unblended_tonal_route(
                step_index,
                operation_id,
                opacity,
                mask.is_some(),
                "Color Mapping",
            )?;
            let candidate = apply_colormapping_with_cancellation(
                config,
                pixels,
                dimensions,
                frame.encoding(),
                &cancelled,
            )
            .map_err(|error| {
                if error.is_cancelled() {
                    EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    }
                } else {
                    operation_plan_error(step_index, operation_id, error)
                }
            })?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::RgbLevels { config } => {
            require_unblended_tonal_route(
                step_index,
                operation_id,
                opacity,
                mask.is_some(),
                "RGB Levels",
            )?;
            let profile = crate::operations::agx::resolve_builtin_working_profile(*frame)
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let evidence = crate::operations::rgblevels::RgbLevelsProfileEvidence::new_linear(
                profile.matrix_in_row_major(),
            );
            let plan = crate::operations::rgblevels::RgbLevelsPlan::new(*config, Some(evidence))
                .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            let rgba = pixels
                .iter()
                .copied()
                .map(|pixel| {
                    crate::operations::rgblevels::RgbLevelsPixel::new(
                        pixel.red().get(),
                        pixel.green().get(),
                        pixel.blue().get(),
                        1.0,
                    )
                })
                .collect::<Vec<_>>();
            let candidate =
                plan.execute_with_cancel(&rgba, &cancelled)
                    .map_err(|error| match error {
                        crate::operations::rgblevels::RgbLevelsExecutionError::Cancelled => {
                            EvaluationError::Cancelled {
                                step_index,
                                operation_id,
                            }
                        }
                        error => operation_plan_error(step_index, operation_id, error),
                    })?;
            let candidate = linear_rgb_from_rgba(
                candidate
                    .into_iter()
                    .map(crate::operations::rgblevels::RgbLevelsPixel::channels),
                step_index,
                operation_id,
                pixel_index_offset,
            )?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::BasicAdj { config } => {
            let plan = if let Some(plan) = basicadj_plans.and_then(|plans| plans.plan(operation_id))
            {
                plan.clone()
            } else {
                if config.auto_controls().is_active() {
                    return Err(operation_plan_error(
                        step_index,
                        operation_id,
                        OperationExecutionError::UnsupportedCapability(
                            "basicadj automatic controls require a published full-frame plan",
                        ),
                    ));
                }
                crate::operations::basicadj::BasicAdjPlan::new(*config)
                    .map_err(|error| operation_plan_error(step_index, operation_id, error))?
            };
            let candidate = plan
                .execute_with_working_frame_and_cancellation(
                    pixels,
                    pixel_index_offset,
                    *frame,
                    &cancelled,
                )
                .map_err(|error| match error {
                    OperationExecutionError::Cancelled => EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    },
                    error => operation_plan_error(step_index, operation_id, error),
                })?;
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
                .execute(&rgba, &cancelled)
                .map_err(|error| censorize_operation_error(step_index, operation_id, error))?;
            if cancelled() {
                return Err(EvaluationError::Cancelled {
                    step_index,
                    operation_id,
                });
            }
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
            if cancelled() {
                return Err(EvaluationError::Cancelled {
                    step_index,
                    operation_id,
                });
            }
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
                .execute(&rgba, &cancelled)
                .map_err(|error| clahe_operation_error(step_index, operation_id, error))?;
            if cancelled() {
                return Err(EvaluationError::Cancelled {
                    step_index,
                    operation_id,
                });
            }
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
            if cancelled() {
                return Err(EvaluationError::Cancelled {
                    step_index,
                    operation_id,
                });
            }
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
            let candidate = apply_bloom_with_cancellation(
                *config,
                pixels,
                dimensions,
                frame.encoding(),
                mask_route.native_values(),
                opacity,
                &cancelled,
            )
            .map_err(|error| {
                if error.is_cancelled() {
                    EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    }
                } else {
                    operation_plan_error(step_index, operation_id, error)
                }
            })?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::Soften { config } => {
            // The pixelpipe intercepts active Soften nodes before this RGB-only
            // processing boundary so it can retain native RGBA and ROI scale.
            // This branch remains the unit-scale leaf for processing callers.
            let plan = crate::operations::soften::SoftenPlan::new_with_scale(
                *config, dimensions, 1.0, 1.0,
            )
            .map_err(|error| operation_error(step_index, operation_id, error))?;
            let candidate = plan
                .execute_with_cancel(pixels, dimensions, &cancelled)
                .map_err(|error| match error {
                    OperationExecutionError::Cancelled => EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    },
                    error => operation_error(step_index, operation_id, error),
                })?;
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
            let candidate = apply_relight_with_cancellation(
                *config,
                pixels,
                dimensions,
                frame.encoding(),
                opacity,
                &cancelled,
            )
            .map_err(|error| {
                if error.is_cancelled() {
                    EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    }
                } else {
                    operation_plan_error(step_index, operation_id, error)
                }
            })?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::Velvia { config } => {
            if config.normalized_strength() <= 0.0 {
                return Ok(());
            }
            let candidate = crate::operations::velvia::VelviaPlan::new(*config).execute(pixels);
            apply_reconstruction(
                pixels,
                &candidate,
                opacity,
                step_index,
                operation_id,
                pixel_index_offset,
            )
        }
        ProcessingOperationKind::ChannelMixer { config } => {
            let plan = crate::operations::channelmixer::ChannelMixerPlan::new(*config);
            let rgba = pixels
                .iter()
                .copied()
                .map(|pixel| {
                    crate::operations::channelmixer::ChannelMixerPixel::new(
                        pixel.red().get(),
                        pixel.green().get(),
                        pixel.blue().get(),
                        1.0,
                    )
                })
                .collect::<Vec<_>>();
            // Native NORMAL2 combines opacity and the conditional mask in one
            // direct weighted blend. Do not route this operation through the
            // generic delta-first reconstruction or post-operation mask blend.
            let candidate = plan
                .execute_normal_blend_with_cancellation(
                    &rgba,
                    mask_route.native_values(),
                    opacity,
                    &cancelled,
                )
                .map_err(|error| match error {
                    crate::operations::channelmixer::ChannelMixerExecutionError::Cancelled => {
                        EvaluationError::Cancelled {
                            step_index,
                            operation_id,
                        }
                    }
                })?;
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
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::ColorContrast { config } => {
            let candidate =
                apply_colorcontrast(*config, pixels, *frame, mask_route.native_values(), opacity)
                    .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::Vibrance { config } => {
            let candidate =
                apply_vibrance(*config, pixels, *frame, mask_route.native_values(), opacity)
                    .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::ColorZones { plan } => {
            let candidate =
                apply_colorzones(plan, pixels, *frame, mask_route.native_values(), opacity)
                    .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::Sharpen { .. } => Err(EvaluationError::OperationExecution {
            step_index,
            operation_id,
            reason: "Sharpen requires the pixelpipe Lab D50 neighborhood route".to_owned(),
        }),
        ProcessingOperationKind::Shadhi { config } => {
            let candidate = apply_shadhi_with_cancellation(
                *config,
                pixels,
                dimensions,
                frame.encoding(),
                mask_route.native_values(),
                opacity,
                &cancelled,
            )
            .map_err(|error| {
                if error.is_cancelled() {
                    EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    }
                } else {
                    operation_plan_error(step_index, operation_id, error)
                }
            })?;
            pixels.copy_from_slice(&candidate);
            Ok(())
        }
        ProcessingOperationKind::Vignette { config } => {
            let plan = crate::operations::vignette::VignettePlan::new(*config, dimensions)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            let candidate = plan
                .execute_window_with_cancel(pixels, pixel_index_offset, &cancelled)
                .map_err(|error| match error {
                    OperationExecutionError::Cancelled => EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    },
                    error => operation_error(step_index, operation_id, error),
                })?;
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
                .execute_window_with_cancel(pixels, pixel_index_offset, &cancelled)
                .map_err(|error| match error {
                    OperationExecutionError::Cancelled => EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    },
                    error => operation_error(step_index, operation_id, error),
                })?;
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
            let candidate = apply_colorreconstruction_with_cancellation(
                *config, pixels, dimensions, *frame, &cancelled,
            )
            .map_err(|error| {
                if error.is_cancelled() {
                    EvaluationError::Cancelled {
                        step_index,
                        operation_id,
                    }
                } else {
                    operation_plan_error(step_index, operation_id, error)
                }
            })?;
            apply_reconstruction(
                pixels,
                &candidate,
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
            let candidate =
                apply_colorcorrection(*config, pixels, *frame, mask_route.native_values(), opacity)
                    .map_err(|error| operation_plan_error(step_index, operation_id, error))?;
            pixels.copy_from_slice(&candidate);
            Ok(())
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
    if cancelled() {
        return Err(EvaluationError::Cancelled {
            step_index,
            operation_id,
        });
    }
    if let (Some(mask), Some(before)) = (mask_route.working_rgb_blend(), before_mask.as_deref()) {
        apply_mask_blend(
            pixels,
            before,
            mask,
            step_index,
            operation_id,
            pixel_index_offset,
        )?;
    }
    if cancelled() {
        return Err(EvaluationError::Cancelled {
            step_index,
            operation_id,
        });
    }
    Ok(())
}

fn require_unblended_tonal_route(
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    opacity: f32,
    has_mask: bool,
    operation_name: &'static str,
) -> Result<(), EvaluationError> {
    if opacity.to_bits() != 1.0_f32.to_bits() {
        return Err(EvaluationError::OperationExecution {
            step_index,
            operation_id,
            reason: format!(
                "{operation_name} outer blending is deferred; only full opacity is executable"
            ),
        });
    }
    if has_mask {
        return Err(EvaluationError::OperationExecution {
            step_index,
            operation_id,
            reason: format!(
                "{operation_name} imported mask semantics are deferred and cannot be approximated"
            ),
        });
    }
    Ok(())
}

fn linear_rgb_from_rgba(
    channels: impl IntoIterator<Item = [f32; 4]>,
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    pixel_index_offset: usize,
) -> Result<Vec<LinearRgb>, EvaluationError> {
    channels
        .into_iter()
        .enumerate()
        .map(|(local_index, channels)| {
            let pixel_index = pixel_index_offset + local_index;
            Ok(LinearRgb::new(
                FiniteF32::new(channels[0]).map_err(|_| {
                    EvaluationError::NonFiniteChannelResult {
                        step_index,
                        operation_id,
                        pixel_index,
                        channel: RgbChannel::Red,
                    }
                })?,
                FiniteF32::new(channels[1]).map_err(|_| {
                    EvaluationError::NonFiniteChannelResult {
                        step_index,
                        operation_id,
                        pixel_index,
                        channel: RgbChannel::Green,
                    }
                })?,
                FiniteF32::new(channels[2]).map_err(|_| {
                    EvaluationError::NonFiniteChannelResult {
                        step_index,
                        operation_id,
                        pixel_index,
                        channel: RgbChannel::Blue,
                    }
                })?,
            ))
        })
        .collect()
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

fn censorize_operation_error(
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    error: crate::operations::censorize::CensorizeExecutionError,
) -> EvaluationError {
    match error {
        crate::operations::censorize::CensorizeExecutionError::Cancelled => {
            EvaluationError::Cancelled {
                step_index,
                operation_id,
            }
        }
        error => operation_plan_error(step_index, operation_id, error),
    }
}

fn clahe_operation_error(
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    error: crate::operations::clahe::ClaheExecutionError,
) -> EvaluationError {
    match error {
        crate::operations::clahe::ClaheExecutionError::Cancelled => EvaluationError::Cancelled {
            step_index,
            operation_id,
        },
        error => operation_plan_error(step_index, operation_id, error),
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
            Self::Cancelled {
                step_index,
                operation_id,
            } => write!(
                formatter,
                "operation {operation_id} at pipeline step {} was cancelled",
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
