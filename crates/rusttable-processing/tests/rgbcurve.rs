//! Source-derived RGB Curve leaf coverage. The operation is intentionally
//! included by path because registry, pixelpipe, app, and GTK integration are
//! separate migration lanes.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::field_reassign_with_default,
    clippy::assertions_on_constants,
    clippy::similar_names,
    clippy::needless_range_loop,
    clippy::unreadable_literal,
    clippy::too_many_lines,
    reason = "source-derived vectors intentionally use direct f32 and fixed-array assertions"
)]

#[path = "../src/operations/rgbcurve/mod.rs"]
mod rgbcurve;

use rgbcurve::source_map::RESPONSIBILITIES;
use rgbcurve::{
    ALLOW_TILING, CHANNELS, CompiledCurve, DEFAULT_V1_FIXTURE, EditorError, LUT_RESOLUTION,
    MAX_NODES, MIN_X_DISTANCE, PARAMETER_BYTES, PARAMETER_VERSION, PROFILE_MATRIX_ORIENTATION,
    PreserveColors, RgbCurveAutoscale, RgbCurveChannel, RgbCurveEditorState,
    RgbCurveExecutionError, RgbCurveHistory, RgbCurveNode, RgbCurveParametersV1, RgbCurvePixel,
    RgbCurvePlan, RgbCurvePresetBlendColorspace, RgbCurveProfileEvidence, RgbCurveRuntime,
    RgbCurveTile, RgbCurveType, SUPPORTS_BLENDING, abi_offsets, capabilities, compile_parameters,
    init_presets, native_gpu_extrapolation_mismatch,
};
use rusttable_processing::common::curve_tools::CurveAnchor;

const TEST_FIXTURE: &str = include_str!("fixtures/rgbcurve/default_v1.txt");

const fn node(x: f32, y: f32) -> RgbCurveNode {
    RgbCurveNode::new(x, y)
}

fn params_with_curve(curve: [RgbCurveNode; 2]) -> RgbCurveParametersV1 {
    let mut parameters = RgbCurveParametersV1::default();
    for channel in 0..CHANNELS {
        parameters.curve_nodes[channel][0] = curve[0];
        parameters.curve_nodes[channel][1] = curve[1];
    }
    parameters
}

const fn pixel(channels: [f32; 4]) -> RgbCurvePixel {
    RgbCurvePixel::from_channels(channels)
}

fn linear_profile() -> RgbCurveProfileEvidence {
    RgbCurveProfileEvidence::new_linear(
        1,
        b"linear.icc".to_vec(),
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
    )
}

#[test]
fn native_abi_offsets_default_fixture_and_inactive_tails_are_explicit() {
    assert_eq!(PARAMETER_BYTES, 516);
    assert_eq!(abi_offsets(), [0, 480, 492, 504, 508, 512]);
    for fixture in [DEFAULT_V1_FIXTURE, TEST_FIXTURE] {
        assert!(fixture.contains("version=1"));
        assert!(fixture.contains("payload_bytes=516"));
        assert!(fixture.contains("node_counts_offset=480"));
        assert!(fixture.contains("runtime_initial_table_value=0.0"));
    }

    let parameters = RgbCurveParametersV1::default();
    let bytes = parameters.to_bytes();
    assert_eq!(&bytes[0..8], &[0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(&bytes[8..16], &[0, 0, 128, 63, 0, 0, 128, 63]);
    assert_eq!(&bytes[480..492], &[2, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(&bytes[492..504], &[2, 0, 0, 0, 2, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(&bytes[504..516], &[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);

    let mut retained = parameters;
    retained.curve_nodes[2][19] = node(f32::from_bits(0x7fc1_2345), f32::from_bits(0x8000_0001));
    let encoded = retained.to_bytes();
    let decoded = RgbCurveParametersV1::from_bytes(&encoded).expect("valid active values");
    assert_eq!(decoded.curve_nodes[2][19].x.to_bits(), 0x7fc1_2345);
    assert_eq!(decoded.curve_nodes[2][19].y.to_bits(), 0x8000_0001);
}

#[test]
fn history_rejects_malformed_v1_and_retains_zero_and_max_unknown_versions() {
    let defaults = RgbCurveParametersV1::default();
    assert_eq!(
        RgbCurveHistory::decode(PARAMETER_VERSION, &[0; 515])
            .unwrap_err()
            .to_string(),
        "RGB Curve payload has 515 bytes; expected 516"
    );
    let mut bytes = defaults.to_bytes();
    bytes[504..508].copy_from_slice(&2_i32.to_le_bytes());
    assert!(RgbCurveHistory::decode(PARAMETER_VERSION, &bytes).is_err());
    bytes = defaults.to_bytes();
    bytes[8..12].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(RgbCurveHistory::decode(PARAMETER_VERSION, &bytes).is_err());

    for version in [0, u16::MAX] {
        let opaque_bytes = vec![0x00, 0xff, 0x7a];
        let opaque =
            RgbCurveHistory::decode(version, &opaque_bytes).expect("unknown versions are opaque");
        assert_eq!(opaque.version(), version);
        assert_eq!(opaque.payload(), opaque_bytes);
        assert!(opaque.current().is_err());
    }
    assert!(RgbCurveHistory::migration_edges().is_empty());
}

#[test]
fn all_interpolators_and_twenty_anchor_capacity_compile_with_strict_x_order() {
    let mut parameters = RgbCurveParametersV1::default();
    parameters.curve_num_nodes = [MAX_NODES as u32; CHANNELS];
    for channel in 0..CHANNELS {
        for index in 0..MAX_NODES {
            let x = index as f32 / (MAX_NODES - 1) as f32;
            parameters.curve_nodes[channel][index] = node(x, x * x);
        }
    }
    for curve_type in [
        RgbCurveType::CubicSpline,
        RgbCurveType::CatmullRom,
        RgbCurveType::MonotoneHermite,
    ] {
        parameters.curve_type = [curve_type; CHANNELS];
        assert!(
            compile_parameters(&parameters, None).is_ok(),
            "{curve_type:?}"
        );
    }
    parameters.curve_nodes[0][1].x = parameters.curve_nodes[0][0].x;
    assert!(compile_parameters(&parameters, None).is_err());
    assert_eq!(LUT_RESOLUTION, 65_536);
}

#[test]
fn nontrivial_interpolators_match_source_golden_lut_values() {
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
            0.31993103_f32,
            0.21664429_f32,
            0.89949036_f32,
            0.99998474_f32,
        ],
        [
            0.030334473_f32,
            0.31532288_f32,
            0.21759033_f32,
            0.89945984_f32,
            0.99998474_f32,
        ],
        [
            0.030319214_f32,
            0.3157959_f32,
            0.21847534_f32,
            0.90774536_f32,
            0.99998474_f32,
        ],
    ];
    for (curve_index, curve_type) in [
        RgbCurveType::CubicSpline,
        RgbCurveType::CatmullRom,
        RgbCurveType::MonotoneHermite,
    ]
    .into_iter()
    .enumerate()
    {
        let curve = CompiledCurve::from_nodes(&anchors, curve_type).expect("source-valid curve");
        for (index, expected) in indices.into_iter().zip(expected[curve_index]) {
            assert_eq!(
                curve.table()[index].to_bits(),
                expected.to_bits(),
                "{curve_type:?} at {index}"
            );
        }
    }
}

#[test]
fn runtime_quantizes_below_and_above_final_anchor_and_uses_reciprocal_threshold() {
    let identity = CompiledCurve::from_nodes(
        &[CurveAnchor::new(0.0, 0.0), CurveAnchor::new(1.0, 1.0)],
        RgbCurveType::CatmullRom,
    )
    .expect("identity curve");
    assert_eq!(
        identity.table()[65_535].to_bits(),
        (65_535.0_f32 / 65_536.0).to_bits()
    );

    let mut parameters = params_with_curve([node(0.0, 0.0), node(0.8, 0.8)]);
    parameters.curve_type = [RgbCurveType::CatmullRom; CHANNELS];
    let set = compile_parameters(&parameters, None).expect("movable endpoint");
    let curve = set.channel(0);
    let coefficients = curve.coefficients();
    let expected = coefficients[1] * (coefficients[0] * 1.0).powf(coefficients[2]);
    assert_eq!(curve.evaluate(1.0).to_bits(), expected.to_bits());

    let gpu_input = 65_535.0_f32 / 65_536.0;
    let cpu = curve.evaluate(gpu_input);
    let gpu_like = curve.table()[0xffff];
    assert_ne!(cpu.to_bits(), gpu_like.to_bits());
    assert_eq!(
        native_gpu_extrapolation_mismatch(),
        "CPU branches at input < 1.0 / coeffs[0]; OpenCL lookup_unbounded branches at input < 1.0"
    );

    let small_final = f32::from_bits(0x2ffa_76b3);
    let small = CompiledCurve::from_nodes(
        &[
            CurveAnchor::new(0.0, 0.0),
            CurveAnchor::new(small_final, small_final),
        ],
        RgbCurveType::CatmullRom,
    )
    .expect("small final anchor");
    assert_eq!(
        small.extrapolation_threshold().to_bits(),
        (1.0 / small.coefficients()[0]).to_bits()
    );
    assert_ne!(
        small.extrapolation_threshold().to_bits(),
        small.final_x().to_bits()
    );
}

#[test]
fn profile_matrices_are_row_major_and_nonlinear_trc_is_independent() {
    let matrix_in = [[1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]];
    let profile =
        RgbCurveProfileEvidence::new_linear(4, b"matrix.icc".to_vec(), matrix_in, matrix_in);
    assert_eq!(
        PROFILE_MATRIX_ORIENTATION,
        "matrix_in and matrix_out are row-major (non-transposed) 3x3 matrices"
    );
    assert_eq!(
        profile.luminance([1.0, 2.0, 3.0]).to_bits(),
        32.0_f32.to_bits()
    );

    let lut = std::array::from_fn(|_| vec![0.0, 0.1, 1.0]);
    let coeffs = [[1.0, 1.0, 1.0]; 3];
    let nonlinear = RgbCurveProfileEvidence::new_with_trc(
        5,
        b"trc.icc".to_vec(),
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        lut.clone(),
        lut.clone(),
        coeffs,
        coeffs,
        true,
    )
    .expect("nonlinear profile");
    let linear = RgbCurveProfileEvidence::new_with_trc(
        5,
        b"trc.icc".to_vec(),
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        lut,
        std::array::from_fn(|_| vec![0.0, 0.1, 1.0]),
        coeffs,
        coeffs,
        false,
    )
    .expect("linear profile with independent flag");
    assert!(nonlinear.luminance([0.5, 0.5, 0.5]) < linear.luminance([0.5, 0.5, 0.5]));
}

#[test]
fn profile_trc_selection_uses_lut_marker_not_extrapolation_coefficients() {
    let identity = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let disabled_lut = std::array::from_fn(|_| vec![-1.0, 0.1, 1.0]);
    let positive_coefficients = [[1.0, 1.0, 1.0]; CHANNELS];
    let disabled = RgbCurveProfileEvidence::new_with_trc(
        6,
        b"marker-disabled.icc".to_vec(),
        identity,
        identity,
        disabled_lut.clone(),
        disabled_lut,
        positive_coefficients,
        positive_coefficients,
        true,
    )
    .expect("valid marker-disabled profile");
    assert_eq!(
        disabled.luminance([0.5, 0.5, 0.5]).to_bits(),
        0.5_f32.to_bits()
    );

    let enabled_lut = std::array::from_fn(|_| vec![0.0, 0.1, 1.0]);
    let negative_coefficients = [[-1.0, 1.0, 1.0]; CHANNELS];
    let enabled = RgbCurveProfileEvidence::new_with_trc(
        7,
        b"marker-enabled.icc".to_vec(),
        identity,
        identity,
        enabled_lut.clone(),
        enabled_lut,
        negative_coefficients,
        negative_coefficients,
        true,
    )
    .expect("valid marker-enabled profile");
    assert_ne!(
        enabled.luminance([0.5, 0.5, 0.5]).to_bits(),
        0.5_f32.to_bits()
    );
}

#[test]
fn middle_grey_profile_conversion_preserves_native_f32_operation_order() {
    let value = f32::from_bits(0x3c11_1aa7);
    let profile = linear_profile();
    assert_eq!(profile.compensate_middle_grey(value).to_bits(), 0x3da3_d70a);
}

#[test]
fn middle_grey_uses_profile_when_present_and_native_raw_fallback_when_absent() {
    let mut parameters = RgbCurveParametersV1::default();
    parameters.compensate_middle_grey = true;
    let raw = compile_parameters(&parameters, None).expect("native no-profile raw nodes");
    let uncompensated = compile_parameters(
        &RgbCurveParametersV1 {
            compensate_middle_grey: false,
            ..parameters
        },
        None,
    )
    .expect("raw nodes");
    assert_eq!(raw, uncompensated);
    assert!(compile_parameters(&parameters, Some(&linear_profile())).is_ok());
    assert!(!capabilities().middle_grey_requires_profile_evidence);
}

#[test]
fn automatic_and_manual_modes_preserve_alpha_for_every_color_norm() {
    let alpha = f32::from_bits(0x7fc0_1234);
    let mut parameters = params_with_curve([node(0.0, 0.0), node(1.0, 0.5)]);
    parameters.curve_autoscale = RgbCurveAutoscale::ManualRgb;
    let plan = RgbCurvePlan::new(parameters.clone(), None).expect("manual plan");
    let input = [pixel([0.25, 0.5, 1.5, alpha])];
    let output = plan
        .execute_with_cancel(&input, || false)
        .expect("manual output");
    assert_eq!(output.pixels[0].channels()[3].to_bits(), alpha.to_bits());
    assert!(output.pixels[0].channels()[0] < output.pixels[0].channels()[1]);

    parameters.curve_autoscale = RgbCurveAutoscale::AutomaticRgb;
    for mode in [
        PreserveColors::None,
        PreserveColors::Luminance,
        PreserveColors::Max,
        PreserveColors::Average,
        PreserveColors::Sum,
        PreserveColors::Norm,
        PreserveColors::Power,
    ] {
        parameters.preserve_colors = mode;
        let plan = RgbCurvePlan::new(parameters.clone(), None).expect("automatic plan");
        let output = plan
            .execute_with_cancel(&[pixel([0.2, 0.4, 0.6, alpha])], || false)
            .expect("automatic output");
        assert_eq!(
            output.pixels[0].channels()[3].to_bits(),
            alpha.to_bits(),
            "{mode:?}"
        );
        assert!(
            output.pixels[0].channels()[..3]
                .iter()
                .all(|value| value.is_finite()),
            "{mode:?}"
        );
    }
    let mut zero = parameters;
    zero.preserve_colors = PreserveColors::Power;
    let plan = RgbCurvePlan::new(zero, None).expect("zero norm plan");
    let output = plan
        .execute_with_cancel(&[pixel([0.0, 0.0, 0.0, alpha])], || false)
        .unwrap();
    let zero_channels = output.pixels[0].channels();
    assert_eq!(&zero_channels[..3], &[0.0, 0.0, 0.0]);
    assert_eq!(zero_channels[3].to_bits(), alpha.to_bits());
}

#[test]
fn cancellation_copy_through_and_tile_validation_publish_only_complete_results() {
    let plan = RgbCurvePlan::new(RgbCurveParametersV1::default(), None).expect("identity plan");
    let input: Vec<_> = (0..12)
        .map(|index| pixel([index as f32 / 12.0, 0.2, 0.3, 0.4]))
        .collect();
    let full = plan.execute_with_cancel(&input, || false).unwrap();
    let tiles = [
        RgbCurveTile::new(0, 0, 2, 2),
        RgbCurveTile::new(2, 0, 2, 2),
        RgbCurveTile::new(0, 2, 2, 1),
        RgbCurveTile::new(2, 2, 2, 1),
    ];
    let tiled = plan
        .execute_tiles_with_cancel(&input, 4, 3, &tiles, true, || false)
        .unwrap();
    assert_eq!(full, tiled);
    let copied = plan
        .execute_required_format_with_cancel(&input, false, || false)
        .unwrap();
    assert!(copied.input_format_problem);
    assert_eq!(copied.pixels, input);
    assert_eq!(
        plan.execute_tiles_with_cancel(&input, 4, 3, &tiles[..3], true, || false),
        Err(RgbCurveExecutionError::IncompleteTiles)
    );
    assert_eq!(
        plan.execute_tiles_with_cancel(
            &input,
            4,
            3,
            &[RgbCurveTile::new(0, 0, 3, 2), RgbCurveTile::new(2, 1, 2, 2)],
            true,
            || false,
        ),
        Err(RgbCurveExecutionError::OverlappingTiles)
    );
    assert_eq!(
        plan.execute_tiles_with_cancel(
            &input,
            4,
            3,
            &[RgbCurveTile::new(0, 0, 4, 3)],
            true,
            || false,
        ),
        Ok(full)
    );
    assert_eq!(
        plan.execute_tiles_with_cancel(
            &input,
            4,
            3,
            &[RgbCurveTile::new(4, 0, 1, 1)],
            false,
            || false,
        ),
        Err(RgbCurveExecutionError::InvalidTile)
    );
    let mut checks = 0;
    assert_eq!(
        plan.execute_with_cancel(&input, || {
            checks += 1;
            checks > 2
        })
        .unwrap_err(),
        RgbCurveExecutionError::Cancelled
    );
    assert!(ALLOW_TILING && SUPPORTS_BLENDING);
}

#[test]
fn commit_state_tracks_preview_histogram_type_changes_and_profile_cache() {
    let defaults = RgbCurveParametersV1::default();
    let mut runtime = RgbCurveRuntime::new(defaults.clone());
    assert!(!runtime.lut_is_built());
    assert_eq!(runtime.initial_table_value(0, 65_535), 0.0);
    let mut changed = defaults.clone();
    changed.curve_type = [RgbCurveType::CubicSpline; CHANNELS];
    changed.compensate_middle_grey = true;
    runtime.commit_params(changed, true);
    assert_eq!(runtime.curve_changed(), [true; CHANNELS]);
    assert!(runtime.request_histogram());
    assert!(runtime.histogram_middle_grey());
    let _ = runtime.plan(Some(linear_profile())).unwrap();
    assert!(runtime.lut_is_built());
    assert_eq!(runtime.curve_changed(), [false; CHANNELS]);
    runtime.commit_params(defaults, false);
    assert!(!runtime.request_histogram());
    assert!(!runtime.lut_is_built());
}

#[test]
fn editor_state_matches_visibility_copy_reset_change_image_middle_grey_picker_and_sentinels() {
    let defaults = RgbCurveParametersV1::default();
    let mut editor = RgbCurveEditorState::new(defaults.clone());
    assert!(!editor.channel_tabs_visible());
    assert!(editor.preserve_colors_visible());
    assert_eq!(
        editor.select_channel(RgbCurveChannel::Green),
        Err(EditorError::ChannelUnavailable)
    );
    editor.add_node(0.5, 0.25).expect("red node");
    editor.set_autoscale(RgbCurveAutoscale::ManualRgb);
    assert!(editor.channel_tabs_visible());
    assert_eq!(
        editor.parameters().curve_nodes[1],
        editor.parameters().curve_nodes[0]
    );
    assert_eq!(editor.parameters().curve_num_nodes[1], 3);
    editor.select_channel(RgbCurveChannel::Green).unwrap();
    editor.select_nearest(0.5, 0.25).unwrap();
    editor.secondary_reset_or_delete().unwrap();
    assert_eq!(editor.selected(), -2);
    assert_eq!(editor.parameters().curve_num_nodes[1], 2);
    assert_eq!(editor.add_node(0.6, 0.4), Err(EditorError::SentinelActive));
    assert_eq!(
        editor.move_selected(0.1, 0.1),
        Err(EditorError::NoSelection)
    );
    editor.select_channel(RgbCurveChannel::Red).unwrap();
    editor.select_nearest(0.5, 0.25).unwrap();
    editor.move_selected(MIN_X_DISTANCE, 0.1).unwrap();
    editor.reset_curve().unwrap();
    assert_eq!(editor.parameters().curve_num_nodes, [2, 2, 3]);
    assert_eq!(editor.parameters().curve_type, defaults.curve_type);
    assert_eq!(
        &editor.parameters().curve_nodes[0][..2],
        &defaults.curve_nodes[0][..2]
    );
    assert_eq!(
        editor.parameters().curve_autoscale,
        RgbCurveAutoscale::ManualRgb
    );

    editor.select_channel(RgbCurveChannel::Blue).unwrap();
    editor.zoom_at(0.5, 0.5, -2.0).unwrap();
    editor.select_nearest(0.5, 0.5).unwrap();
    editor.change_image();
    assert_eq!(editor.channel(), RgbCurveChannel::Blue);
    assert_eq!(editor.zoom_factor(), 1.0);
    assert_eq!(editor.offsets(), (0.0, 0.0));
    assert_eq!(editor.selected(), -1);

    let profile = linear_profile();
    editor.set_compensate_middle_grey(true, &profile);
    assert!(editor.parameters().compensate_middle_grey);
    editor.set_compensate_middle_grey(false, &profile);
    assert!(!editor.parameters().compensate_middle_grey);
    editor.select_channel(RgbCurveChannel::Red).unwrap();
    editor.apply_picker_curve(0.1, 0.6, 0.9, 1).unwrap();
    assert_eq!(editor.parameters().curve_num_nodes[0], 6);
    assert!(editor.evaluate_gui(0.5).unwrap().is_finite());
    editor.reset_view();
    assert_eq!(editor.channel(), RgbCurveChannel::Red);
    assert_eq!(editor.zoom_factor(), 1.0);
    assert_eq!(editor.offsets(), (0.0, 0.0));
    assert_eq!(editor.selected(), -1);
}

#[test]
fn presets_metadata_and_source_map_keep_deferred_surfaces_explicit() {
    let presets = init_presets();
    assert_eq!(presets.len(), 10);
    assert_eq!(presets[0].name, "contrast | compression");
    assert_eq!(presets[0].localization_key, presets[0].name);
    assert!(presets.iter().all(|preset| preset.generic));
    assert!(
        presets
            .iter()
            .all(|preset| { preset.blend_colorspace == RgbCurvePresetBlendColorspace::RgbDisplay })
    );
    assert_eq!(presets[0].parameters.curve_num_nodes, [6, 7, 7]);
    assert_eq!(rgbcurve::OPERATION_NAME, "rgb curve");
    assert_eq!(rgbcurve::GPU_PROGRAM_INDEX, 25);
    assert_eq!(rgbcurve::GPU_KERNEL_NAME, "rgbcurve");
    assert!(capabilities().cpu_supported);
    assert!(!capabilities().gpu_supported);
    assert!(!capabilities().gtk_supported);
    assert!(capabilities().outer_blend_deferred);
    assert!(RESPONSIBILITIES.iter().any(|entry| {
        entry.native_symbol == "process_cl" && entry.status.contains("unavailable")
    }));
    assert!(
        RESPONSIBILITIES.iter().any(|entry| {
            entry.native_symbol == "gui_changed" && entry.status.contains("state")
        })
    );
    assert!(RESPONSIBILITIES.iter().any(|entry| {
        entry.native_symbol.contains("cleanup_pipe") && entry.status.contains("deferred")
    }));
}

#[test]
fn editor_rejects_invalid_node_spacing_without_mutating_state() {
    let mut editor = RgbCurveEditorState::new(RgbCurveParametersV1::default());
    assert_eq!(
        editor.add_node(0.001, 0.2),
        Err(EditorError::MinimumXDistance)
    );
}
