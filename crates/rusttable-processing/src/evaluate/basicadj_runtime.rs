use std::collections::BTreeMap;

use sha2::Digest;

use super::{
    BasicAdjPlanSet, FrameBoundaryMode, FrameBoundaryOptions, apply_operation_with_profile,
    graph_has_frame_geometry,
};
use crate::{BasicAdjAnalysisRaster, BasicAdjPlan, CompiledOperationGraph, EvaluationError};

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
    if graph_has_frame_geometry(graph) {
        let alpha = vec![1.0; input.pixel_slice().len()];
        return super::evaluate_graph_at_frame_boundaries_with_plans(
            graph,
            input,
            &alpha,
            FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
            None,
            || false,
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
            let raster = BasicAdjAnalysisRaster::new(input.dimensions(), &current, None).map_err(
                |error| EvaluationError::OperationExecution {
                    step_index: node.pipeline_step_index(),
                    operation_id: operation.operation_id(),
                    reason: error.to_string(),
                },
            )?;
            let plan = BasicAdjPlan::resolve(*config, raster).map_err(|error| {
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
        apply_operation_with_profile(
            node.pipeline_step_index(),
            operation,
            &mut current,
            input.dimensions(),
            0,
            Some(&plan_set),
            &mut frame,
            &mut terminal,
            None,
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
