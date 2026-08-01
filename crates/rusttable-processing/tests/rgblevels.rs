//! Source-derived bounded RGB Levels CPU leaf coverage.
//!
//! The operation is registered for bounded CPU execution. OpenCL, auto-analysis,
//! configured profile transforms, GUI/presets, masks, and outer blending remain
//! explicit deferred seams.

#![allow(
    clippy::assertions_on_constants,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::needless_range_loop,
    clippy::too_many_lines,
    clippy::unreadable_literal,
    reason = "source-derived vectors assert native ABI order and f32 boundaries"
)]

use rusttable_processing::operations::rgblevels;

use std::mem::size_of;

use rgblevels::source_map::{RGBLEVELS_SOURCE_MAP, RgbLevelsPortStatus};
use rgblevels::{
    RGBLEVELS_DEFAULT_COLORSPACE, RGBLEVELS_DEFAULT_ENABLED, RGBLEVELS_DEFAULT_GROUPS,
    RGBLEVELS_DEFAULT_ORDER, RGBLEVELS_DEFAULT_VISIBLE, RGBLEVELS_GPU_KERNEL,
    RGBLEVELS_GPU_PROGRAM, RGBLEVELS_MIGRATION_EDGES, RGBLEVELS_PARAMETER_BYTES,
    RGBLEVELS_SCHEMA_VERSION, RgbLevelsAutoscale, RgbLevelsCapabilityError, RgbLevelsConfig,
    RgbLevelsExecutionError, RgbLevelsHistory, RgbLevelsParametersV1, RgbLevelsPixel,
    RgbLevelsPlan, RgbLevelsPreserveColors, RgbLevelsProfileError, RgbLevelsProfileEvidence,
};

const DEFAULT_FIXTURE: &str = include_str!("fixtures/rgblevels/default-v1.hex");

#[test]
fn native_abi_defaults_and_fixture_preserve_two_enums_then_nine_floats() {
    assert_eq!(RGBLEVELS_SCHEMA_VERSION, 1);
    assert_eq!(RGBLEVELS_PARAMETER_BYTES, 44);
    assert_eq!(size_of::<RgbLevelsParametersV1>(), 44);
    assert_eq!(RGBLEVELS_MIGRATION_EDGES, &[]);
    assert!(!RGBLEVELS_DEFAULT_ENABLED);
    assert!(RGBLEVELS_DEFAULT_VISIBLE);
    assert_eq!(RGBLEVELS_DEFAULT_ORDER, 126);
    assert_eq!(RGBLEVELS_DEFAULT_GROUPS, ["tone", "grading"]);
    assert_eq!(RGBLEVELS_DEFAULT_COLORSPACE, "RGB");
    assert_eq!(RGBLEVELS_GPU_PROGRAM, 29);
    assert_eq!(RGBLEVELS_GPU_KERNEL, "rgblevels");

    let defaults = RgbLevelsParametersV1::defaults();
    assert_eq!(defaults.autoscale, RgbLevelsAutoscale::LinkedChannels);
    assert_eq!(defaults.preserve_colors, RgbLevelsPreserveColors::Luminance);
    assert_eq!(
        defaults.levels,
        [[0.0, 0.5, 1.0], [0.0, 0.5, 1.0], [0.0, 0.5, 1.0]]
    );
    assert_eq!(
        bytes_to_hex(&defaults.to_bytes()),
        fixture_value(DEFAULT_FIXTURE, "payload_hex")
    );
    assert!(DEFAULT_FIXTURE.contains("payload_bytes=44"));
    assert!(DEFAULT_FIXTURE.contains(
        "field_order=autoscale,preserve_colors,levels[0][0],levels[0][1],levels[0][2],levels[1][0],levels[1][1],levels[1][2],levels[2][0],levels[2][1],levels[2][2]"
    ));
    assert!(DEFAULT_FIXTURE.contains("migration_edges=[]"));
}

#[test]
fn current_v1_round_trips_and_future_versions_stay_opaque() {
    let parameters = RgbLevelsParametersV1::new(
        RgbLevelsAutoscale::IndependentChannels,
        RgbLevelsPreserveColors::Power,
        [[0.1, 0.35, 0.9], [0.0, 0.5, 1.0], [0.2, 0.6, 1.4]],
    );
    let history = RgbLevelsHistory::decode(1, &parameters.to_bytes()).expect("valid v1 history");
    assert_eq!(history, RgbLevelsHistory::V1(parameters));
    assert_eq!(history.version(), 1);
    assert_eq!(history.payload(), parameters.to_bytes());
    assert_eq!(history.current().expect("v1 materializes"), parameters);

    let future_payload = vec![0xde, 0xad, 0xbe, 0xef, 0x2a];
    let future = RgbLevelsHistory::decode(2, &future_payload).expect("future remains opaque");
    assert_eq!(future.version(), 2);
    assert_eq!(future.payload(), future_payload);
    assert_eq!(
        future.current(),
        Err(rgblevels::RgbLevelsCodecError::UnsupportedVersion(2))
    );
}

#[test]
fn malformed_known_payloads_reject_length_and_enum_drift() {
    assert_eq!(
        RgbLevelsHistory::decode(1, &[0; 43]),
        Err(rgblevels::RgbLevelsCodecError::InvalidLength {
            expected: 44,
            actual: 43,
        })
    );
    let mut invalid_autoscale = RgbLevelsParametersV1::defaults().to_bytes();
    invalid_autoscale[0..4].copy_from_slice(&9_i32.to_le_bytes());
    assert_eq!(
        RgbLevelsHistory::decode(1, &invalid_autoscale),
        Err(rgblevels::RgbLevelsCodecError::InvalidAutoscale(9))
    );
    let mut invalid_preserve = RgbLevelsParametersV1::defaults().to_bytes();
    invalid_preserve[4..8].copy_from_slice(&9_i32.to_le_bytes());
    assert_eq!(
        RgbLevelsHistory::decode(1, &invalid_preserve),
        Err(rgblevels::RgbLevelsCodecError::InvalidPreserveColors(9))
    );
}

#[test]
fn configuration_rejects_nonfinite_and_degenerate_ranges_without_clamping() {
    let mut parameters = RgbLevelsParametersV1::defaults();
    parameters.levels[0] = [-2.0, -1.0, 3.0];
    assert!(RgbLevelsConfig::new(parameters).is_ok());

    parameters.levels[1][0] = f32::NAN;
    assert_eq!(
        RgbLevelsConfig::new(parameters),
        Err(rgblevels::RgbLevelsParameterError::NonFiniteLevel {
            channel: 1,
            point: 0,
        })
    );

    parameters = RgbLevelsParametersV1::defaults();
    parameters.levels[2][2] = parameters.levels[2][0];
    assert_eq!(
        RgbLevelsConfig::new(parameters),
        Err(rgblevels::RgbLevelsParameterError::NonIncreasingRange {
            channel: 2,
            minimum: 0.0,
            maximum: 0.0,
        })
    );
}

#[test]
fn commit_preserves_linked_expansion_and_native_pow_order() {
    let parameters = RgbLevelsParametersV1::new(
        RgbLevelsAutoscale::LinkedChannels,
        RgbLevelsPreserveColors::Luminance,
        [[0.1, 0.25, 0.9], [0.0, 0.75, 1.0], [0.2, 0.5, 1.4]],
    );
    let config = RgbLevelsConfig::new(parameters).expect("finite levels");
    let plan = RgbLevelsPlan::new(config, None).expect("valid LUT");
    assert_eq!(plan.effective_levels(), [parameters.levels[0]; 3]);

    let delta = (0.9_f32 - 0.1_f32) / 2.0;
    let mid = 0.1_f32 + delta;
    let tmp = (0.25_f32 - mid) / delta;
    let expected_gamma = (10.0_f64).powf(f64::from(tmp)) as f32;
    assert_eq!(plan.inv_gamma(), [expected_gamma; 3]);
    let expected_multiplier = 1.0_f32 / (0.9_f32 - 0.1_f32);
    assert_eq!(plan.multipliers(), [expected_multiplier; 3]);

    let table = plan.lut(0).expect("red LUT");
    let table_index = 0x1234_usize;
    let percentage = table_index as f32 / 65_536.0_f32;
    let expected_lut = (f64::from(percentage)).powf(f64::from(expected_gamma)) as f32;
    assert_eq!(table[table_index].to_bits(), expected_lut.to_bits());

    let independent = RgbLevelsParametersV1::new(
        RgbLevelsAutoscale::IndependentChannels,
        RgbLevelsPreserveColors::None,
        parameters.levels,
    );
    let independent_plan = RgbLevelsPlan::new(
        RgbLevelsConfig::new(independent).expect("independent levels"),
        None,
    )
    .expect("independent LUT");
    assert_eq!(independent_plan.effective_levels(), parameters.levels);
    assert_ne!(
        independent_plan.inv_gamma()[0],
        independent_plan.inv_gamma()[1]
    );
}

#[test]
fn independent_channels_follow_clip_lut_extrapolation_and_leave_alpha_zero() {
    let parameters = RgbLevelsParametersV1::new(
        RgbLevelsAutoscale::IndependentChannels,
        RgbLevelsPreserveColors::Luminance,
        [[0.1, 0.4, 0.8], [0.2, 0.5, 0.9], [0.0, 0.5, 1.0]],
    );
    let plan = RgbLevelsPlan::new(
        RgbLevelsConfig::new(parameters).expect("valid levels"),
        None,
    )
    .expect("valid plan");
    let input = [RgbLevelsPixel::new(0.0, 0.2, 1.25, 0.75)];
    let output = plan.execute(&input).expect("CPU output");
    let channels = output[0].channels();
    assert_eq!(channels[0].to_bits(), 0.0_f32.to_bits());
    assert_eq!(
        channels[1].to_bits(),
        plan.lut(1).expect("green LUT")[0].to_bits()
    );
    let blue_percentage = (1.25_f32 - 0.0) * plan.multipliers()[2];
    let expected_blue = blue_percentage.powf(plan.inv_gamma()[2]);
    assert_eq!(channels[2].to_bits(), expected_blue.to_bits());
    assert_eq!(channels[3].to_bits(), 0.0_f32.to_bits());
}

#[test]
fn linked_default_nontrivial_luminance_ratio_keeps_fourth_lane_canonical() {
    let plan = RgbLevelsPlan::new(RgbLevelsConfig::defaults(), None).expect("default plan");
    let rgb = [0.37_f32, 0.61, 0.19];
    let input_alpha = 0.875_f32;
    let output = plan
        .execute(&[RgbLevelsPixel::new(rgb[0], rgb[1], rgb[2], input_alpha)])
        .expect("linked default output");

    let luminance = rgb[0] * 0.222_504_5 + rgb[1] * 0.716_878_6 + rgb[2] * 0.060_616_9;
    let percentage = luminance * plan.multipliers()[0];
    let lut_index = (percentage * 65_536.0_f32) as usize;
    let curve_luminance = plan.lut(0).expect("red LUT")[lut_index];
    let ratio = curve_luminance / luminance;
    assert_ne!(ratio.to_bits(), 1.0_f32.to_bits());
    assert_eq!(
        output[0].channels().map(f32::to_bits),
        [
            (ratio * rgb[0]).to_bits(),
            (ratio * rgb[1]).to_bits(),
            (ratio * rgb[2]).to_bits(),
            0.0_f32.to_bits(),
        ]
    );
    assert!((0.0..=1.0).contains(&output[0].channels()[3]));
}

#[test]
fn linked_brightening_cannot_publish_ratio_scaled_alpha() {
    let mut parameters = RgbLevelsParametersV1::defaults();
    parameters.levels[0] = [0.0, 0.25, 1.0];
    let plan = RgbLevelsPlan::new(
        RgbLevelsConfig::new(parameters).expect("linked brightening levels"),
        None,
    )
    .expect("linked brightening plan");
    let rgb = [0.2_f32, 0.4, 0.6];
    let input_alpha = 0.9_f32;
    let output = plan
        .execute(&[RgbLevelsPixel::new(rgb[0], rgb[1], rgb[2], input_alpha)])
        .expect("linked brightening output");

    let luminance = rgb[0] * 0.222_504_5 + rgb[1] * 0.716_878_6 + rgb[2] * 0.060_616_9;
    let percentage = luminance * plan.multipliers()[0];
    let lut_index = (percentage * 65_536.0_f32) as usize;
    let curve_luminance = plan.lut(0).expect("red LUT")[lut_index];
    let ratio = curve_luminance / luminance;
    assert!(ratio * input_alpha > 1.0);
    assert_eq!(
        output[0].channels().map(f32::to_bits),
        [
            (ratio * rgb[0]).to_bits(),
            (ratio * rgb[1]).to_bits(),
            (ratio * rgb[2]).to_bits(),
            0.0_f32.to_bits(),
        ]
    );
}

#[test]
fn linked_channels_apply_every_rgb_norm_without_scaling_fourth_lane() {
    let modes = [
        RgbLevelsPreserveColors::Luminance,
        RgbLevelsPreserveColors::Max,
        RgbLevelsPreserveColors::Average,
        RgbLevelsPreserveColors::Sum,
        RgbLevelsPreserveColors::Norm,
        RgbLevelsPreserveColors::Power,
    ];
    let input = [RgbLevelsPixel::new(1.0, 0.5, 0.25, 0.75)];
    for mode in modes {
        let parameters = RgbLevelsParametersV1::new(
            RgbLevelsAutoscale::LinkedChannels,
            mode,
            RgbLevelsParametersV1::defaults().levels,
        );
        let plan = RgbLevelsPlan::new(
            RgbLevelsConfig::new(parameters).expect("default levels"),
            None,
        )
        .expect("valid plan");
        let output = plan.execute(&input).expect("linked output");
        let channels = output[0].channels();
        assert!(channels[..3].iter().copied().all(f32::is_finite));
        assert_eq!(channels[3].to_bits(), 0.0_f32.to_bits());
        assert!(channels[0] >= channels[1]);
        assert!(channels[1] >= channels[2]);
    }

    let none = RgbLevelsParametersV1::new(
        RgbLevelsAutoscale::LinkedChannels,
        RgbLevelsPreserveColors::None,
        RgbLevelsParametersV1::defaults().levels,
    );
    let output = RgbLevelsPlan::new(RgbLevelsConfig::new(none).expect("none"), None)
        .expect("none plan")
        .execute(&input)
        .expect("none output");
    assert_eq!(output[0].channels()[3].to_bits(), 0.0_f32.to_bits());
}

#[test]
fn linked_below_black_does_not_read_the_fourth_lane_and_writes_zero() {
    let plan = RgbLevelsPlan::new(RgbLevelsConfig::defaults(), None).expect("default plan");
    for rgb in [[-1.0, -0.5, -0.25], [0.0, 0.0, 0.0]] {
        for unread_lane in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let output = plan
                .execute(&[RgbLevelsPixel::new(rgb[0], rgb[1], rgb[2], unread_lane)])
                .expect("below-black path does not read lane three");
            assert_eq!(output[0].channels().map(f32::to_bits), [0, 0, 0, 0]);
        }
    }
}

#[test]
fn fourth_lane_is_unread_across_direct_and_linked_rgb_paths() {
    let rgb_only = RgbLevelsParametersV1::new(
        RgbLevelsAutoscale::LinkedChannels,
        RgbLevelsPreserveColors::None,
        RgbLevelsParametersV1::defaults().levels,
    );
    let rgb_only_plan = RgbLevelsPlan::new(
        RgbLevelsConfig::new(rgb_only).expect("RGB-only levels"),
        None,
    )
    .expect("RGB-only plan");
    let output = rgb_only_plan
        .execute(&[RgbLevelsPixel::new(0.25, 0.5, 0.75, f32::NAN)])
        .expect("direct RGB branch leaves lane three unread");
    assert_eq!(output[0].channels()[3].to_bits(), 0.0_f32.to_bits());

    let linked_plan = RgbLevelsPlan::new(RgbLevelsConfig::defaults(), None).expect("linked plan");
    let output = linked_plan
        .execute(&[RgbLevelsPixel::new(1.0, 0.5, 0.25, f32::NAN)])
        .expect("linked ratio branch leaves SIMD spare lane unread");
    assert!(
        output[0].channels()[..3]
            .iter()
            .copied()
            .all(f32::is_finite)
    );
    assert_eq!(output[0].channels()[3].to_bits(), 0.0_f32.to_bits());
}

#[test]
fn explicit_profile_reproduces_matrix_luminance_and_rejects_bad_trc_evidence() {
    let profile =
        RgbLevelsProfileEvidence::new_linear([[0.6, 0.1, 0.1], [0.2, 0.3, 0.5], [0.1, 0.2, 0.7]]);
    let rgb = [0.25, 0.5, 0.75];
    let expected = 0.2_f32 * rgb[0] + 0.3_f32 * rgb[1] + 0.5_f32 * rgb[2];
    assert_eq!(profile.luminance(rgb).to_bits(), expected.to_bits());
    let parameters = RgbLevelsParametersV1::new(
        RgbLevelsAutoscale::LinkedChannels,
        RgbLevelsPreserveColors::Luminance,
        RgbLevelsParametersV1::defaults().levels,
    );
    let plan = RgbLevelsPlan::new(
        RgbLevelsConfig::new(parameters).expect("levels"),
        Some(profile),
    )
    .expect("profile plan");
    assert!(plan.profile().is_some());

    assert_eq!(
        RgbLevelsProfileEvidence::new_with_trc(
            [[0.0; 3]; 3],
            [vec![0.0], vec![0.0], vec![0.0]],
            [[0.0; 3]; 3],
            1,
            true,
        ),
        Err(RgbLevelsProfileError::InvalidLut)
    );
    assert_eq!(
        RgbLevelsProfileEvidence::new_with_trc(
            [[0.0; 3]; 3],
            [vec![0.0, 1.0], vec![0.0, 1.0], vec![0.0, 1.0]],
            [[f32::NAN; 3]; 3],
            2,
            true,
        ),
        Err(RgbLevelsProfileError::NonFiniteCoefficients)
    );
}

#[test]
fn nonlinear_profile_trc_precedes_matrix_and_linked_cpu_luminance() {
    let profile_matrix = [[0.0, 0.0, 0.0], [0.25, 0.5, 0.25], [0.0, 0.0, 0.0]];
    let profile = RgbLevelsProfileEvidence::new_with_trc(
        profile_matrix,
        [
            vec![0.0, 0.5, 1.0],
            vec![0.0, 0.5, 1.0],
            vec![0.0, 0.5, 1.0],
        ],
        [[1.0; 3]; 3],
        3,
        true,
    )
    .expect("valid nonlinear profile");
    let rgb = [0.25_f32, 0.5, 1.5];
    let expected_luminance = 0.25_f32 * 0.25 + 0.5 * 0.5 + 0.25 * 1.5;
    assert_eq!(
        profile.luminance(rgb).to_bits(),
        expected_luminance.to_bits()
    );

    let parameters = RgbLevelsParametersV1::new(
        RgbLevelsAutoscale::LinkedChannels,
        RgbLevelsPreserveColors::Luminance,
        [[0.1, 0.5, 0.9]; 3],
    );
    let plan = RgbLevelsPlan::new(
        RgbLevelsConfig::new(parameters).expect("levels"),
        Some(profile),
    )
    .expect("profile plan");
    let output = plan
        .execute(&[RgbLevelsPixel::new(rgb[0], rgb[1], rgb[2], 0.8)])
        .expect("profile CPU output");
    let percentage = (expected_luminance - 0.1) / (0.9 - 0.1);
    let curve_luminance = plan.lut(0).expect("red LUT")[(percentage * 65_536.0_f32) as usize];
    let ratio = curve_luminance / expected_luminance;
    assert_eq!(
        output[0].channels().map(f32::to_bits),
        [
            (ratio * rgb[0]).to_bits(),
            (ratio * rgb[1]).to_bits(),
            (ratio * rgb[2]).to_bits(),
            0.0_f32.to_bits(),
        ]
    );
}

#[test]
fn cancellation_is_fail_closed_and_required_format_copies_through_first() {
    let plan = RgbLevelsPlan::new(RgbLevelsConfig::defaults(), None).expect("default plan");
    let input = vec![
        RgbLevelsPixel::new(0.1, 0.2, 0.3, 0.1),
        RgbLevelsPixel::new(0.2, 0.3, 0.4, 0.2),
        RgbLevelsPixel::new(0.3, 0.4, 0.5, 0.3),
        RgbLevelsPixel::new(0.4, 0.5, 0.6, 0.4),
    ];
    let mut calls = 0;
    assert_eq!(
        plan.execute_with_cancel(&input, || {
            calls += 1;
            calls > 2
        }),
        Err(RgbLevelsExecutionError::Cancelled)
    );

    let mut format_calls = 0;
    let copied = plan
        .execute_required_format_with_cancel(&input, false, || {
            format_calls += 1;
            true
        })
        .expect("format copy-through");
    assert!(copied.input_format_problem);
    assert_eq!(copied.pixels, input);
    assert_eq!(format_calls, 0);
}

#[test]
fn capabilities_and_source_map_keep_unowned_surfaces_fail_closed() {
    let capabilities = rgblevels::capabilities();
    assert!(capabilities.cpu_supported);
    assert!(capabilities.profile_luminance_supported);
    assert!(capabilities.alpha_semantics_source_shaped);
    assert!(!capabilities.gpu_supported);
    assert!(!capabilities.gtk_supported);
    assert!(!capabilities.masks_consumed);
    assert!(capabilities.outer_blending_deferred);
    assert!(!capabilities.production_routing_deferred);
    assert_eq!(
        capabilities.require_gpu(),
        Err(RgbLevelsCapabilityError::GpuUnavailable)
    );
    assert_eq!(
        capabilities.require_gtk(),
        Err(RgbLevelsCapabilityError::GtkUnavailable)
    );
    assert_eq!(
        capabilities.require_masks(),
        Err(RgbLevelsCapabilityError::MasksUnavailable)
    );
    assert_eq!(capabilities.require_production_routing(), Ok(()));
    assert!(RGBLEVELS_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("DEFAULT_VISIBLE")
            && entry.rust_symbol.contains("DEFAULT_ENABLED")
            && entry.rust_symbol.contains("DEFAULT_VISIBLE")
            && entry.status == RgbLevelsPortStatus::Ported
    }));
    assert!(RGBLEVELS_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("_compute_lut") && entry.status == RgbLevelsPortStatus::Ported
    }));
    assert!(RGBLEVELS_SOURCE_MAP.iter().any(|entry| {
        entry.native_file.contains("rgblevels.cl")
            && entry.status == RgbLevelsPortStatus::ExplicitlyDeferred
    }));
    assert!(RGBLEVELS_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("work-profile")
            && entry.status == RgbLevelsPortStatus::ExistingDependency
    }));

    let architecture_map =
        include_str!("../../../architecture/rusttable-rgblevels-source-map.toml");
    assert!(architecture_map.contains("The current native payload is 44 bytes"));
    assert!(architecture_map.contains(
        "CMake DEFAULT_VISIBLE controls default listing visibility independently of activation"
    ));
    assert!(architecture_map.contains("parameter-abi"));
    assert!(architecture_map.contains("status = \"deferred\""));
    assert!(architecture_map.contains(
        "Shared architecture metadata now records the verified 44-byte v1 payload with a typed decoder"
    ));
}

#[test]
fn nonfinite_input_and_output_are_rejected_without_partial_publication() {
    let plan = RgbLevelsPlan::new(RgbLevelsConfig::defaults(), None).expect("default plan");
    assert_eq!(
        plan.execute(&[RgbLevelsPixel::new(f32::NAN, 0.0, 0.0, 1.0)]),
        Err(RgbLevelsExecutionError::NonFiniteInput {
            pixel: 0,
            channel: 0,
        })
    );
}

fn fixture_value(fixture: &str, key: &str) -> String {
    fixture
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .expect("fixture key")
        .to_owned()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        write!(&mut result, "{byte:02x}").expect("writing to String");
    }
    result
}
