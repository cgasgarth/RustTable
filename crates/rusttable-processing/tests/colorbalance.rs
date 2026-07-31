#![allow(
    clippy::float_cmp,
    reason = "source-derived ABI and branch tests compare exact f32 bits"
)]
#![allow(
    clippy::excessive_precision,
    reason = "independent native-derived f32 golden vectors retain their decimal evidence"
)]
#![allow(
    clippy::unreadable_literal,
    reason = "native golden vectors retain their source decimal notation"
)]

// The integration seam is intentionally not registered in the shared hubs yet.
// Re-export the processing crate's descriptor types so this operation-local
// module can be compiled and tested without editing those exclusive hubs.
pub mod descriptor {
    pub use rusttable_processing::descriptor::*;
}

#[path = "../src/operations/colorbalance/mod.rs"]
mod colorbalance;

use colorbalance::codec::{
    CHANNEL_BLUE, CHANNEL_FACTOR, CHANNEL_GREEN, CHANNEL_RED, CHANNEL_SIZE,
    COLORBALANCE_INTROSPECTION_VERSION, COLORBALANCE_V1_PARAMETER_BYTES,
    COLORBALANCE_V2_PARAMETER_BYTES, COLORBALANCE_V3_PARAMETER_BYTES, ColorBalanceCodecError,
    ColorBalanceHistory, ColorBalanceMode, ColorBalanceParametersV1, ColorBalanceParametersV2,
    ColorBalanceParametersV3,
};
use colorbalance::execution::{
    ColorBalanceConfig, ColorBalanceExecutionError, ColorBalanceParameterError, ColorBalancePixel,
    ColorBalancePlan, blend_lab_normal_pixel_for_test,
};
use std::sync::atomic::{AtomicUsize, Ordering};

use colorbalance::math;
use colorbalance::{COLORBALANCE_COMPATIBILITY_ID, COLORBALANCE_RUST_ID, colorbalance_descriptor};
use rusttable_processing::descriptor::{OperationFlags, ParameterDefault, ParameterKind, RoiKind};

fn fixture(name: &str) -> Vec<u8> {
    let source = match name {
        "v1" => include_str!("fixtures/colorbalance-v1.hex"),
        "v2" => include_str!("fixtures/colorbalance-v2.hex"),
        "v3" => include_str!("fixtures/colorbalance-v3.hex"),
        _ => panic!("unknown fixture"),
    };
    source
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("fixture is hexadecimal"))
        .collect()
}

fn sample_parameters(mode: ColorBalanceMode) -> ColorBalanceParametersV3 {
    ColorBalanceParametersV3::new(
        mode,
        [0.9, 1.1, 0.8, 1.2],
        [1.2, 0.7, 1.4, 0.6],
        [1.1, 0.9, 1.3, 0.8],
        1.15,
        0.85,
        18.0,
        0.75,
    )
}

fn plan(parameters: ColorBalanceParametersV3) -> ColorBalancePlan {
    ColorBalancePlan::new(ColorBalanceConfig::new(parameters).expect("sample is finite"))
}

fn lab_from_rgb(rgb: [f32; 4]) -> ColorBalancePixel {
    ColorBalancePixel::from_channels(math::prophoto_to_lab(rgb))
}

fn assert_same_bits(left: [f32; 4], right: [f32; 4]) {
    assert_eq!(
        left.map(f32::to_bits),
        right.map(f32::to_bits),
        "f32 lanes differ: {left:?} != {right:?}"
    );
}

fn assert_native_golden(actual: [f32; 4], expected: [f32; 4], tolerance: f32) {
    for (index, (actual, expected)) in actual.into_iter().zip(expected).enumerate() {
        assert!(
            (actual - expected).abs() <= tolerance,
            "native golden lane {index} differs: {actual:?} != {expected:?} (tol {tolerance})"
        );
    }
}

#[test]
fn abi_sizes_offsets_little_endian_and_negative_zero_are_native_ordered() {
    assert_eq!(COLORBALANCE_V1_PARAMETER_BYTES, 48);
    assert_eq!(COLORBALANCE_V2_PARAMETER_BYTES, 64);
    assert_eq!(COLORBALANCE_V3_PARAMETER_BYTES, 68);

    let v3 = ColorBalanceParametersV3::new(
        ColorBalanceMode::SlopeOffsetPower,
        [1.0, -0.0, 0.25, 2.0],
        [1.5, 0.75, -2.0, -1.0],
        [0.5, 1.25, 0.0, -3.0],
        0.75,
        1.25,
        18.5,
        0.8,
    );
    let bytes = v3.to_bytes();
    assert_eq!(&bytes[..4], &1_i32.to_le_bytes());
    assert_eq!(&bytes[4..8], &1.0_f32.to_le_bytes());
    assert_eq!(&bytes[8..12], &(-0.0_f32).to_le_bytes());
    assert_eq!(&bytes[20..24], &1.5_f32.to_le_bytes());
    assert_eq!(&bytes[36..40], &0.5_f32.to_le_bytes());
    assert_eq!(&bytes[52..56], &0.75_f32.to_le_bytes());
    assert_eq!(&bytes[56..60], &1.25_f32.to_le_bytes());
    assert_eq!(&bytes[60..64], &18.5_f32.to_le_bytes());
    assert_eq!(&bytes[64..68], &0.8_f32.to_le_bytes());
    assert_eq!(
        f32::from_le_bytes(bytes[8..12].try_into().unwrap()).to_bits(),
        0x8000_0000
    );

    let v1 = ColorBalanceParametersV1::new(v3.lift, v3.gamma, v3.gain);
    assert_eq!(v1.to_bytes().to_vec(), fixture("v1"));
    let v2 = ColorBalanceParametersV2::new(
        v3.mode,
        v3.lift,
        v3.gamma,
        v3.gain,
        v3.saturation,
        v3.contrast,
        v3.grey,
    );
    assert_eq!(v2.to_bytes().to_vec(), fixture("v2"));
    assert_eq!(bytes.to_vec(), fixture("v3"));
    assert_eq!(ColorBalanceParametersV3::from_bytes(&bytes).unwrap(), v3);
}

#[test]
fn fixtures_decode_and_round_trip_all_native_versions() {
    let v1 = ColorBalanceHistory::decode(1, &fixture("v1")).unwrap();
    let v2 = ColorBalanceHistory::decode(2, &fixture("v2")).unwrap();
    let v3 = ColorBalanceHistory::decode(3, &fixture("v3")).unwrap();
    assert_eq!(v1.version(), 1);
    assert_eq!(v2.version(), 2);
    assert_eq!(v3.version(), 3);
    assert_eq!(v1.payload(), fixture("v1"));
    assert_eq!(v2.payload(), fixture("v2"));
    assert_eq!(v3.payload(), fixture("v3"));
    assert_eq!(
        v3.current().unwrap().mode,
        ColorBalanceMode::SlopeOffsetPower
    );
    assert_eq!(COLORBALANCE_INTROSPECTION_VERSION, 3);
}

#[test]
fn migrations_copy_arrays_and_apply_exact_native_defaults() {
    let v1 = ColorBalanceParametersV1::new(
        [1.0, -0.0, 0.25, 2.0],
        [1.5, 0.75, -2.0, -1.0],
        [0.5, 1.25, 0.0, -3.0],
    );
    let migrated_v1 = ColorBalanceHistory::V1(v1).current().unwrap();
    assert_eq!(migrated_v1.mode, ColorBalanceMode::Legacy);
    assert_eq!(migrated_v1.lift, v1.lift);
    assert_eq!(migrated_v1.gamma, v1.gamma);
    assert_eq!(migrated_v1.gain, v1.gain);
    assert_eq!(
        [
            migrated_v1.saturation,
            migrated_v1.contrast,
            migrated_v1.grey,
            migrated_v1.saturation_out
        ],
        [1.0, 1.0, 18.0, 1.0]
    );

    let v2 = ColorBalanceParametersV2::new(
        ColorBalanceMode::LiftGammaGain,
        v1.lift,
        v1.gamma,
        v1.gain,
        0.5,
        1.5,
        20.0,
    );
    let migrated_v2 = ColorBalanceHistory::V2(v2).current().unwrap();
    assert_eq!(migrated_v2.mode, v2.mode);
    assert_eq!(migrated_v2.lift, v2.lift);
    assert_eq!(migrated_v2.gamma, v2.gamma);
    assert_eq!(migrated_v2.gain, v2.gain);
    assert_eq!(migrated_v2.saturation, v2.saturation);
    assert_eq!(migrated_v2.contrast, v2.contrast);
    assert_eq!(migrated_v2.grey, v2.grey);
    assert_eq!(migrated_v2.saturation_out, 1.0);
}

#[test]
fn malformed_unknown_and_nonfinite_history_fail_closed_without_clamping_finite_outliers() {
    assert!(matches!(
        ColorBalanceParametersV1::from_bytes(&[0; 47]),
        Err(ColorBalanceCodecError::InvalidLength {
            expected: 48,
            actual: 47
        })
    ));
    assert!(matches!(
        ColorBalanceParametersV3::from_bytes(&[0; 67]),
        Err(ColorBalanceCodecError::InvalidLength {
            expected: 68,
            actual: 67
        })
    ));

    let mut unknown_mode = ColorBalanceParametersV3::defaults().to_bytes();
    unknown_mode[..4].copy_from_slice(&(-7_i32).to_le_bytes());
    assert!(matches!(
        ColorBalanceParametersV3::from_bytes(&unknown_mode),
        Err(ColorBalanceCodecError::UnknownMode(-7))
    ));

    let opaque_bytes = vec![0xde, 0xad, 0xbe, 0xef];
    let opaque = ColorBalanceHistory::decode(99, &opaque_bytes).unwrap();
    assert_eq!(opaque.payload(), opaque_bytes);
    assert!(matches!(
        opaque.current(),
        Err(ColorBalanceCodecError::UnsupportedVersion(99))
    ));

    let mut finite_outlier = ColorBalanceParametersV3::defaults();
    finite_outlier.lift[CHANNEL_BLUE] = 99.0;
    finite_outlier.grey = -1000.0;
    let config = ColorBalanceConfig::new(finite_outlier).unwrap();
    assert_eq!(config.lift()[CHANNEL_BLUE], 99.0);
    assert_eq!(config.grey(), -1000.0);

    finite_outlier.saturation = f32::NAN;
    assert!(matches!(
        ColorBalanceConfig::new(finite_outlier),
        Err(ColorBalanceParameterError::NonFinite {
            field: "saturation",
            index: None
        })
    ));
    finite_outlier.saturation = f32::INFINITY;
    assert!(ColorBalanceConfig::new(finite_outlier).is_err());
}

#[test]
fn descriptor_preserves_native_order_ranges_defaults_and_fail_closed_capabilities() {
    let descriptor = colorbalance_descriptor();
    descriptor.validate().unwrap();
    assert_eq!(
        descriptor.id.compatibility_name,
        COLORBALANCE_COMPATIBILITY_ID
    );
    assert_eq!(descriptor.id.rust_id, COLORBALANCE_RUST_ID);
    assert_eq!(descriptor.stage, "display-referred-lab-d50");
    assert_eq!(descriptor.roi, RoiKind::Identity);
    assert_eq!(
        descriptor
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect::<Vec<_>>(),
        [
            "mode",
            "lift",
            "gamma",
            "gain",
            "saturation",
            "contrast",
            "grey",
            "saturation_out"
        ]
    );
    assert!(matches!(
        descriptor.parameters[0].kind,
        ParameterKind::Enum { .. }
    ));
    assert_eq!(
        descriptor.parameters[0].default,
        ParameterDefault::Enum("slope-offset-power".to_owned())
    );
    for id in ["lift", "gamma", "gain"] {
        let parameter = descriptor
            .parameters
            .iter()
            .find(|parameter| parameter.id == id)
            .unwrap();
        assert_eq!(
            parameter.kind,
            ParameterKind::Vector {
                dimensions: 4,
                minimum: 0.0,
                maximum: 2.0
            }
        );
        assert_eq!(parameter.default, ParameterDefault::Vector(vec![1.0; 4]));
    }
    assert_eq!(
        descriptor.parameters[5].kind,
        ParameterKind::Scalar {
            minimum: 0.01,
            maximum: 1.99
        }
    );
    assert_eq!(
        descriptor.parameters[6].default,
        ParameterDefault::Scalar(18.0)
    );
    assert!(descriptor.capability.cpu_supported);
    assert!(descriptor.flags.contains(OperationFlags::FULL_IMAGE));
    assert!(!descriptor.flags.contains(OperationFlags::TILEABLE));
    assert!(descriptor.capability.gpu_tier.is_none());
    assert!(!descriptor.capability.deterministic_gpu);
    assert!(!descriptor.flags.contains(OperationFlags::DETERMINISTIC_GPU));
    assert!(descriptor.ui.is_none());
    assert!(descriptor.mask_blend.consumes_mask);
    assert!(!descriptor.mask_blend.blend_if);
}

#[test]
fn source_map_keeps_unported_integration_fail_closed() {
    let source_map =
        include_str!("../../../architecture/rusttable-colorbalance-compat-source-map.toml");
    assert!(source_map.contains(
        "production_registration = \"deferred until shared operation, history, pixelpipe, and UI seams are complete\""
    ));
    assert!(source_map.contains("upstream_path = \"src/develop/blends/blendif_lab.c\""));
    assert!(!source_map.contains("blendif_rgb_jzczhz.c"));
    for responsibility in [
        "id = \"shared-operation-export\"",
        "id = \"history-dispatch\"",
        "id = \"lab-alpha-pixelpipe-boundary\"",
        "id = \"gpu-capability\"",
        "id = \"ui-presets-optimizers\"",
    ] {
        let start = source_map
            .find(responsibility)
            .expect("source map names every deferred integration seam");
        let entry = &source_map[start..];
        assert!(
            entry[..entry.find("[[responsibility]]").unwrap_or(entry.len())]
                .contains("status = \"deferred\""),
            "{responsibility} must remain deferred"
        );
    }
}

#[test]
fn commit_corrects_lift_gamma_gain_luminance_for_both_prophoto_modes() {
    for mode in [
        ColorBalanceMode::LiftGammaGain,
        ColorBalanceMode::SlopeOffsetPower,
    ] {
        let parameters = sample_parameters(mode);
        let config = ColorBalanceConfig::new(parameters).unwrap();
        let committed = ColorBalancePlan::new(config).coefficients().committed;
        for (persisted, actual) in [
            (parameters.lift, committed.lift()),
            (parameters.gamma, committed.gamma()),
            (parameters.gain, committed.gain()),
        ] {
            let xyz = math::prophoto_to_xyz([persisted[1], persisted[2], persisted[3], 0.0]);
            let expected = [
                persisted[0],
                (persisted[1] - xyz[1]) + 1.0,
                (persisted[2] - xyz[1]) + 1.0,
                (persisted[3] - xyz[1]) + 1.0,
            ];
            assert_eq!(actual, expected);
        }
    }
}

#[test]
fn legacy_commit_copies_all_curve_values_without_luminance_correction() {
    let parameters = sample_parameters(ColorBalanceMode::Legacy);
    let committed = ColorBalancePlan::new(ColorBalanceConfig::new(parameters).unwrap())
        .coefficients()
        .committed;
    assert_eq!(committed.lift(), parameters.lift);
    assert_eq!(committed.gamma(), parameters.gamma);
    assert_eq!(committed.gain(), parameters.gain);
}

#[test]
fn derived_coefficients_preserve_native_guards_and_arithmetic_order() {
    let mut parameters = ColorBalanceParametersV3::defaults();
    parameters.mode = ColorBalanceMode::SlopeOffsetPower;
    parameters.contrast = 0.0;
    parameters.gamma = [0.0; CHANNEL_SIZE];
    let coefficients =
        ColorBalancePlan::new(ColorBalanceConfig::new(parameters).unwrap()).coefficients();
    assert_eq!(coefficients.contrast_power, 1_000_000.0);
    assert_eq!(coefficients.grey, 0.18);
    assert_eq!(coefficients.lgg_gamma[0], 0.0);
    assert_eq!(coefficients.legacy_gamma_inv[0], 1_000_000.0);
    assert_eq!(coefficients.lgg_gamma_inv[0], 2_200_000.0);

    let parameters = sample_parameters(ColorBalanceMode::SlopeOffsetPower);
    let coefficients =
        ColorBalancePlan::new(ColorBalanceConfig::new(parameters).unwrap()).coefficients();
    let committed = coefficients.committed;
    assert_eq!(
        coefficients.sop_lift[0],
        committed.lift()[0] + committed.lift()[CHANNEL_FACTOR] - 2.0
    );
    assert_eq!(
        coefficients.sop_gamma[0],
        (2.0 - committed.gamma()[0]) * (2.0 - committed.gamma()[CHANNEL_FACTOR])
    );
    assert_eq!(coefficients.sop_gain[0], coefficients.lgg_gain[0]);
}

#[test]
fn all_modes_execute_scalar_and_cover_clamp_saturation_contrast_and_legacy_ignores_master_controls()
{
    let input = [
        lab_from_rgb([-0.2, 0.35, 1.8, 99.0]),
        lab_from_rgb([0.2, 0.4, 0.6, -3.0]),
    ];
    for mode in [
        ColorBalanceMode::Legacy,
        ColorBalanceMode::LiftGammaGain,
        ColorBalanceMode::SlopeOffsetPower,
    ] {
        let output = plan(sample_parameters(mode)).execute_lab(&input);
        assert_eq!(output.len(), input.len());
        assert!(output.iter().all(|pixel| pixel.channels()[0].is_finite()));
        assert_eq!(output[0].alpha_or_spare().to_bits(), 0);
    }

    let legacy_a = sample_parameters(ColorBalanceMode::Legacy);
    let mut legacy_b = legacy_a;
    legacy_b.saturation = 0.01;
    legacy_b.contrast = 1.99;
    legacy_b.grey = 0.1;
    legacy_b.saturation_out = 1.99;
    let output_a = plan(legacy_a).execute_lab(&input);
    let output_b = plan(legacy_b).execute_lab(&input);
    for (left, right) in output_a.iter().zip(output_b) {
        assert_same_bits(left.channels(), right.channels());
    }

    let mut saturated = sample_parameters(ColorBalanceMode::SlopeOffsetPower);
    saturated.saturation = 1.0 + 2e-6;
    saturated.saturation_out = 1.5;
    saturated.contrast = 0.5;
    saturated.grey = 10.0;
    let neutral = sample_parameters(ColorBalanceMode::SlopeOffsetPower);
    let neutral_output = plan(neutral).execute_lab(&input);
    let saturated_output = plan(saturated).execute_lab(&input);
    assert!(
        neutral_output
            .iter()
            .zip(saturated_output.iter())
            .any(|(left, right)| left.channels()[1].to_bits() != right.channels()[1].to_bits())
    );

    let mut neutral = neutral;
    neutral.saturation = 1.0;
    neutral.saturation_out = 1.0;
    neutral.contrast = 1.0;
    let neutral_output = plan(neutral).execute_lab(&input);
    let mut threshold = neutral;
    threshold.saturation = 1.0 + 0.5e-6;
    let threshold_output = plan(threshold).execute_lab(&input);
    for (left, right) in neutral_output.iter().zip(threshold_output) {
        assert_same_bits(left.channels(), right.channels());
    }
}

// These expected values are independent f32 evaluations of the retained native
// equations and constants, not values obtained through the Rust conversion helpers.
#[test]
fn native_golden_f32_vectors_cover_modes_guards_thresholds_and_round_trips() {
    let legacy = ColorBalanceParametersV3::new(
        ColorBalanceMode::Legacy,
        [1.05, 0.92, 1.08, 1.0],
        [0.9, 1.1, 1.0, 1.2],
        [1.1, 0.95, 1.05, 0.9],
        1.0,
        1.0,
        18.0,
        1.0,
    );
    assert_native_golden(
        plan(legacy)
            .execute_pixel(ColorBalancePixel::new(48.0, 8.0, -12.0, 0.0))
            .channels(),
        [51.3960495, 23.4370823, -25.4538651, 0.0],
        0.00003,
    );

    let lgg = ColorBalanceParametersV3::new(
        ColorBalanceMode::LiftGammaGain,
        [1.05, 0.92, 1.08, 1.0],
        [0.95, 1.05, 0.9, 1.1],
        [1.1, 0.95, 1.05, 0.9],
        1.17,
        0.83,
        20.0,
        0.79,
    );
    assert_native_golden(
        plan(lgg)
            .execute_pixel(ColorBalancePixel::new(62.0, -14.0, 22.0, 0.0))
            .channels(),
        [69.4160080, 17.2930355, 17.1728363, 0.0],
        0.00003,
    );

    let sop = ColorBalanceParametersV3::new(
        ColorBalanceMode::SlopeOffsetPower,
        [1.05, 0.92, 1.08, 1.0],
        [0.95, 1.05, 0.9, 1.1],
        [1.1, 0.95, 1.05, 0.9],
        0.88,
        1.21,
        16.0,
        1.11,
    );
    assert_native_golden(
        plan(sop)
            .execute_pixel(ColorBalancePixel::new(35.0, 4.0, 30.0, 0.0))
            .channels(),
        [32.8135529, 62.5003433, -14.4844828, 0.0],
        0.00003,
    );

    let mut guarded = ColorBalanceParametersV3::defaults();
    guarded.mode = ColorBalanceMode::SlopeOffsetPower;
    guarded.gamma = [0.0; CHANNEL_SIZE];
    guarded.contrast = 0.0;
    let coefficients = plan(guarded).coefficients();
    assert_eq!(
        [
            coefficients.contrast_power,
            coefficients.legacy_gamma_inv[0],
            coefficients.lgg_gamma_inv[0],
        ],
        [1_000_000.0, 1_000_000.0, 2_200_000.0]
    );

    let neutral = ColorBalanceParametersV3::defaults();
    let neutral_output = plan(neutral)
        .execute_pixel(ColorBalancePixel::new(48.0, 8.0, -12.0, 0.0))
        .channels();
    let mut threshold = neutral;
    threshold.saturation = 1.0 + 0.5e-6;
    threshold.saturation_out = 1.0 + 0.5e-6;
    threshold.contrast = 1.0 + 0.5e-6;
    assert_same_bits(
        neutral_output,
        plan(threshold)
            .execute_pixel(ColorBalancePixel::new(48.0, 8.0, -12.0, 0.0))
            .channels(),
    );

    let rgb = [0.18, 0.42, 0.73, 0.0];
    let xyz = math::prophoto_to_xyz(rgb);
    assert_native_golden(xyz, [0.22324997, 0.35089692, 0.60240328, 0.0], 0.0000005);
    let lab = math::xyz_to_lab(xyz);
    assert_native_golden(lab, [65.8184357, -45.6375771, -39.0385628, 0.0], 0.00003);
    assert_native_golden(math::lab_to_prophoto(lab), rgb, 0.00003);
}

#[test]
fn math_vectors_use_native_d50_and_prophoto_constants_with_bounded_approximation() {
    let xyz = math::lab_to_xyz([50.0, 0.0, 0.0, 123.0]);
    assert!((xyz[0] - 0.17758).abs() < 0.0002);
    assert!((xyz[1] - 0.18419).abs() < 0.0002);
    assert!((xyz[2] - 0.15191).abs() < 0.0002);
    assert_eq!(xyz[3].to_bits(), 0);

    let white = math::prophoto_to_xyz([1.0, 1.0, 1.0, 0.0]);
    assert!((white[0] - 0.96422).abs() < 0.0001);
    assert!((white[1] - 1.0).abs() < 0.0001);
    assert!((white[2] - 0.82521).abs() < 0.0001);
    assert!((math::approximate_powf(0.37, 1.7) - 0.37_f32.powf(1.7)).abs() < 0.01);
}

#[cfg(all(
    any(target_arch = "x86", target_arch = "x86_64"),
    target_feature = "sse2"
))]
#[test]
fn approximate_exp2_matches_native_sse2_nearest_even_half_integers() {
    for (value, expected_bits) in [
        (0.0, 0x3f80_0016),
        (1.0, 0x3fff_ffd5),
        (-2.0, 0x3e80_0016),
        (3.0, 0x40ff_ffd5),
        (-4.0, 0x3d80_0016),
    ] {
        assert_eq!(math::approximate_exp2(value).to_bits(), expected_bits);
    }
}

#[cfg(not(all(
    any(target_arch = "x86", target_arch = "x86_64"),
    target_feature = "sse2"
)))]
#[test]
fn approximate_exp2_matches_native_scalar_round_away_from_zero_half_integers() {
    for (value, expected_bits) in [
        (0.0, 0x3f7f_ffd5),
        (1.0, 0x4000_0016),
        (-2.0, 0x3e7f_ffd5),
        (3.0, 0x4100_0016),
        (-4.0, 0x3d7f_ffd5),
    ] {
        assert_eq!(math::approximate_exp2(value).to_bits(), expected_bits);
    }
}

#[test]
fn normal_blend_scales_lab_before_interpolation_and_preserves_native_rounding() {
    let result = blend_lab_normal_pixel_for_test(
        ColorBalancePixel::new(10.0, 20.0, 30.0, 0.37),
        ColorBalancePixel::new(40.0, -50.0, 100.0, -0.25),
        0.125,
    );
    assert_same_bits(result.channels(), [13.749999046, 11.25, 38.75, 0.125]);
}

#[test]
fn normal_blend_clamps_global_opacity_before_mask_and_publishes_coverage() {
    let processing = plan(ColorBalanceParametersV3::defaults());
    let source = [ColorBalancePixel::new(50.0, 0.0, 0.0, 0.37)];
    let candidate = processing.execute_lab(&source)[0];

    let over_opacity = processing
        .execute_lab_normal_blend(&source, Some(&[3.0]), 2.0)
        .unwrap()[0];
    assert_eq!(over_opacity.alpha_or_spare(), 3.0);
    assert_same_bits(
        over_opacity.channels(),
        blend_lab_normal_pixel_for_test(source[0], candidate, 3.0).channels(),
    );

    let under_opacity = processing
        .execute_lab_normal_blend(&source, Some(&[3.0]), -1.0)
        .unwrap()[0];
    assert_eq!(under_opacity.alpha_or_spare(), 0.0);
    assert_same_bits(
        under_opacity.channels(),
        blend_lab_normal_pixel_for_test(source[0], candidate, 0.0).channels(),
    );
    assert!(
        processing
            .execute_lab_normal_blend(&source, Some(&[]), 1.0)
            .is_err()
    );
}

#[test]
fn fourth_lane_and_external_alpha_are_distinct_from_blend_coverage() {
    let processing = plan(ColorBalanceParametersV3::defaults());
    let source = [ColorBalancePixel::new(50.0, 0.0, 0.0, 0.42)];
    let native_lane = processing.execute_lab(&source)[0];
    assert_eq!(native_lane.alpha_or_spare().to_bits(), 0);
    let preserved = processing
        .execute_lab_with_external_alpha(&source, &[0.42])
        .unwrap()[0];
    assert_eq!(preserved.alpha_or_spare(), 0.42);
    let blended = processing
        .execute_lab_normal_blend(&source, None, 0.25)
        .unwrap()[0];
    assert_eq!(blended.alpha_or_spare(), 0.25);
    assert_ne!(blended.alpha_or_spare(), preserved.alpha_or_spare());
}

#[test]
fn tiled_partition_matches_pointwise_and_cancellation_never_publishes_partial_output() {
    let processing = plan(sample_parameters(ColorBalanceMode::LiftGammaGain));
    let input: Vec<_> = (0..11)
        .map(|index| {
            let index = f32::from(u16::try_from(index).expect("fixture index fits u16"));
            lab_from_rgb([0.1 + index * 0.01, 0.4, 0.8, 0.0])
        })
        .collect();
    let pointwise = processing.execute_lab(&input);
    let tiled = processing.execute_lab_tiled(&input, 3, None).unwrap();
    for (left, right) in pointwise.iter().zip(tiled) {
        assert_same_bits(left.channels(), right.channels());
    }

    let cancelled = processing.execute_lab_tiled(&input, 3, Some(&|processed| processed >= 3));
    assert_eq!(
        cancelled,
        Err(ColorBalanceExecutionError::Cancelled { processed: 3 })
    );
    assert_eq!(
        processing.execute_lab_tiled(&input, 0, None),
        Err(ColorBalanceExecutionError::InvalidTileSize)
    );

    let blend_cancelled = processing.execute_lab_normal_blend_tiled(
        &input,
        None,
        0.5,
        4,
        Some(&|processed| processed >= 4),
    );
    assert_eq!(
        blend_cancelled,
        Err(ColorBalanceExecutionError::Cancelled { processed: 4 })
    );
}

#[test]
fn oversized_tiles_cancel_after_processing_begins_without_publishing_partial_output() {
    let processing = plan(ColorBalanceParametersV3::defaults());
    let input = vec![ColorBalancePixel::new(50.0, 0.0, 0.0, 0.0); 257];
    let cancelled =
        processing.execute_lab_tiled(&input, usize::MAX, Some(&|processed| processed >= 256));
    assert_eq!(
        cancelled,
        Err(ColorBalanceExecutionError::Cancelled { processed: 256 })
    );
}

#[test]
fn normal_blend_polls_cancellation_after_private_processing_completes() {
    let processing = plan(ColorBalanceParametersV3::defaults());
    let input = [ColorBalancePixel::new(50.0, 0.0, 0.0, 0.0); 3];
    let callback_count = AtomicUsize::new(0);
    let cancelled = processing.execute_lab_normal_blend_tiled(
        &input,
        None,
        0.5,
        usize::MAX,
        Some(&|processed| {
            let call = callback_count.fetch_add(1, Ordering::Relaxed);
            call >= 3 && processed == 1
        }),
    );
    assert_eq!(
        cancelled,
        Err(ColorBalanceExecutionError::Cancelled { processed: 1 })
    );
    assert!(callback_count.load(Ordering::Relaxed) >= 4);
}

#[test]
fn channel_indices_match_native_declarations() {
    assert_eq!(
        (
            CHANNEL_FACTOR,
            CHANNEL_RED,
            CHANNEL_GREEN,
            CHANNEL_BLUE,
            CHANNEL_SIZE
        ),
        (0, 1, 2, 3, 4)
    );
}
