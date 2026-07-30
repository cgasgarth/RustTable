use rusttable_image::Roi;
use rusttable_processing::operations::liquify::{
    LiquifyConfig, LiquifyInterpolation, LiquifyNode, LiquifyNodeType, LiquifyParametersV1,
    LiquifyPathKind, LiquifyPlan, LiquifyPoint, LiquifyStatus, LiquifyWarpType,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

fn point(x: f32, y: f32) -> LiquifyPoint {
    LiquifyPoint::new(x, y).expect("fixture point")
}

fn node(path: LiquifyPathKind, center: (f32, f32), strength: (f32, f32)) -> LiquifyNode {
    LiquifyNode::new(
        path,
        LiquifyNodeType::Cusp,
        point(center.0, center.1),
        point(strength.0, strength.1),
        point(center.0 + 3.0, center.1),
        0.25,
        0.75,
        LiquifyWarpType::Linear,
        LiquifyStatus::NONE,
        point(center.0, center.1),
        point(center.0, center.1),
    )
    .expect("fixture node")
}

fn line_config() -> LiquifyConfig {
    let first = node(LiquifyPathKind::MoveToV1, (3.0, 3.0), (4.0, 3.0)).with_links(-1, 0, 1);
    let second = node(LiquifyPathKind::LineToV1, (7.0, 3.0), (8.0, 3.0)).with_links(0, 1, -1);
    LiquifyConfig::new(vec![first, second]).expect("line config")
}

#[test]
fn canonical_fixture_round_trips_node_order_and_path_bytes() {
    let config = line_config();
    let bytes = config.to_bytes().expect("encode");
    assert_eq!(
        bytes.len(),
        8 + 2 * rusttable_processing::LIQUIFY_PARAMETER_BYTES
    );
    let decoded = LiquifyConfig::from_bytes(&bytes).expect("decode");
    assert_eq!(decoded.nodes(), config.nodes());
    assert_eq!(decoded.to_bytes().expect("re-encode"), bytes);
    assert_eq!(
        LiquifyParametersV1::from_bytes(&bytes)
            .expect("typed decode")
            .nodes(),
        config.nodes()
    );
}

#[test]
fn inverse_field_is_frozen_and_point_mask_image_transforms_share_it() {
    let dimensions = RasterDimensions::new(12, 8).expect("dimensions");
    let plan = LiquifyPlan::new_with_interpolation(
        line_config(),
        dimensions,
        LiquifyInterpolation::Bilinear,
    )
    .expect("plan");
    assert!(plan.stamps() > 1);
    assert!(plan.maximum_displacement() > 0);
    let pixel_count = usize::try_from(dimensions.pixel_count()).expect("test dimensions fit");
    assert_eq!(plan.field().len(), pixel_count);
    assert!(plan.gpu_dispatch().expect("dispatch").uses_cpu_fallback());

    let mut points = [3.0, 3.0, 7.0, 3.0];
    plan.forward_transform(&mut points).expect("forward points");
    let transformed = points;
    plan.back_transform(&mut points).expect("back points");
    assert!((points[0] - 3.0).abs() < 0.1);
    assert!((points[1] - 3.0).abs() < 0.1);
    assert!(
        transformed
            .iter()
            .zip([3.0, 3.0, 7.0, 3.0])
            .any(|(actual, expected)| (actual - expected).abs() > f32::EPSILON)
    );

    let input = (0..dimensions.pixel_count())
        .map(|value| {
            let value = f32::from(u16::try_from(value).expect("test value fits"));
            LinearRgb::new(
                FiniteF32::new(value).expect("red"),
                FiniteF32::new(value + 1.0).expect("green"),
                FiniteF32::new(value + 2.0).expect("blue"),
            )
        })
        .collect::<Vec<_>>();
    let output = plan.execute(&input, || false).expect("image");
    let mask = plan
        .execute_mask(&vec![0.0; pixel_count], || false)
        .expect("mask");
    assert_eq!(output.pixels().len(), input.len());
    assert_eq!(mask.len(), input.len());
    assert!(
        plan.input_roi(Roi::new(4, 2, 2, 2).expect("ROI"))
            .expect("expanded ROI")
            .x()
            < 4
    );
}

#[test]
fn identity_and_cancellation_are_exact_and_publish_nothing() {
    let dimensions = RasterDimensions::new(2, 2).expect("dimensions");
    let plan = LiquifyPlan::new(LiquifyConfig::identity(), dimensions).expect("identity plan");
    let input = vec![
        LinearRgb::new(
            FiniteF32::new(0.1).expect("finite"),
            FiniteF32::new(0.2).expect("finite"),
            FiniteF32::new(0.3).expect("finite")
        );
        4
    ];
    let output = plan.execute(&input, || false).expect("identity execution");
    assert_eq!(output.pixels(), input.as_slice());
    assert_eq!(
        plan.execute(&input, || true).expect_err("cancelled"),
        rusttable_processing::LiquifyExecutionError::Cancelled
    );
}

#[test]
fn unknown_future_status_is_blocking() {
    let mut bytes = LiquifyConfig::identity()
        .to_bytes()
        .expect("identity bytes");
    bytes[6] = 1;
    bytes[7] = 0;
    bytes.extend_from_slice(&[0; rusttable_processing::LIQUIFY_PARAMETER_BYTES]);
    bytes[8 + 41] = 0x80;
    let config = LiquifyConfig::from_bytes(&bytes).expect("opaque future status");
    assert_eq!(config.to_bytes().expect("preserved bytes"), bytes);
    assert_eq!(
        LiquifyPlan::new(config, RasterDimensions::new(2, 2).expect("dimensions"))
            .expect_err("opaque status blocks execution"),
        rusttable_processing::LiquifyExecutionError::UnsupportedOpaquePayload(1)
    );
}
