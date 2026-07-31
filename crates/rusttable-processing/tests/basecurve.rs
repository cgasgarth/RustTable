//! Source-derived tests for the bounded Basecurve CPU leaf.

#![allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::unreadable_literal,
    reason = "tests compare native f32 boundaries and transcribe source fixture values"
)]

#[path = "../src/operations/basecurve/mod.rs"]
mod basecurve;

use std::cell::Cell;

use basecurve::source_map::{BASECURVE_SOURCE_MAP, BasecurvePortStatus};
use basecurve::*;

fn fixture_bytes() -> Vec<u8> {
    include_str!("../src/operations/basecurve/fixtures/v6-default.hex")
        .split_whitespace()
        .flat_map(|line| {
            (0..line.len()).step_by(2).map(move |index| {
                u8::from_str_radix(&line[index..index + 2], 16).expect("fixture hex")
            })
        })
        .collect()
}

fn nonlinear_parameters() -> BasecurveParameters {
    let mut value = BasecurveParameters::defaults();
    value.basecurve[0] = [BasecurveNode::new(0.0, 0.0); MAX_NODES];
    value.basecurve[0][0] = BasecurveNode::new(0.0, 0.0);
    value.basecurve[0][1] = BasecurveNode::new(0.5, 0.1);
    value.basecurve[0][2] = BasecurveNode::new(1.0, 1.0);
    value.basecurve_nodes[0] = 3;
    value
}

#[test]
fn native_v6_abi_and_default_fixture_are_exact() {
    assert_eq!(std::mem::size_of::<BasecurveNode>(), 8);
    assert_eq!(
        std::mem::size_of::<BasecurveParameters>(),
        BASECURVE_V6_PARAMETER_BYTES
    );
    assert_eq!(fixture_bytes().len(), BASECURVE_V6_PARAMETER_BYTES);
    let decoded = BasecurveParameters::from_bytes(&fixture_bytes()).expect("v6 fixture");
    assert_eq!(decoded, BasecurveParameters::defaults());
    assert_eq!(decoded.to_bytes().as_slice(), fixture_bytes().as_slice());
}

#[test]
fn all_native_migration_edges_preserve_prefix_and_defaults() {
    let mut v1_bytes = Vec::new();
    for index in 0..6 {
        v1_bytes.extend_from_slice(&(index as f32 / 10.0).to_le_bytes());
    }
    for index in 0..6 {
        v1_bytes.extend_from_slice(&(index as f32 / 20.0).to_le_bytes());
    }
    v1_bytes.extend_from_slice(&99_i32.to_le_bytes());
    let v1 = decode_history(1, &v1_bytes).expect("v1");
    let migrated = v1.current();
    assert_eq!(migrated.basecurve_nodes, [6, 3, 3]);
    assert_eq!(
        migrated.basecurve_type,
        [CUBIC_SPLINE, MONOTONE_HERMITE, MONOTONE_HERMITE]
    );
    assert_eq!(migrated.preserve_colors, DT_RGB_NORM_NONE);
    assert_eq!(migrated.basecurve[0][5].y, 0.25);

    let current = nonlinear_parameters();
    let state = BasecurveCurveState {
        basecurve: current.basecurve,
        basecurve_nodes: current.basecurve_nodes,
        basecurve_type: current.basecurve_type,
    };
    let mut v2_payload = vec![0_u8; BASECURVE_V2_PARAMETER_BYTES];
    let mut offset = 0;
    for curve in state.basecurve {
        for node in curve {
            v2_payload[offset..offset + 4].copy_from_slice(&node.x.to_le_bytes());
            offset += 4;
            v2_payload[offset..offset + 4].copy_from_slice(&node.y.to_le_bytes());
            offset += 4;
        }
    }
    for integer in state
        .basecurve_nodes
        .into_iter()
        .chain(state.basecurve_type)
    {
        v2_payload[offset..offset + 4].copy_from_slice(&integer.to_le_bytes());
        offset += 4;
    }
    assert_eq!(offset, BASECURVE_V2_PARAMETER_BYTES);
    assert_eq!(
        decode_history(2, &v2_payload)
            .expect("v2")
            .current()
            .exposure_stops,
        1.0
    );

    let mut v3_payload = v2_payload.clone();
    v3_payload.extend_from_slice(&0_i32.to_le_bytes());
    v3_payload.extend_from_slice(&0.0_f32.to_le_bytes());
    assert_eq!(
        decode_history(3, &v3_payload)
            .expect("v3")
            .current()
            .exposure_stops,
        1.0
    );
    assert_eq!(
        decode_history(4, &v3_payload)
            .expect("v4")
            .current()
            .exposure_stops,
        0.0
    );

    let mut v5_payload = v3_payload.clone();
    v5_payload.extend_from_slice(&(-0.25_f32).to_le_bytes());
    assert_eq!(
        decode_history(5, &v5_payload)
            .expect("v5")
            .current()
            .exposure_bias,
        -0.25
    );
    assert!(matches!(
        decode_history(7, &fixture_bytes()),
        Err(BasecurveCodecError::UnsupportedVersion(7))
    ));
}

#[test]
fn native_match_is_anchored_prefix_matching() {
    assert!(match_pattern("NIKON CORPORATION", "NIKON"));
    assert!(match_pattern("NIKON D750", ""));
    assert!(match_pattern("EOS 5D Mark II", "EOS 5D Mark%"));
    assert!(!match_pattern("NIKON", "NIKON CORPORATION"));
    // `%D____%` becomes the native regex `*D....*`, which is invalid because
    // the leading `*` has no preceding expression.
    assert!(!match_pattern("NIKON D750", "%D____%"));
}

#[test]
fn defaults_presets_and_auto_selection_follow_native_order() {
    let defaults = BasecurveParameters::defaults();
    assert!(!default_state().enabled);
    assert_eq!(default_state().parameters, defaults);
    assert_eq!(defaults.basecurve[0][0], BasecurveNode::new(0.0, 0.0));
    assert_eq!(defaults.basecurve[0][1], BasecurveNode::new(1.0, 1.0));
    assert_eq!(defaults.basecurve_nodes, [2, 0, 0]);
    assert_eq!(defaults.basecurve_type, [MONOTONE_HERMITE; 3]);
    assert_eq!(defaults.preserve_colors, DT_RGB_NORM_LUMINANCE);

    let generic = basecurve_presets();
    let camera = basecurve_camera_presets();
    assert_eq!(generic.len(), 18);
    assert_eq!(camera.len(), 14);
    assert_eq!(generic[0].name, "cubic spline");
    assert_eq!(camera[0].name, "Nikon D750");
    assert_eq!(generic[1].parameters.exposure_stops, 0.0);
    assert_eq!(
        generic[1].parameters.basecurve_type,
        [MONOTONE_HERMITE, 0, 0]
    );
    assert_eq!(
        check_camera(
            BasecurveCameraMetadata {
                exif_maker: "Canon",
                exif_model: "EOS 5D Mark II",
                camera_maker: "",
                camera_alias: "",
            },
            &generic
        )
        .expect("specific generic preset")
        .basecurve[0][1]
            .y,
        0.029677
    );

    // Native k > 0 intentionally skips the first camera preset.
    let d750 = BasecurveCameraMetadata {
        exif_maker: "NIKON CORPORATION",
        exif_model: "NIKON D750",
        camera_maker: "",
        camera_alias: "",
    };
    assert!(check_camera(d750, &camera).is_none());
    assert_eq!(
        reload_defaults(defaults, 1, Some(d750), true).basecurve_nodes,
        [2, 0, 0]
    );
    let nikon_like = reload_defaults(defaults, 0, Some(d750), true);
    assert_eq!(nikon_like.basecurve_nodes, [6, 0, 0]);

    let d7000 = BasecurveCameraMetadata {
        exif_maker: "NIKON CORPORATION",
        exif_model: "NIKON D7000",
        camera_maker: "",
        camera_alias: "",
    };
    let camera_default = reload_defaults(defaults, 0, Some(d7000), true);
    assert_eq!(camera_default.basecurve_nodes, [8, 0, 0]);
    assert_eq!(
        reload_defaults(camera_default, 0, None, true),
        camera_default,
        "native reload_defaults retains the previous default when no image can be matched"
    );

    let registrations = init_presets(false);
    assert_eq!(registrations.len(), 32);
    assert_eq!(registrations[0].maker, "");
    assert_eq!(
        registrations[0]
            .parameters
            .expect("registered cubic spline")
            .exposure_stops,
        1.0
    );
    assert_eq!(registrations[0].iso_min, 0);
    assert_eq!(registrations[0].iso_max, f32::MAX);
    assert_eq!(registrations[18].maker, "NIKON CORPORATION");
    assert_eq!(registrations[18].model, "NIKON D750");
    assert_eq!(registrations[18].iso_min, 0);
    assert_eq!(registrations[18].iso_max, f32::MAX);
    assert_eq!(init_presets(true).len(), 33);
    assert!(
        init_presets(true)
            .last()
            .expect("display default")
            .auto_apply
    );
}

#[test]
fn curve_compilation_keeps_native_quantization_and_threshold() {
    let plan = BasecurvePlan::compile(BasecurveParameters::defaults()).expect("identity curve");
    assert_eq!(plan.table().len(), LUT_RESOLUTION);
    assert_eq!(plan.table()[0].to_bits(), 0.0_f32.to_bits());
    assert_eq!(
        plan.table()[LUT_RESOLUTION - 1].to_bits(),
        (65535.0 / 65536.0_f32).to_bits()
    );
    let mut unused_fusion_parameters = BasecurveParameters::defaults();
    unused_fusion_parameters.exposure_stops = f32::NAN;
    unused_fusion_parameters.exposure_bias = f32::NAN;
    assert!(BasecurvePlan::compile(unused_fusion_parameters).is_ok());

    let mut parameters = nonlinear_parameters();
    parameters.preserve_colors = DT_RGB_NORM_NONE;
    let plan = BasecurvePlan::compile(parameters).expect("nonlinear curve");
    let pixel = BasecurvePixel::new(1.0, 0.5, 0.25, 0.37);
    let output = plan.execute_rgba(&[pixel]).expect("CPU LUT")[0].channels();
    let expected = plan.unbounded_coefficients()[1]
        * (plan.unbounded_coefficients()[0]).powf(plan.unbounded_coefficients()[2]);
    assert_eq!(output[0].to_bits(), expected.max(0.0).to_bits());
    assert_eq!(output[3].to_bits(), 0.37_f32.to_bits());
}

#[test]
fn legacy_and_color_preserving_paths_keep_alpha_and_profile_semantics() {
    let mut legacy_parameters = nonlinear_parameters();
    legacy_parameters.preserve_colors = DT_RGB_NORM_NONE;
    let legacy = BasecurvePlan::compile(legacy_parameters).expect("legacy path");
    let input = BasecurvePixel::new(-0.25, 0.4, 0.8, 0.63);
    let output = legacy.execute_rgba(&[input]).expect("legacy output")[0].channels();
    assert_eq!(output[0], 0.0);
    assert_eq!(output[3].to_bits(), 0.63_f32.to_bits());

    let profile = BasecurveProfileEvidence::new(
        [[1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
        [vec![0.0, 0.1, 0.4, 0.8], vec![-1.0], vec![-1.0]],
        [[1.0; 3]; 3],
        4,
        true,
    )
    .expect("explicit ICC evidence");
    let luminance = profile.working_luminance([0.5, 0.0, 0.0, 1.0]);
    assert!((luminance - 0.25).abs() < 1e-6);
    let color = BasecurvePlan::compile(nonlinear_parameters()).expect("color path");
    let output = color
        .execute_rgba_with_profile(
            &[BasecurvePixel::new(0.5, 0.0, 0.0, 0.63)],
            Some(&profile),
            || false,
        )
        .expect("color output")[0]
        .channels();
    assert!(output[0] > 0.0);
    assert_eq!(output[3].to_bits(), 0.63_f32.to_bits());

    let no_profile_luminance =
        BasecurvePlan::compile(nonlinear_parameters()).expect("camera luminance");
    let camera = no_profile_luminance
        .execute_rgba(&[BasecurvePixel::new(0.5, 0.0, 0.0, 1.0)])
        .expect("camera path");
    assert!(camera[0].channels()[0].is_finite());
}

#[test]
fn profile_evidence_rejects_incomplete_icc_data_without_fallback() {
    assert!(matches!(
        BasecurveProfileEvidence::new(
            [[1.0; 3]; 3],
            [vec![0.0], vec![-1.0], vec![-1.0]],
            [[1.0; 3]; 3],
            4,
            true,
        ),
        Err(BasecurveProfileError::LutTooShort { channel: 0, .. })
    ));
    let inactive_marker = BasecurveProfileEvidence::new(
        [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        [vec![f32::NAN], vec![-1.0], vec![-1.0]],
        [[1.0; 3]; 3],
        2,
        true,
    )
    .expect("non-finite native inactive marker");
    assert_eq!(
        inactive_marker.working_luminance([0.5, 0.25, 0.0, 1.0]),
        0.25
    );
    assert!(unsupported_working_frame_profile().is_err());
}

#[test]
fn unsupported_fusion_capabilities_and_cancellation_fail_closed() {
    let mut parameters = BasecurveParameters::defaults();
    parameters.exposure_fusion = 1;
    assert!(matches!(
        BasecurvePlan::compile(parameters),
        Err(BasecurveCompileError::UnsupportedExposureFusion { steps: 1 })
    ));
    let capabilities = BasecurvePlan::capabilities();
    assert!(capabilities.cpu_lut);
    assert!(!capabilities.gpu);
    assert!(!capabilities.gtk);
    assert!(!capabilities.consumes_masks);
    assert_eq!(capabilities.outer_blending, DeferredCapability::Deferred);
    assert_eq!(capabilities.tiling.factor_milli, 2_000);
    assert_eq!(capabilities.tiling.overlap_pixels, 0);
    assert_eq!(capabilities.tiling.alignment_pixels, 1);
    assert!(matches!(
        capabilities.require_gpu(),
        Err(BasecurveExecutionError::UnsupportedCapability(_))
    ));
    assert!(matches!(
        capabilities.require_gtk(),
        Err(BasecurveExecutionError::UnsupportedCapability(_))
    ));
    assert!(matches!(
        capabilities.require_masks(),
        Err(BasecurveExecutionError::UnsupportedCapability(_))
    ));
    assert!(matches!(
        capabilities.require_production_routing(),
        Err(BasecurveExecutionError::UnsupportedCapability(_))
    ));

    let plan = BasecurvePlan::compile(BasecurveParameters::defaults()).expect("plan");
    let input = vec![BasecurvePixel::new(0.25, 0.5, 0.75, 1.0); 300];
    let calls = Cell::new(0_u32);
    let result = plan.execute_rgba_with_profile(&input, None, || {
        let current = calls.get();
        calls.set(current + 1);
        current > 1
    });
    assert!(matches!(result, Err(BasecurveExecutionError::Cancelled)));
}

#[test]
fn source_map_records_deferred_native_surfaces() {
    assert!(
        BASECURVE_SOURCE_MAP
            .iter()
            .any(|entry| entry.status == BasecurvePortStatus::Ported)
    );
    assert!(
        BASECURVE_SOURCE_MAP
            .iter()
            .any(|entry| entry.status == BasecurvePortStatus::ExplicitlyDeferred)
    );
    assert!(
        BASECURVE_SOURCE_MAP
            .iter()
            .any(|entry| entry.native_symbol.contains("process_fusion"))
    );
}
