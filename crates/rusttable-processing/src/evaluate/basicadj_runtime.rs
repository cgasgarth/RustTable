use std::collections::BTreeMap;

use sha2::Digest;

use super::{
    BasicAdjPlanSet, FrameBoundaryMode, FrameBoundaryOptions,
    apply_operation_with_profile_with_cancellation, graph_has_frame_geometry,
};
use crate::{
    BasicAdjAnalysisError, BasicAdjAnalysisRaster, BasicAdjConfig, BasicAdjPlan,
    CompiledOperationGraph, EvaluationError, LinearRgb, PipelineStepIndex, RasterDimensions,
};
use rusttable_core::OperationId;

/// Resolves automatic basic-adjustment nodes against the full graph input.
///
/// Geometry graphs are evaluated through the frame-boundary planner so an
/// analysis node sees the same dimensions and pixels as production execution.
/// Non-geometry graphs retain the existing sequential analysis contract.
///
/// # Errors
///
/// Returns the first invalid analysis raster, automatic-plan, or operation
/// execution error encountered while preparing the graph.
pub fn prepare_basicadj_plans(
    graph: &CompiledOperationGraph,
    input: &crate::WorkingRgbImage,
) -> Result<BasicAdjPlanSet, EvaluationError> {
    prepare_basicadj_plans_with_cancellation(graph, input, || false)
}

/// Resolves automatic basic-adjustment nodes with deterministic cancellation
/// checks during full-frame analysis and preceding-node execution.
///
/// # Errors
///
/// Returns [`EvaluationError::Cancelled`] with the active node context when
/// cancellation is observed, without publishing a partial plan set.
pub fn prepare_basicadj_plans_with_cancellation<C: Fn() -> bool>(
    graph: &CompiledOperationGraph,
    input: &crate::WorkingRgbImage,
    cancelled: C,
) -> Result<BasicAdjPlanSet, EvaluationError> {
    if graph_has_frame_geometry(graph) {
        let alpha = vec![1.0; input.pixel_slice().len()];
        return super::evaluate_graph_at_frame_boundaries_with_plans(
            graph,
            input,
            &alpha,
            FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
            None,
            &cancelled,
        )
        .map(|evaluated| evaluated.basicadj_plans().clone());
    }

    let mut current = input.pixel_slice().to_vec();
    let mut frame = input.frame();
    let mut terminal = None;
    let mut plans = BTreeMap::new();
    for node in graph.nodes() {
        let operation = node.operation();
        if let crate::ProcessingOperationKind::BasicAdj { config } = operation.kind()
            && operation.is_enabled()
            && operation.opacity().get().to_bits() != 0.0_f32.to_bits()
            && config.auto_controls().is_active()
        {
            let plan = resolve_auto_basicadj_plan(
                *config,
                input.dimensions(),
                &current,
                node.pipeline_step_index(),
                operation.operation_id(),
                &cancelled,
            )?;
            plans.insert(operation.operation_id(), plan);
        }
        let plan_set = BasicAdjPlanSet {
            plans: plans.clone(),
            identity: [0; 32],
        };
        apply_operation_with_profile_with_cancellation(
            node.pipeline_step_index(),
            operation,
            &mut current,
            input.dimensions(),
            0,
            Some(&plan_set),
            &mut frame,
            &mut terminal,
            None,
            &cancelled,
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

fn resolve_auto_basicadj_plan<C: Fn() -> bool>(
    config: BasicAdjConfig,
    dimensions: RasterDimensions,
    current: &[LinearRgb],
    step_index: PipelineStepIndex,
    operation_id: OperationId,
    cancelled: &C,
) -> Result<BasicAdjPlan, EvaluationError> {
    let operation_error = |reason: String| EvaluationError::OperationExecution {
        step_index,
        operation_id,
        reason,
    };
    let raster = BasicAdjAnalysisRaster::new(dimensions, current, None)
        .map_err(|error| operation_error(error.to_string()))?;
    BasicAdjPlan::resolve_with_cancellation(config, raster, cancelled).map_err(
        |error| match error {
            BasicAdjAnalysisError::Cancelled => EvaluationError::Cancelled {
                step_index,
                operation_id,
            },
            error => operation_error(error.to_string()),
        },
    )
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::{BasicAdjAutoControls, FiniteF32};

    #[test]
    fn automatic_plan_preparation_cancels_between_analysis_rows() {
        let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
        let sample = LinearRgb::new(
            FiniteF32::new(0.1).expect("red"),
            FiniteF32::new(0.2).expect("green"),
            FiniteF32::new(0.3).expect("blue"),
        );
        let pixels = [sample; 16];
        let step_index = PipelineStepIndex::new(3);
        let operation_id = OperationId::new(0xba51).expect("operation ID");
        let polls = Cell::new(0_usize);

        let error = resolve_auto_basicadj_plan(
            BasicAdjConfig::defaults().with_auto_controls(BasicAdjAutoControls::all()),
            dimensions,
            &pixels,
            step_index,
            operation_id,
            &|| {
                let next = polls.get() + 1;
                polls.set(next);
                next >= 2
            },
        )
        .expect_err("mid-analysis cancellation must not return a partial plan");

        assert_eq!(
            error,
            EvaluationError::Cancelled {
                step_index,
                operation_id,
            }
        );
        assert_eq!(polls.get(), 2);
    }
}
