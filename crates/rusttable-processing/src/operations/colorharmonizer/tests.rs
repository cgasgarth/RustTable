//! Source-derived tests for the unregistered Color Harmonizer leaf.
//!
//! Numerical tests use exact source boundaries and invariants.  The distinct
//! profile case below records source-equation golden vectors for the native
//! f32 call order, including both fused and Gaussian paths.

#![allow(
    clippy::excessive_precision,
    clippy::float_cmp,
    clippy::similar_names,
    clippy::unreadable_literal,
    reason = "source-derived floating-point assertions preserve exact native values"
)]

use std::cell::Cell;

use crate::operations::ReconstructionBudget;

use super::*;

fn identity_profile() -> WorkingProfileMatrices {
    WorkingProfileMatrices::new(
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
        [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    )
}

fn distinct_profile() -> WorkingProfileMatrices {
    WorkingProfileMatrices::new(
        [
            [0.72, 0.18, 0.05, 0.0],
            [0.20, 0.75, 0.10, 0.0],
            [0.08, 0.07, 0.85, 0.0],
        ],
        [
            [1.25, -0.10, 0.02, 0.0],
            [-0.15, 1.20, -0.04, 0.0],
            [0.03, -0.08, 1.10, 0.0],
        ],
    )
}

fn assert_pixels_close(actual: &[[f32; 4]], expected: &[[f32; 4]]) {
    assert_eq!(actual.len(), expected.len());
    for (actual_pixel, expected_pixel) in actual.iter().zip(expected) {
        for (actual_channel, expected_channel) in actual_pixel.iter().zip(expected_pixel) {
            assert!((actual_channel - expected_channel).abs() <= 2.0e-6);
        }
    }
}

#[test]
fn v1_codec_preserves_exact_native_offsets() {
    let parameters = ColorHarmonizerParametersV1::new(
        ColorHarmonizerRule::Triad,
        0.125,
        0.25,
        0.375,
        1.25,
        [0.0, 0.25, 0.5, 0.75],
        3,
        [0.5, 1.0, 1.5, 2.0],
        1.75,
    );
    let bytes = parameters.to_bytes();
    assert_eq!(bytes.len(), COLORHARMONIZER_PARAMETER_BYTES);
    assert_eq!(i32::from_le_bytes(bytes[0..4].try_into().unwrap()), 6);
    assert_eq!(f32::from_le_bytes(bytes[4..8].try_into().unwrap()), 0.125);
    assert_eq!(f32::from_le_bytes(bytes[8..12].try_into().unwrap()), 0.25);
    assert_eq!(f32::from_le_bytes(bytes[12..16].try_into().unwrap()), 0.375);
    assert_eq!(f32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1.25);
    assert_eq!(f32::from_le_bytes(bytes[20..24].try_into().unwrap()), 0.0);
    assert_eq!(f32::from_le_bytes(bytes[32..36].try_into().unwrap()), 0.75);
    assert_eq!(i32::from_le_bytes(bytes[36..40].try_into().unwrap()), 3);
    assert_eq!(f32::from_le_bytes(bytes[40..44].try_into().unwrap()), 0.5);
    assert_eq!(f32::from_le_bytes(bytes[52..56].try_into().unwrap()), 2.0);
    assert_eq!(f32::from_le_bytes(bytes[56..60].try_into().unwrap()), 1.75);
    assert_eq!(
        ColorHarmonizerParametersV1::from_bytes(&bytes),
        Ok(parameters)
    );
}

#[test]
fn codec_rejects_malformed_and_unknown_known_payloads() {
    assert!(matches!(
        ColorHarmonizerParametersV1::from_bytes(&[0; COLORHARMONIZER_PARAMETER_BYTES - 1]),
        Err(ColorHarmonizerCodecError::InvalidLength { .. })
    ));
    assert!(matches!(
        ColorHarmonizerParametersV1::from_bytes(&[0; COLORHARMONIZER_PARAMETER_BYTES + 1]),
        Err(ColorHarmonizerCodecError::InvalidLength { .. })
    ));
    let mut unknown_rule = ColorHarmonizerParametersV1::defaults().to_bytes();
    unknown_rule[0..4].copy_from_slice(&42_i32.to_le_bytes());
    assert_eq!(
        ColorHarmonizerParametersV1::from_bytes(&unknown_rule),
        Err(ColorHarmonizerCodecError::UnknownRule(42))
    );
}

#[test]
fn unknown_version_is_opaque_and_non_executable() {
    let bytes = [0xA5_u8; 13];
    let history = ColorHarmonizerHistory::decode(99, &bytes).unwrap();
    assert_eq!(history.version(), 99);
    assert_eq!(history.payload(), bytes);
    assert_eq!(
        history.current(),
        Err(ColorHarmonizerCodecError::UnsupportedVersion(99))
    );
}

#[test]
fn defaults_follow_declaration_and_init_order() {
    let defaults = ColorHarmonizerParametersV1::defaults();
    assert_eq!(defaults.rule, ColorHarmonizerRule::Complementary);
    assert_eq!(defaults.anchor_hue, 0.1);
    assert_eq!(defaults.pull_strength, 0.0);
    assert_eq!(defaults.neutral_protection, 0.5);
    assert_eq!(defaults.pull_width, 1.0);
    assert_eq!(defaults.custom_hue, [0.0, 0.25, 0.5, 0.75]);
    assert_eq!(defaults.num_custom_nodes, 4);
    assert_eq!(defaults.node_saturation, [1.0; 4]);
    assert_eq!(defaults.smoothing, 0.0);
}

#[test]
fn config_rejects_nonfinite_and_invalid_hue_domains_without_clamping() {
    let mut nonfinite = ColorHarmonizerParametersV1::defaults();
    nonfinite.pull_strength = f32::NAN;
    assert_eq!(
        ColorHarmonizerConfig::new(nonfinite),
        Err(ColorHarmonizerCodecError::NonFinite("pull_strength"))
    );

    let mut anchor = ColorHarmonizerParametersV1::defaults();
    anchor.anchor_hue = -0.001;
    assert_eq!(
        ColorHarmonizerConfig::new(anchor),
        Err(ColorHarmonizerCodecError::HueOutOfRange("anchor_hue"))
    );

    let mut custom = ColorHarmonizerParametersV1::defaults();
    custom.custom_hue[2] = 1.001;
    assert_eq!(
        ColorHarmonizerConfig::new(custom),
        Err(ColorHarmonizerCodecError::HueOutOfRange("custom_hue"))
    );

    let mut width = ColorHarmonizerParametersV1::defaults();
    width.pull_width = 0.0;
    assert_eq!(
        ColorHarmonizerConfig::new(width),
        Err(ColorHarmonizerCodecError::NonPositivePullWidth)
    );
}

#[test]
fn config_rejects_each_parameter_outside_native_ranges() {
    let mut pull_strength = ColorHarmonizerParametersV1::defaults();
    pull_strength.pull_strength = 1.001;
    assert!(matches!(
        ColorHarmonizerConfig::new(pull_strength),
        Err(ColorHarmonizerCodecError::ParameterOutOfRange {
            name: "pull_strength",
            ..
        })
    ));

    let mut neutral_protection = ColorHarmonizerParametersV1::defaults();
    neutral_protection.neutral_protection = -0.001;
    assert!(matches!(
        ColorHarmonizerConfig::new(neutral_protection),
        Err(ColorHarmonizerCodecError::ParameterOutOfRange {
            name: "neutral_protection",
            ..
        })
    ));

    let mut pull_width = ColorHarmonizerParametersV1::defaults();
    pull_width.pull_width = 0.249;
    assert!(matches!(
        ColorHarmonizerConfig::new(pull_width),
        Err(ColorHarmonizerCodecError::ParameterOutOfRange {
            name: "pull_width",
            ..
        })
    ));
    pull_width.pull_width = 4.001;
    assert!(matches!(
        ColorHarmonizerConfig::new(pull_width),
        Err(ColorHarmonizerCodecError::ParameterOutOfRange {
            name: "pull_width",
            ..
        })
    ));

    let mut node_count = ColorHarmonizerParametersV1::defaults();
    node_count.num_custom_nodes = 1;
    assert!(matches!(
        ColorHarmonizerConfig::new(node_count),
        Err(ColorHarmonizerCodecError::NodeCountOutOfRange {
            value: 1,
            minimum: 2,
            maximum: 4,
        })
    ));
    node_count.num_custom_nodes = 5;
    assert!(matches!(
        ColorHarmonizerConfig::new(node_count),
        Err(ColorHarmonizerCodecError::NodeCountOutOfRange {
            value: 5,
            minimum: 2,
            maximum: 4,
        })
    ));

    let mut node_saturation = ColorHarmonizerParametersV1::defaults();
    node_saturation.node_saturation[0] = 2.001;
    assert!(matches!(
        ColorHarmonizerConfig::new(node_saturation),
        Err(ColorHarmonizerCodecError::ParameterOutOfRange {
            name: "node_saturation",
            ..
        })
    ));

    let mut smoothing = ColorHarmonizerParametersV1::defaults();
    smoothing.smoothing = 2.001;
    assert!(matches!(
        ColorHarmonizerConfig::new(smoothing),
        Err(ColorHarmonizerCodecError::ParameterOutOfRange {
            name: "smoothing",
            ..
        })
    ));
}

#[test]
fn all_predefined_geometry_tables_have_source_counts_and_offsets() {
    let expected = [
        (ColorHarmonizerRule::Monochromatic, &[0.0][..]),
        (
            ColorHarmonizerRule::Analogous,
            &[-1.0 / 12.0, 0.0, 1.0 / 12.0][..],
        ),
        (
            ColorHarmonizerRule::AnalogousComplementary,
            &[-1.0 / 12.0, 0.0, 1.0 / 12.0, 6.0 / 12.0][..],
        ),
        (ColorHarmonizerRule::Complementary, &[0.0, 6.0 / 12.0][..]),
        (
            ColorHarmonizerRule::SplitComplementary,
            &[0.0, 5.0 / 12.0, 7.0 / 12.0][..],
        ),
        (ColorHarmonizerRule::Dyad, &[-1.0 / 12.0, 1.0 / 12.0][..]),
        (
            ColorHarmonizerRule::Triad,
            &[0.0, 4.0 / 12.0, 8.0 / 12.0][..],
        ),
        (
            ColorHarmonizerRule::Tetrad,
            &[-1.0 / 12.0, 1.0 / 12.0, 5.0 / 12.0, 7.0 / 12.0][..],
        ),
        (
            ColorHarmonizerRule::Square,
            &[0.0, 3.0 / 12.0, 6.0 / 12.0, 9.0 / 12.0][..],
        ),
    ];
    for (rule, offsets) in expected {
        assert_eq!(rule.geometry(), offsets);
    }
    let tables = HarmonyTables::build();
    let (wrapped, wrapped_count) = harmony_nodes(
        ColorHarmonizerRule::Complementary,
        1.0,
        &[0.0; 4],
        4,
        &tables,
    );
    let (zero, _zero_count) = harmony_nodes(
        ColorHarmonizerRule::Complementary,
        0.0,
        &[0.0; 4],
        4,
        &tables,
    );
    assert_eq!(wrapped_count, 2);
    assert_eq!(wrapped, zero);
    let (_, one_count) = harmony_nodes(
        ColorHarmonizerRule::Custom,
        0.0,
        &[0.1, 0.2, 0.3, 0.4],
        1,
        &tables,
    );
    assert_eq!(one_count, 1);
}

#[test]
fn ryb_knots_and_circular_interpolation_follow_source() {
    let rgb_knots = [
        0.0,
        1.0 / 6.0,
        2.0 / 6.0,
        3.0 / 6.0,
        4.0 / 6.0,
        5.0 / 6.0,
        1.0,
    ];
    let ryb_knots = [0.0, 1.0 / 3.0, 0.472217, 0.611105, 0.715271, 5.0 / 6.0, 1.0];
    for (index, (rgb, ryb)) in rgb_knots.into_iter().zip(ryb_knots).enumerate() {
        if index < 6 {
            assert_eq!(rgb_hue_to_ryb_hue(rgb), ryb);
        } else {
            // The native `h - floorf(h)` wraps the terminal control point to zero.
            assert_eq!(rgb_hue_to_ryb_hue(rgb), 0.0);
        }
    }
    assert!(hue_lerp(0.99, 0.01, 0.5).abs() <= f32::EPSILON);
    assert!(hue_lerp(0.01, 0.99, 0.5).abs() <= f32::EPSILON);
    assert_eq!(hue_lerp(0.2, 0.4, 0.25), 0.25);
}

#[test]
fn lut_is_720_entries_and_inverse_values_are_circular_domain_values() {
    let tables = HarmonyTables::build();
    assert_eq!(tables.forward().len(), COLORHARMONIZER_RYB_INVERSE_STEPS);
    assert_eq!(tables.inverse().len(), COLORHARMONIZER_RYB_INVERSE_STEPS);
    assert!(
        tables
            .forward()
            .iter()
            .all(|value| (0.0..=1.0).contains(value))
    );
    assert!(
        tables
            .inverse()
            .iter()
            .all(|value| (0.0..=1.0).contains(value))
    );
    assert_eq!(tables.ucs_to_ryb(1.0), tables.ucs_to_ryb(0.0));
    assert_eq!(tables.ryb_to_ucs(1.0), tables.ryb_to_ucs(0.0));
}

#[test]
fn gaussian_winner_uses_first_node_on_equal_midpoint() {
    let winner = weighted_hue_shift(0.25, &[0.0, 0.5], 2, 1.0);
    assert_eq!(winner.winning_index, 0);
    assert_eq!(winner.hue_shift, -0.25 * winner.maximum_weight);
    let at_node = weighted_hue_shift(0.0, &[0.0], 1, 0.25);
    assert_eq!(at_node.hue_shift, 0.0);
    assert_eq!(at_node.maximum_weight, 1.0);
    let wrap = weighted_hue_shift(0.99, &[0.01], 1, 1.0);
    assert!(wrap.hue_shift > 0.0);
}

#[test]
fn smoothing_sigma_preserves_native_scale_order() {
    let mut parameters = ColorHarmonizerParametersV1::defaults();
    parameters.smoothing = 0.5;
    parameters.pull_width = 2.0;
    let config = ColorHarmonizerConfig::new(parameters).unwrap();
    assert_eq!(smoothing_sigma(config, 2.0, 4.0).unwrap(), 4.0);
    assert!(matches!(
        smoothing_sigma(config, 0.0, 4.0),
        Err(ColorHarmonizerExecutionError::InvalidScale { .. })
    ));
}

#[test]
fn distinct_profile_matches_source_golden_for_fused_and_smoothed_paths() {
    let input = [
        [0.18, 0.32, 0.57, 0.125],
        [0.63, 0.14, 0.29, 0.25],
        [0.41, 0.76, 0.22, 0.5],
        [0.87, 0.27, 0.11, 0.75],
    ];
    let dimensions = FrameDimensions::new(2, 2).unwrap();
    let profile = distinct_profile();
    let mut parameters = ColorHarmonizerParametersV1::new(
        ColorHarmonizerRule::Custom,
        0.1,
        0.7,
        0.3,
        1.3,
        [0.05, 0.35, 0.65, 0.9],
        4,
        [0.8, 1.1, 1.3, 0.6],
        0.0,
    );
    let fused = ColorHarmonizerPlan::new(ColorHarmonizerConfig::new(parameters).unwrap())
        .execute(&input, dimensions, profile, 1.0, 1.0)
        .unwrap();
    // Source-equation golden values retain the native f32 operation ordering,
    // including distinct working-profile matrices and CAT16 adaptation.
    assert_pixels_close(
        &fused,
        &[
            [0.23382747, 0.31963134, 0.44954234, 0.125],
            [0.60401900, 0.19445087, 0.58534455, 0.25],
            [0.55951420, 0.70947367, 0.40114966, 0.5],
            [0.75232500, 0.38560593, -0.15645430, 0.75],
        ],
    );

    parameters.smoothing = 0.5;
    let smoothed = ColorHarmonizerPlan::new(ColorHarmonizerConfig::new(parameters).unwrap())
        .execute(&input, dimensions, profile, 1.0, 1.0)
        .unwrap();
    assert_pixels_close(
        &smoothed,
        &[
            [0.25631523, 0.31270100, 0.52611260, 0.125],
            [0.58539500, 0.21025662, 0.37569153, 0.25],
            [0.51598907, 0.72029890, 0.29055318, 0.5],
            [0.80194200, 0.35568914, 0.22496900, 0.75],
        ],
    );
}

#[test]
fn cpu_path_is_full_frame_cancellation_safe_and_preserves_alpha_bits() {
    let config = ColorHarmonizerConfig::new(ColorHarmonizerParametersV1::defaults()).unwrap();
    let plan = ColorHarmonizerPlan::new(config);
    let dimensions = FrameDimensions::new(2, 2).unwrap();
    let alpha = f32::from_bits(0x3f01_2345);
    let input = [[0.0, 0.0, 0.0, alpha]; 4];
    let output = plan
        .execute(&input, dimensions, identity_profile(), 1.0, 1.0)
        .unwrap();
    assert!(output.iter().all(|pixel| {
        pixel[0] == 0.0
            && pixel[1] == 0.0
            && pixel[2] == 0.0
            && pixel[3].to_bits() == alpha.to_bits()
    }));

    let polls = Cell::new(0);
    let cancelled =
        plan.execute_with_cancellation(&input, dimensions, identity_profile(), 1.0, 1.0, || {
            let poll = polls.get();
            polls.set(poll + 1);
            poll >= 2
        });
    assert_eq!(cancelled, Err(ColorHarmonizerExecutionError::Cancelled));
    assert!(polls.get() >= 3);

    let invalid_dimensions = FrameDimensions {
        width: usize::MAX,
        height: 2,
    };
    assert!(matches!(
        plan.execute(&[], invalid_dimensions, identity_profile(), 1.0, 1.0),
        Err(ColorHarmonizerExecutionError::InvalidDimensions { .. })
    ));
}

#[test]
fn validation_cancellation_precedes_trailing_nonfinite_pixel() {
    let plan = ColorHarmonizerPlan::new(ColorHarmonizerConfig::defaults());
    let dimensions = FrameDimensions::new(1_000, 100).unwrap();
    let mut input = vec![[0.0_f32; 4]; dimensions.pixels()];
    input[dimensions.pixels() - 1][0] = f32::NAN;
    let polls = Cell::new(0);

    let result =
        plan.execute_with_cancellation(&input, dimensions, identity_profile(), 1.0, 1.0, || {
            polls.set(polls.get() + 1);
            true
        });

    assert_eq!(result, Err(ColorHarmonizerExecutionError::Cancelled));
    assert_eq!(polls.get(), 1);
}

#[test]
fn validation_without_cancellation_reports_trailing_nonfinite_pixel() {
    let plan = ColorHarmonizerPlan::new(ColorHarmonizerConfig::defaults());
    let dimensions = FrameDimensions::new(1_000, 100).unwrap();
    let mut input = vec![[0.0_f32; 4]; dimensions.pixels()];
    input[dimensions.pixels() - 1][0] = f32::NAN;
    let result =
        plan.execute_with_cancellation(&input, dimensions, identity_profile(), 1.0, 1.0, || false);

    assert_eq!(
        result,
        Err(ColorHarmonizerExecutionError::NonFiniteInput {
            index: dimensions.pixels() - 1,
            channel: 0,
        })
    );
}

#[test]
fn negative_rgb_is_clamped_before_ucs_and_hdr_and_unclipped_outputs_are_retained() {
    let mut parameters = ColorHarmonizerParametersV1::defaults();
    parameters.pull_strength = 1.0;
    let plan = ColorHarmonizerPlan::new(ColorHarmonizerConfig::new(parameters).unwrap());
    let dimensions = FrameDimensions::new(1, 1).unwrap();
    let negative = plan
        .execute(
            &[[-1.0, -2.0, -3.0, 0.25]],
            dimensions,
            identity_profile(),
            1.0,
            1.0,
        )
        .unwrap();
    assert_eq!(negative[0][0..3], [0.0, 0.0, 0.0]);

    let unclipped_profile = WorkingProfileMatrices::new(
        identity_profile().matrix_in_transposed,
        [
            [-1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
        ],
    );
    let hdr = plan
        .execute(
            &[[4.0, 2.0, 1.0, 0.5]],
            dimensions,
            unclipped_profile,
            1.0,
            1.0,
        )
        .unwrap();
    assert!(hdr[0][0].is_sign_negative());
    assert!(hdr[0][0].is_finite());
}

#[test]
fn smoothing_path_blurs_full_frame_and_cancels_before_publication() {
    let mut parameters = ColorHarmonizerParametersV1::defaults();
    parameters.pull_strength = 1.0;
    parameters.smoothing = 0.5;
    let plan = ColorHarmonizerPlan::new(ColorHarmonizerConfig::new(parameters).unwrap());
    let dimensions = FrameDimensions::new(2, 2).unwrap();
    let alpha = f32::from_bits(0x3eaa_aaab);
    let input = [
        [0.8, 0.2, 0.1, alpha],
        [0.1, 0.7, 0.3, alpha],
        [0.2, 0.4, 0.9, alpha],
        [0.5, 0.6, 0.2, alpha],
    ];
    let output = plan
        .execute(&input, dimensions, identity_profile(), 1.0, 1.0)
        .unwrap();
    assert_eq!(output.len(), input.len());
    assert!(
        output
            .iter()
            .all(|pixel| pixel[3].to_bits() == alpha.to_bits())
    );

    let polls = Cell::new(0);
    let cancelled =
        plan.execute_with_cancellation(&input, dimensions, identity_profile(), 1.0, 1.0, || {
            let poll = polls.get();
            polls.set(poll + 1);
            poll >= 3
        });
    assert_eq!(cancelled, Err(ColorHarmonizerExecutionError::Cancelled));
    assert!(polls.get() >= 4);
}

#[test]
fn oversized_smoothing_frame_is_rejected_before_input_scan_or_publication() {
    let mut parameters = ColorHarmonizerParametersV1::defaults();
    parameters.smoothing = 0.5;
    let plan = ColorHarmonizerPlan::new(ColorHarmonizerConfig::new(parameters).unwrap());
    let dimensions = FrameDimensions::new(10_000, 10_000).unwrap();
    let result = plan.execute_with_budget(
        &[],
        dimensions,
        identity_profile(),
        1.0,
        1.0,
        ReconstructionBudget::new(512 * 1024 * 1024),
    );
    assert_eq!(
        result,
        Err(ColorHarmonizerExecutionError::MemoryBudgetExceeded {
            required: 4_400_000_000,
            budget: 512 * 1024 * 1024,
        })
    );
}

#[test]
fn descriptor_keeps_operation_unavailable_without_generic_blend_or_ui() {
    let descriptor = colorharmonizer_descriptor();
    assert_eq!(descriptor.parameters.len(), 9);
    assert_eq!(descriptor.parameters[0].id, "rule");
    assert_eq!(descriptor.parameters[5].id, "custom_hue");
    assert_eq!(descriptor.parameters[6].id, "num_custom_nodes");
    assert_eq!(descriptor.parameters[7].id, "node_saturation");
    assert_eq!(descriptor.parameters[8].id, "smoothing");
    assert_eq!(
        &descriptor.parameters[5].default,
        &crate::descriptor::ParameterDefault::Vector(vec![0.0, 0.25, 0.5, 0.75])
    );
    assert_eq!(descriptor.parameters[1].precision, 1);
    assert_eq!(descriptor.parameters[5].precision, 1);
    assert_eq!(descriptor.parameters[6].precision, 0);
    assert_eq!(descriptor.parameters[7].precision, 0);
    assert!(descriptor.validate().is_ok());
    const {
        assert!(!COLORHARMONIZER_REGISTERED);
        assert!(!COLORHARMONIZER_GPU_AVAILABLE);
        assert!(!COLORHARMONIZER_UI_AVAILABLE);
    }
    assert!(!descriptor.mask_blend.consumes_mask);
    assert!(!descriptor.mask_blend.publishes_mask);
    assert!(descriptor.ui.is_none());
}
