//! Source-derived bounded Sigmoid CPU leaf coverage.
//!
//! The leaf is included by path: registry, production history, pixelpipe,
//! GPU/OpenCL, GTK/presets, masks, and outer blending are separate deferred
//! seams.

#![allow(
    clippy::assertions_on_constants,
    clippy::cast_possible_truncation,
    clippy::float_cmp,
    clippy::needless_range_loop,
    clippy::too_many_lines,
    clippy::unreadable_literal,
    reason = "source-derived vectors assert native f32 field order and transfer boundaries"
)]

#[path = "../src/operations/sigmoid/mod.rs"]
mod sigmoid;

use sigmoid::source_map::{SIGMOID_SOURCE_MAP, SigmoidPortStatus};
use sigmoid::{
    SIGMOID_DEFAULT_COLORSPACE, SIGMOID_DEFAULT_GROUPS, SIGMOID_GPU_KERNELS, SIGMOID_GPU_PROGRAM,
    SIGMOID_MIGRATION_EDGES, SIGMOID_PARAMETER_BYTES_V1, SIGMOID_PARAMETER_BYTES_V2,
    SIGMOID_PARAMETER_BYTES_V3, SIGMOID_SCHEMA_VERSION, SigmoidBasePrimaries,
    SigmoidCapabilityError, SigmoidCodecError, SigmoidColorProcessing, SigmoidConfig,
    SigmoidExecutionError, SigmoidHistory, SigmoidParametersV1, SigmoidParametersV2,
    SigmoidParametersV3, SigmoidPixel, SigmoidPlan, SigmoidPlanError, SigmoidProfile,
    SigmoidProfileError,
};

const DEFAULT_FIXTURE: &str = include_str!("fixtures/sigmoid/default-v3.hex");

#[test]
fn native_abi_defaults_and_fixture_preserve_declaration_order() {
    assert_eq!(SIGMOID_SCHEMA_VERSION, 3);
    assert_eq!(SIGMOID_PARAMETER_BYTES_V1, 24);
    assert_eq!(SIGMOID_PARAMETER_BYTES_V2, 52);
    assert_eq!(SIGMOID_PARAMETER_BYTES_V3, 56);
    assert_eq!(SIGMOID_MIGRATION_EDGES, &[(1, 3), (2, 3)]);
    assert_eq!(SIGMOID_DEFAULT_COLORSPACE, "RGB");
    assert_eq!(SIGMOID_DEFAULT_GROUPS, ["tone", "technical"]);
    assert_eq!(SIGMOID_GPU_PROGRAM, 36);
    assert_eq!(
        SIGMOID_GPU_KERNELS,
        [
            "sigmoid_loglogistic_per_channel",
            "sigmoid_loglogistic_rgb_ratio"
        ]
    );

    let defaults = SigmoidParametersV3::defaults();
    assert_eq!(defaults.middle_grey_contrast, 1.5);
    assert_eq!(defaults.display_white_target, 100.0);
    assert_eq!(defaults.display_black_target, 0.0152);
    assert_eq!(
        defaults.color_processing,
        SigmoidColorProcessing::PerChannel
    );
    assert_eq!(defaults.hue_preservation, 100.0);
    assert_eq!(
        defaults.base_primaries,
        SigmoidBasePrimaries::WorkingProfile
    );
    assert_eq!(defaults.to_bytes().len(), 56);
    assert!(DEFAULT_FIXTURE.contains("payload_bytes=56"));
    assert!(DEFAULT_FIXTURE.contains("field_order=middle_grey_contrast,contrast_skewness,display_white_target,display_black_target,color_processing,hue_preservation,red_inset,red_rotation,green_inset,green_rotation,blue_inset,blue_rotation,purity,base_primaries"));
    assert!(DEFAULT_FIXTURE.contains("migration_edges=[(1,3),(2,3)]"));
    assert!(DEFAULT_FIXTURE.contains(
        "cpu_output_bits=[[1043564490,1050433347,1059759804,1040187392],[1010793712,1055942552,1061895119,1048576000],[1056050843,3171032336,1048395022,1056964608],[1064084440,1049480264,3166904614,1063256064]]"
    ));
}

#[test]
fn native_v1_and_v2_migrations_zero_only_new_v3_fields() {
    let v1 = SigmoidParametersV1::new(
        1.22,
        0.65,
        100.0,
        0.0152,
        SigmoidColorProcessing::PerChannel,
        100.0,
    );
    let v1_history = SigmoidHistory::decode(1, &v1.to_bytes()).expect("valid v1 history");
    assert_eq!(v1_history.version(), 1);
    assert_eq!(v1_history.payload(), v1.to_bytes());
    assert_eq!(
        v1_history.current().expect("v1 migrates"),
        SigmoidParametersV3::new(
            1.22,
            0.65,
            100.0,
            0.0152,
            SigmoidColorProcessing::PerChannel,
            100.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            0.0,
            SigmoidBasePrimaries::WorkingProfile,
        )
    );

    let v2 = SigmoidParametersV2::new(
        1.5,
        -0.2,
        100.0,
        0.0152,
        SigmoidColorProcessing::PerChannel,
        0.0,
        0.1,
        0.02,
        0.1,
        -0.01,
        0.15,
        -0.03,
        0.0,
    );
    let v2_history = SigmoidHistory::decode(2, &v2.to_bytes()).expect("valid v2 history");
    assert_eq!(
        v2_history.current().expect("v2 migrates").base_primaries,
        SigmoidBasePrimaries::WorkingProfile
    );
    assert_eq!(v2_history.current().expect("v2 migrates").red_inset, 0.1);
    assert_eq!(
        v2_history.current().expect("v2 migrates").blue_rotation,
        -0.03
    );
}

#[test]
fn malformed_known_payloads_fail_and_future_payloads_round_trip_opaque() {
    assert_eq!(
        SigmoidHistory::decode(1, &[0; 23]),
        Err(SigmoidCodecError::InvalidLength {
            expected: 24,
            actual: 23,
        })
    );
    assert_eq!(
        SigmoidHistory::decode(3, &[0; 55]),
        Err(SigmoidCodecError::InvalidLength {
            expected: 56,
            actual: 55,
        })
    );
    let mut invalid_method = SigmoidParametersV3::defaults().to_bytes();
    invalid_method[16..20].copy_from_slice(&9_i32.to_le_bytes());
    assert_eq!(
        SigmoidHistory::decode(3, &invalid_method),
        Err(SigmoidCodecError::InvalidColorProcessing(9))
    );

    let future = vec![0xde, 0xad, 0xbe, 0xef, 0x01];
    let history = SigmoidHistory::decode(99, &future).expect("future remains opaque");
    assert_eq!(history.version(), 99);
    assert_eq!(history.payload(), future);
    assert_eq!(
        history.current(),
        Err(SigmoidCodecError::UnsupportedVersion(99))
    );
}

#[test]
fn finite_commit_preserves_native_ui_ranges_as_non_clamping_validation() {
    let mut parameters = SigmoidParametersV3::defaults();
    parameters.display_black_target = -12.0;
    parameters.red_inset = 1.5;
    parameters.purity = -3.0;
    assert!(SigmoidConfig::new(parameters).is_ok());

    parameters.hue_preservation = f32::NAN;
    assert!(matches!(
        SigmoidConfig::new(parameters),
        Err(sigmoid::SigmoidParameterError::NonFinite(
            "hue_preservation"
        ))
    ));
}

#[test]
fn generalized_loglogistic_keeps_stable_equation_and_nan_magnitude_fallback() {
    let mapped = sigmoid::generalized_loglogistic_sigmoid(0.1845, 1.0, 3.0, 0.0, 1.5, 1.0);
    assert!(mapped.is_finite());
    assert!(mapped > 0.0);
    assert_eq!(
        sigmoid::generalized_loglogistic_sigmoid(f32::NAN, 7.0, 0.0, 0.0, 0.0, 0.0,),
        7.0
    );
}

#[test]
fn per_channel_profile_path_preserves_alpha_and_profile_matrix_contract() {
    let dimensions = rusttable_processing::RasterDimensions::new(2, 2).expect("dimensions");
    let mut parameters = SigmoidParametersV3::defaults();
    parameters.hue_preservation = 0.0;
    parameters.red_inset = 0.1;
    parameters.red_rotation = 0.02;
    parameters.green_inset = 0.1;
    parameters.green_rotation = -0.01;
    parameters.blue_inset = 0.15;
    parameters.blue_rotation = -0.03;
    parameters.base_primaries = SigmoidBasePrimaries::Rec2020;
    let config = SigmoidConfig::new(parameters).expect("finite parameters");
    let plan = SigmoidPlan::new_with_profile(config, dimensions, SigmoidProfile::srgb())
        .expect("valid adjusted profile matrices");
    assert!(plan.film_power().is_finite());
    assert!(plan.paper_power().is_finite());
    let input = vec![
        SigmoidPixel::new(0.12, 0.25, 0.8, 0.125),
        SigmoidPixel::new(-0.1, 0.4, 1.2, 0.25),
        SigmoidPixel::new(0.7, -0.2, 0.3, 0.5),
        SigmoidPixel::new(2.0, 0.1, -0.4, 0.875),
    ];
    let output = plan.execute(&input).expect("CPU output");
    assert_eq!(output.len(), input.len());
    assert_eq!(
        output
            .iter()
            .map(|pixel| pixel.channels().map(f32::to_bits))
            .collect::<Vec<_>>(),
        vec![
            [1043564490, 1050433347, 1059759804, 1040187392],
            [1010793712, 1055942552, 1061895119, 1048576000],
            [1056050843, 3171032336, 1048395022, 1056964608],
            [1064084440, 1049480264, 3166904614, 1063256064],
        ]
    );
    for (source, result) in input.into_iter().zip(output) {
        assert_eq!(result.alpha().to_bits(), source.alpha().to_bits());
        assert!(result.channels()[..3].iter().all(|value| value.is_finite()));
    }
    let (pipe_to_base, base_to_rendering, rendering_to_pipe) = plan.matrices();
    assert!(pipe_to_base.into_iter().flatten().all(f32::is_finite));
    assert!(base_to_rendering.into_iter().flatten().all(f32::is_finite));
    assert!(rendering_to_pipe.into_iter().flatten().all(f32::is_finite));
}

#[test]
fn rgb_ratio_path_desaturates_negative_values_and_preserves_alpha() {
    let dimensions = rusttable_processing::RasterDimensions::new(1, 2).expect("dimensions");
    let mut parameters = SigmoidParametersV3::defaults();
    parameters.color_processing = SigmoidColorProcessing::RgbRatio;
    parameters.middle_grey_contrast = 1.0;
    parameters.contrast_skewness = 0.0;
    let plan = SigmoidPlan::new(
        SigmoidConfig::new(parameters).expect("finite parameters"),
        dimensions,
    )
    .expect("valid default transfer");
    let input = [
        SigmoidPixel::new(-1.0, 0.5, 2.0, 0.33),
        SigmoidPixel::new(0.0, 0.0, 0.0, 0.66),
    ];
    let output = plan.execute(&input).expect("RGB-ratio CPU output");
    for (source, result) in input.into_iter().zip(output) {
        assert_eq!(result.alpha().to_bits(), source.alpha().to_bits());
        assert!(result.channels().into_iter().all(f32::is_finite));
    }
}

#[test]
fn cancellation_and_fail_closed_validation_never_publish_partial_output() {
    let dimensions = rusttable_processing::RasterDimensions::new(2, 2).expect("dimensions");
    let plan = SigmoidPlan::new(SigmoidConfig::defaults(), dimensions).expect("default plan");
    let input = vec![
        SigmoidPixel::new(0.1, 0.2, 0.3, 0.1),
        SigmoidPixel::new(0.2, 0.3, 0.4, 0.2),
        SigmoidPixel::new(0.3, 0.4, 0.5, 0.3),
        SigmoidPixel::new(0.4, 0.5, 0.6, 0.4),
    ];
    let mut calls = 0;
    assert_eq!(
        plan.execute_with_cancel(&input, || {
            calls += 1;
            calls > 2
        }),
        Err(SigmoidExecutionError::Cancelled)
    );

    let mut invalid = input;
    invalid[3] = SigmoidPixel::new(0.4, 0.5, 0.6, f32::INFINITY);
    assert_eq!(
        plan.execute(&invalid),
        Err(SigmoidExecutionError::NonFiniteInput {
            pixel: 3,
            channel: sigmoid::SigmoidChannel::Alpha,
        })
    );
    assert_eq!(
        plan.execute(&invalid[..2]),
        Err(SigmoidExecutionError::DimensionsMismatch {
            expected: 4,
            actual: 2,
        })
    );
}

#[test]
fn capabilities_and_source_map_keep_unowned_surfaces_fail_closed() {
    let capabilities = sigmoid::capabilities();
    assert!(capabilities.cpu_supported);
    assert!(capabilities.profile_transforms_supported);
    assert!(!capabilities.gpu_supported);
    assert!(!capabilities.gtk_supported);
    assert!(!capabilities.masks_consumed);
    assert!(capabilities.outer_blending_deferred);
    assert!(capabilities.production_routing_deferred);
    assert!(capabilities.alpha_preserved);
    assert_eq!(
        capabilities.require_gpu(),
        Err(SigmoidCapabilityError::GpuUnavailable)
    );
    assert_eq!(
        capabilities.require_gtk(),
        Err(SigmoidCapabilityError::GtkUnavailable)
    );
    assert_eq!(
        capabilities.require_production_routing(),
        Err(SigmoidCapabilityError::ProductionRoutingDeferred)
    );
    assert!(SIGMOID_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("commit_params") && entry.status == SigmoidPortStatus::Ported
    }));
    assert!(SIGMOID_SOURCE_MAP.iter().any(|entry| {
        entry.native_file == "data/kernels/sigmoid.cl"
            && entry.status == SigmoidPortStatus::ExplicitlyDeferred
    }));
}

#[test]
fn profile_constructor_rejects_nonfinite_and_singular_inputs() {
    assert_eq!(
        SigmoidProfile::from_primaries(
            [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]],
            [f32::NAN, 0.32902],
        ),
        Err(SigmoidProfileError::NonFinite)
    );
    assert!(matches!(
        SigmoidProfile::from_primaries([[0.2, 0.3], [0.2, 0.3], [0.2, 0.3]], [0.31271, 0.32902],),
        Err(SigmoidProfileError::SingularMatrix)
    ));
    assert!(matches!(
        SigmoidPlan::new_with_profile(
            SigmoidConfig::defaults(),
            rusttable_processing::RasterDimensions::new(1, 1).expect("dimensions"),
            SigmoidProfile::from_matrices(
                [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]],
                [0.31271, 0.32902],
                [[0.0; 3]; 3],
                [[0.0; 3]; 3],
            )
            .expect("finite profile matrices"),
        ),
        Err(SigmoidPlanError::SingularMatrix)
    ));
}
