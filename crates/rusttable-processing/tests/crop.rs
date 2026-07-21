#![allow(clippy::cast_precision_loss, clippy::float_cmp)]

use rusttable_processing::operations::crop;
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

use crop::{
    CropCodecError, CropConfig, CropLegacyParametersV1, CropMigrationContext, CropParametersV3,
    CropPlan, CropPlanMode, CropRoi, MIN_OUTPUT_EDGE, crop_descriptor, decode_legacy, migrate_v1,
    migrate_v2,
};

fn pixel(value: f32) -> LinearRgb {
    let value = FiniteF32::new(value).expect("finite pixel");
    LinearRgb::new(value, value, value)
}

#[test]
fn modern_payload_round_trips_and_preserves_padding() {
    let config = CropConfig::new(0.125, 0.25, 0.875, 0.75, 3, -2).expect("config");
    let payload = CropParametersV3::with_padding(config, [1, 2, 3, 4, 5, 6, 7, 8]);
    let decoded = CropParametersV3::from_bytes(&payload.to_bytes()).expect("decode");
    assert_eq!(decoded, payload);
}

#[test]
fn descriptor_and_legacy_boundary_match_the_modern_contract() {
    let descriptor = crop_descriptor();
    descriptor.validate().expect("valid crop descriptor");
    assert_eq!(descriptor.id.compatibility_name, "crop");
    assert_eq!(descriptor.id.parameter_version, 3);
    assert_eq!(descriptor.migration.source_versions, vec![1, 2, 3]);
    assert_eq!(
        descriptor.roi,
        rusttable_processing::descriptor::RoiKind::Crop
    );

    let error = decode_legacy(1, &[0; 184]).expect_err("legacy payload stays opaque");
    assert_eq!(
        error,
        CropCodecError::LegacyPayloadOpaque {
            version: 1,
            expected: 184,
        }
    );
}

#[test]
fn legacy_migrations_add_and_remove_only_the_historical_alignment_field() {
    let v1 = CropLegacyParametersV1 {
        cx: 0.1,
        cy: 0.2,
        cw: 0.9,
        ch: 0.8,
        ratio_n: 3,
        ratio_d: 2,
    };
    let v2 = migrate_v1(v1).expect("v1 migration");
    assert!(!v2.aligned);
    let v3 = migrate_v2(v2, CropMigrationContext::new(4000, 3000, false)).expect("v2 migration");
    assert_eq!(
        v3.config(),
        CropConfig::new(0.1, 0.2, 0.9, 0.8, 3, 2).unwrap()
    );
}

#[test]
fn plan_clamps_normalized_bounds_and_copies_the_exact_integer_roi() {
    let dimensions = RasterDimensions::new(8, 6).unwrap();
    let config = CropConfig::new(-1.0, 0.25, 0.75, 0.9, 0, 0).unwrap();
    let plan = CropPlan::new(config, dimensions).expect("plan");
    assert_eq!(plan.input_roi(), CropRoi::new(0, 1, 6, 4).unwrap());

    let input: Vec<_> = (0..48).map(|value| pixel(value as f32)).collect();
    let output = plan.execute(&input).expect("crop execution");
    let values: Vec<_> = output
        .pixels()
        .iter()
        .map(|value| value.red().get())
        .collect();
    assert_eq!(output.dimensions(), RasterDimensions::new(6, 4).unwrap());
    assert_eq!(
        values,
        (8..14)
            .chain(16..22)
            .chain(24..30)
            .chain(32..38)
            .map(|value| value as f32)
            .collect::<Vec<_>>()
    );
}

#[test]
fn transforms_round_trip_using_the_planned_integer_offset() {
    let plan = CropPlan::new(
        CropConfig::new(0.25, 0.25, 0.75, 0.75, 0, 0).unwrap(),
        RasterDimensions::new(16, 12).unwrap(),
    )
    .unwrap();
    let mut points = [7.5, 8.25, 12.0, 9.0];
    let original = points;
    plan.forward_transform(&mut points).unwrap();
    plan.back_transform(&mut points).unwrap();
    assert_eq!(points, original);
}

#[test]
fn export_mode_applies_small_integer_ratio_alignment() {
    let plan = CropPlan::new_with_mode(
        CropConfig::new(0.0, 0.0, 0.99, 0.99, 2, 3).unwrap(),
        RasterDimensions::new(100, 100).unwrap(),
        CropPlanMode::Export,
    )
    .unwrap();
    assert_eq!(plan.output_dimensions().width() % 3, 0);
    assert_eq!(plan.output_dimensions().height() % 2, 0);
    assert!(plan.output_dimensions().width() >= MIN_OUTPUT_EDGE);
}
