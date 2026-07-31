#![allow(
    clippy::excessive_precision,
    clippy::unreadable_literal,
    clippy::float_cmp,
    clippy::similar_names,
    reason = "native ABI and conversion vectors retain source decimal evidence"
)]

// The integration seam is intentionally not registered in shared hubs.
pub mod descriptor {
    pub use rusttable_processing::descriptor::*;
}

#[path = "../src/operations/colorbalancergb/mod.rs"]
mod colorbalancergb;

use colorbalancergb::codec::{
    COLORBALANCERGB_INTROSPECTION_VERSION, COLORBALANCERGB_V1_PARAMETER_BYTES,
    COLORBALANCERGB_V2_PARAMETER_BYTES, COLORBALANCERGB_V3_PARAMETER_BYTES,
    COLORBALANCERGB_V4_PARAMETER_BYTES, COLORBALANCERGB_V5_PARAMETER_BYTES, ColorBalanceRgbHistory,
    ColorBalanceRgbParametersV1, ColorBalanceRgbParametersV2, ColorBalanceRgbParametersV3,
    ColorBalanceRgbParametersV4, ColorBalanceRgbParametersV5, ColorBalanceRgbSaturationFormula,
    migrate_v1_to_v5,
};
use colorbalancergb::execution::{
    ColorBalanceRgbAlphaBehavior, ColorBalanceRgbConfig, ColorBalanceRgbExecutionError,
    ColorBalanceRgbPlan, ColorBalanceRgbProfile, capabilities, opacity_masks,
};
use colorbalancergb::math;
use colorbalancergb::source_map::{COLORBALANCERGB_SOURCE_MAP, ColorBalanceRgbPortStatus};
use colorbalancergb::{
    COLORBALANCERGB_COMPATIBILITY_ID, COLORBALANCERGB_RUST_ID, colorbalancergb_descriptor,
};
use rusttable_processing::RasterDimensions;
use rusttable_processing::descriptor::{OperationFlags, ParameterDefault};

fn fixture(name: &str) -> Vec<u8> {
    let source = match name {
        "v1" => include_str!("fixtures/colorbalancergb/v1.hex"),
        "v2" => include_str!("fixtures/colorbalancergb/v2.hex"),
        "v3" => include_str!("fixtures/colorbalancergb/v3.hex"),
        "v4" => include_str!("fixtures/colorbalancergb/v4.hex"),
        "v5" => include_str!("fixtures/colorbalancergb/v5.hex"),
        _ => panic!("unknown fixture"),
    };
    source
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("hex fixture"))
        .collect()
}

fn v1_sample() -> ColorBalanceRgbParametersV1 {
    ColorBalanceRgbParametersV1::new([
        -0.2, 0.15, 35.0, 0.1, 0.25, 210.0, -0.3, 0.4, 300.0, 0.05, 0.1, 120.0, 1.25, -1.0, 0.75,
        -0.1, 0.2, 0.3, -0.15, 25.0, -0.2, 0.15, 0.35, -22.0,
    ])
}

fn v5_sample(formula: ColorBalanceRgbSaturationFormula) -> ColorBalanceRgbParametersV5 {
    let v1 = v1_sample();
    let v2 = ColorBalanceRgbParametersV2::new(v1, [0.1, -0.2, 0.15, -0.05]);
    let v3 = ColorBalanceRgbParametersV3::new(v2, 0.22);
    let v4 = ColorBalanceRgbParametersV4::new(v3, 0.12, 0.1845, -0.1);
    ColorBalanceRgbParametersV5::new(v4, formula)
}

#[test]
fn native_abi_sizes_and_v5_formula_order_are_explicit() {
    assert_eq!(COLORBALANCERGB_V1_PARAMETER_BYTES, 96);
    assert_eq!(COLORBALANCERGB_V2_PARAMETER_BYTES, 112);
    assert_eq!(COLORBALANCERGB_V3_PARAMETER_BYTES, 116);
    assert_eq!(COLORBALANCERGB_V4_PARAMETER_BYTES, 128);
    assert_eq!(COLORBALANCERGB_V5_PARAMETER_BYTES, 132);
    let v5 = v5_sample(ColorBalanceRgbSaturationFormula::DarktableUcs2022);
    let bytes = v5.to_bytes();
    assert_eq!(bytes.to_vec(), fixture("v5"));
    assert_eq!(v1_sample().to_bytes().to_vec(), fixture("v1"));
    assert_eq!(
        ColorBalanceRgbParametersV2::new(v1_sample(), [0.1, -0.2, 0.15, -0.05])
            .to_bytes()
            .to_vec(),
        fixture("v2")
    );
    assert_eq!(
        ColorBalanceRgbParametersV3::new(
            ColorBalanceRgbParametersV2::new(v1_sample(), [0.1, -0.2, 0.15, -0.05]),
            0.22,
        )
        .to_bytes()
        .to_vec(),
        fixture("v3")
    );
    assert_eq!(
        ColorBalanceRgbParametersV4::new(
            ColorBalanceRgbParametersV3::new(
                ColorBalanceRgbParametersV2::new(v1_sample(), [0.1, -0.2, 0.15, -0.05]),
                0.22,
            ),
            0.12,
            0.1845,
            -0.1,
        )
        .to_bytes()
        .to_vec(),
        fixture("v4")
    );
    assert_eq!(&bytes[0..4], &(-0.2_f32).to_le_bytes());
    assert_eq!(&bytes[124..128], &(-0.1_f32).to_le_bytes());
    assert_eq!(&bytes[128..132], &1_i32.to_le_bytes());
    assert_eq!(ColorBalanceRgbParametersV5::from_bytes(&bytes).unwrap(), v5);
    assert_eq!(
        ColorBalanceRgbParametersV5::defaults().v4.grey_fulcrum,
        0.1845
    );
    assert_eq!(
        ColorBalanceRgbParametersV5::legacy_default_v5()
            .v4
            .grey_fulcrum,
        0.0
    );
    assert_eq!(
        ColorBalanceRgbParametersV5::legacy_default_v5().saturation_formula,
        ColorBalanceRgbSaturationFormula::DarktableUcs2022
    );
}

#[test]
fn native_module_default_matches_descriptor_and_codec() {
    let expected = ColorBalanceRgbSaturationFormula::DarktableUcs2022;
    assert_eq!(
        ColorBalanceRgbParametersV5::defaults().saturation_formula,
        expected
    );

    let descriptor = colorbalancergb_descriptor();
    assert_eq!(
        descriptor.parameters[32].default,
        ParameterDefault::Enum("darktable-ucs-2022".to_owned())
    );
}

#[test]
fn history_round_trips_all_native_versions_and_migrates_exact_defaults() {
    let v1 = v1_sample();
    let v2 = ColorBalanceRgbParametersV2::new(v1, [0.1, -0.2, 0.15, -0.05]);
    let v3 = ColorBalanceRgbParametersV3::new(v2, 0.22);
    let v4 = ColorBalanceRgbParametersV4::new(v3, 0.12, 0.1845, -0.1);
    let v5 = v5_sample(ColorBalanceRgbSaturationFormula::JzAzBz);
    for (version, payload) in [
        (1, v1.to_bytes().to_vec()),
        (2, v2.to_bytes().to_vec()),
        (3, v3.to_bytes().to_vec()),
        (4, v4.to_bytes().to_vec()),
        (5, v5.to_bytes().to_vec()),
    ] {
        let history = ColorBalanceRgbHistory::decode(version, &payload).unwrap();
        assert_eq!(history.version(), version);
        assert_eq!(history.payload(), payload);
        assert_eq!(
            history.current().unwrap().to_bytes().len(),
            COLORBALANCERGB_V5_PARAMETER_BYTES
        );
    }
    let migrated = migrate_v1_to_v5(v1);
    assert_eq!(migrated.v4.v3.v2.v1.saturation_global, 0.25);
    assert_eq!(migrated.v4.v3.mask_grey_fulcrum, 0.1845);
    assert_eq!(migrated.v4.grey_fulcrum, 0.1845);
    assert_eq!(
        migrated.saturation_formula,
        ColorBalanceRgbSaturationFormula::JzAzBz
    );
    let opaque = ColorBalanceRgbHistory::decode(77, &[0xde, 0xad]).unwrap();
    assert!(opaque.current().is_err());
    assert_eq!(COLORBALANCERGB_INTROSPECTION_VERSION, 5);
}

#[test]
fn descriptor_preserves_native_order_and_fail_closed_capabilities() {
    let descriptor = colorbalancergb_descriptor();
    descriptor.validate().unwrap();
    assert_eq!(
        descriptor.id.compatibility_name,
        COLORBALANCERGB_COMPATIBILITY_ID
    );
    assert_eq!(descriptor.id.rust_id, COLORBALANCERGB_RUST_ID);
    assert_eq!(descriptor.stage, "scene-referred-rgb-profile-d50");
    assert!(descriptor.flags.contains(OperationFlags::TILEABLE));
    assert!(!descriptor.flags.contains(OperationFlags::FULL_IMAGE));
    assert_eq!(descriptor.parameters.len(), 33);
    assert_eq!(descriptor.parameters[0].id, "shadows_y");
    assert_eq!(descriptor.parameters[23].id, "hue_angle");
    assert_eq!(descriptor.parameters[32].id, "saturation_formula");
    assert_eq!(
        descriptor.parameters[32].default,
        ParameterDefault::Enum("darktable-ucs-2022".to_owned())
    );
    assert!(descriptor.capability.cpu_supported);
    assert!(descriptor.capability.gpu_tier.is_none());
    assert!(descriptor.ui.is_none());
    assert!(descriptor.mask_blend.consumes_mask);
    assert!(!descriptor.mask_blend.blend_if);
}

#[test]
fn source_map_marks_only_leaf_math_ported() {
    assert!(COLORBALANCERGB_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol == "dt_XYZ_2_JzAzBz / dt_JzAzBz_2_XYZ"
            && entry.status == ColorBalanceRgbPortStatus::Ported
    }));
    assert!(COLORBALANCERGB_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol == "outer mask blend / blend-if / alpha publication"
            && entry.status == ColorBalanceRgbPortStatus::ExplicitlyDeferred
    }));
    assert!(COLORBALANCERGB_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol == "process_cl / colorbalancergb OpenCL kernel"
            && entry.status == ColorBalanceRgbPortStatus::ExplicitlyDeferred
    }));
    let map = include_str!("../../../architecture/rusttable-colorbalancergb-source-map.toml");
    assert!(map.contains("production_registration = \"deferred"));
    assert!(map.contains("src/common/darktable_ucs_22_helpers.h"));
}

#[test]
fn internal_masks_and_commit_coefficients_follow_native_order() {
    let config =
        ColorBalanceRgbConfig::new(v5_sample(ColorBalanceRgbSaturationFormula::JzAzBz)).unwrap();
    let coefficients = colorbalancergb::ColorBalanceRgbCoefficients::commit(config);
    assert_eq!(coefficients.chroma[0], -0.1);
    assert_eq!(coefficients.chroma[1], -0.15);
    assert_eq!(coefficients.chroma[2], 0.2);
    assert_eq!(coefficients.saturation[0], 0.35);
    assert_eq!(coefficients.saturation[1], 0.15);
    assert_eq!(coefficients.saturation[2], -0.2);
    assert_eq!(coefficients.brilliance[0], -0.05);
    assert_eq!(coefficients.brilliance[1], 0.15);
    assert_eq!(coefficients.brilliance[2], -0.2);
    let masks = opacity_masks(0.1845_f32.powf(0.4101205819200422), coefficients);
    assert!(masks.shadows.is_finite());
    assert!(masks.midtones.is_finite());
    assert!(masks.highlights.is_finite());
    assert!((0.0..=1.0).contains(&masks.shadows));
    assert!((0.0..=1.0).contains(&masks.highlights));
}

#[test]
fn profile_conversion_uses_cat16_and_jzazbz_not_generic_rgb_math() {
    let adapted =
        math::xyz_d65_input_matrix(ColorBalanceRgbProfile::identity().input_rgb_to_xyz_d50());
    assert_ne!(adapted, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    let xyz = math::apply(adapted, [0.25, 0.5, 0.75]);
    let jab = math::xyz_to_jzazbz([xyz[0], xyz[1], xyz[2], 0.0]);
    let roundtrip = math::jzazbz_to_xyz(jab);
    for (actual, expected) in roundtrip.into_iter().zip(xyz) {
        assert!((actual - expected).abs() < 0.003, "{actual} != {expected}");
    }
    let ucs = math::xyy_to_ucs_jch(
        math::xyz_to_xyy([0.2, 0.4, 0.1, 0.0]),
        math::y_to_ucs_lstar(1.0),
    );
    let back = math::xyy_to_xyz(math::ucs_jch_to_xyy(ucs, math::y_to_ucs_lstar(1.0)));
    assert!(back.into_iter().all(f32::is_finite));
}

#[test]
fn native_process_input_matrix_includes_xyz_to_lms_in_commit_order() {
    let identity = ColorBalanceRgbProfile::identity();
    let process_matrix = math::input_matrix(identity.input_rgb_to_xyz_d50());
    assert_eq!(
        process_matrix,
        [
            [0.24974132, 0.85491127, -0.030628867],
            [-0.39667058, 1.2010254, 0.119133756],
            [0.06435915, -0.07092515, 0.7309533],
        ]
    );
    assert_ne!(
        process_matrix,
        math::xyz_d65_input_matrix(identity.input_rgb_to_xyz_d50())
    );
    assert_eq!(
        math::apply(process_matrix, [0.25, 0.5, 0.75]),
        [0.4669193, 0.5906954, 0.5288422]
    );
}

#[test]
fn native_global_grading_controls_reach_each_plan_lane_and_cpu_process() {
    let mut v1 = ColorBalanceRgbParametersV1::defaults();
    v1.chroma_global = 0.2;
    v1.saturation_global = 0.2;
    let v2 = ColorBalanceRgbParametersV2::new(v1, [0.2, 0.0, 0.0, 0.0]);
    let v3 = ColorBalanceRgbParametersV3::new(v2, 0.1845);
    let parameters = ColorBalanceRgbParametersV5::new(
        ColorBalanceRgbParametersV4::new(v3, 0.0, 0.1845, 0.0),
        ColorBalanceRgbSaturationFormula::DarktableUcs2022,
    );
    let config = ColorBalanceRgbConfig::new(parameters).unwrap();
    let coefficients = colorbalancergb::ColorBalanceRgbCoefficients::commit(config);
    assert_eq!(coefficients.chroma[3], 0.2);
    assert_eq!(coefficients.saturation[3], 0.2);
    assert_eq!(coefficients.brilliance[3], 0.2);

    let neutral = ColorBalanceRgbConfig::defaults();
    let global_plan = ColorBalanceRgbPlan::new(config, ColorBalanceRgbProfile::identity()).unwrap();
    let neutral_plan =
        ColorBalanceRgbPlan::new(neutral, ColorBalanceRgbProfile::identity()).unwrap();
    let dimensions = RasterDimensions::new(1, 1).unwrap();
    let input = [[0.35, 0.25, 0.15, 0.0]];
    let global_output = global_plan.execute(dimensions, &input).unwrap();
    let neutral_output = neutral_plan.execute(dimensions, &input).unwrap();
    assert_ne!(global_output, neutral_output);
}

#[test]
fn cpu_leaf_is_identity_roi_transactional_and_records_native_alpha_lane() {
    let config = ColorBalanceRgbConfig::new(v5_sample(
        ColorBalanceRgbSaturationFormula::DarktableUcs2022,
    ))
    .unwrap();
    let plan = ColorBalanceRgbPlan::new(config, ColorBalanceRgbProfile::identity()).unwrap();
    let dimensions = RasterDimensions::new(2, 2).unwrap();
    let input = [
        [0.2, 0.3, 0.4, 0.11],
        [0.4, 0.3, 0.2, 0.22],
        [0.8, 0.2, 0.1, 0.33],
        [0.1, 0.2, 0.8, 0.44],
    ];
    let output = plan.execute(dimensions, &input).unwrap();
    assert_eq!(output.len(), input.len());
    assert!(output.iter().all(|pixel| pixel[3] == 0.0));
    assert_eq!(
        capabilities().cpu_alpha_behavior,
        ColorBalanceRgbAlphaBehavior::NativeCpuFourthLaneZero
    );
    assert_eq!(colorbalancergb::tiling().overlap_pixels, 0);
    assert_eq!(colorbalancergb::tiling().alignment_pixels, 1);
    assert!(capabilities().require_external_blending().is_err());
    let mut polls = 0;
    let cancelled = plan.execute_with_cancel(dimensions, &input, || {
        polls += 1;
        polls > 1
    });
    assert_eq!(cancelled, Err(ColorBalanceRgbExecutionError::Cancelled));
}

#[test]
fn plan_cancellation_can_abort_expensive_gamut_lut_before_publication() {
    let config =
        ColorBalanceRgbConfig::new(v5_sample(ColorBalanceRgbSaturationFormula::JzAzBz)).unwrap();
    let result =
        ColorBalanceRgbPlan::new_with_cancel(config, ColorBalanceRgbProfile::identity(), || true);
    assert!(matches!(
        result,
        Err(ColorBalanceRgbExecutionError::Cancelled)
    ));
}
