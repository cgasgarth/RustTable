use rusttable_compat::{
    CompatHistory, CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions,
    HistoryDecoder, HistoryLimits, HistoryOrderSource,
};
use rusttable_import::darktable::{
    DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode, decode_history_step,
};
use rusttable_processing::operations::agx::{AgxConfig, AgxParametersV7};
use rusttable_processing::operations::bloom::{
    BLOOM_PARAMETER_BYTES, BloomConfig, BloomParametersV1,
};
use rusttable_processing::operations::colorcontrast::{
    ColorContrastConfig, ColorContrastParametersV2,
};
use rusttable_processing::operations::colorcorrection::{
    ColorCorrectionConfig, ColorCorrectionParametersV1,
};
use rusttable_processing::operations::colormapping::{
    COLOR_MAPPING_PARAMETER_BYTES, ColorMappingConfig, ColorMappingParametersV1,
};
use rusttable_processing::operations::colorreconstruction::{
    ColorReconstructionConfig, ColorReconstructionPrecedence,
};
use rusttable_processing::operations::colortransfer::{
    COLORTRANSFER_NATIVE_PARAMETER_BYTES, ColorTransferParameters,
};
use rusttable_processing::operations::crop::{
    CROP_LEGACY_V1_BYTES, CROP_LEGACY_V2_BYTES, CROP_PARAMETER_BYTES, CropConfig,
    CropLegacyParametersV1, CropLegacyParametersV2, CropParametersV3,
};
use rusttable_processing::operations::enlargecanvas::{
    CanvasColor, ENLARGECANVAS_PARAMETER_BYTES, EnlargeCanvasParametersV1,
};
use rusttable_processing::operations::levels::{
    LevelsConfig, LevelsMode, LevelsParametersV1, LevelsParametersV2,
};
use rusttable_processing::operations::rgblevels::{RgbLevelsConfig, RgbLevelsParametersV1};
use rusttable_processing::operations::velvia::{
    VelviaConfig, VelviaParametersV1, VelviaParametersV2,
};
use rusttable_processing::operations::vibrance::{VibranceConfig, VibranceParametersV2};
use rusttable_processing::{
    COLORZONES_V5_PARAMETER_BYTES, ColorZonesConfig, ColorZonesHistory, ColorZonesParametersV5,
};
use rusttable_sqlite_native::{
    DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow, RawModuleOrderRow,
};

const BLOOM_V1_DEFAULT_NATIVE_LE: &[u8; BLOOM_PARAMETER_BYTES] =
    include_bytes!("fixtures/bloom-v1-default.bin");
const COLORCONTRAST_V1_NATIVE_LE: [u8; 16] = [
    0x00, 0x00, 0xc0, 0x3f, // a_steepness = 1.5
    0x00, 0x00, 0x00, 0xc0, // a_offset = -2.0
    0x00, 0x00, 0x40, 0x3f, // b_steepness = 0.75
    0x00, 0x00, 0x80, 0x40, // b_offset = 4.0
];
const COLORCONTRAST_V2_NATIVE_LE: [u8; 20] = [
    0x00, 0x00, 0x00, 0x41, // a_steepness = 8.0
    0x00, 0x00, 0x96, 0xc3, // a_offset = -300.0
    0x00, 0x00, 0x00, 0xc0, // b_steepness = -2.0
    0x00, 0x00, 0xc8, 0x43, // b_offset = 400.0
    0xf9, 0xff, 0xff, 0xff, // unbound = -7
];
const COLORCORRECTION_V1_BENCHMARK_LE: [u8; 20] = [
    0x66, 0x3b, 0x9a, 0x40, // hia = 4.8197508
    0x31, 0x53, 0x4c, 0x40, // hib = 3.1925776
    0x7f, 0x04, 0x95, 0xc0, // loa = -4.656799
    0x0a, 0x72, 0x7e, 0xc0, // lob = -3.9757104
    0x00, 0x00, 0x80, 0x3f, // saturation = 1.0
];
const COLORRECONSTRUCT_V1_NATIVE_LE: &[u8; 12] =
    include_bytes!("fixtures/colorreconstruct-v1-native-le.bin");
const COLORRECONSTRUCT_V2_NATIVE_LE: &[u8; 16] =
    include_bytes!("fixtures/colorreconstruct-v2-native-le.bin");
const COLORRECONSTRUCT_V3_NATIVE_LE: &[u8; 20] =
    include_bytes!("fixtures/colorreconstruct-v3-native-le.bin");
const VIBRANCE_V2_NATIVE_LE: [u8; 4] = [
    0x00, 0x00, 0xc8, 0x41, // amount = 25.0
];
const ENLARGECANVAS_V1_NATIVE_LE: &[u8; ENLARGECANVAS_PARAMETER_BYTES] =
    include_bytes!("../../../fixtures/corpus/assets/operation-enlargecanvas-params-v1.bin");

#[test]
fn agx_v7_and_legacy_rows_materialize_canonical_typed_parameters() {
    let parameters = AgxParametersV7::defaults();
    let source = history_step(b"agx", Some(7), Some(1), parameters.to_bytes().to_vec());
    let DarktableHistoryStepDecode::AgxPendingBlend(imported) = decode_history_step(&source) else {
        panic!("canonical AgX v7 row must decode as typed pending blend");
    };
    assert_eq!(
        imported.config,
        AgxConfig::new(parameters).expect("defaults")
    );
    assert_eq!(imported.canonical_parameters, parameters.to_bytes());
    assert!(!imported.migrated);
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );

    let legacy = history_step(b"agx", Some(6), Some(0), vec![0xde, 0xad]);
    let DarktableHistoryStepDecode::AgxPendingBlend(imported) = decode_history_step(&legacy) else {
        panic!("pre-v7 AgX row must use the native scene-referred reset");
    };
    let expected = AgxParametersV7::scene_referred_defaults();
    assert_eq!(imported.canonical_parameters, expected.to_bytes());
    assert!(imported.migrated);
    assert!(!imported.enabled);

    let future = history_step(b"agx", Some(8), Some(1), vec![1, 2, 3]);
    assert_preserved_exact(
        &future,
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );
}

#[test]
fn levels_v1_and_v2_rows_materialize_the_exact_28_byte_current_abi() {
    let v1 = LevelsParametersV1::new([0.125, 0.5, 0.875], 7);
    let source = history_step(b"levels", Some(1), Some(1), v1.to_bytes().to_vec());
    let DarktableHistoryStepDecode::LevelsPendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("Levels v1 row must decode and migrate");
    };
    let expected =
        LevelsParametersV2::new(LevelsMode::Manual, 0.0, 50.0, 100.0, [0.125, 0.5, 0.875]);
    assert_eq!(imported.canonical_parameters, expected.to_bytes());
    assert_eq!(
        imported.config,
        LevelsConfig::new(expected).expect("levels")
    );
    assert!(imported.migrated);

    let v2 = LevelsParametersV2::defaults();
    let source = history_step(b"levels", Some(2), Some(0), v2.to_bytes().to_vec());
    let DarktableHistoryStepDecode::LevelsPendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("Levels v2 row must decode");
    };
    assert_eq!(imported.canonical_parameters.len(), 28);
    assert!(!imported.migrated);

    let stale_opaque = history_step(b"levels", Some(3), Some(1), vec![0; 264]);
    assert_preserved_exact(
        &stale_opaque,
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );
}

#[test]
fn rgblevels_v1_materializes_the_exact_44_byte_native_payload() {
    let parameters = RgbLevelsParametersV1::defaults();
    let source = history_step(
        b"rgblevels",
        Some(1),
        Some(1),
        parameters.to_bytes().to_vec(),
    );
    let DarktableHistoryStepDecode::RgbLevelsPendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("RGB Levels v1 row must decode as typed pending blend");
    };
    assert_eq!(imported.canonical_parameters.len(), 44);
    assert_eq!(imported.canonical_parameters, parameters.to_bytes());
    assert_eq!(
        imported.config,
        RgbLevelsConfig::new(parameters).expect("RGB Levels defaults")
    );
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );

    let stale_20_byte = history_step(b"rgblevels", Some(1), Some(1), vec![0; 20]);
    assert_preserved_exact(
        &stale_20_byte,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );
}

#[test]
fn colormapping_v1_materializes_the_exact_16600_byte_native_payload() {
    let parameters = ColorMappingParametersV1::defaults();
    let payload = parameters.to_bytes();
    let source = history_step(b"colormapping", Some(1), Some(1), payload.clone());
    let DarktableHistoryStepDecode::ColorMappingPendingBlend(imported) =
        decode_history_step(&source)
    else {
        panic!("Color Mapping v1 row must decode as typed pending blend");
    };
    assert_eq!(
        imported.canonical_parameters.len(),
        COLOR_MAPPING_PARAMETER_BYTES
    );
    assert_eq!(imported.canonical_parameters, payload);
    assert_eq!(
        *imported.config,
        ColorMappingConfig::new(parameters).expect("Color Mapping defaults")
    );
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );

    let malformed = history_step(b"colormapping", Some(1), Some(1), vec![0; 332]);
    assert_preserved_exact(
        &malformed,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );
    let future = history_step(b"colormapping", Some(2), Some(1), vec![1, 2, 3]);
    assert_preserved_exact(
        &future,
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );
}

#[test]
fn colortransfer_v1_materializes_the_exact_8280_byte_native_payload() {
    let parameters = ColorTransferParameters::default();
    let payload = parameters.to_bytes();
    let source = history_step(b"colortransfer", Some(1), Some(0), payload.clone());
    let DarktableHistoryStepDecode::ColorTransferPendingRuntime(imported) =
        decode_history_step(&source)
    else {
        panic!("Color Transfer v1 row must decode as typed pending runtime state");
    };
    assert_eq!(
        imported.canonical_parameters.len(),
        COLORTRANSFER_NATIVE_PARAMETER_BYTES
    );
    assert_eq!(imported.canonical_parameters, payload);
    assert_eq!(*imported.parameters, parameters);
    assert!(!imported.enabled);
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::DeferredRuntimeState
    );

    let stale = history_step(b"colortransfer", Some(1), Some(1), vec![0; 92]);
    assert_preserved_exact(
        &stale,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );
    let future = history_step(b"colortransfer", Some(2), Some(1), vec![1, 2, 3]);
    assert_preserved_exact(
        &future,
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );
}

#[test]
fn bloom_v1_decodes_source_ordered_little_endian_fields_but_remains_pending_blend() {
    assert_eq!(
        BLOOM_V1_DEFAULT_NATIVE_LE,
        &[
            0x00, 0x00, 0xa0, 0x41, // size = 20.0
            0x00, 0x00, 0xb4, 0x42, // threshold = 90.0
            0x00, 0x00, 0xc8, 0x41, // strength = 25.0
        ]
    );
    let payload = BLOOM_V1_DEFAULT_NATIVE_LE.to_vec();
    let source = history_step(b"bloom", Some(1), Some(0), payload.clone());

    let DarktableHistoryStepDecode::BloomPendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("canonical Bloom v1 row must decode as typed pending blend");
    };

    assert_eq!(imported.source, source);
    assert_eq!(imported.source.operation.raw_name, b"bloom");
    assert_eq!(imported.source.operation.name.as_deref(), Some("bloom"));
    assert_eq!(imported.source.operation_params.bytes, payload);
    assert_eq!(imported.source.blend_params.bytes, [0xaa, 0xbb]);
    assert_eq!(imported.source_version, 1);
    assert!(!imported.enabled);
    assert_eq!(imported.canonical_parameters, *BLOOM_V1_DEFAULT_NATIVE_LE);
    assert_eq!(imported.config, BloomConfig::defaults());
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert!(imported.execution_blocker.detail.contains("blend/mask"));
    assert!(imported.execution_blocker.detail.contains("multi-instance"));
}

#[test]
fn bloom_unknown_malformed_and_nonfinite_payloads_remain_byte_exact() {
    let unknown = history_step(b"bloom", Some(2), Some(1), vec![9, 8, 7]);
    assert_preserved_exact(
        &unknown,
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );

    let malformed = history_step(
        b"bloom",
        Some(1),
        Some(1),
        vec![0; BLOOM_PARAMETER_BYTES - 1],
    );
    assert_preserved_exact(
        &malformed,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    let nonfinite = history_step(
        b"bloom",
        Some(1),
        Some(1),
        BloomParametersV1::new(20.0, f32::NAN, 25.0)
            .to_bytes()
            .to_vec(),
    );
    assert_preserved_exact(
        &nonfinite,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );
}

#[test]
fn colorcorrection_v1_decodes_exact_checked_payload_but_remains_pending_blend() {
    let payload = COLORCORRECTION_V1_BENCHMARK_LE.to_vec();
    let source = history_step(b"colorcorrection", Some(1), Some(1), payload.clone());

    let DarktableHistoryStepDecode::ColorCorrectionPendingBlend(imported) =
        decode_history_step(&source)
    else {
        panic!("known Color Correction v1 row must decode");
    };

    assert_eq!(imported.source.operation_params.bytes, payload);
    assert_eq!(imported.source_version, 1);
    assert!(imported.enabled);
    assert_eq!(
        imported.canonical_parameters,
        COLORCORRECTION_V1_BENCHMARK_LE
    );
    assert_eq!(
        imported.config,
        ColorCorrectionConfig::new(
            f32::from_bits(0x409a_3b66),
            f32::from_bits(0x404c_5331),
            f32::from_bits(0xc095_047f),
            f32::from_bits(0xc07e_720a),
            1.0,
        )
        .expect("checked finite native parameters")
    );
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert_eq!(imported.source.blend_params.bytes, [0xaa, 0xbb]);
}

#[test]
fn colorcorrection_finite_outlier_decodes_while_unknown_malformed_and_nonfinite_stay_exact() {
    let outlier = ColorCorrectionParametersV1::new(400.0, -500.0, 600.0, -700.0, 8.0);
    let source = history_step(
        b"colorcorrection",
        Some(1),
        Some(0),
        outlier.to_bytes().to_vec(),
    );
    let DarktableHistoryStepDecode::ColorCorrectionPendingBlend(imported) =
        decode_history_step(&source)
    else {
        panic!("native commit_params accepts finite persisted values outside UI bounds");
    };
    assert!(!imported.enabled);
    assert_eq!(
        imported.config,
        ColorCorrectionConfig::try_from(outlier).expect("finite native parameters")
    );
    assert_eq!(imported.canonical_parameters, outlier.to_bytes());

    assert_preserved(
        &history_step(b"colorcorrection", Some(9), Some(1), vec![9, 8, 7]),
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
        &[9, 8, 7],
    );
    assert_preserved(
        &history_step(b"colorcorrection", Some(1), Some(1), vec![1, 2, 3]),
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        &[1, 2, 3],
    );
    let nonfinite = ColorCorrectionParametersV1::new(f32::NAN, 0.0, 0.0, 0.0, 1.0)
        .to_bytes()
        .to_vec();
    assert_preserved(
        &history_step(b"colorcorrection", Some(1), Some(1), nonfinite.clone()),
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        &nonfinite,
    );
}

#[test]
fn colorreconstruct_v1_uses_the_exact_three_float_layout_and_migrates_directly_to_v3() {
    assert_eq!(
        COLORRECONSTRUCT_V1_NATIVE_LE,
        &[
            0x00, 0x00, 0xf7, 0x42, // threshold = 123.5
            0x00, 0xa0, 0xa0, 0x43, // spatial = 321.25
            0x00, 0x00, 0x4c, 0x41, // range = 12.75
        ]
    );
    let source = history_step(
        b"colorreconstruct",
        Some(1),
        Some(1),
        COLORRECONSTRUCT_V1_NATIVE_LE.to_vec(),
    );

    let DarktableHistoryStepDecode::ColorReconstructionPendingBlend(imported) =
        decode_history_step(&source)
    else {
        panic!("known Color Reconstruction v1 row must decode");
    };

    let mut expected_canonical = [0_u8; 20];
    expected_canonical[..12].copy_from_slice(COLORRECONSTRUCT_V1_NATIVE_LE);
    expected_canonical[12..16].copy_from_slice(&0.66_f32.to_le_bytes());
    expected_canonical[16..20].copy_from_slice(&0_i32.to_le_bytes());
    assert_eq!(imported.source, source);
    assert_eq!(imported.source.operation.raw_name, b"colorreconstruct");
    assert_eq!(
        imported.source.operation.name.as_deref(),
        Some("colorreconstruct")
    );
    assert_eq!(imported.source_version, 1);
    assert!(imported.migrated);
    assert!(imported.enabled);
    assert_eq!(imported.canonical_parameters, expected_canonical);
    assert_eq!(
        imported.config,
        ColorReconstructionConfig::new(
            123.5,
            321.25,
            12.75,
            0.66,
            ColorReconstructionPrecedence::None,
        )
        .expect("migrated v1 parameters")
    );
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert!(imported.execution_blocker.detail.contains("blend/mask"));
    assert!(imported.execution_blocker.detail.contains("multi-instance"));
    assert_eq!(imported.source.blend_params.bytes, [0xaa, 0xbb]);
}

#[test]
fn colorreconstruct_v2_preserves_precedence_and_appends_hue_in_v3_field_order() {
    assert_eq!(
        COLORRECONSTRUCT_V2_NATIVE_LE,
        &[
            0x00, 0x80, 0x9a, 0x42, // threshold = 77.25
            0x00, 0x20, 0x20, 0x44, // spatial = 640.5
            0x00, 0x00, 0x02, 0x41, // range = 8.125
            0x02, 0x00, 0x00, 0x00, // precedence = hue
        ]
    );
    let source = history_step(
        b"colorreconstruct",
        Some(2),
        Some(0),
        COLORRECONSTRUCT_V2_NATIVE_LE.to_vec(),
    );

    let DarktableHistoryStepDecode::ColorReconstructionPendingBlend(imported) =
        decode_history_step(&source)
    else {
        panic!("known Color Reconstruction v2 row must decode");
    };

    let mut expected_canonical = [0_u8; 20];
    expected_canonical[..12].copy_from_slice(&COLORRECONSTRUCT_V2_NATIVE_LE[..12]);
    expected_canonical[12..16].copy_from_slice(&0.66_f32.to_le_bytes());
    expected_canonical[16..20].copy_from_slice(&2_i32.to_le_bytes());
    assert_eq!(
        imported.source.operation_params.bytes,
        *COLORRECONSTRUCT_V2_NATIVE_LE
    );
    assert_eq!(imported.source_version, 2);
    assert!(imported.migrated);
    assert!(!imported.enabled);
    assert_eq!(imported.canonical_parameters, expected_canonical);
    assert_eq!(
        imported.config,
        ColorReconstructionConfig::new(
            77.25,
            640.5,
            8.125,
            0.66,
            ColorReconstructionPrecedence::Hue,
        )
        .expect("migrated v2 parameters")
    );
}

#[test]
fn colorreconstruct_v3_decodes_threshold_spatial_range_hue_then_precedence() {
    assert_eq!(
        COLORRECONSTRUCT_V3_NATIVE_LE,
        &[
            0x00, 0x80, 0xdb, 0x42, // threshold = 109.75
            0x00, 0x40, 0xe4, 0x43, // spatial = 456.5
            0x00, 0x00, 0xb2, 0x41, // range = 22.25
            0x00, 0x00, 0xc0, 0x3e, // hue = 0.375
            0x01, 0x00, 0x00, 0x00, // precedence = saturated colors
        ]
    );
    let source = history_step(
        b"colorreconstruct",
        Some(3),
        Some(1),
        COLORRECONSTRUCT_V3_NATIVE_LE.to_vec(),
    );

    let DarktableHistoryStepDecode::ColorReconstructionPendingBlend(imported) =
        decode_history_step(&source)
    else {
        panic!("known Color Reconstruction v3 row must decode");
    };

    assert_eq!(imported.source, source);
    assert_eq!(imported.source_version, 3);
    assert!(!imported.migrated);
    assert!(imported.enabled);
    assert_eq!(
        imported.canonical_parameters,
        *COLORRECONSTRUCT_V3_NATIVE_LE
    );
    assert_eq!(
        imported.config,
        ColorReconstructionConfig::new(
            109.75,
            456.5,
            22.25,
            0.375,
            ColorReconstructionPrecedence::Chroma,
        )
        .expect("native v3 parameters")
    );
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
}

#[test]
fn colorreconstruct_finite_outliers_decode_for_every_native_version() {
    let cases = [
        (
            1,
            [200.0_f32, 2_000.0, 75.0]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .collect::<Vec<_>>(),
        ),
        (
            2,
            [200.0_f32, 2_000.0, 75.0]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .chain(1_i32.to_le_bytes())
                .collect::<Vec<_>>(),
        ),
        (
            3,
            [200.0_f32, 2_000.0, 75.0, -0.25]
                .into_iter()
                .flat_map(f32::to_le_bytes)
                .chain(2_i32.to_le_bytes())
                .collect::<Vec<_>>(),
        ),
    ];

    for (version, payload) in cases {
        let source = history_step(b"colorreconstruct", Some(version), Some(1), payload);
        let DarktableHistoryStepDecode::ColorReconstructionPendingBlend(imported) =
            decode_history_step(&source)
        else {
            panic!("finite Color Reconstruction v{version} outlier must decode");
        };
        assert_eq!(
            imported.config.threshold().get().to_bits(),
            200.0_f32.to_bits()
        );
        assert_eq!(
            imported.config.spatial().get().to_bits(),
            2_000.0_f32.to_bits()
        );
        assert_eq!(imported.config.range().get().to_bits(), 75.0_f32.to_bits());
        assert_eq!(
            imported.config.hue().get().to_bits(),
            if version < 3 { 0.66_f32 } else { -0.25_f32 }.to_bits()
        );
    }
}

#[test]
fn colorreconstruct_unknown_malformed_and_invalid_payloads_remain_byte_exact() {
    let unknown = history_step(b"colorreconstruct", Some(4), Some(1), vec![9, 8, 7]);
    assert_preserved_exact(
        &unknown,
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );

    for (version, malformed) in [
        (1, vec![0; COLORRECONSTRUCT_V1_NATIVE_LE.len() - 1]),
        (2, vec![0; COLORRECONSTRUCT_V2_NATIVE_LE.len() + 1]),
        (3, vec![0; COLORRECONSTRUCT_V3_NATIVE_LE.len() - 1]),
    ] {
        let source = history_step(b"colorreconstruct", Some(version), Some(1), malformed);
        assert_preserved_exact(
            &source,
            DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        );
    }

    let mut nonfinite = *COLORRECONSTRUCT_V3_NATIVE_LE;
    nonfinite[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    let nonfinite = history_step(b"colorreconstruct", Some(3), Some(1), nonfinite.to_vec());
    assert_preserved_exact(
        &nonfinite,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    let mut unknown_precedence = *COLORRECONSTRUCT_V3_NATIVE_LE;
    unknown_precedence[16..20].copy_from_slice(&99_i32.to_le_bytes());
    let unknown_precedence = history_step(
        b"colorreconstruct",
        Some(3),
        Some(1),
        unknown_precedence.to_vec(),
    );
    assert_preserved_exact(
        &unknown_precedence,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );
}

#[test]
fn crop_v3_decodes_exact_24_byte_native_field_order_pending_blend() {
    let parameters = CropParametersV3::new(
        CropConfig::new(0.125, 0.25, 0.875, 0.75, 3, -2).expect("finite crop parameters"),
    );
    let payload = vec![
        0x00, 0x00, 0x00, 0x3e, // cx = 0.125
        0x00, 0x00, 0x80, 0x3e, // cy = 0.25
        0x00, 0x00, 0x60, 0x3f, // cw = 0.875
        0x00, 0x00, 0x40, 0x3f, // ch = 0.75
        0x03, 0x00, 0x00, 0x00, // ratio_n = 3
        0xfe, 0xff, 0xff, 0xff, // ratio_d = -2
    ];
    assert_eq!(CROP_PARAMETER_BYTES, 24);
    assert_eq!(parameters.to_bytes().as_slice(), payload.as_slice());
    let source = history_step(b"crop", Some(3), Some(1), payload.clone());

    let DarktableHistoryStepDecode::CropPendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("known Crop v3 row must decode as typed pending blend");
    };

    assert_eq!(imported.source, source);
    assert_eq!(imported.source_version, 3);
    assert!(imported.enabled);
    assert!(!imported.migrated);
    assert_eq!(imported.source.operation_params.bytes, payload);
    assert_eq!(imported.canonical_parameters, parameters.to_bytes());
    assert_eq!(imported.config, parameters.config());
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert!(imported.execution_blocker.detail.contains("blend/mask"));
    assert!(imported.execution_blocker.detail.contains("multi-instance"));
}

#[test]
fn crop_legacy_rows_use_native_sizes_and_remain_byte_exact_without_image_context() {
    let v1 = CropLegacyParametersV1 {
        cx: 0.125,
        cy: 0.25,
        cw: 0.875,
        ch: 0.75,
        ratio_n: 3,
        ratio_d: -2,
    };
    assert_eq!(CROP_LEGACY_V1_BYTES, 24);
    assert_preserved_exact(
        &history_step(b"crop", Some(1), Some(1), v1.to_bytes().to_vec()),
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );

    let v2 = CropLegacyParametersV2 {
        cx: v1.cx,
        cy: v1.cy,
        cw: v1.cw,
        ch: v1.ch,
        ratio_n: v1.ratio_n,
        ratio_d: v1.ratio_d,
        aligned: 1,
    };
    assert_eq!(CROP_LEGACY_V2_BYTES, 28);
    assert_eq!(&v2.to_bytes()[..24], v1.to_bytes().as_slice());
    assert_eq!(&v2.to_bytes()[24..], &1_i32.to_le_bytes());
    assert_preserved_exact(
        &history_step(b"crop", Some(2), Some(1), v2.to_bytes().to_vec()),
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );
}

#[test]
fn crop_malformed_and_nonfinite_rows_remain_byte_exact() {
    for (version, payload) in [
        (1, vec![0; CROP_LEGACY_V1_BYTES - 1]),
        (2, vec![0; CROP_LEGACY_V2_BYTES + 1]),
        (3, vec![0; CROP_PARAMETER_BYTES - 1]),
    ] {
        assert_preserved_exact(
            &history_step(b"crop", Some(version), Some(1), payload),
            DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        );
    }

    for (version, length) in [
        (1, CROP_LEGACY_V1_BYTES),
        (2, CROP_LEGACY_V2_BYTES),
        (3, CROP_PARAMETER_BYTES),
    ] {
        let mut nonfinite = vec![0_u8; length];
        nonfinite[..4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert_preserved_exact(
            &history_step(b"crop", Some(version), Some(1), nonfinite),
            DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        );
    }
}

#[test]
fn enlargecanvas_v1_decodes_exact_four_float_and_native_color_layout_pending_blend() {
    assert_eq!(
        ENLARGECANVAS_V1_NATIVE_LE,
        &[
            0x00, 0x00, 0x48, 0x41, // percent_left = 12.5
            0x00, 0x00, 0xc8, 0x41, // percent_right = 25.0
            0x00, 0x00, 0x48, 0x42, // percent_top = 50.0
            0x00, 0x00, 0x00, 0x00, // percent_bottom = 0.0
            0x02, 0x00, 0x00, 0x00, // color = blue
        ]
    );
    let parameters = EnlargeCanvasParametersV1::from_bytes(ENLARGECANVAS_V1_NATIVE_LE)
        .expect("source-sized v1 Enlarge Canvas payload");
    let source = history_step(
        b"enlargecanvas",
        Some(1),
        Some(1),
        ENLARGECANVAS_V1_NATIVE_LE.to_vec(),
    );

    let DarktableHistoryStepDecode::EnlargeCanvasPendingBlend(imported) =
        decode_history_step(&source)
    else {
        panic!("known Enlarge Canvas v1 row must decode as typed pending blend");
    };

    assert_eq!(imported.source, source);
    assert_eq!(imported.source.operation.raw_name, b"enlargecanvas");
    assert_eq!(
        imported.source.operation.name.as_deref(),
        Some("enlargecanvas")
    );
    assert_eq!(imported.source_version, 1);
    assert!(imported.enabled);
    assert_eq!(imported.canonical_parameters, *ENLARGECANVAS_V1_NATIVE_LE);
    assert_eq!(imported.config, parameters.config());
    assert_eq!(
        imported.config.percent_left().get().to_bits(),
        12.5_f32.to_bits()
    );
    assert_eq!(
        imported.config.percent_right().get().to_bits(),
        25.0_f32.to_bits()
    );
    assert_eq!(
        imported.config.percent_top().get().to_bits(),
        50.0_f32.to_bits()
    );
    assert_eq!(
        imported.config.percent_bottom().get().to_bits(),
        0.0_f32.to_bits()
    );
    assert_eq!(imported.config.color(), CanvasColor::Blue);
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert!(imported.execution_blocker.detail.contains("blend/mask"));
    assert!(imported.execution_blocker.detail.contains("multi-instance"));
    assert_eq!(imported.source.blend_params.bytes, [0xaa, 0xbb]);
    assert_eq!(imported.source.multi_name.bytes, b"base");
}

#[test]
fn enlargecanvas_unknown_malformed_nonfinite_color_and_invalid_enabled_rows_remain_exact() {
    let unknown = history_step(b"enlargecanvas", Some(2), Some(1), vec![9, 8, 7]);
    assert_preserved_exact(
        &unknown,
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );

    let malformed = history_step(
        b"enlargecanvas",
        Some(1),
        Some(1),
        ENLARGECANVAS_V1_NATIVE_LE[..ENLARGECANVAS_PARAMETER_BYTES - 1].to_vec(),
    );
    assert_preserved_exact(
        &malformed,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    let mut nonfinite = *ENLARGECANVAS_V1_NATIVE_LE;
    nonfinite[..4].copy_from_slice(&f32::NAN.to_le_bytes());
    let nonfinite = history_step(b"enlargecanvas", Some(1), Some(1), nonfinite.to_vec());
    assert_preserved_exact(
        &nonfinite,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    let mut unknown_color = *ENLARGECANVAS_V1_NATIVE_LE;
    unknown_color[16..20].copy_from_slice(&99_u32.to_le_bytes());
    let unknown_color = history_step(b"enlargecanvas", Some(1), Some(1), unknown_color.to_vec());
    assert_preserved_exact(
        &unknown_color,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    for (offset, value) in [(0, -1.0_f32), (4, 101.0_f32)] {
        let mut invalid_percent = *ENLARGECANVAS_V1_NATIVE_LE;
        invalid_percent[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        let invalid_percent =
            history_step(b"enlargecanvas", Some(1), Some(1), invalid_percent.to_vec());
        assert_preserved_exact(
            &invalid_percent,
            DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        );
    }

    let invalid_enabled = history_step(
        b"enlargecanvas",
        Some(1),
        Some(7),
        ENLARGECANVAS_V1_NATIVE_LE.to_vec(),
    );
    assert_preserved_exact(
        &invalid_enabled,
        DarktableHistoryDecodeFindingCode::InvalidEnabledState,
    );
}

#[test]
fn colorzones_v1_migrates_to_canonical_v5_but_retains_the_exact_pending_row() {
    let payload = colorzones_v1_payload();
    let source = history_step(b"colorzones", Some(1), Some(1), payload.clone());
    let current = ColorZonesHistory::decode(1, &payload)
        .expect("source-sized v1 Color Zones payload")
        .current()
        .expect("pinned v1-to-v5 migration");
    let expected_config =
        ColorZonesConfig::try_from(&current).expect("migrated active curve semantics");

    let DarktableHistoryStepDecode::ColorZonesPendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("known Color Zones v1 row must decode as pending blend");
    };

    assert_eq!(imported.source, source);
    assert_eq!(imported.source.operation_params.bytes, payload);
    assert_eq!(imported.source.blend_params.bytes, [0xaa, 0xbb]);
    assert_eq!(imported.source_version, 1);
    assert!(imported.migrated);
    assert!(imported.enabled);
    assert_eq!(imported.config, expected_config);
    assert_eq!(imported.canonical_parameters, current.to_bytes());
    assert_eq!(
        imported.canonical_parameters.len(),
        COLORZONES_V5_PARAMETER_BYTES
    );
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
}

#[test]
fn colorzones_v5_is_canonical_without_claiming_executable_blend_semantics() {
    let parameters = ColorZonesParametersV5::defaults();
    let payload = parameters.to_bytes().to_vec();
    let source = history_step(b"colorzones", Some(5), Some(0), payload.clone());

    let DarktableHistoryStepDecode::ColorZonesPendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("known Color Zones v5 row must decode as pending blend");
    };

    assert_eq!(imported.source, source);
    assert_eq!(imported.source.operation_params.bytes, payload);
    assert_eq!(imported.source.blend_params.bytes, [0xaa, 0xbb]);
    assert_eq!(imported.source_version, 5);
    assert!(!imported.migrated);
    assert!(!imported.enabled);
    assert_eq!(
        imported.config,
        ColorZonesConfig::try_from(&parameters).expect("default v5 active curve semantics")
    );
    assert_eq!(imported.canonical_parameters, parameters.to_bytes());
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
}

#[test]
fn colorzones_v5_spline_v2_single_node_curves_remain_pending_blend() {
    let mut parameters = ColorZonesParametersV5::defaults();
    parameters.curve_num_nodes = [1, 1, 1];
    let payload = parameters.to_bytes().to_vec();
    let source = history_step(b"colorzones", Some(5), Some(1), payload.clone());

    let DarktableHistoryStepDecode::ColorZonesPendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("native spline-v2 one-node curves must remain typed pending blend");
    };

    assert_eq!(imported.source, source);
    assert_eq!(imported.source.operation_params.bytes, payload);
    assert!(
        imported
            .config
            .curves()
            .iter()
            .all(|curve| curve.node_count() == 1)
    );
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
}

#[test]
fn colorzones_unknown_malformed_and_invalid_active_semantics_remain_byte_exact() {
    let unknown = history_step(b"colorzones", Some(6), Some(1), vec![9, 8, 7]);
    assert_preserved_exact(
        &unknown,
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );

    let malformed = history_step(
        b"colorzones",
        Some(5),
        Some(1),
        vec![0; COLORZONES_V5_PARAMETER_BYTES - 1],
    );
    assert_preserved_exact(
        &malformed,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    let mut invalid_active = ColorZonesParametersV5::defaults().to_bytes();
    invalid_active[..4].copy_from_slice(&99_i32.to_le_bytes());
    let invalid_active = history_step(b"colorzones", Some(5), Some(1), invalid_active.to_vec());
    assert_preserved_exact(
        &invalid_active,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );
}

#[test]
fn colorcontrast_v1_decodes_exact_bounded_migration_but_remains_pending_blend() {
    let payload = COLORCONTRAST_V1_NATIVE_LE.to_vec();
    let source = history_step(b"colorcontrast", Some(1), Some(1), payload.clone());

    let DarktableHistoryStepDecode::ColorContrastPendingBlend(imported) =
        decode_history_step(&source)
    else {
        panic!("known Color Contrast v1 row must decode");
    };

    assert_eq!(imported.source.operation_params.bytes, payload);
    assert_eq!(imported.source_version, 1);
    assert!(imported.migrated);
    assert_eq!(
        imported.canonical_parameters,
        [
            0x00, 0x00, 0xc0, 0x3f, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x40, 0x3f, 0x00, 0x00,
            0x80, 0x40, 0x00, 0x00, 0x00, 0x00,
        ]
    );
    assert!(imported.enabled);
    assert_eq!(
        imported.config,
        ColorContrastConfig::new(1.5, -2.0, 0.75, 4.0, 0).expect("migrated config")
    );
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert_eq!(imported.source.blend_params.bytes, [0xaa, 0xbb]);
}

#[test]
fn colorcontrast_v2_retains_hidden_offsets_raw_unbound_and_disabled_state() {
    let payload = COLORCONTRAST_V2_NATIVE_LE.to_vec();
    let source = history_step(b"colorcontrast", Some(2), Some(0), payload.clone());

    let DarktableHistoryStepDecode::ColorContrastPendingBlend(decoded) =
        decode_history_step(&source)
    else {
        panic!("known Color Contrast v2 row must decode");
    };
    assert_eq!(decoded.source.operation_params.bytes, payload);
    assert_eq!(decoded.source_version, 2);
    assert!(!decoded.migrated);
    assert!(!decoded.enabled);
    assert_eq!(decoded.canonical_parameters.as_slice(), payload.as_slice());
    assert_eq!(
        decoded.config,
        ColorContrastConfig::new(8.0, -300.0, -2.0, 400.0, -7).expect("finite native parameters")
    );
    assert_eq!(decoded.config.unbound(), -7);
}

#[test]
fn colorcontrast_unknown_malformed_and_nonfinite_payloads_remain_exact() {
    assert_preserved(
        &history_step(b"colorcontrast", Some(9), Some(1), vec![9, 8, 7]),
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
        &[9, 8, 7],
    );
    assert_preserved(
        &history_step(b"colorcontrast", Some(2), Some(1), vec![1, 2, 3]),
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        &[1, 2, 3],
    );

    let nonfinite = ColorContrastParametersV2::new(f32::NAN, 0.0, 1.0, 0.0, 1)
        .to_bytes()
        .to_vec();
    assert_preserved(
        &history_step(b"colorcontrast", Some(2), Some(1), nonfinite.clone()),
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        &nonfinite,
    );
}

#[test]
fn velvia_v1_decodes_exact_migration_but_remains_pending_blend() {
    let payload = VelviaParametersV1::new(50.0, 80.0, 0.25, 0.75)
        .to_bytes()
        .to_vec();
    let source = history_step(b"velvia", Some(1), Some(1), payload.clone());

    let DarktableHistoryStepDecode::VelviaPendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("known Velvia v1 row must decode");
    };

    assert_eq!(imported.source.operation_params.bytes, payload);
    assert_eq!(imported.source_version, 1);
    assert!(imported.migrated);
    assert_eq!(
        imported.canonical_parameters,
        VelviaParametersV2::new(40.0, 0.25).to_bytes()
    );
    assert!(imported.enabled);
    assert_eq!(
        imported.config,
        VelviaConfig::new(40.0, 0.25).expect("migrated config")
    );
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert_eq!(imported.source.blend_params.bytes, [0xaa, 0xbb]);
}

#[test]
fn velvia_v2_decoding_is_stable_and_respects_disabled_state() {
    let payload = VelviaParametersV2::new(25.0, 1.0).to_bytes().to_vec();
    let source = history_step(b"velvia", Some(2), Some(0), payload.clone());

    let DarktableHistoryStepDecode::VelviaPendingBlend(first) = decode_history_step(&source) else {
        panic!("known Velvia v2 row must decode");
    };
    let DarktableHistoryStepDecode::VelviaPendingBlend(second) = decode_history_step(&source)
    else {
        panic!("known Velvia v2 row must decode deterministically");
    };

    assert_eq!(first.source.operation_params.bytes, payload);
    assert_eq!(first.source_version, 2);
    assert!(!first.migrated);
    assert!(!first.enabled);
    assert_eq!(first.config, VelviaConfig::defaults());
    assert_eq!(first.canonical_parameters, second.canonical_parameters);
    assert_eq!(first.execution_blocker, second.execution_blocker);
}

#[test]
fn finite_persisted_values_outside_ui_bounds_still_decode() {
    let parameters = VelviaParametersV2::new(125.0, -0.25);
    let source = history_step(b"velvia", Some(2), Some(1), parameters.to_bytes().to_vec());

    let DarktableHistoryStepDecode::VelviaPendingBlend(decoded) = decode_history_step(&source)
    else {
        panic!("native commit_params accepts every finite value");
    };
    assert_eq!(
        decoded.config,
        VelviaConfig::new(125.0, -0.25).expect("finite native parameters")
    );
    assert_eq!(decoded.canonical_parameters, parameters.to_bytes());
}

#[test]
fn unknown_version_and_malformed_or_nonfinite_payloads_remain_exact() {
    assert_preserved(
        &history_step(b"velvia", Some(9), Some(1), vec![9, 8, 7]),
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
        &[9, 8, 7],
    );
    assert_preserved(
        &history_step(b"velvia", Some(2), Some(1), vec![1, 2, 3]),
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        &[1, 2, 3],
    );

    let mut nonfinite = Vec::new();
    nonfinite.extend_from_slice(&f32::NAN.to_le_bytes());
    nonfinite.extend_from_slice(&1.0_f32.to_le_bytes());
    assert_preserved(
        &history_step(b"velvia", Some(2), Some(1), nonfinite.clone()),
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        &nonfinite,
    );
}

#[test]
fn vibrance_v2_decodes_exact_native_payload_but_remains_pending_blend() {
    let payload = VIBRANCE_V2_NATIVE_LE.to_vec();
    let source = history_step(b"vibrance", Some(2), Some(0), payload.clone());

    let DarktableHistoryStepDecode::VibrancePendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("known Vibrance v2 row must decode");
    };

    assert_eq!(imported.source.operation_params.bytes, payload);
    assert_eq!(imported.source_version, 2);
    assert!(!imported.enabled);
    assert_eq!(imported.canonical_parameters, VIBRANCE_V2_NATIVE_LE);
    assert_eq!(
        imported.config,
        VibranceConfig::new(25.0).expect("native default amount")
    );
    assert_eq!(
        imported.execution_blocker.code,
        DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert_eq!(imported.source.blend_params.bytes, [0xaa, 0xbb]);
}

#[test]
fn vibrance_finite_outlier_decodes_while_unknown_malformed_and_nonfinite_stay_exact() {
    let outlier = VibranceParametersV2::new(125.0).to_bytes().to_vec();
    let source = history_step(b"vibrance", Some(2), Some(1), outlier.clone());
    let DarktableHistoryStepDecode::VibrancePendingBlend(imported) = decode_history_step(&source)
    else {
        panic!("native commit_params accepts finite persisted values outside UI bounds");
    };
    assert_eq!(
        imported.config,
        VibranceConfig::new(125.0).expect("finite native amount")
    );
    assert_eq!(imported.canonical_parameters.as_slice(), outlier.as_slice());

    assert_preserved(
        &history_step(b"vibrance", Some(9), Some(1), vec![9, 8, 7]),
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
        &[9, 8, 7],
    );
    assert_preserved(
        &history_step(b"vibrance", Some(2), Some(1), vec![1, 2, 3]),
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        &[1, 2, 3],
    );
    let nonfinite = f32::NAN.to_le_bytes().to_vec();
    assert_preserved(
        &history_step(b"vibrance", Some(2), Some(1), nonfinite.clone()),
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        &nonfinite,
    );
}

#[test]
fn unsupported_operation_and_invalid_row_state_remain_exact() {
    assert_preserved(
        &history_step(b"future", Some(2), Some(1), vec![4, 5, 6]),
        DarktableHistoryDecodeFindingCode::UnsupportedOperation,
        &[4, 5, 6],
    );
    assert_preserved(
        &history_step(
            b"velvia",
            None,
            Some(1),
            VelviaParametersV2::new(25.0, 1.0).to_bytes().to_vec(),
        ),
        DarktableHistoryDecodeFindingCode::MissingModuleVersion,
        &VelviaParametersV2::new(25.0, 1.0).to_bytes(),
    );
    assert_preserved(
        &history_step(
            b"velvia",
            Some(2),
            Some(7),
            VelviaParametersV2::new(25.0, 1.0).to_bytes().to_vec(),
        ),
        DarktableHistoryDecodeFindingCode::InvalidEnabledState,
        &VelviaParametersV2::new(25.0, 1.0).to_bytes(),
    );
}

#[test]
fn redo_step_decoding_retains_nonselected_source_state() {
    let payload = VelviaParametersV2::defaults().to_bytes().to_vec();
    let histories = HistoryDecoder::new(HistoryDecodeOptions {
        limits: HistoryLimits::default(),
        manifest: manifest(),
    })
    .decode(
        DarktableSchema::new(57, 13),
        HistoryRows {
            history: vec![
                raw_history_row(41, 0, b"velvia", Some(2), Some(1), payload),
                raw_history_row(42, 1, b"future", Some(1), Some(1), vec![9, 8, 7]),
            ],
            images: vec![RawImageHistoryRow {
                source_row: 11,
                image_id: 7,
                history_end: Some(1),
            }],
            ..HistoryRows::default()
        },
    );
    let history = &histories[0];
    assert_eq!(history.selection.selected_rows.len(), 1);
    assert_eq!(history.selection.redo_rows.len(), 1);

    let DarktableHistoryStepDecode::Preserved { source: redo, .. } =
        decode_history_step(&history.steps[1])
    else {
        panic!("unsupported redo row must remain preserved");
    };
    assert!(!redo.selected);
    assert_eq!(redo.operation_params.bytes, [9, 8, 7]);
}

#[test]
fn production_manifest_uses_v30_native_order_across_colorcontrast_neighbors() {
    let history = built_in_history(vec![
        raw_history_row(41, 0, b"bloom", Some(1), Some(1), vec![]),
        raw_history_row(42, 1, b"colorcontrast", Some(2), Some(1), vec![]),
        raw_history_row(43, 2, b"globaltonemap", Some(3), Some(1), vec![]),
        raw_history_row(44, 3, b"vibrance", Some(2), Some(1), vec![]),
        raw_history_row(45, 4, b"colorcorrection", Some(1), Some(1), vec![]),
        raw_history_row(46, 5, b"bilat", Some(3), Some(1), vec![]),
        raw_history_row(47, 6, b"colorzones", Some(5), Some(1), vec![]),
        raw_history_row(48, 7, b"velvia", Some(2), Some(1), vec![]),
        raw_history_row(49, 8, b"relight", Some(1), Some(1), vec![]),
    ]);

    assert_eq!(
        history.order_source,
        Some(HistoryOrderSource::BuiltInModuleOrder)
    );
    assert!(history.order_proven);
    assert_eq!(
        ordered_operation_names(&history),
        [
            "globaltonemap",
            "relight",
            "bilat",
            "colorcorrection",
            "colorcontrast",
            "velvia",
            "vibrance",
            "colorzones",
            "bloom",
        ]
    );
}

#[test]
fn production_manifest_orders_and_decodes_the_ready_tonal_set() {
    let history = built_in_history(vec![
        raw_history_row(
            41,
            0,
            b"agx",
            Some(7),
            Some(1),
            AgxParametersV7::defaults().to_bytes().to_vec(),
        ),
        raw_history_row(
            42,
            1,
            b"rgblevels",
            Some(1),
            Some(1),
            RgbLevelsParametersV1::defaults().to_bytes().to_vec(),
        ),
        raw_history_row(
            43,
            2,
            b"levels",
            Some(2),
            Some(1),
            LevelsParametersV2::defaults().to_bytes().to_vec(),
        ),
    ]);

    assert!(history.order_proven);
    assert_eq!(
        ordered_operation_names(&history),
        ["rgblevels", "agx", "levels"]
    );
    let ordered = history
        .operation_order
        .iter()
        .map(|id| {
            history
                .steps
                .iter()
                .find(|step| step.instance_id == *id)
                .expect("ordered tonal row")
        })
        .map(decode_history_step)
        .collect::<Vec<_>>();
    assert!(matches!(
        ordered[0],
        DarktableHistoryStepDecode::RgbLevelsPendingBlend(_)
    ));
    assert!(matches!(
        ordered[1],
        DarktableHistoryStepDecode::AgxPendingBlend(_)
    ));
    assert!(matches!(
        ordered[2],
        DarktableHistoryStepDecode::LevelsPendingBlend(_)
    ));
}

#[test]
fn production_manifest_decodes_colorcontrast_before_velvia() {
    let history = built_in_history(vec![
        raw_history_row(
            41,
            0,
            b"velvia",
            Some(2),
            Some(1),
            VelviaParametersV2::defaults().to_bytes().to_vec(),
        ),
        raw_history_row(
            42,
            1,
            b"colorcontrast",
            Some(2),
            Some(1),
            COLORCONTRAST_V2_NATIVE_LE.to_vec(),
        ),
    ]);

    assert!(history.order_proven);
    assert_eq!(
        ordered_operation_names(&history),
        ["colorcontrast", "velvia"]
    );
    let ordered_steps = history
        .operation_order
        .iter()
        .map(|id| {
            history
                .steps
                .iter()
                .find(|step| step.instance_id == *id)
                .expect("ordered instance has a history step")
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        decode_history_step(ordered_steps[0]),
        DarktableHistoryStepDecode::ColorContrastPendingBlend(_)
    ));
    assert!(matches!(
        decode_history_step(ordered_steps[1]),
        DarktableHistoryStepDecode::VelviaPendingBlend(_)
    ));
}

#[test]
fn production_manifest_recognizes_canonical_colorreconstruct_history_name() {
    let history = built_in_history(vec![raw_history_row(
        41,
        0,
        b"colorreconstruct",
        Some(3),
        Some(1),
        COLORRECONSTRUCT_V3_NATIVE_LE.to_vec(),
    )]);
    let step = &history.steps[0];

    assert_eq!(step.operation.raw_name, b"colorreconstruct");
    assert_eq!(step.operation.name.as_deref(), Some("colorreconstruct"));
    assert!(matches!(
        decode_history_step(step),
        DarktableHistoryStepDecode::ColorReconstructionPendingBlend(_)
    ));
}

#[test]
fn production_manifest_recognizes_enlargecanvas_history_name_and_typed_payload() {
    let history = built_in_history(vec![raw_history_row(
        41,
        0,
        b"enlargecanvas",
        Some(1),
        Some(1),
        ENLARGECANVAS_V1_NATIVE_LE.to_vec(),
    )]);
    let step = &history.steps[0];

    assert_eq!(step.operation.raw_name, b"enlargecanvas");
    assert_eq!(step.operation.name.as_deref(), Some("enlargecanvas"));
    assert!(matches!(
        decode_history_step(step),
        DarktableHistoryStepDecode::EnlargeCanvasPendingBlend(_)
    ));
}

fn history_step(
    operation: &[u8],
    module: Option<i64>,
    enabled: Option<i64>,
    operation_params: Vec<u8>,
) -> CompatHistoryStep {
    let mut histories = HistoryDecoder::new(HistoryDecodeOptions {
        limits: HistoryLimits::default(),
        manifest: manifest(),
    })
    .decode(
        DarktableSchema::new(57, 13),
        HistoryRows {
            history: vec![raw_history_row(
                41,
                0,
                operation,
                module,
                enabled,
                operation_params,
            )],
            images: vec![RawImageHistoryRow {
                source_row: 11,
                image_id: 7,
                history_end: Some(1),
            }],
            ..HistoryRows::default()
        },
    );
    histories.remove(0).steps.remove(0)
}

fn manifest() -> DarktableOperationManifest {
    let mut manifest = DarktableOperationManifest::new();
    manifest.insert("bloom", 1, [1], None);
    manifest.insert("colorcontrast", 2, [1, 2], None);
    manifest.insert("colorcorrection", 1, [1], None);
    manifest.insert("colorreconstruct", 3, [1, 2, 3], None);
    manifest.insert("crop", 3, [1, 2, 3], None);
    manifest.insert("enlargecanvas", 1, [1], None);
    manifest.insert("colorzones", 5, [1, 2, 3, 4, 5], None);
    manifest.insert("velvia", 2, [1, 2], None);
    manifest.insert("vibrance", 2, [2], None);
    manifest
}

fn built_in_history(history: Vec<RawHistoryRow>) -> CompatHistory {
    let history_end = i64::try_from(history.len()).expect("fixture row count fits i64");
    HistoryDecoder::new(HistoryDecodeOptions {
        limits: HistoryLimits::default(),
        manifest: DarktableOperationManifest::reference(),
    })
    .decode(
        DarktableSchema::new(57, 13),
        HistoryRows {
            history,
            images: vec![RawImageHistoryRow {
                source_row: 11,
                image_id: 7,
                history_end: Some(history_end),
            }],
            module_orders: vec![RawModuleOrderRow {
                source_row: 12,
                image_id: 7,
                version: Some(2),
                operation_list: None,
            }],
            ..HistoryRows::default()
        },
    )
    .remove(0)
}

fn ordered_operation_names(history: &CompatHistory) -> Vec<&str> {
    history
        .operation_order
        .iter()
        .map(|id| {
            history
                .instances
                .iter()
                .find(|instance| instance.id == *id)
                .and_then(|instance| instance.operation.name.as_deref())
                .expect("ordered instance has a known operation")
        })
        .collect()
}

fn raw_history_row(
    source_row: u64,
    num: i64,
    operation: &[u8],
    module: Option<i64>,
    enabled: Option<i64>,
    operation_params: Vec<u8>,
) -> RawHistoryRow {
    RawHistoryRow {
        source_row,
        image_id: 7,
        num,
        module,
        operation: Some(operation.to_vec()),
        operation_params: Some(operation_params),
        enabled,
        blend_params: Some(vec![0xaa, 0xbb]),
        blend_version: Some(13),
        multi_priority: Some(0),
        multi_name: Some(b"base".to_vec()),
        multi_name_hand_edited: Some(0),
    }
}

fn assert_preserved(
    source: &CompatHistoryStep,
    code: DarktableHistoryDecodeFindingCode,
    bytes: &[u8],
) {
    let DarktableHistoryStepDecode::Preserved {
        source: preserved,
        finding,
    } = decode_history_step(source)
    else {
        panic!("row must remain opaque");
    };
    assert_eq!(finding.code, code);
    assert_eq!(preserved.operation_params.bytes, bytes);
    assert_eq!(preserved.blend_params.bytes, [0xaa, 0xbb]);
}

fn assert_preserved_exact(source: &CompatHistoryStep, code: DarktableHistoryDecodeFindingCode) {
    let DarktableHistoryStepDecode::Preserved {
        source: preserved,
        finding,
    } = decode_history_step(source)
    else {
        panic!("row must remain opaque");
    };
    assert_eq!(finding.code, code);
    assert_eq!(&preserved, source);
    assert_eq!(preserved.blend_params.bytes, [0xaa, 0xbb]);
}

fn colorzones_v1_payload() -> Vec<u8> {
    let mut payload = Vec::with_capacity(148);
    payload.extend_from_slice(&2_i32.to_le_bytes());
    for _channel in 0..3 {
        for x in [0.0_f32, 0.2, 0.4, 0.6, 0.8, 1.0] {
            payload.extend_from_slice(&x.to_le_bytes());
        }
    }
    for _channel in 0..3 {
        for y in [0.5_f32; 6] {
            payload.extend_from_slice(&y.to_le_bytes());
        }
    }
    assert_eq!(payload.len(), 148);
    payload
}
