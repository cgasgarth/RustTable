use std::{collections::BTreeMap, fmt};

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
use sha2::Digest;

/// Immutable resolved automatic plans keyed by authored operation ID.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BasicAdjPlanSet {
    plans: BTreeMap<OperationId, crate::operations::basicadj::BasicAdjPlan>,
    identity: [u8; 32],
}

impl BasicAdjPlanSet {
    #[must_use]
    pub fn plan(
        &self,
        operation_id: OperationId,
    ) -> Option<&crate::operations::basicadj::BasicAdjPlan> {
        self.plans.get(&operation_id)
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

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
    let mut output = input.to_vec();
    for (step_index, operation) in steps {
        apply_operation_with_plans(
            step_index,
            operation.operation(),
            &mut output,
            dimensions,
            pixel_index_offset,
            basicadj_plans,
        )?;
    }
    Ok(output)
}

/// Resolves every automatic basicadj node against the full preceding image,
/// then executes the graph once to establish the next node's analysis input.
/// The returned set is reusable by every tile of that snapshot.
///
/// # Errors
///
/// Returns the first graph-operation or automatic-analysis failure.
pub fn prepare_basicadj_plans(
    graph: &crate::CompiledOperationGraph,
    input: &WorkingRgbImage,
) -> Result<BasicAdjPlanSet, EvaluationError> {
    let mut current = input.pixel_slice().to_vec();
    let mut plans = BTreeMap::new();
    for node in graph.nodes() {
        let operation = node.operation();
        if let crate::ProcessingOperationKind::BasicAdj { config } = operation.kind()
            && operation.is_enabled()
            && operation.opacity().get().to_bits() != 0.0_f32.to_bits()
            && config.auto_controls().is_active()
        {
            let raster = crate::BasicAdjAnalysisRaster::new(input.dimensions(), &current, None)
                .map_err(|error| EvaluationError::OperationExecution {
                    step_index: node.pipeline_step_index(),
                    operation_id: operation.operation_id(),
                    reason: error.to_string(),
                })?;
            let plan = crate::BasicAdjPlan::resolve(*config, raster).map_err(|error| {
                EvaluationError::OperationExecution {
                    step_index: node.pipeline_step_index(),
                    operation_id: operation.operation_id(),
                    reason: error.to_string(),
                }
            })?;
            plans.insert(operation.operation_id(), plan);
        }
        let plan_set = BasicAdjPlanSet {
            plans: plans.clone(),
            identity: [0; 32],
        };
        apply_operation_with_plans(
            node.pipeline_step_index(),
            operation,
            &mut current,
            input.dimensions(),
            0,
            Some(&plan_set),
        )?;
    }
    let identity = if plans.is_empty() {
        [0; 32]
    } else {
        let mut hasher = sha2::Sha256::new();
        hasher.update(b"rusttable.basicadj.plan-set.v1");
        for (operation_id, plan) in &plans {
            hasher.update(operation_id.get().to_le_bytes());
            hasher.update(plan.identity());
        }
        hasher.finalize().into()
    };
    Ok(BasicAdjPlanSet { plans, identity })
}

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
    let operation_id = operation.operation_id();
    let opacity = operation.opacity().get();
    if !operation.is_enabled() || opacity.to_bits() == 0.0f32.to_bits() {
        return Ok(());
    }
    match operation.kind() {
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
            let plan = crate::operations::relight::RelightPlan::new(*config, dimensions);
            let candidate = plan
                .execute(pixels)
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
        ProcessingOperationKind::Shadhi { config } => {
            let plan = crate::operations::shadhi::ShadhiPlan::new(*config, dimensions)
                .map_err(|error| operation_error(step_index, operation_id, error))?;
            let candidate = plan
                .execute(pixels)
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
        | ProcessingOperationKind::ScalePixels { .. }
        | ProcessingOperationKind::FinalScale { .. }
        | ProcessingOperationKind::EnlargeCanvas { .. }
        | ProcessingOperationKind::Perspective { .. }
        | ProcessingOperationKind::LensCorrection { .. } => Err(operation_error(
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
            apply_operation_with_plans(
                step.index(),
                step.operation(),
                &mut pixels,
                RasterDimensions::new(1, 1).expect("test dimensions"),
                0,
                None,
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
