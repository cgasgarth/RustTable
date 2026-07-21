#![allow(clippy::cast_precision_loss, clippy::float_cmp)]

use rusttable_processing::RasterDimensions;
use rusttable_processing::operations::clipping::{
    ClippingConfig, ClippingInterpolation, ClippingParametersV5, ClippingPlan, decode_history,
    migrate_history,
};
use rusttable_processing::operations::perspective::Point;

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn config(update: impl FnOnce(&mut ClippingParametersV5)) -> ClippingConfig {
    let mut parameters = ClippingParametersV5::default();
    update(&mut parameters);
    ClippingConfig::new(parameters).expect("config")
}

#[test]
fn identity_plan_preserves_dimensions_and_pixels() {
    let plan = ClippingPlan::new(
        dimensions(8, 6),
        config(|p| p.crop_auto = false),
        ClippingInterpolation::Bilinear,
    )
    .expect("plan");
    assert!(plan.is_identity());
    assert_eq!(plan.output_dimensions(), dimensions(8, 6));
    let input = (0..48)
        .map(|value| {
            let value = value as f32;
            rusttable_processing::LinearRgb::new(
                rusttable_processing::FiniteF32::new(value).expect("red"),
                rusttable_processing::FiniteF32::new(value + 1.0).expect("green"),
                rusttable_processing::FiniteF32::new(value + 2.0).expect("blue"),
            )
        })
        .collect::<Vec<_>>();
    let output = plan.execute(&input).expect("execute");
    assert_eq!(output.pixels(), input.as_slice());
}

#[test]
fn explicit_aspect_crop_changes_output_geometry() {
    let plan = ClippingPlan::new(
        dimensions(100, 80),
        config(|p| {
            p.cx = 0.25;
            p.cw = 0.75;
            p.crop_auto = false;
        }),
        ClippingInterpolation::Nearest,
    )
    .expect("plan");
    assert_eq!(plan.output_dimensions(), dimensions(50, 80));
    assert_eq!(plan.crop().width(), 50.0);
}

#[test]
fn rotation_changes_bounds_and_round_trips_points() {
    let plan = ClippingPlan::new(
        dimensions(100, 60),
        config(|p| {
            p.angle = 90.0;
            p.crop_auto = false;
        }),
        ClippingInterpolation::Bilinear,
    )
    .expect("plan");
    assert_eq!(plan.output_dimensions(), dimensions(60, 100));
    let point = Point::new(20.0, 15.0);
    let transformed = plan.forward_point(point).expect("forward");
    let restored = plan.back_point(transformed).expect("back");
    assert!((restored.x() - point.x()).abs() < 1.0e-6);
    assert!((restored.y() - point.y()).abs() < 1.0e-6);
}

#[test]
fn legacy_history_migrates_and_unknown_versions_remain_opaque() {
    let mut bytes = [0_u8; 28];
    bytes[0..4].copy_from_slice(&12.0_f32.to_le_bytes());
    let history = decode_history(3, &bytes).expect("history");
    let config = migrate_history(&history).expect("migration");
    assert_eq!(config.parameters().angle, 12.0);
    assert!(matches!(
        decode_history(99, &[1, 2, 3]).expect("opaque"),
        rusttable_processing::operations::clipping::ClippingHistory::Opaque { version: 99, .. }
    ));
}
