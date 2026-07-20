use super::roi_contracts::{
    NodeRoiContract, ROI_SCHEMA_VERSION, RoiBackwardStep, RoiDescriptor, RoiDescriptorIdentity,
    RoiForwardStep, RoiNode, RoiPlan, RoiPlanIdentity, RoiPlanningError, RoiRect, RoiRequest,
    RoiRequestPolicy, RoiSupport,
};
use super::roi_distortion::enclose;
use super::roi_geometry::{
    dimensions_for_bounds, expand, map_orientation, scale_in, scale_out, scaled_dimensions,
    translate, translate_clipped,
};
use sha2::{Digest, Sha256};

/// Pure forward/reverse planner for prepared immutable node contracts.
#[derive(Debug, Clone, Copy, Default)]
pub struct RoiPlanner;

impl RoiPlanner {
    /// Plans every node in canonical order and then propagates the request in reverse.
    pub fn plan(
        &self,
        source: RoiDescriptor,
        nodes: &[RoiNode],
        request: RoiRequest,
    ) -> Result<RoiPlan, RoiPlanningError> {
        source
            .bounds
            .within(RoiRect::full(source.dimensions))
            .map_err(RoiPlanningError::InvalidSource)?;
        let mut forward = Vec::with_capacity(nodes.len());
        let mut input_descriptor = source;
        let mut input_roi = source.bounds;
        for node in nodes {
            let output_descriptor = forward_descriptor(node, input_descriptor)?;
            let output_roi = forward_roi(node, input_descriptor, output_descriptor, input_roi)?;
            forward.push(RoiForwardStep {
                node: node.clone(),
                input_descriptor,
                output_descriptor,
                input_roi,
                output_roi,
            });
            input_descriptor = output_descriptor;
            input_roi = output_roi;
        }
        let requested_output = match request.output.within(input_descriptor.bounds) {
            Ok(roi) => roi,
            Err(_) if request.policy == RoiRequestPolicy::ClipToFinalBounds => {
                request.output.intersection(input_descriptor.bounds)
            }
            Err(_) => {
                return Err(RoiPlanningError::RequestOutOfBounds {
                    requested: request.output,
                    bounds: input_descriptor.bounds,
                });
            }
        };
        let mut backward = Vec::with_capacity(nodes.len());
        let mut required = requested_output;
        for (index, node) in nodes.iter().enumerate().rev() {
            let step = &forward[index];
            let input_required = reverse_roi(
                node,
                step.input_descriptor,
                step.output_descriptor,
                required,
            )?
            .intersection(step.input_descriptor.bounds);
            backward.push(RoiBackwardStep {
                node: node.clone(),
                output_required: required,
                input_required,
            });
            required = input_required;
        }
        let identity = plan_identity(source, request, requested_output, &forward, &backward);
        Ok(RoiPlan {
            source,
            request,
            final_descriptor: input_descriptor,
            requested_output,
            forward,
            backward,
            source_required: required,
            identity,
        })
    }
}

fn forward_descriptor(
    node: &RoiNode,
    input: RoiDescriptor,
) -> Result<RoiDescriptor, RoiPlanningError> {
    let (dimensions, bounds) = match &node.contract {
        NodeRoiContract::Identity
        | NodeRoiContract::Neighborhood { .. }
        | NodeRoiContract::FullImage => (input.dimensions, input.bounds),
        NodeRoiContract::Crop { output_bounds, .. }
        | NodeRoiContract::Canvas { output_bounds, .. } => (
            dimensions_for_bounds(*output_bounds).map_err(|reason| invalid(node, reason))?,
            *output_bounds,
        ),
        NodeRoiContract::Scale {
            rational_x,
            rational_y,
            ..
        } => {
            let dimensions = scaled_dimensions(input.dimensions, *rational_x, *rational_y)
                .map_err(|reason| invalid(node, reason))?;
            (dimensions, RoiRect::full(dimensions))
        }
        NodeRoiContract::Orientation { orientation } => {
            let dimensions = orientation.output_dimensions(input.dimensions);
            (dimensions, RoiRect::full(dimensions))
        }
        NodeRoiContract::Distortion { binding } => {
            let dimensions = binding.output_dimensions(input.dimensions);
            (dimensions, RoiRect::full(dimensions))
        }
        NodeRoiContract::Unsupported { reason } => {
            return Err(RoiPlanningError::Unsupported {
                operation_id: node.operation_id,
                compatibility_name: node.compatibility_name.clone(),
                reason: reason.clone(),
            });
        }
    };
    let identity = descriptor_identity(node, input, dimensions);
    RoiDescriptor::new(dimensions, bounds, identity)
        .map_err(|error| invalid(node, error.to_string()))
}

fn forward_roi(
    node: &RoiNode,
    input: RoiDescriptor,
    output: RoiDescriptor,
    roi: RoiRect,
) -> Result<RoiRect, RoiPlanningError> {
    match &node.contract {
        NodeRoiContract::Distortion { binding } => {
            enclose(node, binding, roi, false, output.bounds)
        }
        NodeRoiContract::Orientation { orientation } => {
            map_orientation(roi, input.dimensions, *orientation)
                .map_err(|reason| invalid(node, reason))
        }
        NodeRoiContract::Scale {
            rational_x,
            rational_y,
            ..
        } => scale_out(roi, *rational_x, *rational_y).map_err(|reason| invalid(node, reason)),
        NodeRoiContract::Crop { .. } | NodeRoiContract::Canvas { .. } => Ok(output.bounds),
        _ => Ok(roi.intersection(output.bounds)),
    }
}

fn reverse_roi(
    node: &RoiNode,
    input: RoiDescriptor,
    output: RoiDescriptor,
    roi: RoiRect,
) -> Result<RoiRect, RoiPlanningError> {
    match &node.contract {
        NodeRoiContract::Identity => Ok(roi),
        NodeRoiContract::Neighborhood {
            support,
            asymmetric_support,
        } => {
            let support = RoiSupport::symmetric(*support)
                .add(*asymmetric_support)
                .map_err(|_| RoiPlanningError::Arithmetic {
                    operation_id: node.operation_id,
                })?;
            expand(roi, support).map_err(|_| RoiPlanningError::Arithmetic {
                operation_id: node.operation_id,
            })
        }
        NodeRoiContract::Crop {
            output_bounds,
            input_offset,
        } => translate(
            roi,
            i64::from(input_offset.0) - i64::from(output_bounds.x),
            i64::from(input_offset.1) - i64::from(output_bounds.y),
        )
        .map_err(|reason| invalid(node, reason)),
        NodeRoiContract::Scale {
            rational_x,
            rational_y,
            filter_support,
        } => {
            let mapped =
                scale_in(roi, *rational_x, *rational_y).map_err(|reason| invalid(node, reason))?;
            expand(mapped, *filter_support).map_err(|_| RoiPlanningError::Arithmetic {
                operation_id: node.operation_id,
            })
        }
        NodeRoiContract::Canvas { source_offset, .. } => Ok(translate_clipped(
            roi,
            -i64::from(source_offset.0),
            -i64::from(source_offset.1),
            input.bounds,
        )),
        NodeRoiContract::Orientation { orientation } => {
            map_orientation(roi, output.dimensions, orientation.inverse())
                .map_err(|reason| invalid(node, reason))
        }
        NodeRoiContract::Distortion { binding } => enclose(node, binding, roi, true, input.bounds),
        NodeRoiContract::FullImage => Ok(input.bounds),
        NodeRoiContract::Unsupported { reason } => Err(RoiPlanningError::Unsupported {
            operation_id: node.operation_id,
            compatibility_name: node.compatibility_name.clone(),
            reason: reason.clone(),
        }),
    }
}

fn invalid(node: &RoiNode, reason: impl Into<String>) -> RoiPlanningError {
    RoiPlanningError::InvalidContract {
        operation_id: node.operation_id,
        reason: reason.into(),
    }
}

fn descriptor_identity(
    node: &RoiNode,
    input: RoiDescriptor,
    dimensions: rusttable_image::ImageDimensions,
) -> RoiDescriptorIdentity {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.pixelpipe.roi.descriptor.v1");
    hasher.update(node.operation_id.to_le_bytes());
    hasher.update(node.compatibility_name.as_bytes());
    hasher.update(input.identity.as_bytes());
    hasher.update(dimensions.width().to_le_bytes());
    hasher.update(dimensions.height().to_le_bytes());
    RoiDescriptorIdentity(hasher.finalize().into())
}

fn plan_identity(
    source: RoiDescriptor,
    request: RoiRequest,
    requested: RoiRect,
    forward: &[RoiForwardStep],
    backward: &[RoiBackwardStep],
) -> RoiPlanIdentity {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.pixelpipe.roi.plan.v1");
    hasher.update(ROI_SCHEMA_VERSION.to_le_bytes());
    write_descriptor(&mut hasher, source);
    write_rect(&mut hasher, request.output);
    hasher.update([match request.policy {
        RoiRequestPolicy::RejectOutOfBounds => 0,
        RoiRequestPolicy::ClipToFinalBounds => 1,
    }]);
    write_rect(&mut hasher, requested);
    for step in forward {
        write_rect(&mut hasher, step.input_roi);
        write_rect(&mut hasher, step.output_roi);
        write_descriptor(&mut hasher, step.output_descriptor);
        write_node(&mut hasher, &step.node);
    }
    for step in backward {
        write_rect(&mut hasher, step.output_required);
        write_rect(&mut hasher, step.input_required);
        write_node(&mut hasher, &step.node);
    }
    RoiPlanIdentity(hasher.finalize().into())
}

fn write_descriptor(hasher: &mut Sha256, descriptor: RoiDescriptor) {
    hasher.update(descriptor.identity.as_bytes());
    hasher.update(descriptor.dimensions.width().to_le_bytes());
    hasher.update(descriptor.dimensions.height().to_le_bytes());
    write_rect(hasher, descriptor.bounds);
}

fn write_rect(hasher: &mut Sha256, rect: RoiRect) {
    hasher.update(rect.x.to_le_bytes());
    hasher.update(rect.y.to_le_bytes());
    hasher.update(rect.width.to_le_bytes());
    hasher.update(rect.height.to_le_bytes());
}

fn write_node(hasher: &mut Sha256, node: &RoiNode) {
    hasher.update(node.operation_id.to_le_bytes());
    hasher.update((node.compatibility_name.len() as u64).to_le_bytes());
    hasher.update(node.compatibility_name.as_bytes());
    hasher.update(format!("{:?}", node.contract).as_bytes());
}
