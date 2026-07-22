use std::collections::BTreeMap;

use rusttable_core::OperationId;
use rusttable_image::Orientation;
use sha2::{Digest, Sha256};

use super::{BasicAdjPlanSet, apply_operation_with_profile};
use crate::operations::{
    crop::{CropPlan, CropPlanMode},
    enlargecanvas::{CanvasFill, EnlargeCanvasPlan},
    finalscale::FinalScalePlan,
    flip::FlipPlan,
    rotatepixels::{RotatePixelsInterpolation, RotatePixelsPlan},
    scalepixels::ScalePixelsPlan,
};
use crate::{
    BasicAdjAnalysisRaster, BasicAdjPlan, CompiledOperationGraph, EvaluationError, FiniteF32,
    LinearRgb, OperationGraphNode, PipelineStepIndex, ProcessingOperationKind, RasterDimensions,
    WorkingRgbImage,
};

/// Purpose-specific choices needed while resolving shape-changing operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameBoundaryMode {
    Preview,
    Export,
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
    steps: Vec<FramePlanStep>,
    boundary_count: usize,
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
                plan: boundary,
            });
            if matches!(
                steps.last(),
                Some(FramePlanStep::Boundary {
                    plan: DiscreteGeometryPlan::Flip(_),
                    ..
                })
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
        Ok(Self {
            source_dimensions,
            output_dimensions: dimensions,
            steps,
            boundary_count,
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

    #[must_use]
    pub const fn boundary_count(&self) -> usize {
        self.boundary_count
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
        plan: DiscreteGeometryPlan,
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
    alpha: Vec<f32>,
    basicadj_plans: BasicAdjPlanSet,
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

impl EvaluatedFrame {
    #[must_use]
    pub const fn image(&self) -> &WorkingRgbImage {
        &self.image
    }

    #[must_use]
    pub fn alpha(&self) -> &[f32] {
        &self.alpha
    }

    #[must_use]
    pub const fn basicadj_plans(&self) -> &BasicAdjPlanSet {
        &self.basicadj_plans
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
    evaluate_graph_at_frame_boundaries_with_plans(graph, input, alpha, options, None, cancelled)
}

pub(crate) fn evaluate_graph_at_frame_boundaries_with_plans<F: Fn() -> bool>(
    graph: &CompiledOperationGraph,
    input: &WorkingRgbImage,
    alpha: &[f32],
    options: FrameBoundaryOptions,
    provided_basicadj: Option<&BasicAdjPlanSet>,
    cancelled: F,
) -> Result<EvaluatedFrame, EvaluationError> {
    let plan = FrameBoundaryPlan::new(graph, input.dimensions(), options)?;
    validate_alpha(alpha, input.dimensions())?;
    let nodes = graph.nodes().collect::<Vec<_>>();
    let mut pixels = input.pixel_slice().to_vec();
    let mut alpha = alpha.to_vec();
    let mut dimensions = input.dimensions();
    let mut frame = input.frame();
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
                    apply_operation_with_profile(
                        node.pipeline_step_index(),
                        node.operation(),
                        &mut pixels,
                        dimensions,
                        0,
                        Some(plans),
                        &mut frame,
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
    Ok(EvaluatedFrame {
        image: WorkingRgbImage::from_validated_parts_with_frame(dimensions, pixels, frame),
        alpha,
        basicadj_plans,
    })
}

fn plan_boundary(
    node: &OperationGraphNode,
    dimensions: RasterDimensions,
    options: FrameBoundaryOptions,
) -> Result<Option<DiscreteGeometryPlan>, EvaluationError> {
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
    ) {
        return Ok(None);
    }
    if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
        return Err(node_error(
            node,
            "discrete geometry requires full opacity at a frame boundary".to_owned(),
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
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::Flip { config } => {
            FlipPlan::new(dimensions, config.clone(), options.source_orientation)
                .map(DiscreteGeometryPlan::Flip)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::RotatePixels { config } => {
            RotatePixelsPlan::new(dimensions, config.clone(), options.rotate_interpolation)
                .map(DiscreteGeometryPlan::Rotate)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::ScalePixels { config } => {
            ScalePixelsPlan::new(config.clone(), dimensions)
                .map(DiscreteGeometryPlan::Scale)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::FinalScale { config } => {
            FinalScalePlan::from_config(dimensions, config.clone())
                .map(DiscreteGeometryPlan::FinalScale)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        ProcessingOperationKind::EnlargeCanvas { config } => {
            EnlargeCanvasPlan::new(*config, dimensions)
                .map(DiscreteGeometryPlan::EnlargeCanvas)
                .map_err(|error| node_error(node, error.to_string()))?
        }
        _ => unreachable!("checked discrete geometry kind"),
    };
    Ok(Some(boundary))
}

fn execute_boundary<F: Fn() -> bool>(
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
    let plan = BasicAdjPlan::resolve_with_cancellation(*config, raster, cancelled)
        .map_err(|error| node_error(node, error.to_string()))?;
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
    node_error(node, "frame-boundary execution was cancelled".to_owned())
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
