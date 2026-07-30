use rusttable_image::{ImageDimensions, Orientation};
use rusttable_pixelpipe::{
    DistortionBinding, FillValue, NodeRoiContract, RationalScale, RoiDescriptor, RoiNode,
    RoiPlanner, RoiRect, RoiRequest, RoiRequestPolicy, RoiSupport,
};

fn descriptor(width: u32, height: u32) -> RoiDescriptor {
    let dimensions = ImageDimensions::new(width, height).expect("dimensions");
    RoiDescriptor::source(dimensions, RoiRect::full(dimensions), [7; 32]).expect("descriptor")
}

fn plan(nodes: &[RoiNode], request: RoiRect) -> rusttable_pixelpipe::RoiPlan {
    RoiPlanner
        .plan(
            descriptor(16, 12),
            nodes,
            RoiRequest::new(request, RoiRequestPolicy::RejectOutOfBounds),
        )
        .expect("ROI plan")
}

#[test]
fn identity_and_asymmetric_neighborhood_propagate_in_both_directions() {
    let nodes = [
        RoiNode::new(1, "identity", NodeRoiContract::Identity),
        RoiNode::new(
            2,
            "neighborhood",
            NodeRoiContract::Neighborhood {
                support: 1,
                asymmetric_support: RoiSupport::new(2, 3, 4, 5),
            },
        ),
    ];
    let plan = plan(&nodes, RoiRect::new(4, 3, 2, 2).expect("request"));

    assert_eq!(plan.forward().len(), 2);
    assert_eq!(
        plan.forward()[0].output_roi(),
        RoiRect::full(ImageDimensions::new(16, 12).unwrap())
    );
    assert_eq!(plan.backward()[0].node().operation_id(), 2);
    assert_eq!(
        plan.backward()[0].input_required(),
        RoiRect::new(1, 0, 9, 11).unwrap()
    );
    assert_eq!(plan.source_required(), RoiRect::new(1, 0, 9, 11).unwrap());
}

#[test]
fn crop_scale_canvas_orientation_and_full_image_are_composable() {
    let scale = RationalScale::new(3, 2).expect("scale");
    let nodes = [
        RoiNode::new(
            10,
            "crop",
            NodeRoiContract::Crop {
                output_bounds: RoiRect::new(2, 1, 12, 10).unwrap(),
                input_offset: (4, 3),
            },
        ),
        RoiNode::new(
            11,
            "scale",
            NodeRoiContract::Scale {
                rational_x: scale,
                rational_y: scale,
                filter_support: RoiSupport::symmetric(1),
            },
        ),
        RoiNode::new(
            12,
            "canvas",
            NodeRoiContract::Canvas {
                output_bounds: RoiRect::new(0, 0, 24, 20).unwrap(),
                source_offset: (2, 1),
                fill: FillValue::Transparent,
            },
        ),
        RoiNode::new(
            13,
            "orientation",
            NodeRoiContract::Orientation {
                orientation: Orientation::Rotate90,
            },
        ),
        RoiNode::new(14, "analysis", NodeRoiContract::FullImage),
    ];
    let plan = RoiPlanner
        .plan(
            descriptor(16, 12),
            &nodes,
            RoiRequest::new(
                RoiRect::new(1, 1, 2, 2).unwrap(),
                RoiRequestPolicy::ClipToFinalBounds,
            ),
        )
        .expect("composed plan");

    assert_eq!(plan.forward().len(), 5);
    assert_eq!(
        plan.final_descriptor().dimensions(),
        ImageDimensions::new(20, 24).unwrap()
    );
    assert_eq!(plan.source_required(), RoiRect::new(4, 3, 12, 9).unwrap());
    assert_ne!(plan.identity().as_bytes(), [0; 32]);
}

#[test]
fn rational_scale_uses_edge_floor_ceil_and_support_before_clipping() {
    let scale = RationalScale::new(3, 2).unwrap();
    let node = RoiNode::new(
        20,
        "scale",
        NodeRoiContract::Scale {
            rational_x: scale,
            rational_y: scale,
            filter_support: RoiSupport::symmetric(2),
        },
    );
    let plan = RoiPlanner
        .plan(
            descriptor(10, 10),
            &[node],
            RoiRequest::new(
                RoiRect::new(1, 1, 3, 3).unwrap(),
                RoiRequestPolicy::RejectOutOfBounds,
            ),
        )
        .expect("scaled plan");
    assert_eq!(
        plan.final_descriptor().dimensions(),
        ImageDimensions::new(15, 15).unwrap()
    );
    assert_eq!(plan.source_required(), RoiRect::new(0, 0, 5, 5).unwrap());
}

#[test]
fn distortion_enclosure_is_deterministic_and_singular_mappings_fail() {
    let binding =
        DistortionBinding::affine("shift", [1.0, 0.0, 1.25, 0.0, 1.0, -0.5], None, 0.001).unwrap();
    let node = RoiNode::new(30, "homography", NodeRoiContract::Distortion { binding });
    let first = plan(
        std::slice::from_ref(&node),
        RoiRect::new(3, 4, 2, 2).unwrap(),
    );
    let second = plan(
        std::slice::from_ref(&node),
        RoiRect::new(3, 4, 2, 2).unwrap(),
    );
    assert_eq!(first.identity(), second.identity());
    assert!(first.source_required().x() <= 2);
    assert!(first.source_required().y() <= 4);

    assert!(
        DistortionBinding::affine("singular", [1.0, 2.0, 0.0, 2.0, 4.0, 0.0], None, 0.1).is_err()
    );
}

#[test]
fn request_policy_controls_final_clipping_and_unsupported_nodes_block_planning() {
    let node = RoiNode::new(
        40,
        "unsupported",
        NodeRoiContract::Unsupported {
            reason: "pending binding".to_owned(),
        },
    );
    let error = RoiPlanner
        .plan(
            descriptor(8, 8),
            &[node],
            RoiRequest::new(
                RoiRect::new(0, 0, 9, 1).unwrap(),
                RoiRequestPolicy::RejectOutOfBounds,
            ),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        rusttable_pixelpipe::RoiPlanningError::Unsupported {
            operation_id: 40,
            ..
        }
    ));

    let clipped = RoiPlanner
        .plan(
            descriptor(8, 8),
            &[],
            RoiRequest::new(
                RoiRect::new(7, 7, 4, 4).unwrap(),
                RoiRequestPolicy::ClipToFinalBounds,
            ),
        )
        .expect("clip request");
    assert_eq!(
        clipped.requested_output(),
        RoiRect::new(7, 7, 1, 1).unwrap()
    );
}
