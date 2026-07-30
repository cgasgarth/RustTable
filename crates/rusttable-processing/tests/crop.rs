#![allow(clippy::cast_precision_loss, clippy::float_cmp)]

use rusttable_processing::operations::crop;
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

use crop::{
    CROP_LEGACY_V1_BYTES, CROP_LEGACY_V2_BYTES, CROP_PARAMETER_BYTES, CropCodecError, CropConfig,
    CropLegacyParametersV1, CropLegacyParametersV2, CropMigrationContext, CropParametersV3,
    CropPlan, CropPlanMode, CropRoi, MIN_OUTPUT_EDGE, crop_descriptor, decode_legacy, migrate_v1,
    migrate_v2,
};

fn pixel(value: f32) -> LinearRgb {
    let value = FiniteF32::new(value).expect("finite pixel");
    LinearRgb::new(value, value, value)
}

#[test]
fn modern_payload_is_the_exact_24_byte_native_v3_layout() {
    let config = CropConfig::new(0.125, 0.25, 0.875, 0.75, 3, -2).expect("config");
    let payload = CropParametersV3::new(config);
    let bytes = payload.to_bytes();
    assert_eq!(CROP_PARAMETER_BYTES, 24);
    assert_eq!(
        bytes,
        [
            0x00, 0x00, 0x00, 0x3e, // cx = 0.125
            0x00, 0x00, 0x80, 0x3e, // cy = 0.25
            0x00, 0x00, 0x60, 0x3f, // cw = 0.875
            0x00, 0x00, 0x40, 0x3f, // ch = 0.75
            0x03, 0x00, 0x00, 0x00, // ratio_n = 3
            0xfe, 0xff, 0xff, 0xff, // ratio_d = -2
        ]
    );
    assert_eq!(CropParametersV3::from_bytes(&bytes).unwrap(), payload);
}

#[test]
fn crop_codecs_reject_malformed_and_nonfinite_payloads() {
    assert_eq!(
        CropParametersV3::from_bytes(&[0; CROP_PARAMETER_BYTES - 1]),
        Err(CropCodecError::InvalidLength {
            expected: CROP_PARAMETER_BYTES,
            actual: CROP_PARAMETER_BYTES - 1,
        })
    );
    let mut nonfinite_v3 = [0; CROP_PARAMETER_BYTES];
    nonfinite_v3[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert_eq!(
        CropParametersV3::from_bytes(&nonfinite_v3),
        Err(CropCodecError::Config(crop::CropConfigError::NonFinite))
    );

    let mut nonfinite_v2 = [0; CROP_LEGACY_V2_BYTES];
    nonfinite_v2[4..8].copy_from_slice(&f32::INFINITY.to_le_bytes());
    assert_eq!(
        decode_legacy(2, &nonfinite_v2),
        Err(CropCodecError::Config(crop::CropConfigError::NonFinite))
    );
}

#[test]
fn descriptor_and_legacy_boundary_match_native_abi_sizes() {
    let descriptor = crop_descriptor();
    descriptor.validate().expect("valid crop descriptor");
    assert_eq!(descriptor.id.compatibility_name, "crop");
    assert_eq!(descriptor.id.parameter_version, 3);
    assert_eq!(descriptor.migration.source_versions, vec![1, 2, 3]);
    assert_eq!(
        descriptor.roi,
        rusttable_processing::descriptor::RoiKind::Crop
    );
    assert_eq!(CROP_LEGACY_V1_BYTES, 24);
    assert_eq!(CROP_LEGACY_V2_BYTES, 28);

    let v1 = CropLegacyParametersV1 {
        cx: 0.125,
        cy: 0.25,
        cw: 0.875,
        ch: 0.75,
        ratio_n: 3,
        ratio_d: -2,
    };
    let v2 = CropLegacyParametersV2 {
        aligned: -7,
        ..migrate_v1(v1).unwrap()
    };
    assert_eq!(&v2.to_bytes()[..24], v1.to_bytes().as_slice());
    assert_eq!(&v2.to_bytes()[24..28], &(-7_i32).to_le_bytes());
    assert_eq!(
        CropLegacyParametersV1::from_bytes(&v1.to_bytes()).unwrap(),
        v1
    );
    assert_eq!(
        CropLegacyParametersV2::from_bytes(&v2.to_bytes()).unwrap(),
        v2
    );

    assert_eq!(
        decode_legacy(1, &v1.to_bytes()),
        Err(CropCodecError::LegacyPayloadOpaque {
            version: 1,
            expected: 24,
        })
    );
    assert_eq!(
        decode_legacy(2, &v2.to_bytes()),
        Err(CropCodecError::LegacyPayloadOpaque {
            version: 2,
            expected: 28,
        })
    );
}

#[test]
fn legacy_migrations_only_add_then_drop_the_native_alignment_field_without_context() {
    let v1 = CropLegacyParametersV1 {
        cx: 0.1,
        cy: 0.2,
        cw: 0.9,
        ch: 0.8,
        ratio_n: 3,
        ratio_d: 2,
    };
    let v2 = migrate_v1(v1).expect("v1 migration");
    assert_eq!(v2.aligned, 0);
    assert_eq!(&v2.to_bytes()[..24], v1.to_bytes().as_slice());
    let v3 = migrate_v2(v2, None).expect("context-free v2 migration");
    assert_eq!(v3.to_bytes(), v1.to_bytes());
}

#[test]
fn v2_square_crop_recovery_runs_only_with_available_image_context() {
    let v2 = CropLegacyParametersV2 {
        cx: 0.0,
        cy: 0.0,
        cw: 0.375,
        ch: 0.5,
        ratio_n: 0,
        ratio_d: 1,
        aligned: 1,
    };
    let unchanged = migrate_v2(v2, None).unwrap();
    assert_eq!(unchanged.to_bytes(), v2.to_bytes()[..24]);

    let recovered = migrate_v2(v2, Some(CropMigrationContext::new(4000, 3000, false))).unwrap();
    assert_eq!(recovered.config().cw().get(), 0.375);
    assert_eq!(recovered.config().ch().get(), 0.375);
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

#[test]
fn export_aspect_correction_is_diagnostic_and_finalizes_from_the_raw_roi() {
    let plan = CropPlan::new_with_mode(
        CropConfig::new(0.0, 0.0, 0.5, 0.5, 1, 2).unwrap(),
        RasterDimensions::new(2000, 1600).unwrap(),
        CropPlanMode::Export,
    )
    .unwrap();

    // Native computes a diagnostic 1000x500 aspect-corrected size, but then
    // aligns and returns the raw 1000x800 ROI.
    assert_eq!(plan.input_roi(), CropRoi::new(0, 0, 1000, 800).unwrap());
}

#[test]
fn export_alignment_preserves_raw_portrait_and_landscape_dimensions() {
    let landscape = CropPlan::new_with_mode(
        CropConfig::new(0.0, 0.0, 0.92, 0.99, 2, 3).unwrap(),
        RasterDimensions::new(100, 80).unwrap(),
        CropPlanMode::Export,
    )
    .unwrap();
    assert_eq!(landscape.input_roi(), CropRoi::new(1, 0, 90, 78).unwrap());

    let portrait = CropPlan::new_with_mode(
        CropConfig::new(0.0, 0.0, 0.99, 0.92, 2, 3).unwrap(),
        RasterDimensions::new(80, 100).unwrap(),
        CropPlanMode::Export,
    )
    .unwrap();
    assert_eq!(portrait.input_roi(), CropRoi::new(0, 1, 78, 90).unwrap());
}

#[test]
fn export_alignment_uses_raw_roi_orientation_before_aspect_correction() {
    let plan = CropPlan::new_with_mode(
        CropConfig::new(0.0, 0.0, 0.625, 0.5, 5, 3).unwrap(),
        RasterDimensions::new(4000, 3001).unwrap(),
        CropPlanMode::Export,
    )
    .unwrap();

    // The raw 2500x1500 ROI is landscape. Native computes a taller diagnostic
    // aspect correction, but assigns and applies both remainders to the raw ROI.
    assert_eq!(plan.input_roi(), CropRoi::new(0, 0, 2499, 1500).unwrap());
}

#[test]
fn export_negative_ratio_uses_raw_orientation_and_absolute_reduced_aligners() {
    let raw_portrait = CropPlan::new_with_mode(
        CropConfig::new(0.0, 0.0, 0.6, 0.99, 2, -3).unwrap(),
        RasterDimensions::new(100, 80).unwrap(),
        CropPlanMode::Export,
    )
    .unwrap();
    assert_eq!(
        raw_portrait.input_roi(),
        CropRoi::new(0, 0, 60, 78).unwrap()
    );

    let raw_landscape = CropPlan::new_with_mode(
        CropConfig::new(0.0, 0.0, 0.99, 0.6, 2, -3).unwrap(),
        RasterDimensions::new(80, 100).unwrap(),
        CropPlanMode::Export,
    )
    .unwrap();
    assert_eq!(
        raw_landscape.input_roi(),
        CropRoi::new(0, 0, 78, 60).unwrap()
    );
}

#[test]
fn export_reduces_common_aligners_before_centering_remainders() {
    let plan = CropPlan::new_with_mode(
        CropConfig::new(0.0, 0.0, 0.92, 0.99, 8, 12).unwrap(),
        RasterDimensions::new(100, 80).unwrap(),
        CropPlanMode::Export,
    )
    .unwrap();
    assert_eq!(plan.input_roi(), CropRoi::new(1, 0, 90, 78).unwrap());
}

#[test]
fn export_keeps_the_native_minimum_output_edge_at_small_crop_boundaries() {
    let plan = CropPlan::new_with_mode(
        CropConfig::new(0.48, 0.48, 0.5, 0.5, 2, 3).unwrap(),
        RasterDimensions::new(100, 100).unwrap(),
        CropPlanMode::Export,
    )
    .unwrap();
    assert_eq!(plan.input_roi(), CropRoi::new(48, 48, 4, 4).unwrap());
    assert_eq!(
        plan.output_dimensions(),
        RasterDimensions::new(4, 4).unwrap()
    );

    let freehand = CropPlan::new_with_mode(
        CropConfig::new(0.48, 0.48, 0.5, 0.5, 0, 0).unwrap(),
        RasterDimensions::new(100, 100).unwrap(),
        CropPlanMode::Export,
    )
    .unwrap();
    assert_eq!(
        freehand.output_dimensions(),
        RasterDimensions::new(4, 4).unwrap()
    );
}
