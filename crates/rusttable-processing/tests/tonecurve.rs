//! Source-derived bounded Tone Curve CPU leaf coverage.
//!
//! Production registry, typed history, and CPU pixelpipe dispatch are routed;
//! GPU, GTK, masks/outer blending, and preset integration remain deferred seams.

#![allow(
    clippy::assertions_on_constants,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::excessive_precision,
    clippy::field_reassign_with_default,
    clippy::float_cmp,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::suboptimal_flops,
    clippy::too_many_lines,
    clippy::unreadable_literal,
    reason = "source-derived vectors intentionally assert f32 bit patterns and fixed arrays"
)]

#[path = "../src/operations/tonecurve/mod.rs"]
mod tonecurve;

use rusttable_processing::common::curve_tools::CurveAnchor;
use tonecurve::source_map::RESPONSIBILITIES;
use tonecurve::{
    ALLOW_TILING, CHANNELS, DEFAULT_V5_FIXTURE, GPU_SUPPORTED, GTK_SUPPORTED, LUT_RESOLUTION,
    MAX_NODES, PARAMETER_BYTES, PARAMETER_VERSION, PROFILE_MATRIX_ORIENTATION, PreserveColors,
    SUPPORTS_BLENDING, ToneCurveAutoscale, ToneCurveCodecError, ToneCurveExecutionError,
    ToneCurveHistory, ToneCurveNode, ToneCurveParametersV5, ToneCurvePixel, ToneCurvePlan,
    ToneCurveProfileEvidence, ToneCurveTile, ToneCurveType, abi_offsets, capabilities,
    compile_parameters,
};

const PRODUCER_FIXTURE: &str = include_str!("fixtures/tonecurve/producers.txt");

const fn node(x: f32, y: f32) -> ToneCurveNode {
    ToneCurveNode::new(x, y)
}

fn params_with_l_curve(curve: [ToneCurveNode; 2]) -> ToneCurveParametersV5 {
    let mut parameters = ToneCurveParametersV5::default();
    parameters.tonecurve_nodes = [2, 2, 2];
    for channel in 0..CHANNELS {
        parameters.tonecurve[channel][0] = curve[0];
        parameters.tonecurve[channel][1] = curve[1];
    }
    parameters
}

fn linear_profile() -> ToneCurveProfileEvidence {
    ToneCurveProfileEvidence::prophoto()
}

fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[test]
fn native_v5_abi_defaults_and_inactive_bit_patterns_are_exact() {
    assert_eq!(PARAMETER_VERSION, 5);
    assert_eq!(PARAMETER_BYTES, 520);
    assert_eq!(abi_offsets(), [0, 480, 492, 504, 508, 512, 516]);
    assert_eq!(LUT_RESOLUTION, 65_536);
    assert_eq!(MAX_NODES, 20);
    assert!(DEFAULT_V5_FIXTURE.contains("payload_bytes=520"));
    assert!(DEFAULT_V5_FIXTURE.contains("node_counts=[2,3,3]"));
    assert!(DEFAULT_V5_FIXTURE.contains("preserve_colors=3"));

    let defaults = ToneCurveParametersV5::default();
    assert_eq!(defaults.tonecurve_nodes, [2, 3, 3]);
    assert_eq!(defaults.tonecurve_type, [ToneCurveType::MonotoneHermite; 3]);
    assert_eq!(
        defaults.tonecurve_autoscale_ab,
        ToneCurveAutoscale::AutomaticRgb
    );
    assert!(defaults.tonecurve_unbound_ab);
    assert_eq!(defaults.preserve_colors, PreserveColors::Average);
    let bytes = defaults.to_bytes();
    assert_eq!(
        &bytes[0..16],
        &[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 128, 63, 0, 0, 128, 63]
    );
    assert_eq!(&bytes[480..492], &[2, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0]);
    assert_eq!(&bytes[492..504], &[2, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(
        &bytes[504..520],
        &[3, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0]
    );

    let mut with_tail = defaults;
    with_tail.tonecurve[2][19] = node(f32::from_bits(0x7fc1_2345), f32::from_bits(0x8000_0001));
    let decoded = ToneCurveParametersV5::from_bytes(&with_tail.to_bytes()).unwrap();
    assert_eq!(decoded.tonecurve[2][19].x.to_bits(), 0x7fc1_2345);
    assert_eq!(decoded.tonecurve[2][19].y.to_bits(), 0x8000_0001);
}

#[test]
fn legacy_v1_v3_v4_migrations_match_native_defaults_and_failures() {
    let mut v1 = [0_u8; 52];
    let x = [0.0, 0.1, 0.2, 0.45, 0.8, 1.0];
    let y = [0.0, 0.04, 0.25, 0.55, 0.9, 1.0];
    for index in 0..6 {
        put_f32(&mut v1, index * 4, x[index]);
        put_f32(&mut v1, 24 + index * 4, y[index]);
    }
    put_i32(&mut v1, 48, 17);
    let migrated = ToneCurveHistory::decode(1, &v1).unwrap();
    let parameters = migrated.current();
    assert_eq!(parameters.tonecurve_nodes, [6, 3, 3]);
    assert_eq!(parameters.tonecurve_type[0], ToneCurveType::CubicSpline);
    assert_eq!(
        parameters.tonecurve_autoscale_ab,
        ToneCurveAutoscale::AutomaticLab
    );
    assert_eq!(parameters.tonecurve_preset, 17);
    assert!(!parameters.tonecurve_unbound_ab);
    assert_eq!(parameters.preserve_colors, PreserveColors::None);
    assert_eq!(parameters.tonecurve[0][4].y.to_bits(), y[4].to_bits());

    let source = ToneCurveParametersV5::default().to_bytes();
    let v3 = ToneCurveHistory::decode(3, &source[..512]).unwrap();
    assert!(!v3.current().tonecurve_unbound_ab);
    assert_eq!(v3.current().preserve_colors, PreserveColors::None);
    assert_eq!(
        v3.current().tonecurve_autoscale_ab,
        ToneCurveAutoscale::AutomaticRgb
    );

    let v4 = ToneCurveHistory::decode(4, &source[..516]).unwrap();
    assert!(v4.current().tonecurve_unbound_ab);
    assert_eq!(v4.current().preserve_colors, PreserveColors::None);
    assert_eq!(
        ToneCurveHistory::migration_edges(),
        &[(1, 5), (3, 5), (4, 5)]
    );
    assert_eq!(
        ToneCurveHistory::decode(2, &[]),
        Err(ToneCurveCodecError::UnsupportedVersion(2))
    );
    assert_eq!(
        ToneCurveHistory::decode(6, &[]),
        Err(ToneCurveCodecError::UnsupportedVersion(6))
    );
    assert_eq!(
        ToneCurveHistory::decode(5, &[0; 519]).unwrap_err(),
        ToneCurveCodecError::InvalidLength {
            expected: 520,
            actual: 519,
        }
    );
}

#[test]
fn legacy_producer_fixture_records_chart_v4_and_lightroom_v3_payloads() {
    assert!(PRODUCER_FIXTURE.contains("producer=src/chart/main.c"));
    assert!(PRODUCER_FIXTURE.contains("function=encode_tonecurve"));
    assert!(PRODUCER_FIXTURE.contains("version=4\npayload_bytes=516"));
    assert!(PRODUCER_FIXTURE.contains("producer=src/develop/lightroom.c"));
    assert!(PRODUCER_FIXTURE.contains("version=3\npayload_bytes=512"));
}

#[test]
fn all_interpolators_match_source_golden_lut_values_after_lab_scaling() {
    let anchors = [
        CurveAnchor::new(0.0, 0.0),
        CurveAnchor::new(0.17, 0.31),
        CurveAnchor::new(0.53, 0.22),
        CurveAnchor::new(0.81, 0.93),
        CurveAnchor::new(1.0, 1.0),
    ];
    let indices = [1_024, 12_345, 34_567, 52_000, 65_535];
    let expected = [
        [
            0.037094116_f32,
            0.31993103,
            0.21664429,
            0.89949036,
            0.99998474,
        ],
        [
            0.030334473_f32,
            0.31532288,
            0.21759033,
            0.89945984,
            0.99998474,
        ],
        [
            0.030319214_f32,
            0.3157959,
            0.21847534,
            0.90774536,
            0.99998474,
        ],
    ];
    for (curve_index, curve_type) in [
        ToneCurveType::CubicSpline,
        ToneCurveType::CatmullRom,
        ToneCurveType::MonotoneHermite,
    ]
    .into_iter()
    .enumerate()
    {
        let mut parameters = ToneCurveParametersV5::default();
        parameters.tonecurve_autoscale_ab = ToneCurveAutoscale::ManualLab;
        parameters.tonecurve_nodes = [5; CHANNELS];
        parameters.tonecurve_type = [curve_type; CHANNELS];
        for channel in 0..CHANNELS {
            for (index, anchor) in anchors.iter().enumerate() {
                parameters.tonecurve[channel][index] = node(anchor.x(), anchor.y());
            }
        }
        let compiled = compile_parameters(&parameters, None).unwrap();
        for (index, expected) in indices.into_iter().zip(expected[curve_index]) {
            assert_eq!(
                compiled.channel(0).table()[index].to_bits(),
                (expected * 100.0).to_bits()
            );
        }
    }
}

#[test]
fn runtime_quantization_uses_truncation_and_reciprocal_fit_threshold() {
    let mut identity_parameters = params_with_l_curve([node(0.0, 0.0), node(1.0, 1.0)]);
    identity_parameters.tonecurve_autoscale_ab = ToneCurveAutoscale::ManualLab;
    let identity = compile_parameters(&identity_parameters, None).unwrap();
    assert_eq!(
        identity.channel(0).table()[65_535].to_bits(),
        ((65_535.0_f32 / 65_536.0_f32) * 100.0_f32).to_bits()
    );
    let identity_curve = identity.channel(0);
    let identity_at_one = identity_curve.coefficients()[1]
        * (identity_curve.coefficients()[0] * 1.0).powf(identity_curve.coefficients()[2]);
    assert_eq!(
        identity_curve.evaluate(1.0).to_bits(),
        identity_at_one.to_bits()
    );

    let mut parameters = params_with_l_curve([node(0.0, 0.0), node(0.8, 0.8)]);
    parameters.tonecurve_type = [ToneCurveType::CatmullRom; CHANNELS];
    let compiled = compile_parameters(&parameters, None).unwrap();
    let curve = compiled.channel(0);
    assert_eq!(
        curve.extrapolation_threshold().to_bits(),
        (1.0 / curve.coefficients()[0]).to_bits()
    );
    let expected =
        curve.coefficients()[1] * (curve.coefficients()[0] * 1.0).powf(curve.coefficients()[2]);
    assert_eq!(curve.evaluate(1.0).to_bits(), expected.to_bits());
}

#[test]
fn xyz_and_rgb_autoscale_replace_l_table_before_fitting() {
    let parameters = params_with_l_curve([node(0.0, 0.0), node(1.0, 0.5)]);
    let mut xyz = parameters.clone();
    xyz.tonecurve_autoscale_ab = ToneCurveAutoscale::AutomaticXyz;
    let xyz_compiled = compile_parameters(&xyz, None).unwrap();
    assert!(
        xyz_compiled
            .channel(0)
            .table()
            .iter()
            .all(|value| value.is_finite())
    );
    assert_ne!(
        xyz_compiled.channel(0).table()[32_768].to_bits(),
        50.0_f32.to_bits()
    );

    let mut rgb = parameters;
    rgb.tonecurve_autoscale_ab = ToneCurveAutoscale::AutomaticRgb;
    rgb.preserve_colors = PreserveColors::Average;
    let rgb_compiled = compile_parameters(&rgb, None).unwrap();
    assert!(
        rgb_compiled
            .channel(0)
            .table()
            .iter()
            .all(|value| value.is_finite())
    );
    assert!(
        rgb_compiled
            .channel(0)
            .coefficients()
            .iter()
            .all(|value| value.is_finite())
    );
}

#[test]
fn rgb_luminance_requires_profile_evidence_and_rejects_nonlinear_evidence() {
    let mut parameters = ToneCurveParametersV5::default();
    parameters.tonecurve_autoscale_ab = ToneCurveAutoscale::AutomaticRgb;
    parameters.preserve_colors = PreserveColors::Luminance;
    assert!(compile_parameters(&parameters, None).is_ok());
    assert_eq!(
        ToneCurvePlan::new(parameters, None).unwrap_err(),
        ToneCurveExecutionError::Curve(tonecurve::CurveCompileError::MissingProfileEvidence)
    );
    assert_eq!(
        PROFILE_MATRIX_ORIENTATION,
        "matrix_in is row-major (non-transposed) and its row 1 supplies ProPhoto Y"
    );

    let profile = ToneCurveProfileEvidence::prophoto();
    let rgb = [0.2_f32, 0.7, 0.1];
    let expected = 0.2880402_f32 * rgb[0] + 0.7118741_f32 * rgb[1] + 0.0000857_f32 * rgb[2];
    assert_eq!(profile.luminance(rgb).to_bits(), expected.to_bits());
    assert_ne!(
        profile.luminance(rgb).to_bits(),
        ((rgb[0] + rgb[1] + rgb[2]) / 3.0).to_bits()
    );

    let nonlinear = ToneCurveProfileEvidence::new_with_trc(
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        std::array::from_fn(|_| vec![0.0, 0.1, 1.0]),
        [[1.0, 1.0, 1.0]; 3],
        3,
        true,
    );
    assert!(nonlinear.is_err());
}

#[test]
fn prophoto_profile_evidence_drives_non_neutral_rgb_luminance_execution() {
    let mut parameters = params_with_l_curve([node(0.0, 0.0), node(1.0, 0.5)]);
    parameters.tonecurve_autoscale_ab = ToneCurveAutoscale::AutomaticRgb;
    parameters.preserve_colors = PreserveColors::Luminance;
    let profile = ToneCurveProfileEvidence::prophoto();
    let input = ToneCurvePixel::new(50.0, 20.0, -10.0, 0.73);
    let output = ToneCurvePlan::new(parameters, Some(profile))
        .expect("ProPhoto profile plan")
        .execute_with_cancel(&[input], || false)
        .expect("ProPhoto profile execution")
        .pixels[0]
        .channels();
    assert_ne!(output[..3], input.channels()[..3]);
    assert_eq!(output[3].to_bits(), 0.73_f32.to_bits());
}

#[test]
fn low_light_equality_uses_the_low_approximation_entry() {
    let parameters = params_with_l_curve([node(0.0, 0.0), node(1.0, 0.5)]);
    let mut parameters = parameters;
    parameters.tonecurve_autoscale_ab = ToneCurveAutoscale::AutomaticLab;
    let plan = ToneCurvePlan::new(parameters, None).unwrap();
    let input = ToneCurvePixel::new(1.0, 20.0, -30.0, 0.5);
    let output = plan.execute_with_cancel(&[input], || false).unwrap().pixels[0].channels();
    let expected = 20.0_f32 * plan.curves().low_approximation();
    assert_eq!(output[1].to_bits(), expected.to_bits());
    assert_eq!(
        output[2].to_bits(),
        (-30.0_f32 * plan.curves().low_approximation()).to_bits()
    );
}

#[test]
fn all_preserve_modes_and_all_autoscale_branches_keep_alpha_exact() {
    let alpha = f32::from_bits(0x7fc0_1234);
    let base = params_with_l_curve([node(0.0, 0.0), node(1.0, 0.75)]);
    let modes = [
        PreserveColors::None,
        PreserveColors::Luminance,
        PreserveColors::Max,
        PreserveColors::Average,
        PreserveColors::Sum,
        PreserveColors::Norm,
        PreserveColors::Power,
    ];

    for autoscale in [
        ToneCurveAutoscale::ManualLab,
        ToneCurveAutoscale::AutomaticLab,
        ToneCurveAutoscale::AutomaticXyz,
    ] {
        let mut parameters = base.clone();
        parameters.tonecurve_autoscale_ab = autoscale;
        let plan = ToneCurvePlan::new(parameters, None).unwrap();
        let output = plan
            .execute_with_cancel(&[ToneCurvePixel::new(50.0, 20.0, -10.0, alpha)], || false)
            .unwrap()
            .pixels[0]
            .channels();
        assert_eq!(output[3].to_bits(), alpha.to_bits(), "{autoscale:?}");
    }

    for mode in modes {
        let mut parameters = base.clone();
        parameters.tonecurve_autoscale_ab = ToneCurveAutoscale::AutomaticRgb;
        parameters.preserve_colors = mode;
        let profile = (mode == PreserveColors::Luminance).then(linear_profile);
        let plan = ToneCurvePlan::new(parameters, profile).unwrap();
        let output = plan
            .execute_with_cancel(&[ToneCurvePixel::new(50.0, 20.0, -10.0, alpha)], || false)
            .unwrap()
            .pixels[0]
            .channels();
        assert_eq!(output[3].to_bits(), alpha.to_bits(), "{mode:?}");
        assert!(
            output[..3].iter().all(|value| value.is_finite()),
            "{mode:?}"
        );
    }
}

#[test]
fn nonfinite_alpha_is_preserved_while_rgb_still_runs() {
    let alpha = f32::from_bits(0x7fc0_1234);
    let parameters = params_with_l_curve([node(0.0, 0.0), node(1.0, 0.75)]);
    let mut parameters = parameters;
    parameters.tonecurve_autoscale_ab = ToneCurveAutoscale::ManualLab;
    let plan = ToneCurvePlan::new(parameters, None).unwrap();
    let finite = plan
        .execute_with_cancel(&[ToneCurvePixel::new(50.0, 20.0, -10.0, 1.0)], || false)
        .unwrap()
        .pixels[0]
        .channels();
    let nonfinite = plan
        .execute_with_cancel(&[ToneCurvePixel::new(50.0, 20.0, -10.0, alpha)], || false)
        .unwrap()
        .pixels[0]
        .channels();
    assert_eq!(nonfinite[..3], finite[..3]);
    assert_eq!(nonfinite[3].to_bits(), alpha.to_bits());
    assert_ne!(nonfinite[0].to_bits(), 50.0_f32.to_bits());
}

#[test]
fn cancellation_required_format_copy_through_and_tiles_never_publish_partial_results() {
    let plan = ToneCurvePlan::new(ToneCurveParametersV5::default(), None).unwrap();
    let input: Vec<_> = (0..12)
        .map(|index| ToneCurvePixel::new(index as f32, 0.2, 0.3, f32::from_bits(0x8000_0001)))
        .collect();
    let full = plan.execute_with_cancel(&input, || false).unwrap();
    let tiles = [
        ToneCurveTile::new(0, 0, 2, 2),
        ToneCurveTile::new(2, 0, 2, 2),
        ToneCurveTile::new(0, 2, 2, 1),
        ToneCurveTile::new(2, 2, 2, 1),
    ];
    assert_eq!(
        plan.execute_tiles_with_cancel(&input, 4, 3, &tiles, true, || false)
            .unwrap(),
        full
    );
    let copied = plan
        .execute_required_format_with_cancel(&input, false, || false)
        .unwrap();
    assert!(copied.input_format_problem);
    assert_eq!(copied.pixels, input);
    let copied_before_cancel_or_tile_validation = plan
        .execute_tiles_with_cancel(&input, 0, 0, &[], false, || {
            panic!("format failure must copy through before cancellation")
        })
        .unwrap();
    assert!(copied_before_cancel_or_tile_validation.input_format_problem);
    assert_eq!(copied_before_cancel_or_tile_validation.pixels, input);
    assert_eq!(
        plan.execute_tiles_with_cancel(&input, 4, 3, &tiles[..3], true, || false),
        Err(ToneCurveExecutionError::IncompleteTiles)
    );
    assert_eq!(
        plan.execute_tiles_with_cancel(
            &input,
            4,
            3,
            &[
                ToneCurveTile::new(0, 0, 3, 2),
                ToneCurveTile::new(2, 1, 2, 2)
            ],
            true,
            || false,
        ),
        Err(ToneCurveExecutionError::OverlappingTiles)
    );
    let mut checks = 0;
    assert_eq!(
        plan.execute_with_cancel(&input, || {
            checks += 1;
            checks > 2
        })
        .unwrap_err(),
        ToneCurveExecutionError::Cancelled
    );
    assert!(ALLOW_TILING && SUPPORTS_BLENDING);
}

#[test]
fn runtime_commit_order_and_deferred_surfaces_remain_explicit() {
    let defaults = ToneCurveParametersV5::default();
    let mut runtime = tonecurve::ToneCurveRuntime::new(defaults.clone());
    assert!(!runtime.lut_is_built());
    assert_eq!(
        runtime.initial_table_value(0, 0).to_bits(),
        0.0_f32.to_bits()
    );
    assert_eq!(
        runtime.initial_table_value(0, 1).to_bits(),
        (100.0_f32 / 65_536.0_f32).to_bits()
    );
    assert_eq!(
        runtime.initial_table_value(1, 1).to_bits(),
        (256.0_f32 / 65_536.0_f32 - 128.0_f32).to_bits()
    );
    assert_eq!(
        runtime.initial_table_value(2, 65_535).to_bits(),
        (256.0_f32 * 65_535.0_f32 / 65_536.0_f32 - 128.0_f32).to_bits()
    );
    let mut changed = defaults.clone();
    changed.tonecurve_type = [ToneCurveType::CubicSpline; CHANNELS];
    runtime.commit_params(changed, true);
    assert!(runtime.request_histogram());
    let _ = runtime.plan(None).unwrap();
    assert!(runtime.lut_is_built());
    runtime.commit_params(defaults, false);
    assert!(!runtime.request_histogram());
    assert!(!runtime.lut_is_built());

    let caps = capabilities();
    assert!(caps.cpu_supported);
    assert!(!GPU_SUPPORTED && !GTK_SUPPORTED);
    assert!(caps.rgb_luminance_requires_profile_evidence);
    assert!(caps.runtime_mask_coverage_consumed);
    assert!(caps.runtime_opacity_consumed);
    assert!(caps.imported_native_blend_mask_deferred);
    assert!(RESPONSIBILITIES.iter().any(|entry| {
        entry.native_symbol == "process_cl" && entry.status.contains("unavailable")
    }));
    assert!(RESPONSIBILITIES.iter().any(|entry| {
        entry.native_symbol == "runtime mask coverage / operation opacity"
            && entry.status.contains("implemented")
    }));
    assert!(RESPONSIBILITIES.iter().any(|entry| {
        entry.native_symbol == "imported native blend/mask payloads"
            && entry.status.contains("opaque")
            && entry.status.contains("deferred")
    }));
    assert!(RESPONSIBILITIES.iter().any(|entry| {
        entry.native_symbol == "gui_init/gui_changed" && entry.status.contains("deferred")
    }));
}
