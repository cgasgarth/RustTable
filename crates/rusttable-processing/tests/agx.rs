//! Source-derived bounded AgX CPU leaf coverage.
//!
//! The operation is registered for built-in-profile CPU execution. Configured
//! external profile resolution, GPU/OpenCL, GTK/presets, masks, and outer
//! blending remain explicit deferred seams.

#![allow(
    clippy::assertions_on_constants,
    clippy::cast_possible_truncation,
    clippy::doc_markdown,
    clippy::excessive_precision,
    clippy::float_cmp,
    clippy::format_collect,
    clippy::needless_range_loop,
    clippy::too_many_lines,
    clippy::unreadable_literal,
    reason = "source-derived vectors assert native f32 field order and transfer boundaries"
)]

use agx::source_map::{AGX_SOURCE_MAP, AgxPortStatus};
use agx::{
    AGX_DEFAULT_COLORSPACE, AGX_DEFAULT_GROUPS, AGX_GPU_KERNELS, AGX_GPU_PROGRAM,
    AGX_MIGRATION_EDGES, AGX_PARAMETER_BYTES_V7, AGX_PARAMETER_FIELD_ORDER,
    AGX_PARAMETER_LAYOUT_HASH, AGX_SCHEMA_VERSION, AgxBasePrimaries, AgxCapabilityError,
    AgxCodecError, AgxConfig, AgxExecutionError, AgxHistory, AgxParameterError, AgxParametersV7,
    AgxPlan, AgxPlanError, AgxProfile,
};
use rusttable_processing::operations::agx;

const DEFAULT_FIXTURE: &str = include_str!("fixtures/agx/default-v7.txt");

fn bits(pixel: agx::AgxPixel) -> [u32; 4] {
    pixel.channels().map(f32::to_bits)
}

fn pair_bits(values: [[f32; 2]; 3]) -> [[u32; 2]; 3] {
    values.map(|pair| pair.map(f32::to_bits))
}

fn matrix_bits(matrix: agx::AgxMatrix) -> [[u32; 3]; 3] {
    matrix.map(|row| row.map(f32::to_bits))
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn native_abi_defaults_and_fixture_preserve_declaration_order() {
    assert_eq!(AGX_SCHEMA_VERSION, 7);
    assert_eq!(AGX_PARAMETER_BYTES_V7, 144);
    assert_eq!(AGX_PARAMETER_LAYOUT_HASH.len(), 64);
    assert!(AGX_MIGRATION_EDGES.is_empty());
    assert_eq!(AGX_DEFAULT_COLORSPACE, "RGB");
    assert_eq!(AGX_DEFAULT_GROUPS, ["tone", "technical"]);
    assert_eq!(AGX_GPU_PROGRAM, 39);
    assert_eq!(AGX_GPU_KERNELS, ["kernel_agx"]);

    let defaults = AgxParametersV7::defaults();
    assert_eq!(defaults.look_lift, 0.0);
    assert_eq!(defaults.look_slope, 1.0);
    assert_eq!(defaults.look_brightness, 1.0);
    assert_eq!(defaults.look_saturation, 1.0);
    assert_eq!(defaults.look_original_hue_mix_ratio, 0.6);
    assert_eq!(defaults.range_black_relative_ev, -10.0);
    assert_eq!(defaults.range_white_relative_ev, 6.5);
    assert_eq!(defaults.dynamic_range_scaling, 0.1);
    assert_eq!(defaults.curve_pivot_x, 10.0 / 16.5);
    assert_eq!(defaults.curve_pivot_y_linear_output, 0.18);
    assert_eq!(defaults.curve_contrast_around_pivot, 3.0);
    assert_eq!(defaults.curve_toe_power, 1.5);
    assert_eq!(defaults.curve_shoulder_power, 3.3);
    assert_eq!(defaults.curve_gamma, 2.2);
    assert_eq!(defaults.auto_gamma, 0);
    assert_eq!(defaults.base_primaries, AgxBasePrimaries::Rec2020);
    assert_eq!(defaults.master_outset_ratio, 1.0);
    assert_eq!(defaults.master_unrotation_ratio, 1.0);
    assert_eq!(defaults.completely_reverse_primaries, 0);
    assert_eq!(defaults.to_bytes().len(), AGX_PARAMETER_BYTES_V7);
    assert!(DEFAULT_FIXTURE.contains(&format!("payload_hex={}", hex_bytes(&defaults.to_bytes()))));
    assert!(DEFAULT_FIXTURE.contains("payload_bytes=144"));
    assert!(DEFAULT_FIXTURE.contains(&format!("field_order={AGX_PARAMETER_FIELD_ORDER}")));
    assert!(DEFAULT_FIXTURE.contains("migration_edges=[]"));
}

#[test]
fn native_pre_v7_migration_reencodes_as_v7_scene_referred_defaults() {
    let history = AgxHistory::decode(6, &[0xde, 0xad, 0xbe, 0xef]).expect("legacy migration");
    assert_eq!(history.version(), AGX_SCHEMA_VERSION);
    assert_eq!(
        history.current(),
        AgxParametersV7::scene_referred_defaults()
    );
    assert_eq!(history.payload(), history.current().to_bytes().to_vec());
    assert_eq!(
        AgxHistory::decode(3, &[0xde, 0xad]),
        Ok(AgxHistory::LegacySceneReferred {
            source_version: 3,
            parameters: AgxParametersV7::scene_referred_defaults(),
        })
    );
    assert_eq!(
        AgxHistory::decode(7, &[0; AGX_PARAMETER_BYTES_V7]),
        Ok(AgxHistory::V7(
            AgxParametersV7::from_bytes(&[0; 144]).expect("v7")
        ))
    );
    assert_eq!(
        AgxHistory::decode(8, &[]),
        Err(AgxCodecError::UnsupportedVersion(8))
    );
}

#[test]
fn malformed_payload_and_enum_fail_closed_while_finite_config_preserves_raw_ranges() {
    assert_eq!(
        AgxParametersV7::from_bytes(&[0; AGX_PARAMETER_BYTES_V7 - 1]),
        Err(AgxCodecError::InvalidLength {
            expected: AGX_PARAMETER_BYTES_V7,
            actual: AGX_PARAMETER_BYTES_V7 - 1,
        })
    );
    let mut invalid_enum = AgxParametersV7::defaults().to_bytes();
    invalid_enum[76..80].copy_from_slice(&99_i32.to_le_bytes());
    assert_eq!(
        AgxParametersV7::from_bytes(&invalid_enum),
        Err(AgxCodecError::InvalidBasePrimaries(99))
    );

    let mut parameters = AgxParametersV7::defaults();
    parameters.look_lift = 4.0;
    parameters.red_inset = 1.5;
    assert!(AgxConfig::new(parameters).is_ok());
    parameters.curve_gamma = f32::NAN;
    assert_eq!(
        AgxConfig::new(parameters),
        Err(AgxParameterError::NonFinite("curve_gamma"))
    );
}

#[test]
fn standard_profiles_use_native_icc_d50_matrix_shaper_data() {
    let profiles = [
        (
            "sRGB",
            AgxProfile::srgb(),
            [
                [1059454998, 1051289109],
                [1050964181, 1058606567],
                [1042260639, 1032275498],
            ],
            [
                [1054818361, 1046729284, 1013193356],
                [1053109998, 1060603800, 1036439790],
                [1041398001, 1031290222, 1060553246],
            ],
            [
                [1078499150, 3212480197, 1033069667],
                [3218014797, 1073038179, 3194651713],
                [3204135928, 1023997497, 1068757143],
            ],
        ),
        (
            "linear Rec2020 RGB",
            AgxProfile::rec2020(),
            [
                [1060438309, 1050017204],
                [1044563696, 1061584488],
                [1040472512, 1027676596],
            ],
            [
                [1059875032, 1049550439, 3137164026],
                [1042917076, 1059906448, 1022729124],
                [1040190729, 1027265473, 1061944946],
            ],
            [
                [1070782745, 3207512055, 1022566577],
                [3200878932, 1070785759, 3179342272],
                [3195118203, 1012002258, 1067482957],
            ],
        ),
        (
            "Display P3 RGB",
            AgxProfile::display_p3(),
            [
                [1060002908, 1050888009],
                [1049735436, 1059894440],
                [1042260639, 1032275498],
            ],
            [
                [1057218209, 1047984609, 3129582514],
                [1049984788, 1060190027, 1026263750],
                [1042341522, 1032344038, 1061730507],
            ],
            [
                [1075436352, 3210189721, 1027961773],
                [3212667970, 1072054328, 3183967838],
                [3201013510, 1015244564, 1067651757],
            ],
        ),
        (
            "Adobe RGB (compatible)",
            AgxProfile::adobe_rgb(),
            [
                [1059454999, 1051289110],
                [1047245557, 1060346267],
                [1042260640, 1032275499],
            ],
            [
                [1058805738, 1050626557, 1017085063],
                [1045574820, 1059073184, 1031363713],
                [1041810163, 1031893767, 1061067513],
            ],
            [
                [1073427487, 3212480197, 1022047474],
                [3206304945, 1073038181, 3188725061],
                [3199125770, 1023997494, 1068283201],
            ],
        ),
    ];

    for (name, profile, expected_primaries, expected_in, expected_out) in profiles {
        assert_eq!(
            profile.whitepoint().map(f32::to_bits),
            [1051787257, 1052217951],
            "{name} D50 media white point"
        );
        assert_eq!(
            pair_bits(profile.primaries()),
            expected_primaries,
            "{name} D50 colorant chromaticities"
        );
        assert_eq!(
            matrix_bits(profile.matrix_in_transposed()),
            expected_in,
            "{name} RGB-to-D50 matrix"
        );
        assert_eq!(
            matrix_bits(profile.matrix_out_transposed()),
            expected_out,
            "{name} D50-to-RGB matrix"
        );
    }
}

#[test]
fn default_cpu_fixture_preserves_alpha_and_native_matrix_order() {
    let dimensions = rusttable_processing::RasterDimensions::new(2, 2).expect("dimensions");
    let plan = AgxPlan::new(AgxConfig::defaults(), dimensions).expect("default plan");
    let input = [
        agx::AgxPixel::new(0.12, 0.25, 0.8, 0.125),
        agx::AgxPixel::new(-0.1, 0.4, 1.2, 0.25),
        agx::AgxPixel::new(0.7, -0.2, 0.3, 0.5),
        agx::AgxPixel::new(2.0, 0.1, -0.4, 0.875),
    ];
    let output = plan.execute(&input).expect("CPU output");
    assert_eq!(output.len(), input.len());
    let expected_bits = [
        [1040803846, 1048568457, 1059726311, 1040187392],
        [3179429572, 1051544688, 1061899185, 1048576000],
        [1058120880, 3174966428, 1050135888, 1056964608],
        [1066057832, 1049503354, 3175441816, 1063256064],
    ];
    assert_eq!(
        output.iter().copied().map(bits).collect::<Vec<_>>(),
        expected_bits
    );
    assert!(DEFAULT_FIXTURE.contains(
        "cpu_output_bits=[[1040803846,1048568457,1059726311,1040187392],[3179429572,1051544688,1061899185,1048576000],[1058120880,3174966428,1050135888,1056964608],[1066057832,1049503354,3175441816,1063256064]]"
    ));
    for (source, result) in input.into_iter().zip(output.iter().copied()) {
        assert!(result.channels().into_iter().all(f32::is_finite));
        assert_eq!(result.alpha().to_bits(), source.alpha().to_bits());
    }
    let (rendering_to_xyz, pipe_to_base, base_to_rendering, rendering_to_pipe) = plan.matrices();
    for matrix in [
        rendering_to_xyz,
        pipe_to_base,
        base_to_rendering,
        rendering_to_pipe,
    ] {
        assert!(matrix.into_iter().flatten().all(f32::is_finite));
    }
}

#[test]
fn scene_referred_non_neutral_rgb_matches_native_d50_profile_oracle() {
    let dimensions = rusttable_processing::RasterDimensions::new(1, 2).expect("dimensions");
    let parameters = AgxParametersV7::scene_referred_defaults();
    assert_eq!(parameters.base_primaries, AgxBasePrimaries::Rec2020);
    assert_eq!(parameters.red_inset.to_bits(), 0.29462451_f32.to_bits());
    assert_eq!(parameters.green_inset.to_bits(), 0.25861925_f32.to_bits());
    assert_eq!(parameters.blue_inset.to_bits(), 0.14641371_f32.to_bits());
    assert_eq!(parameters.red_rotation.to_bits(), 0.03540329_f32.to_bits());
    assert_eq!(
        parameters.green_rotation.to_bits(),
        (-0.02108586_f32).to_bits()
    );
    assert_eq!(
        parameters.blue_rotation.to_bits(),
        (-0.06305724_f32).to_bits()
    );

    // Exact `dt_iop_order_iccprofile_info_t` values produced for the native
    // built-in sRGB matrix-shaper profile: D50 colorant chromaticities, D50
    // media white point, and transposed RGB↔D50 PCS matrices.
    let working_profile = AgxProfile::from_matrices(
        [
            [0.6484388113021851, 0.33085694909095764],
            [0.3211733400821686, 0.5978683829307556],
            [0.15589378774166107, 0.06605179607868195],
        ],
        [0.3457029163837433, 0.35853859782218933],
        [
            [
                0.4360368549823761,
                0.22248178720474243,
                0.013922344893217087,
            ],
            [0.38512367010116577, 0.7169127464294434, 0.09707818925380707],
            [0.14303947985172272, 0.06060545891523361, 0.7138994932174683],
        ],
        [
            [3.13423490524292, -0.9787409901618958, 0.07196881622076035],
            [-1.6172577142715454, 1.9161189794540405, -0.2290201336145401],
            [-0.4906919002532959, 0.03343794122338295, 1.4057797193527222],
        ],
    )
    .expect("native sRGB matrix-shaper profile");
    assert_eq!(working_profile, AgxProfile::srgb());

    let config = AgxConfig::new(parameters).expect("finite parameters");
    let plan =
        AgxPlan::new_with_profile(config, dimensions, working_profile).expect("profile transforms");
    let input = [
        agx::AgxPixel::new(0.01, 0.18, 4.0, 0.25),
        agx::AgxPixel::new(1.0, 0.25, 0.02, 0.75),
    ];
    let output = plan.execute(&input).expect("CPU output");
    assert_eq!(
        output.iter().copied().map(bits).collect::<Vec<_>>(),
        [
            [1050095280, 1056302075, 1066224274, 1048576000],
            [1062550022, 1048464760, 1020175256, 1061158912],
        ]
    );
    assert!(DEFAULT_FIXTURE.contains(
        "scene_work_matrix_in_bits=[[1054818361,1046729284,1013193356],[1053109998,1060603800,1036439790],[1041398001,1031290222,1060553246]]"
    ));
    assert!(DEFAULT_FIXTURE.contains(
        "scene_base_matrix_in_bits=[[1059875032,1049550439,3137164026],[1042917076,1059906448,1022729124],[1040190729,1027265473,1061944946]]"
    ));
    assert!(DEFAULT_FIXTURE.contains(
        "scene_output_bits=[[1050095280,1056302075,1066224274,1048576000],[1062550022,1048464760,1020175256,1061158912]]"
    ));
    assert!(
        output
            .iter()
            .flat_map(|pixel| pixel.channels())
            .all(f32::is_finite)
    );
    for (source, result) in input.into_iter().zip(output) {
        assert_eq!(result.alpha().to_bits(), source.alpha().to_bits());
    }
    assert!(plan.tone_mapping_parameters().apply_curve(0.0).is_finite());
    assert!(plan.tone_mapping_parameters().apply_curve(1.0).is_finite());
}

#[test]
fn input_sanitisation_matches_native_nan_and_infinity_boundary() {
    let dimensions = rusttable_processing::RasterDimensions::new(1, 1).expect("dimensions");
    let plan = AgxPlan::new(AgxConfig::defaults(), dimensions).expect("default plan");
    let output = plan
        .execute(&[agx::AgxPixel::new(
            f32::NAN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        )])
        .expect("sanitised CPU output");
    assert!(output[0].channels().into_iter().all(f32::is_finite));
    assert_eq!(output[0].alpha().to_bits(), 0.0_f32.to_bits());
}

#[test]
fn cancellation_never_publishes_partial_output() {
    let dimensions = rusttable_processing::RasterDimensions::new(2, 2).expect("dimensions");
    let plan = AgxPlan::new(AgxConfig::defaults(), dimensions).expect("default plan");
    let input = vec![agx::AgxPixel::new(0.1, 0.2, 0.3, 0.1); 4];
    let mut calls = 0;
    assert_eq!(
        plan.execute_with_cancel(&input, || {
            calls += 1;
            calls > 2
        }),
        Err(AgxExecutionError::Cancelled)
    );
    assert_eq!(
        plan.execute(&input[..2]),
        Err(AgxExecutionError::DimensionsMismatch {
            expected: 4,
            actual: 2,
        })
    );
}

#[test]
fn profile_and_capability_boundaries_are_fail_closed() {
    assert_eq!(
        AgxProfile::from_primaries(
            [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]],
            [f32::NAN, 0.32902],
        ),
        Err(agx::AgxProfileError::NonFinite)
    );
    assert_eq!(
        AgxProfile::from_primaries([[0.2, 0.3], [0.2, 0.3], [0.2, 0.3]], [0.31271, 0.32902],),
        Err(agx::AgxProfileError::SingularMatrix)
    );
    assert!(matches!(
        AgxPlan::new_with_profile(
            AgxConfig::defaults(),
            rusttable_processing::RasterDimensions::new(1, 1).expect("dimensions"),
            AgxProfile::from_matrices(
                [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]],
                [0.31271, 0.32902],
                [[0.0; 3]; 3],
                [[0.0; 3]; 3],
            )
            .expect("finite profile matrices"),
        ),
        Err(AgxPlanError::SingularMatrix)
    ));

    let mut export_parameters = AgxParametersV7::defaults();
    export_parameters.base_primaries = AgxBasePrimaries::ExportProfile;
    let export_config = AgxConfig::new(export_parameters).expect("export parameters");
    assert!(
        AgxPlan::new_with_profiles(
            export_config,
            rusttable_processing::RasterDimensions::new(1, 1).expect("dimensions"),
            AgxProfile::srgb(),
            Some(
                AgxProfile::from_matrices(
                    [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]],
                    [0.31271, 0.32902],
                    [[0.0; 3]; 3],
                    [[0.0; 3]; 3],
                )
                .expect("finite export profile matrices"),
            ),
        )
        .is_ok()
    );

    let capabilities = agx::capabilities();
    assert!(capabilities.cpu_supported);
    assert!(!capabilities.profile_transforms_supported);
    assert!(!capabilities.gpu_supported);
    assert!(!capabilities.gtk_supported);
    assert!(!capabilities.masks_consumed);
    assert!(capabilities.outer_blending_deferred);
    assert!(!capabilities.production_routing_deferred);
    assert!(capabilities.alpha_preserved);
    assert_eq!(
        capabilities.require_gpu(),
        Err(AgxCapabilityError::GpuUnavailable)
    );
    assert_eq!(
        capabilities.require_gtk(),
        Err(AgxCapabilityError::GtkUnavailable)
    );
    assert_eq!(capabilities.require_production_routing(), Ok(()));
    assert!(AGX_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("dt_iop_agx_params_t") && entry.status == AgxPortStatus::Ported
    }));
    assert!(AGX_SOURCE_MAP.iter().any(|entry| {
        entry.native_file == "data/kernels/agx.cl"
            && entry.status == AgxPortStatus::ExplicitlyDeferred
    }));
}
