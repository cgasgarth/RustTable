use rusttable_compat::basicadj::{
    BASICADJ_V1_PARAMETER_BYTES, BASICADJ_V1_VERSION, BASICADJ_V2_PARAMETER_BYTES,
    BASICADJ_V2_VERSION, BasicAdjCodecError, BasicAdjHistory, BasicAdjHistoryDecodeFindingCode,
    BasicAdjHistoryStepDecode, BasicAdjParametersV1, BasicAdjParametersV2, BasicAdjPreserveColors,
    decode_basicadj_history_step, migrate_v1_to_v2,
};
use rusttable_compat::{
    CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions, HistoryDecoder,
    HistoryLimits,
};
use rusttable_sqlite_native::{DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow};

const BASICADJ_V1_NATIVE_LE: &[u8; BASICADJ_V1_PARAMETER_BYTES] =
    include_bytes!("../../rusttable-import/tests/fixtures/basicadj-v1-native-le.bin");
const BASICADJ_V2_NATIVE_LE: &[u8; BASICADJ_V2_PARAMETER_BYTES] =
    include_bytes!("../../rusttable-import/tests/fixtures/basicadj-v2-native-le.bin");

fn v1_parameters() -> BasicAdjParametersV1 {
    BasicAdjParametersV1 {
        black_point: -0.125,
        exposure: 1.25,
        hlcompr: 42.0,
        hlcomprthresh: 17.0,
        contrast: 0.75,
        preserve_colors: BasicAdjPreserveColors::Power,
        middle_grey: 18.42,
        brightness: -0.25,
        saturation: 0.5,
        clip: -0.1,
    }
}

fn v2_parameters() -> BasicAdjParametersV2 {
    BasicAdjParametersV2 {
        black_point: -0.125,
        exposure: 1.25,
        hlcompr: 42.0,
        hlcomprthresh: 17.0,
        contrast: 0.75,
        preserve_colors: BasicAdjPreserveColors::Norm,
        middle_grey: 18.42,
        brightness: -0.25,
        saturation: 0.5,
        vibrance: -0.35,
        clip: -0.1,
    }
}

fn v1_defaults() -> BasicAdjParametersV1 {
    BasicAdjParametersV1 {
        black_point: 0.0,
        exposure: 0.0,
        hlcompr: 0.0,
        hlcomprthresh: 0.0,
        contrast: 0.0,
        preserve_colors: BasicAdjPreserveColors::Luminance,
        middle_grey: 18.42,
        brightness: 0.0,
        saturation: 0.0,
        clip: 0.0,
    }
}

fn v2_defaults() -> BasicAdjParametersV2 {
    BasicAdjParametersV2 {
        black_point: 0.0,
        exposure: 0.0,
        hlcompr: 0.0,
        hlcomprthresh: 0.0,
        contrast: 0.0,
        preserve_colors: BasicAdjPreserveColors::Luminance,
        middle_grey: 18.42,
        brightness: 0.0,
        saturation: 0.0,
        vibrance: 0.0,
        clip: 0.0,
    }
}

#[test]
fn native_fixtures_match_source_sizes_defaults_and_exact_field_order() {
    assert_eq!(BASICADJ_V1_PARAMETER_BYTES, 40);
    assert_eq!(BASICADJ_V2_PARAMETER_BYTES, 44);

    let v1 = BasicAdjParametersV1::from_native_le(BASICADJ_V1_NATIVE_LE)
        .expect("source-derived Basic Adjustments v1 fixture is valid");
    assert_eq!(v1, v1_defaults());
    assert_eq!(v1.to_native_le(), *BASICADJ_V1_NATIVE_LE);
    assert_eq!(&BASICADJ_V1_NATIVE_LE[20..24], &1_i32.to_le_bytes());
    assert_eq!(&BASICADJ_V1_NATIVE_LE[24..28], &18.42_f32.to_le_bytes());

    let v2 = BasicAdjParametersV2::from_native_le(BASICADJ_V2_NATIVE_LE)
        .expect("source-derived Basic Adjustments v2 fixture is valid");
    assert_eq!(v2, v2_defaults());
    assert_eq!(v2.to_native_le(), *BASICADJ_V2_NATIVE_LE);
    assert_eq!(&BASICADJ_V2_NATIVE_LE[20..24], &1_i32.to_le_bytes());
    assert_eq!(&BASICADJ_V2_NATIVE_LE[36..40], &0.0_f32.to_le_bytes());
    assert_eq!(&BASICADJ_V2_NATIVE_LE[40..44], &0.0_f32.to_le_bytes());
}

#[test]
fn finite_outliers_round_trip_without_ui_range_clamping() {
    let v1 = v1_parameters();
    let bytes = v1.to_native_le();
    assert_eq!(BasicAdjParametersV1::from_native_le(&bytes), Ok(v1));
    assert_eq!(&bytes[0..4], &(-0.125_f32).to_le_bytes());
    assert_eq!(&bytes[8..12], &42.0_f32.to_le_bytes());
    assert_eq!(&bytes[20..24], &6_i32.to_le_bytes());

    let v2 = v2_parameters();
    let bytes = v2.to_native_le();
    assert_eq!(BasicAdjParametersV2::from_native_le(&bytes), Ok(v2));
    assert_eq!(&bytes[32..36], &0.5_f32.to_le_bytes());
    assert_eq!(&bytes[36..40], &(-0.35_f32).to_le_bytes());
    assert_eq!(&bytes[40..44], &(-0.1_f32).to_le_bytes());
}

#[test]
fn v1_migration_is_the_direct_native_edge_and_adds_neutral_vibrance() {
    let migrated = migrate_v1_to_v2(v1_parameters());
    assert_eq!(migrated.black_point.to_bits(), (-0.125_f32).to_bits());
    assert_eq!(migrated.exposure.to_bits(), 1.25_f32.to_bits());
    assert_eq!(migrated.hlcompr.to_bits(), 42.0_f32.to_bits());
    assert_eq!(migrated.hlcomprthresh.to_bits(), 17.0_f32.to_bits());
    assert_eq!(migrated.contrast.to_bits(), 0.75_f32.to_bits());
    assert_eq!(migrated.preserve_colors, BasicAdjPreserveColors::Power);
    assert_eq!(migrated.middle_grey.to_bits(), 18.42_f32.to_bits());
    assert_eq!(migrated.brightness.to_bits(), (-0.25_f32).to_bits());
    assert_eq!(migrated.saturation.to_bits(), 0.5_f32.to_bits());
    assert_eq!(migrated.vibrance.to_bits(), 0.0_f32.to_bits());
    assert_eq!(migrated.clip.to_bits(), (-0.1_f32).to_bits());

    let history = BasicAdjHistory::decode(BASICADJ_V1_VERSION, &v1_parameters().to_native_le())
        .expect("v1 payload");
    assert_eq!(history.migrate_v1(), Some(migrated));
    assert_eq!(history.version(), BASICADJ_V1_VERSION);
    assert_eq!(history.payload(), v1_parameters().to_native_le());
}

#[test]
fn v2_history_round_trips_without_reordering_or_migration() {
    let parameters = v2_parameters();
    let history = BasicAdjHistory::decode(BASICADJ_V2_VERSION, &parameters.to_native_le())
        .expect("v2 payload");
    assert_eq!(history.version(), BASICADJ_V2_VERSION);
    assert_eq!(history.payload(), parameters.to_native_le());
    assert_eq!(history.migrate_v1(), Some(parameters));
}

#[test]
fn malformed_nonfinite_and_invalid_enum_payloads_are_rejected_without_partial_decode() {
    let bytes = v2_parameters().to_native_le();
    assert_eq!(
        BasicAdjParametersV2::from_native_le(&bytes[..43]),
        Err(BasicAdjCodecError::WrongLength {
            expected: 44,
            actual: 43,
        })
    );
    assert_eq!(
        BasicAdjParametersV1::from_native_le(&bytes[..39]),
        Err(BasicAdjCodecError::WrongLength {
            expected: 40,
            actual: 39,
        })
    );

    let mut nonfinite = bytes;
    nonfinite[28..32].copy_from_slice(&f32::NAN.to_le_bytes());
    assert_eq!(
        BasicAdjParametersV2::from_native_le(&nonfinite),
        Err(BasicAdjCodecError::NonFinite {
            field: "brightness"
        })
    );

    let mut unknown_mode = bytes;
    unknown_mode[20..24].copy_from_slice(&99_i32.to_le_bytes());
    assert_eq!(
        BasicAdjParametersV2::from_native_le(&unknown_mode),
        Err(BasicAdjCodecError::InvalidPreserveColors { raw: 99 })
    );

    let opaque = BasicAdjHistory::decode(99, &[0xde, 0xad, 0xbe, 0xef])
        .expect("unknown versions stay opaque");
    assert_eq!(opaque.version(), 99);
    assert_eq!(opaque.payload(), [0xde, 0xad, 0xbe, 0xef]);
}

#[test]
fn valid_rows_are_explicitly_pending_and_preserve_all_opaque_metadata() {
    let source = history_step(
        Some(1),
        Some(0),
        v1_parameters().to_native_le().to_vec(),
        true,
        true,
    );
    let BasicAdjHistoryStepDecode::BasicAdjPendingBlend(decoded) =
        decode_basicadj_history_step(&source)
    else {
        panic!("native Basic Adjustments v1 row must decode as pending core");
    };

    assert_eq!(decoded.source, source);
    assert_eq!(decoded.source.operation.raw_name, b"basicadj");
    assert_eq!(decoded.source_version, 1);
    assert!(decoded.migrated);
    assert!(!decoded.enabled);
    assert_eq!(decoded.parameters, migrate_v1_to_v2(v1_parameters()));
    assert_eq!(
        decoded.canonical_parameters,
        decoded.parameters.to_native_le()
    );
    assert_eq!(decoded.source.num, 7);
    assert_eq!(decoded.source.instance_id, source.instance_id);
    assert_eq!(decoded.source.blend_version, Some(13));
    assert!(decoded.source.blend_params.present);
    assert_eq!(decoded.source.blend_params.bytes, [0xaa, 0xbb, 0xcc]);
    assert_eq!(decoded.source.multi_priority, Some(3));
    assert!(decoded.source.multi_name.present);
    assert_eq!(decoded.source.multi_name.bytes, b"named-instance");
    assert_eq!(decoded.source.multi_name_hand_edited, Some(1));
    assert_eq!(
        decoded.execution_blocker.code,
        BasicAdjHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert!(decoded.execution_blocker.detail.contains("blend/mask"));
    assert!(decoded.execution_blocker.detail.contains("multi-instance"));
    assert!(decoded.execution_blocker.detail.contains("no executable"));
}

#[test]
fn valid_core_remains_pending_even_when_opaque_metadata_is_absent() {
    let source = history_step(
        Some(2),
        Some(1),
        v2_parameters().to_native_le().to_vec(),
        false,
        false,
    );
    let BasicAdjHistoryStepDecode::BasicAdjPendingBlend(decoded) =
        decode_basicadj_history_step(&source)
    else {
        panic!("valid Basic Adjustments core must remain pending, not be rejected");
    };
    assert_eq!(decoded.source, source);
    assert!(!decoded.migrated);
    assert!(decoded.enabled);
    assert!(!decoded.source.blend_params.present);
    assert!(decoded.source.multi_name.bytes.is_empty());
}

#[test]
fn missing_future_malformed_nonfinite_and_invalid_enabled_rows_are_preserved_exactly() {
    let mut nonfinite = v2_parameters().to_native_le();
    nonfinite[28..32].copy_from_slice(&f32::INFINITY.to_le_bytes());
    let cases = [
        (
            history_step(
                None,
                Some(1),
                v1_parameters().to_native_le().to_vec(),
                true,
                true,
            ),
            BasicAdjHistoryDecodeFindingCode::MissingModuleVersion,
        ),
        (
            history_step(
                Some(i64::from(u16::MAX) + 1),
                Some(1),
                v1_parameters().to_native_le().to_vec(),
                true,
                true,
            ),
            BasicAdjHistoryDecodeFindingCode::InvalidModuleVersion,
        ),
        (
            history_step(Some(99), Some(1), vec![1, 2, 3], true, true),
            BasicAdjHistoryDecodeFindingCode::UnsupportedParameterVersion,
        ),
        (
            history_step(
                Some(2),
                Some(7),
                v2_parameters().to_native_le().to_vec(),
                true,
                true,
            ),
            BasicAdjHistoryDecodeFindingCode::InvalidEnabledState,
        ),
        (
            history_step(
                Some(2),
                Some(1),
                vec![0; BASICADJ_V2_PARAMETER_BYTES - 1],
                true,
                true,
            ),
            BasicAdjHistoryDecodeFindingCode::InvalidOperationParameters,
        ),
        (
            history_step(Some(2), Some(1), nonfinite.to_vec(), true, true),
            BasicAdjHistoryDecodeFindingCode::InvalidOperationParameters,
        ),
    ];

    for (source, expected) in cases {
        let BasicAdjHistoryStepDecode::Preserved {
            source: preserved,
            finding,
        } = decode_basicadj_history_step(&source)
        else {
            panic!("invalid Basic Adjustments rows must remain opaque");
        };
        assert_eq!(finding.code, expected);
        assert_eq!(preserved, source);
        assert_eq!(
            preserved.operation_params.bytes,
            source.operation_params.bytes
        );
        assert_eq!(preserved.blend_params.bytes, source.blend_params.bytes);
        assert_eq!(preserved.multi_name.bytes, source.multi_name.bytes);
        assert_eq!(preserved.instance_id, source.instance_id);
    }
}

fn history_step(
    module: Option<i64>,
    enabled: Option<i64>,
    operation_params: Vec<u8>,
    blend_present: bool,
    instance_complete: bool,
) -> CompatHistoryStep {
    HistoryDecoder::new(HistoryDecodeOptions {
        limits: HistoryLimits::default(),
        manifest: manifest(),
    })
    .decode(
        DarktableSchema::new(57, 13),
        HistoryRows {
            history: vec![RawHistoryRow {
                source_row: 41,
                image_id: 7,
                num: 7,
                module,
                operation: Some(b"basicadj".to_vec()),
                operation_params: Some(operation_params),
                enabled,
                blend_params: blend_present.then_some(vec![0xaa, 0xbb, 0xcc]),
                blend_version: blend_present.then_some(13),
                multi_priority: instance_complete.then_some(3),
                multi_name: instance_complete.then_some(b"named-instance".to_vec()),
                multi_name_hand_edited: instance_complete.then_some(1),
            }],
            images: vec![RawImageHistoryRow {
                source_row: 11,
                image_id: 7,
                history_end: Some(8),
            }],
            ..HistoryRows::default()
        },
    )
    .remove(0)
    .steps
    .remove(0)
}

fn manifest() -> DarktableOperationManifest {
    let mut manifest = DarktableOperationManifest::new();
    manifest.insert("basicadj", 2, [1, 2], Some(78));
    manifest
}
