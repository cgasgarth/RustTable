use std::collections::BTreeMap;

use rusttable_core::{FiniteF64, OperationId};
use rusttable_image::Orientation;
use sha2::{Digest, Sha256};

use super::{BasicAdjPlanSet, apply_operation_with_profile_with_cancellation};
use crate::operations::{
    clipping::{ClippingInterpolation, ClippingPlan},
    crop::{CropPlan, CropPlanMode},
    enlargecanvas::{CanvasFill, EnlargeCanvasPlan},
    finalscale::FinalScalePlan,
    flip::FlipPlan,
    lenscorrection::LensCorrectionPlan,
    perspective::{BoundaryMode, Interpolation, PerspectivePlan, Point},
    rotatepixels::{RotatePixelsInterpolation, RotatePixelsPlan},
    scalepixels::ScalePixelsPlan,
};
use crate::{
    BasicAdjAnalysisError, BasicAdjAnalysisRaster, BasicAdjPlan, CompiledOperationGraph,
    EvaluationError, FiniteF32, LinearRgb, OperationGraphNode, OperationMaskSet, PipelineStepIndex,
    ProcessingOperationKind, RasterDimensions, TerminalOutputFrame, WorkingRgbImage,
};

/// Purpose-specific choices needed while resolving shape-changing operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameBoundaryMode {
    Preview,
    Export,
}

/// Sampling policy owned by a distortion boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DistortionSamplingPolicy {
    Perspective {
        interpolation: Interpolation,
        border: BoundaryMode,
    },
    Clipping {
        interpolation: ClippingInterpolation,
        border: DistortionBorderMode,
    },
    LensCorrection {
        interpolation: DistortionInterpolation,
        border: DistortionBorderMode,
    },
}

/// Border behavior used by the established distortion samplers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DistortionBorderMode {
    Clamp,
    Reflect,
}

/// Common interpolation names for the plan/receipt contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DistortionInterpolation {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos,
}

/// Immutable policy used to resolve every geometry boundary in one graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameBoundaryOptions {
    mode: FrameBoundaryMode,
    source_orientation: Orientation,
    rotate_interpolation: RotatePixelsInterpolation,
}

impl FrameBoundaryOptions {
    #[must_use]
    pub const fn new(mode: FrameBoundaryMode) -> Self {
        Self {
            mode,
            source_orientation: Orientation::Normal,
            rotate_interpolation: RotatePixelsInterpolation::Bilinear,
        }
    }

    #[must_use]
    pub const fn mode(self) -> FrameBoundaryMode {
        self.mode
    }

    #[must_use]
    pub const fn with_source_orientation(mut self, orientation: Orientation) -> Self {
        self.source_orientation = orientation;
        self
    }

    #[must_use]
    pub const fn with_rotate_interpolation(
        mut self,
        interpolation: RotatePixelsInterpolation,
    ) -> Self {
        self.rotate_interpolation = interpolation;
        self
    }
}

/// A graph segmented into fixed-shape runs and checked geometry boundaries.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameBoundaryPlan {
    source_dimensions: RasterDimensions,
    output_dimensions: RasterDimensions,
    output_source_orientation: Orientation,
    steps: Vec<FramePlanStep>,
    boundary_count: usize,
    identity: [u8; 32],
}

impl FrameBoundaryPlan {
    /// Resolves all frame dimensions and operation plans before allocating output.
    ///
    /// # Errors
    ///
    /// Returns node-scoped planning context when any geometry operation cannot
    /// produce a finite, bounded replacement frame.
    pub fn new(
        graph: &CompiledOperationGraph,
        source_dimensions: RasterDimensions,
        options: FrameBoundaryOptions,
    ) -> Result<Self, EvaluationError> {
        let nodes = graph.nodes().collect::<Vec<_>>();
        let mut dimensions = source_dimensions;
        let mut steps = Vec::new();
        let mut segment_start = 0;
        let mut boundary_count = 0;
        let mut source_orientation = options.source_orientation;

        for (index, node) in nodes.iter().copied().enumerate() {
            let node_options = options.with_source_orientation(source_orientation);
            let Some(boundary) = plan_boundary(node, dimensions, node_options)? else {
                continue;
            };
            if segment_start < index {
                steps.push(FramePlanStep::FixedShape {
                    start: segment_start,
                    end: index,
                    dimensions,
                });
            }
            dimensions = boundary.output_dimensions();
            steps.push(FramePlanStep::Boundary {
                node_index: index,
                plan: Box::new(boundary),
            });
            if matches!(
                steps.last(),
                Some(FramePlanStep::Boundary { plan, .. })
                    if matches!(plan.as_ref(), FrameBoundaryOperation::Discrete(DiscreteGeometryPlan::Flip(_)))
            ) {
                source_orientation = Orientation::Normal;
            }
            boundary_count += 1;
            segment_start = index + 1;
        }
        if segment_start < nodes.len() {
            steps.push(FramePlanStep::FixedShape {
                start: segment_start,
                end: nodes.len(),
                dimensions,
            });
        }
        let identity = plan_identity(source_dimensions, dimensions, options, &steps);
        Ok(Self {
            source_dimensions,
            output_dimensions: dimensions,
            output_source_orientation: source_orientation,
            steps,
            boundary_count,
            identity,
        })
    }

    #[must_use]
    pub const fn source_dimensions(&self) -> RasterDimensions {
        self.source_dimensions
    }

    #[must_use]
    pub const fn output_dimensions(&self) -> RasterDimensions {
        self.output_dimensions
    }

    /// Remaining source orientation after the planned geometry boundaries.
    ///
    /// A flip boundary consumes this evidence; other geometry preserves it
    /// for a later automatic flip, including across a pixelpipe re-ingress.
    #[must_use]
    pub const fn output_source_orientation(&self) -> Orientation {
        self.output_source_orientation
    }

    #[must_use]
    pub const fn boundary_count(&self) -> usize {
        self.boundary_count
    }

    /// Stable identity for all resolved frame-boundary plans and policies.
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, PartialEq)]
enum FramePlanStep {
    FixedShape {
        start: usize,
        end: usize,
        dimensions: RasterDimensions,
    },
    Boundary {
        node_index: usize,
        plan: Box<FrameBoundaryOperation>,
    },
}

#[derive(Debug, Clone, PartialEq)]
enum DiscreteGeometryPlan {
    Crop(CropPlan),
    Flip(FlipPlan),
    Rotate(RotatePixelsPlan),
    Scale(ScalePixelsPlan),
    FinalScale(FinalScalePlan),
    EnlargeCanvas(EnlargeCanvasPlan),
}

#[derive(Debug, Clone, PartialEq)]
enum FrameBoundaryOperation {
    Discrete(DiscreteGeometryPlan),
    Distortion(DistortionPlan),
}

impl FrameBoundaryOperation {
    const fn output_dimensions(&self) -> RasterDimensions {
        match self {
            Self::Discrete(plan) => plan.output_dimensions(),
            Self::Distortion(plan) => plan.output_dimensions(),
        }
    }

    const fn identity(&self) -> [u8; 32] {
        match self {
            Self::Discrete(plan) => discrete_plan_identity(plan),
            Self::Distortion(plan) => plan.identity(),
        }
    }
}

/// One checked output-driven distortion plan.
#[derive(Debug, Clone, PartialEq)]
pub enum DistortionPlan {
    Perspective(PerspectivePlan),
    Clipping(ClippingPlan),
    LensCorrection(LensCorrectionPlan),
}

impl DistortionPlan {
    #[must_use]
    pub const fn source_dimensions(&self) -> RasterDimensions {
        match self {
            Self::Perspective(plan) => plan.source_dimensions(),
            Self::Clipping(plan) => plan.source_dimensions(),
            Self::LensCorrection(plan) => plan.source_dimensions(),
        }
    }

    #[must_use]
    pub const fn output_dimensions(&self) -> RasterDimensions {
        match self {
            Self::Perspective(plan) => plan.output_dimensions(),
            Self::Clipping(plan) => plan.output_dimensions(),
            Self::LensCorrection(plan) => plan.output_dimensions(),
        }
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        match self {
            Self::Perspective(plan) => plan.identity(),
            Self::Clipping(plan) => plan.identity(),
            Self::LensCorrection(plan) => plan.identity(),
        }
    }

    /// Returns the policy used by the operation's sampler and border path.
    #[must_use]
    pub const fn sampling_policy(&self) -> DistortionSamplingPolicy {
        match self {
            Self::Perspective(plan) => DistortionSamplingPolicy::Perspective {
                interpolation: plan.interpolation(),
                border: plan.boundary_mode(),
            },
            Self::Clipping(plan) => DistortionSamplingPolicy::Clipping {
                interpolation: plan.interpolation(),
                border: DistortionBorderMode::Clamp,
            },
            Self::LensCorrection(_) => DistortionSamplingPolicy::LensCorrection {
                interpolation: DistortionInterpolation::Bilinear,
                border: DistortionBorderMode::Clamp,
            },
        }
    }

    /// Maps a source ROI to the operation's output ROI.
    ///
    /// # Errors
    ///
    /// Returns a bounded ROI error when the source ROI is outside the plan's
    /// source frame or the mapped enclosure cannot be represented.
    pub fn output_roi(&self, input: rusttable_image::Roi) -> Result<rusttable_image::Roi, String> {
        match self {
            Self::Perspective(plan) => plan.output_roi(input).map_err(|error| error.to_string()),
            Self::Clipping(plan) => plan
                .modify_roi_out(input)
                .map_err(|error| error.to_string()),
            Self::LensCorrection(plan) => plan
                .modify_roi_out(input)
                .map_err(|error| error.to_string()),
        }
    }

    /// Computes the source ROI, including the sampler halo, for one output ROI.
    ///
    /// # Errors
    ///
    /// Returns a bounded ROI error when the output ROI is outside the plan's
    /// output frame or inverse mapping fails.
    pub fn input_roi(&self, output: rusttable_image::Roi) -> Result<rusttable_image::Roi, String> {
        match self {
            Self::Perspective(plan) => plan.input_roi(output).map_err(|error| error.to_string()),
            Self::Clipping(plan) => plan
                .modify_roi_in(output)
                .map_err(|error| error.to_string()),
            Self::LensCorrection(plan) => plan
                .modify_roi_in(output)
                .map_err(|error| error.to_string()),
        }
    }

    /// Maps a source coordinate into output space.
    ///
    /// # Errors
    ///
    /// Returns a coordinate error when the transform is non-finite or cannot
    /// be represented by the selected plan.
    pub fn forward_point(&self, point: [f64; 2]) -> Result<[f64; 2], String> {
        match self {
            Self::Perspective(plan) => plan
                .forward_point(Point::new(point[0], point[1]))
                .map(|point| [point.x(), point.y()])
                .map_err(|error| format!("{error:?}")),
            Self::Clipping(plan) => plan
                .forward_point(Point::new(point[0], point[1]))
                .map(|point| [point.x(), point.y()])
                .map_err(|error| format!("{error:?}")),
            Self::LensCorrection(plan) => plan
                .forward_point([narrow_coordinate(point[0])?, narrow_coordinate(point[1])?])
                .map(|point| [f64::from(point[0]), f64::from(point[1])])
                .map_err(|error| error.to_string()),
        }
    }

    /// Maps an output coordinate back to the source for inverse sampling.
    ///
    /// # Errors
    ///
    /// Returns a coordinate error when the inverse transform is non-finite or
    /// does not converge.
    pub fn back_point(&self, point: [f64; 2]) -> Result<[f64; 2], String> {
        match self {
            Self::Perspective(plan) => plan
                .back_point(Point::new(point[0], point[1]))
                .map(|point| [point.x(), point.y()])
                .map_err(|error| format!("{error:?}")),
            Self::Clipping(plan) => plan
                .back_point(Point::new(point[0], point[1]))
                .map(|point| [point.x(), point.y()])
                .map_err(|error| format!("{error:?}")),
            Self::LensCorrection(plan) => plan
                .back_point([narrow_coordinate(point[0])?, narrow_coordinate(point[1])?])
                .map(|point| [f64::from(point[0]), f64::from(point[1])])
                .map_err(|error| error.to_string()),
        }
    }
}

fn narrow_coordinate(value: f64) -> Result<f32, String> {
    let finite = FiniteF64::new(value).map_err(|_| "coordinate is non-finite".to_owned())?;
    FiniteF32::try_from(finite)
        .map(FiniteF32::get)
        .map_err(|_| "coordinate is outside the f32 range".to_owned())
}

impl DiscreteGeometryPlan {
    const fn output_dimensions(&self) -> RasterDimensions {
        match self {
            Self::Crop(plan) => plan.output_dimensions(),
            Self::Flip(plan) => plan.output_dimensions(),
            Self::Rotate(plan) => plan.output_dimensions(),
            Self::Scale(plan) => plan.output_dimensions(),
            Self::FinalScale(plan) => plan.output_dimensions(),
            Self::EnlargeCanvas(plan) => plan.output_dimensions(),
        }
    }
}

/// Full-frame RGB and straight-alpha output plus resolved analysis identity.
#[derive(Debug, Clone, PartialEq)]
pub struct EvaluatedFrame {
    image: WorkingRgbImage,
    terminal_output: Option<TerminalOutputFrame>,
    alpha: Vec<f32>,
    basicadj_plans: BasicAdjPlanSet,
    frame_plan_identity: [u8; 32],
    output_source_orientation: Orientation,
}

#[must_use]
pub fn graph_has_discrete_geometry(graph: &CompiledOperationGraph) -> bool {
    graph.nodes().any(|node| {
        let operation = node.operation();
        operation.is_enabled()
            && operation.opacity().get().to_bits() != 0.0_f32.to_bits()
            && matches!(
                operation.kind(),
                ProcessingOperationKind::Crop { .. }
                    | ProcessingOperationKind::Flip { .. }
                    | ProcessingOperationKind::RotatePixels { .. }
                    | ProcessingOperationKind::ScalePixels { .. }
                    | ProcessingOperationKind::FinalScale { .. }
                    | ProcessingOperationKind::EnlargeCanvas { .. }
            )
    })
}

/// Returns whether the graph contains a frame-replacing discrete or
/// inverse-mapped distortion operation.
#[must_use]
pub fn graph_has_frame_geometry(graph: &CompiledOperationGraph) -> bool {
    graph.nodes().any(|node| {
        let operation = node.operation();
        operation.is_enabled()
            && operation.opacity().get().to_bits() != 0.0_f32.to_bits()
            && matches!(
                operation.kind(),
                ProcessingOperationKind::Crop { .. }
                    | ProcessingOperationKind::Flip { .. }
                    | ProcessingOperationKind::RotatePixels { .. }
                    | ProcessingOperationKind::ScalePixels { .. }
                    | ProcessingOperationKind::FinalScale { .. }
                    | ProcessingOperationKind::EnlargeCanvas { .. }
                    | ProcessingOperationKind::Perspective { .. }
                    | ProcessingOperationKind::Clipping { .. }
                    | ProcessingOperationKind::LensCorrection { .. }
            )
    })
}

impl EvaluatedFrame {
    #[must_use]
    pub const fn image(&self) -> &WorkingRgbImage {
        &self.image
    }

    #[must_use]
    pub const fn terminal_output(&self) -> Option<&TerminalOutputFrame> {
        self.terminal_output.as_ref()
    }

    #[must_use]
    pub fn alpha(&self) -> &[f32] {
        &self.alpha
    }

    #[must_use]
    pub const fn basicadj_plans(&self) -> &BasicAdjPlanSet {
        &self.basicadj_plans
    }

    /// Identity of the resolved frame-boundary plan used for this output.
    #[must_use]
    pub const fn frame_plan_identity(&self) -> [u8; 32] {
        self.frame_plan_identity
    }

    /// Source orientation still pending after all frame boundaries.
    #[must_use]
    pub const fn output_source_orientation(&self) -> Orientation {
        self.output_source_orientation
    }
}

/// Executes fixed-shape segments and publishes a replacement frame at every
/// planned discrete geometry boundary.
///
/// # Errors
///
/// Returns node-scoped planning, execution, alpha-validation, or cancellation
/// context without publishing a partial frame.
pub fn evaluate_graph_at_frame_boundaries<F: Fn() -> bool>(
    graph: &CompiledOperationGraph,
    input: &WorkingRgbImage,
    alpha: &[f32],
    options: FrameBoundaryOptions,
    cancelled: F,
) -> Result<EvaluatedFrame, EvaluationError> {
    evaluate_graph_at_frame_boundaries_with_plans_and_masks(
        graph, input, alpha, options, None, None, cancelled,
    )
}

/// Evaluates frame-boundary graphs with detached operation mask rasters.
///
/// # Errors
///
/// Returns the first frame-boundary, operation, mask-shape, or cancellation
/// error encountered while evaluating the graph.
pub fn evaluate_graph_at_frame_boundaries_with_masks<F: Fn() -> bool>(
    graph: &CompiledOperationGraph,
    input: &WorkingRgbImage,
    alpha: &[f32],
    options: FrameBoundaryOptions,
    masks: Option<&OperationMaskSet>,
    cancelled: F,
) -> Result<EvaluatedFrame, EvaluationError> {
    evaluate_graph_at_frame_boundaries_with_plans_and_masks(
        graph, input, alpha, options, None, masks, cancelled,
    )
}

pub(crate) fn evaluate_graph_at_frame_boundaries_with_plans<F: Fn() -> bool>(
    graph: &CompiledOperationGraph,
    input: &WorkingRgbImage,
    alpha: &[f32],
    options: FrameBoundaryOptions,
    provided_basicadj: Option<&BasicAdjPlanSet>,
    cancelled: F,
) -> Result<EvaluatedFrame, EvaluationError> {
    evaluate_graph_at_frame_boundaries_with_plans_and_masks(
        graph,
        input,
        alpha,
        options,
        provided_basicadj,
        None,
        cancelled,
    )
}

pub(crate) fn evaluate_graph_at_frame_boundaries_with_plans_and_masks<F: Fn() -> bool>(
    graph: &CompiledOperationGraph,
    input: &WorkingRgbImage,
    alpha: &[f32],
    options: FrameBoundaryOptions,
    provided_basicadj: Option<&BasicAdjPlanSet>,
    masks: Option<&OperationMaskSet>,
    cancelled: F,
) -> Result<EvaluatedFrame, EvaluationError> {
    let plan = FrameBoundaryPlan::new(graph, input.dimensions(), options)?;
    validate_alpha(alpha, input.dimensions())?;
    let nodes = graph.nodes().collect::<Vec<_>>();
    let mut pixels = input.pixel_slice().to_vec();
    let mut alpha = alpha.to_vec();
    let mut dimensions = input.dimensions();
    let mut frame = input.frame();
    let mut terminal_output = None;
    let mut terminal_working_pixels = None;
    let mut basicadj = BTreeMap::new();

    for step in &plan.steps {
        if cancelled() {
            return Err(cancelled_error(step_context(step, &nodes)));
        }
        match step {
            FramePlanStep::FixedShape {
                start,
                end,
                dimensions: planned_dimensions,
            } => {
                debug_assert_eq!(*planned_dimensions, dimensions);
                for node in &nodes[*start..*end] {
                    if cancelled() {
                        return Err(cancelled_error(node));
                    }
                    if provided_basicadj.is_none() {
                        resolve_basicadj(node, dimensions, &pixels, &mut basicadj, &cancelled)?;
                    }
                    let resolved_plans = plan_set(&basicadj);
                    let plans = provided_basicadj.unwrap_or(&resolved_plans);
                    if matches!(
                        node.operation().kind(),
                        ProcessingOperationKind::ColorOut { .. }
                    ) {
                        terminal_working_pixels = Some(pixels.clone());
                    }
                    apply_operation_with_profile_with_cancellation(
                        node.pipeline_step_index(),
                        node.operation(),
                        &mut pixels,
                        dimensions,
                        0,
                        Some(plans),
                        &mut frame,
                        &mut terminal_output,
                        masks,
                        &cancelled,
                    )?;
                }
            }
            FramePlanStep::Boundary { node_index, plan } => {
                let node = nodes[*node_index];
                let output = execute_boundary(plan, &pixels, &alpha, &cancelled)
                    .map_err(|reason| node_error(node, reason))?;
                dimensions = plan.output_dimensions();
                pixels = output.0;
                alpha = output.1;
            }
        }
    }
    debug_assert_eq!(dimensions, plan.output_dimensions());
    let basicadj_plans = provided_basicadj
        .cloned()
        .unwrap_or_else(|| finalized_plan_set(basicadj));
    let image_pixels = terminal_working_pixels.unwrap_or(pixels);
    Ok(EvaluatedFrame {
        image: WorkingRgbImage::from_validated_parts_with_frame(dimensions, image_pixels, frame),
        terminal_output,
        alpha,
        basicadj_plans,
        frame_plan_identity: plan.identity(),
        output_source_orientation: plan.output_source_orientation(),
    })
}

fn plan_boundary(
    node: &OperationGraphNode,
    dimensions: RasterDimensions,
    options: FrameBoundaryOptions,
) -> Result<Option<FrameBoundaryOperation>, EvaluationError> {
    let operation = node.operation();
    if !operation.is_enabled() || operation.opacity().get().to_bits() == 0.0_f32.to_bits() {
        return Ok(None);
    }
    let kind = operation.kind();
    if !matches!(
        kind,
        ProcessingOperationKind::Crop { .. }
            | ProcessingOperationKind::Flip { .. }
            | ProcessingOperationKind::RotatePixels { .. }
            | ProcessingOperationKind::ScalePixels { .. }
            | ProcessingOperationKind::FinalScale { .. }
            | ProcessingOperationKind::EnlargeCanvas { .. }
            | ProcessingOperationKind::Perspective { .. }
            | ProcessingOperationKind::Clipping { .. }
            | ProcessingOperationKind::LensCorrection { .. }
    ) {
        return Ok(None);
    }
    if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
        return Err(node_error(
            node,
            "geometry requires full opacity at a frame boundary".to_owned(),
        ));
    }
    let boundary = match kind {
        ProcessingOperationKind::Crop { config } => {
            let mode = match options.mode {
                FrameBoundaryMode::Preview => CropPlanMode::Preview,
                FrameBoundaryMode::Export => CropPlanMode::Export,
            };
            CropPlan::new_with_mode(*config, dimensions, mode)
                .map(DiscreteGeometryPlan::Crop)
                .map(FrameBoundaryOperation::Discrete)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::Flip { config } => {
            FlipPlan::new(dimensions, config.clone(), options.source_orientation)
                .map(DiscreteGeometryPlan::Flip)
                .map(FrameBoundaryOperation::Discrete)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::RotatePixels { config } => {
            RotatePixelsPlan::new(dimensions, config.clone(), options.rotate_interpolation)
                .map(DiscreteGeometryPlan::Rotate)
                .map(FrameBoundaryOperation::Discrete)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::ScalePixels { config } => {
            ScalePixelsPlan::new(config.clone(), dimensions)
                .map(DiscreteGeometryPlan::Scale)
                .map(FrameBoundaryOperation::Discrete)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::FinalScale { config } => {
            FinalScalePlan::from_config(dimensions, config.clone())
                .map(DiscreteGeometryPlan::FinalScale)
                .map(FrameBoundaryOperation::Discrete)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::EnlargeCanvas { config } => {
            EnlargeCanvasPlan::new(*config, dimensions)
                .map(DiscreteGeometryPlan::EnlargeCanvas)
                .map(FrameBoundaryOperation::Discrete)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::Perspective { config } => {
            PerspectivePlan::new(config.clone(), dimensions, Interpolation::Bilinear)
                .map(DistortionPlan::Perspective)
                .map(FrameBoundaryOperation::Distortion)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::Clipping { config } => {
            ClippingPlan::new(dimensions, config.clone(), ClippingInterpolation::Bilinear)
                .map(DistortionPlan::Clipping)
                .map(FrameBoundaryOperation::Distortion)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::LensCorrection { config } => {
            LensCorrectionPlan::new(dimensions, config.clone())
                .map(DistortionPlan::LensCorrection)
                .map(FrameBoundaryOperation::Distortion)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        _ => unreachable!("checked frame-boundary geometry kind"),
    };
    Ok(Some(boundary))
}

fn execute_boundary<F: Fn() -> bool>(
    plan: &FrameBoundaryOperation,
    pixels: &[LinearRgb],
    alpha: &[f32],
    cancelled: &F,
) -> Result<(Vec<LinearRgb>, Vec<f32>), String> {
    match plan {
        FrameBoundaryOperation::Discrete(plan) => {
            execute_discrete_boundary(plan, pixels, alpha, cancelled)
        }
        FrameBoundaryOperation::Distortion(plan) => {
            execute_distortion_boundary(plan, pixels, alpha, cancelled)
        }
    }
}

fn execute_discrete_boundary<F: Fn() -> bool>(
    plan: &DiscreteGeometryPlan,
    pixels: &[LinearRgb],
    alpha: &[f32],
    cancelled: &F,
) -> Result<(Vec<LinearRgb>, Vec<f32>), String> {
    let alpha_rgb = alpha_pixels(alpha)?;
    match plan {
        DiscreteGeometryPlan::Crop(plan) => {
            let rgb = plan
                .execute_with_cancel(pixels, cancelled)
                .map_err(display)?;
            let alpha = plan
                .execute_with_cancel(&alpha_rgb, cancelled)
                .map_err(display)?;
            Ok((rgb.pixels().to_vec(), red_plane(alpha.pixels())))
        }
        DiscreteGeometryPlan::Flip(plan) => {
            let rgb = plan
                .execute_with_cancel(pixels, cancelled)
                .map_err(display)?;
            let alpha = plan
                .execute_with_cancel(&alpha_rgb, cancelled)
                .map_err(display)?;
            Ok((rgb.pixels().to_vec(), red_plane(alpha.pixels())))
        }
        DiscreteGeometryPlan::Rotate(plan) => {
            let rgb = plan
                .execute_with_cancel(pixels, cancelled)
                .map_err(display)?;
            let stride = usize::try_from(plan.source_dimensions().width()).map_err(display)?;
            let alpha = plan
                .execute_interleaved_with_cancel(alpha, 1, stride, cancelled)
                .map_err(display)?;
            Ok((rgb.pixels().to_vec(), alpha))
        }
        DiscreteGeometryPlan::Scale(plan) => {
            let rgb = plan
                .execute_with_cancel(pixels, cancelled)
                .map_err(display)?;
            let alpha = plan
                .execute_with_cancel(&alpha_rgb, cancelled)
                .map_err(display)?;
            Ok((rgb.pixels().to_vec(), red_plane(alpha.pixels())))
        }
        DiscreteGeometryPlan::FinalScale(plan) => {
            let rgb = plan
                .execute_with_cancel(pixels, cancelled)
                .map_err(display)?;
            let stride = usize::try_from(plan.source_dimensions().width()).map_err(display)?;
            let alpha = plan
                .execute_interleaved_with_cancel(alpha, 1, stride, cancelled)
                .map_err(display)?;
            Ok((rgb.pixels().to_vec(), alpha))
        }
        DiscreteGeometryPlan::EnlargeCanvas(plan) => {
            let rgb = plan
                .execute_with_cancel(pixels, cancelled)
                .map_err(display)?;
            let fill = plan.fill().alpha().get();
            let alpha_fill = CanvasFill::new(fill, fill, fill, fill).map_err(display)?;
            let alpha_plan = EnlargeCanvasPlan::new_with_fill(
                plan.config(),
                plan.source_dimensions(),
                alpha_fill,
            )
            .map_err(display)?;
            let alpha = alpha_plan
                .execute_with_cancel(&alpha_rgb, cancelled)
                .map_err(display)?;
            Ok((rgb.pixels().to_vec(), red_plane(alpha.pixels())))
        }
    }
}

fn execute_distortion_boundary<F: Fn() -> bool>(
    plan: &DistortionPlan,
    pixels: &[LinearRgb],
    alpha: &[f32],
    cancelled: &F,
) -> Result<(Vec<LinearRgb>, Vec<f32>), String> {
    let source_width = usize::try_from(plan.source_dimensions().width())
        .map_err(|_| "distortion source width overflowed".to_owned())?;
    match plan {
        DistortionPlan::Perspective(plan) => {
            let rgb = plan
                .execute_with_cancel(pixels, cancelled)
                .map_err(display)?;
            let alpha = plan.execute_plane(alpha, cancelled).map_err(display)?;
            Ok((rgb.pixels().to_vec(), alpha))
        }
        DistortionPlan::Clipping(plan) => {
            let rgb = plan
                .execute_with_cancel(pixels, cancelled)
                .map_err(display)?;
            let alpha = plan
                .execute_plane_with_cancel(alpha, source_width, cancelled)
                .map_err(display)?;
            Ok((rgb.pixels().to_vec(), alpha))
        }
        DistortionPlan::LensCorrection(plan) => {
            let rgb = plan
                .execute_with_cancel(pixels, cancelled)
                .map_err(display)?;
            let alpha = plan
                .execute_plane_with_cancel(alpha, source_width, cancelled)
                .map_err(display)?;
            Ok((rgb.pixels().to_vec(), alpha))
        }
    }
}

const fn discrete_plan_identity(plan: &DiscreteGeometryPlan) -> [u8; 32] {
    match plan {
        DiscreteGeometryPlan::Crop(plan) => plan.identity(),
        DiscreteGeometryPlan::Flip(plan) => plan.identity(),
        DiscreteGeometryPlan::Rotate(plan) => plan.identity(),
        DiscreteGeometryPlan::Scale(plan) => plan.identity(),
        DiscreteGeometryPlan::FinalScale(plan) => plan.identity(),
        DiscreteGeometryPlan::EnlargeCanvas(plan) => plan.identity(),
    }
}

fn plan_identity(
    source: RasterDimensions,
    output: RasterDimensions,
    options: FrameBoundaryOptions,
    steps: &[FramePlanStep],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.processing.frame-boundary.v2");
    hasher.update(source.width().to_le_bytes());
    hasher.update(source.height().to_le_bytes());
    hasher.update(output.width().to_le_bytes());
    hasher.update(output.height().to_le_bytes());
    hasher.update([match options.mode {
        FrameBoundaryMode::Preview => 0,
        FrameBoundaryMode::Export => 1,
    }]);
    for step in steps {
        if let FramePlanStep::Boundary { node_index, plan } = step {
            hasher.update((*node_index as u64).to_le_bytes());
            hasher.update(plan.identity());
        }
    }
    hasher.finalize().into()
}

fn resolve_basicadj<F: Fn() -> bool>(
    node: &OperationGraphNode,
    dimensions: RasterDimensions,
    pixels: &[LinearRgb],
    plans: &mut BTreeMap<OperationId, BasicAdjPlan>,
    cancelled: &F,
) -> Result<(), EvaluationError> {
    let operation = node.operation();
    let ProcessingOperationKind::BasicAdj { config } = operation.kind() else {
        return Ok(());
    };
    if !operation.is_enabled()
        || operation.opacity().get().to_bits() == 0.0_f32.to_bits()
        || !config.auto_controls().is_active()
    {
        return Ok(());
    }
    let raster = BasicAdjAnalysisRaster::new(dimensions, pixels, None)
        .map_err(|error| node_error(node, error.to_string()))?;
    let plan =
        BasicAdjPlan::resolve_with_cancellation(*config, raster, cancelled).map_err(|error| {
            match error {
                BasicAdjAnalysisError::Cancelled => cancelled_error(node),
                error => node_error(node, error.to_string()),
            }
        })?;
    plans.insert(operation.operation_id(), plan);
    Ok(())
}

fn plan_set(plans: &BTreeMap<OperationId, BasicAdjPlan>) -> BasicAdjPlanSet {
    BasicAdjPlanSet {
        plans: plans.clone(),
        identity: [0; 32],
    }
}

fn finalized_plan_set(plans: BTreeMap<OperationId, BasicAdjPlan>) -> BasicAdjPlanSet {
    let identity = if plans.is_empty() {
        [0; 32]
    } else {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.basicadj.plan-set.v1");
        for (operation_id, plan) in &plans {
            hasher.update(operation_id.get().to_le_bytes());
            hasher.update(plan.identity());
        }
        hasher.finalize().into()
    };
    BasicAdjPlanSet { plans, identity }
}

fn validate_alpha(alpha: &[f32], dimensions: RasterDimensions) -> Result<(), EvaluationError> {
    let expected = usize::try_from(dimensions.pixel_count()).unwrap_or(usize::MAX);
    if alpha.len() != expected || alpha.iter().any(|value| !value.is_finite()) {
        return Err(EvaluationError::OperationExecution {
            step_index: PipelineStepIndex::new(0),
            operation_id: OperationId::new(1).expect("nonzero synthetic operation ID"),
            reason: format!(
                "frame alpha plane is invalid: expected {expected} finite values, got {}",
                alpha.len()
            ),
        });
    }
    Ok(())
}

fn alpha_pixels(alpha: &[f32]) -> Result<Vec<LinearRgb>, String> {
    alpha
        .iter()
        .copied()
        .enumerate()
        .map(|(index, value)| {
            let value = FiniteF32::new(value)
                .map_err(|_| format!("alpha value at pixel {index} is non-finite"))?;
            Ok(LinearRgb::new(value, value, value))
        })
        .collect()
}

fn red_plane(pixels: &[LinearRgb]) -> Vec<f32> {
    pixels.iter().map(|pixel| pixel.red().get()).collect()
}

fn step_context<'a>(
    step: &FramePlanStep,
    nodes: &'a [&'a OperationGraphNode],
) -> &'a OperationGraphNode {
    let index = match step {
        FramePlanStep::FixedShape { start, .. } => *start,
        FramePlanStep::Boundary { node_index, .. } => *node_index,
    };
    nodes[index]
}

fn cancelled_error(node: &OperationGraphNode) -> EvaluationError {
    EvaluationError::Cancelled {
        step_index: node.pipeline_step_index(),
        operation_id: node.operation().operation_id(),
    }
}

fn node_error(node: &OperationGraphNode, reason: String) -> EvaluationError {
    EvaluationError::OperationExecution {
        step_index: node.pipeline_step_index(),
        operation_id: node.operation().operation_id(),
        reason,
    }
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}
