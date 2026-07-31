//! Source-derived tests for the isolated legacy `filmic` leaf.
//!
//! The leaf is intentionally path-included here because shared registration is
//! deferred.  This keeps the tests operation-local without editing the shared
//! `operations` hub.

#![allow(
    clippy::cast_precision_loss,
    clippy::excessive_precision,
    clippy::float_cmp,
    clippy::unreadable_literal,
    reason = "source-derived decimal fixtures and bitwise compatibility assertions"
)]

#[path = "../src/operations/filmic/mod.rs"]
mod filmic;

use filmic::{
    FILMIC_EPS, FILMIC_LUT_SIZE, FILMIC_V1_PARAMETER_BYTES, FILMIC_V2_PARAMETER_BYTES,
    FILMIC_V3_PARAMETER_BYTES, FilmicConfig, FilmicDescriptor, FilmicHistory, FilmicNodeLoss,
    FilmicParametersV1, FilmicParametersV2, FilmicParametersV3, FilmicPixel, FilmicPlan,
    FilmicPlanError, derive_filmic_nodes, fastlog2, filmic_descriptor, lab_d50_to_xyz, lut_index,
    migrate_filmic_v1_to_v3, migrate_filmic_v2_to_v3, prophoto_rgb_to_lab, vector_exp2,
    xyz_d50_to_prophoto_rgb,
};

fn v1() -> FilmicParametersV1 {
    FilmicParametersV1::new(
        18.0, -8.65, 2.45, 0.0, 18.0, 0.0, 100.0, 2.2, 2.0, 1.5, 60.0, -12.0, 1,
    )
}

fn v2() -> FilmicParametersV2 {
    FilmicParametersV2::new(
        18.0, -8.65, 2.45, 0.0, 18.0, 0.0, 100.0, 2.2, 2.0, 1.5, 60.0, -12.0, 2, 1,
    )
}

#[test]
fn native_payload_sizes_and_little_endian_field_order_are_exact() {
    assert_eq!(FILMIC_V1_PARAMETER_BYTES, 52);
    assert_eq!(FILMIC_V2_PARAMETER_BYTES, 56);
    assert_eq!(FILMIC_V3_PARAMETER_BYTES, 60);

    let old = v1();
    let bytes = old.to_bytes();
    assert_eq!(&bytes[0..4], &18.0_f32.to_le_bytes());
    assert_eq!(&bytes[44..48], &(-12.0_f32).to_le_bytes());
    assert_eq!(&bytes[48..52], &1_i32.to_le_bytes());
    assert_eq!(FilmicParametersV1::from_bytes(&bytes), Ok(old));

    let old = v2();
    let bytes = old.to_bytes();
    assert_eq!(&bytes[48..52], &2_i32.to_le_bytes());
    assert_eq!(&bytes[52..56], &1_i32.to_le_bytes());
    assert_eq!(FilmicParametersV2::from_bytes(&bytes), Ok(old));

    let current = migrate_filmic_v2_to_v3(old);
    let bytes = current.to_bytes();
    assert_eq!(&bytes[44..48], &100.0_f32.to_le_bytes());
    assert_eq!(&bytes[52..56], &2_i32.to_le_bytes());
    assert_eq!(&bytes[56..60], &1_i32.to_le_bytes());
    assert_eq!(FilmicParametersV3::from_bytes(&bytes), Ok(current));
}

#[test]
fn native_migrations_write_v3_directly() {
    let migrated_v1 = migrate_filmic_v1_to_v3(v1());
    assert_eq!(migrated_v1.global_saturation, 100.0);
    assert_eq!(migrated_v1.preserve_color, 0);
    assert_eq!(migrated_v1.interpolator, 1);

    let migrated_v2 = migrate_filmic_v2_to_v3(v2());
    assert_eq!(migrated_v2.global_saturation, 100.0);
    assert_eq!(migrated_v2.preserve_color, 1);
    assert_eq!(migrated_v2.interpolator, 2);
}

#[test]
fn malformed_nonfinite_and_unknown_history_fail_closed_without_losing_opaque_bytes() {
    assert!(FilmicParametersV3::from_bytes(&[0; 59]).is_err());
    assert!(FilmicParametersV3::from_bytes(&[0; 61]).is_err());

    let mut bytes = FilmicParametersV3::defaults().to_bytes();
    bytes[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
    let decoded = FilmicParametersV3::from_bytes(&bytes).expect("length is valid");
    assert!(FilmicConfig::new(decoded).is_err());

    let opaque_bytes = vec![0x00, 0x80, 0x7f, 0xff, 0x31];
    let history = FilmicHistory::decode(99, &opaque_bytes).expect("unknown versions are retained");
    assert_eq!(history.version(), 99);
    assert_eq!(history.payload(), opaque_bytes);
    assert!(history.current().is_err());
}

#[test]
fn defaults_are_native_and_descriptor_is_unavailable_except_for_cpu_leaf() {
    let defaults = FilmicParametersV3::defaults();
    assert_eq!(defaults.grey_point_source, 18.0);
    assert_eq!(defaults.black_point_source, -8.65);
    assert_eq!(defaults.white_point_source, 2.45);
    assert_eq!(defaults.output_power, 2.2);
    assert_eq!(defaults.interpolator, 0);
    assert_eq!(defaults.preserve_color, 0);

    let descriptor: FilmicDescriptor = filmic_descriptor();
    assert!(!descriptor.default_enabled);
    assert!(descriptor.cpu_supported);
    assert_eq!(descriptor.input_stage, "lab-d50");
    assert_eq!(descriptor.output_stage, "lab-d50");
    assert!(descriptor.identity_roi);
    assert_eq!(descriptor.overlap_pixels, 0);
    assert_eq!(descriptor.gpu_tier, None);
    assert!(!descriptor.deterministic_gpu);
    assert!(!descriptor.fallback_to_cpu);
    assert!(!descriptor.consumes_operation_mask);
    assert!(!descriptor.publishes_operation_mask);
    assert_eq!(descriptor.ui, None);
}

#[test]
fn lut_has_source_quantized_endpoints_and_all_interpolators_are_available() {
    for interpolator in 0..=3 {
        let parameters = FilmicParametersV3::new(
            18.0,
            -8.65,
            2.45,
            0.0,
            18.0,
            0.0,
            100.0,
            2.2,
            2.0,
            1.5,
            60.0,
            100.0,
            0.0,
            interpolator,
            0,
        );
        let plan = FilmicPlan::from_parameters(parameters).expect("native defaults derive a LUT");
        assert_eq!(plan.table().len(), FILMIC_LUT_SIZE);
        assert_eq!(plan.table()[0].to_bits(), 0.0_f32.to_bits());
        assert_eq!(
            plan.table()[FILMIC_LUT_SIZE - 1].to_bits(),
            (65535.0_f32 / 65536.0).to_bits()
        );
    }

    let invalid = FilmicParametersV3::new(
        18.0, -8.65, 2.45, 0.0, 18.0, 0.0, 100.0, 2.2, 2.0, 1.5, 60.0, 100.0, 0.0, 99, 0,
    );
    let fallback =
        FilmicPlan::from_parameters(invalid).expect("invalid source interpolator falls back");
    let cubic =
        FilmicPlan::from_parameters(FilmicParametersV3::defaults()).expect("defaults derive");
    assert_eq!(fallback.table(), cubic.table());
}

#[test]
fn lut_index_and_concavity_boundaries_follow_native_quantization() {
    let boundaries = [
        (0.0_f32.to_bits(), 0),
        ((1.0_f32 / 65536.0_f32).to_bits(), 1),
        ((65535.0_f32 / 65536.0_f32).to_bits(), FILMIC_LUT_SIZE - 1),
        (1.0_f32.to_bits(), FILMIC_LUT_SIZE - 1),
    ];
    for (value_bits, expected_index) in boundaries {
        assert_eq!(lut_index(f32::from_bits(value_bits)), expected_index);
    }
    assert_eq!(lut_index(1.0 - 2.0 / 65536.0), FILMIC_LUT_SIZE - 2);

    let zero_sigma = FilmicParametersV3::new(
        18.0, -8.65, 2.45, 0.0, 18.0, 0.0, 100.0, 2.2, 2.0, 0.0, 0.0, 100.0, 0.0, 0, 0,
    );
    let zero_grad = FilmicPlan::from_parameters(zero_sigma).expect("zero saturation plan");
    assert!(zero_grad.grad_2().iter().all(|value| value.to_bits() == 0));

    let nonzero_grad =
        FilmicPlan::from_parameters(FilmicParametersV3::defaults()).expect("default plan");
    assert!(nonzero_grad.grad_2().iter().any(|value| *value > 0.0));
}

#[test]
fn output_power_changes_the_vector_approximation_path() {
    let power_one = FilmicParametersV3::new(
        18.0, -8.65, 2.45, 0.0, 18.0, 0.0, 100.0, 1.0, 2.0, 1.5, 100.0, 100.0, 0.0, 0, 0,
    );
    let power_two = FilmicParametersV3::new(
        18.0, -8.65, 2.45, 0.0, 18.0, 0.0, 100.0, 2.2, 2.0, 1.5, 100.0, 100.0, 0.0, 0, 0,
    );
    let pixel = FilmicPixel::new(42.0, 12.0, -8.0, 0.75);
    let one = FilmicPlan::from_parameters(power_one)
        .expect("power one plan")
        .execute(&[pixel])[0];
    let two = FilmicPlan::from_parameters(power_two)
        .expect("power two plan")
        .execute(&[pixel])[0];
    assert_ne!(one.channels(), two.channels());
}

#[test]
fn commit_contrast_floor_keeps_the_source_target_and_lut_contrast_quirk() {
    let mut parameters = FilmicParametersV3::defaults();
    parameters.grey_point_target = 180.0;
    parameters.black_point_target = -20.0;
    parameters.white_point_target = 140.0;
    parameters.contrast = 0.1;
    let plan = FilmicPlan::from_parameters(parameters).expect("finite source-derived plan");
    let grey_log = parameters.black_point_source.abs()
        / (parameters.white_point_source - parameters.black_point_source);
    let grey_display =
        (parameters.grey_point_target / 100.0_f32).powf(1.0_f32 / parameters.output_power);
    let expected_floor = 1.0001_f32 * grey_display / grey_log;
    assert_eq!(
        plan.effective_contrast().to_bits(),
        expected_floor.to_bits()
    );
    assert_eq!(plan.parameters().contrast.to_bits(), 0.1_f32.to_bits());
}

#[test]
fn derived_state_rejects_dynamic_range_and_duplicate_knot_inputs() {
    let zero_range = FilmicParametersV3::new(
        18.0, 2.0, 2.0, 0.0, 18.0, 0.0, 100.0, 2.2, 2.0, 1.5, 100.0, 100.0, 0.0, 0, 0,
    );
    assert!(matches!(
        FilmicPlan::from_parameters(zero_range),
        Err(FilmicPlanError::Curve(_) | FilmicPlanError::InvalidDerivedState(_))
    ));

    let duplicate = FilmicParametersV3::new(
        18.0, 0.0, 2.45, 0.0, 18.0, 0.0, 100.0, 2.2, 2.0, 1.5, 100.0, 100.0, 0.0, 0, 0,
    );
    assert!(FilmicPlan::from_parameters(duplicate).is_err());
}

#[test]
fn preserve_color_and_global_desaturation_are_distinct_paths_and_zero_spare_lane() {
    let pixel = FilmicPixel::new(55.0, 36.0, -22.0, 0.37);
    let per_channel =
        FilmicPlan::from_parameters(FilmicParametersV3::defaults()).expect("valid plan");
    let preserve = FilmicPlan::from_parameters(FilmicParametersV3::new(
        18.0, -8.65, 2.45, 0.0, 18.0, 0.0, 100.0, 2.2, 2.0, 1.5, 60.0, 100.0, 0.0, 0, 1,
    ))
    .expect("valid preserve-color plan");
    let desaturated = FilmicPlan::from_parameters(FilmicParametersV3::new(
        18.0, -8.65, 2.45, 0.0, 18.0, 0.0, 100.0, 2.2, 2.0, 1.5, 60.0, 70.0, 0.0, 0, 0,
    ))
    .expect("valid global saturation plan");

    let per_channel_out = per_channel.execute(&[pixel])[0];
    let preserve_out = preserve.execute(&[pixel])[0];
    let desaturated_out = desaturated.execute(&[pixel])[0];
    let zero = 0.0_f32.to_bits();
    assert_eq!(per_channel_out.alpha().to_bits(), zero);
    assert_eq!(preserve_out.alpha().to_bits(), zero);
    assert_eq!(desaturated_out.alpha().to_bits(), zero);
    assert_ne!(per_channel_out.channels(), preserve_out.channels());
    assert_ne!(per_channel_out.channels(), desaturated_out.channels());
}

#[test]
fn exact_lab_d50_prophoto_boundaries_include_the_padded_matrix_lane() {
    assert_eq!(
        lab_d50_to_xyz([0.0, 0.0, 0.0, 0.37]).map(f32::to_bits),
        [0x0000_0000, 0x0000_0000, 0x0000_0000, 0x0000_0000]
    );
    assert_eq!(
        lab_d50_to_xyz([100.0, 0.0, 0.0, 0.37]).map(f32::to_bits),
        [0x3f76_d5d0, 0x3f80_0000, 0x3f53_2ca5, 0x0000_0000]
    );

    assert_eq!(
        xyz_d50_to_prophoto_rgb([0.9642, 1.0, 0.8249, 0.37]).map(f32::to_bits),
        [0x3f7f_ff47, 0x3f80_0025, 0x3f7f_e762, 0x0000_0000]
    );
    assert_eq!(
        prophoto_rgb_to_lab([1.0, 1.0, 1.0, 0.37]).map(f32::to_bits),
        [0x42c8_0000, 0x3b62_9000, 0xbccd_4600, 0x0000_0000]
    );
}

#[test]
fn arm_vector_exp2_tie_vectors_are_source_rounding_boundaries() {
    let fixtures = [
        (0.0_f32, 0x3f7f_ffd5),
        (1.0_f32, 0x4000_0016),
        (-2.0_f32, 0x3e7f_ffd5),
        (3.0_f32, 0x4100_0016),
        (-4.0_f32, 0x3d7f_ffd5),
    ];
    for (input, expected_bits) in fixtures {
        assert_eq!(vector_exp2(input).to_bits(), expected_bits);
    }
}

#[test]
fn epsilon_and_hdr_inputs_are_finite_and_native_cpu_zeroes_the_spare_lane() {
    let plan = FilmicPlan::from_parameters(FilmicParametersV3::defaults()).expect("valid plan");
    let inputs = [
        FilmicPixel::new(0.0, 0.0, 0.0, 0.1),
        FilmicPixel::new(-500.0, 400.0, -400.0, 0.2),
        FilmicPixel::new(1000.0, 300.0, -200.0, 0.3),
        FilmicPixel::new(18.0, 0.0, FILMIC_EPS, 0.4),
    ];
    let outputs = plan.execute(&inputs);
    for output in outputs {
        assert_eq!(output.alpha().to_bits(), 0.0_f32.to_bits());
        assert!(output.channels()[..3].iter().all(|value| value.is_finite()));
    }
    assert!(fastlog2(1.0).is_finite());
}

#[test]
fn full_raster_and_zero_overlap_tiles_are_bitwise_equal() {
    let plan = FilmicPlan::from_parameters(FilmicParametersV3::defaults()).expect("valid plan");
    let input: Vec<_> = (0..12)
        .map(|index| FilmicPixel::new(18.0 + index as f32, index as f32, -index as f32, 0.5))
        .collect();
    let full = plan.execute(&input);
    let mut tiled = Vec::new();
    for tile in input.chunks(3) {
        tiled.extend(plan.execute_tile(tile));
    }
    assert_eq!(full, tiled);
}

#[test]
fn config_retains_finite_out_of_editor_range_values_until_derived_validation() {
    let parameters = FilmicParametersV3::new(
        180.0, -8.65, 2.45, 500.0, 180.0, -20.0, 140.0, 2.2, 2.0, 4.5, 600.0, -25.0, 70.0, 3, 0,
    );
    let config = FilmicConfig::new(parameters).expect("finite persisted values are not clamped");
    assert_eq!(config.parameters(), parameters);
}

#[test]
fn each_native_toe_and_shoulder_node_loss_branch_is_retained() {
    let none = derive_filmic_nodes(FilmicParametersV3::defaults())
        .expect("default curve")
        .loss();
    assert_eq!(none, FilmicNodeLoss::None);

    let toe = FilmicParametersV3::new(
        18.0,
        -12.4232756,
        4.7975142,
        0.0,
        5.8430858,
        47.089626,
        109.90266,
        4.0,
        2.7128502,
        0.38485232,
        100.0,
        100.0,
        -39.7028,
        0,
        0,
    );
    assert_eq!(
        derive_filmic_nodes(toe).expect("toe-loss curve").loss(),
        FilmicNodeLoss::Toe
    );

    let shoulder = FilmicParametersV3::new(
        18.0, -13.075881, 5.1326666, 0.0, 26.555546, 91.3948, 134.98764, 0.5, 4.469752, 2.6477094,
        100.0, 100.0, 43.16282, 0, 0,
    );
    assert_eq!(
        derive_filmic_nodes(shoulder)
            .expect("shoulder-loss curve")
            .loss(),
        FilmicNodeLoss::Shoulder
    );

    let both = FilmicParametersV3::new(
        18.0, -7.4693227, 15.287271, 0.0, 17.508604, 47.623806, 53.95938, 0.5, 1.5430943, 0.526182,
        100.0, 100.0, 38.773476, 0, 0,
    );
    assert_eq!(
        derive_filmic_nodes(both).expect("both-loss curve").loss(),
        FilmicNodeLoss::Both
    );
}
