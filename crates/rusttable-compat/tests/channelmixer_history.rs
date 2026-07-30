use rusttable_compat::channelmixer::{
    CHANNELMIXER_MATRIX_VALUES, CHANNELMIXER_OUTPUT_CHANNELS, CHANNELMIXER_V1_PARAMETER_BYTES,
    CHANNELMIXER_V2_PARAMETER_BYTES, ChannelMixerAlgorithm, ChannelMixerCodecError,
    ChannelMixerDestination, ChannelMixerHistory, ChannelMixerHistoryDecodeFindingCode,
    ChannelMixerHistoryStepDecode, ChannelMixerParametersV1, ChannelMixerParametersV2,
    decode_channelmixer_history_step, migrate_v1_to_v2,
};
use rusttable_compat::{
    CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions, HistoryDecoder,
    HistoryLimits,
};
use rusttable_sqlite_native::{DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow};

const CHANNELMIXER_V1_NATIVE_LE: &[u8; CHANNELMIXER_V1_PARAMETER_BYTES] =
    include_bytes!("../../../fixtures/corpus/assets/operation-channelmixer-params-v1.bin");
const CHANNELMIXER_V2_NATIVE_LE: &[u8; CHANNELMIXER_V2_PARAMETER_BYTES] =
    include_bytes!("../../../fixtures/corpus/assets/operation-channelmixer-params-v2.bin");

#[test]
fn native_v1_fixture_covers_all_21_values_in_row_order() {
    let parameters = ChannelMixerParametersV1::from_native_le(CHANNELMIXER_V1_NATIVE_LE)
        .expect("source-derived Channel Mixer v1 fixture is valid");
    assert_eq!(parameters, ChannelMixerParametersV1::defaults());
    assert_eq!(CHANNELMIXER_MATRIX_VALUES, 21);

    for (row, values) in [parameters.red, parameters.green, parameters.blue]
        .into_iter()
        .enumerate()
    {
        for (destination, value) in values.into_iter().enumerate() {
            let offset = (row * CHANNELMIXER_OUTPUT_CHANNELS + destination) * 4;
            assert_eq!(
                &CHANNELMIXER_V1_NATIVE_LE[offset..offset + 4],
                &value.to_le_bytes(),
                "row {row} destination {destination}"
            );
        }
    }
    assert_eq!(parameters.to_native_le(), *CHANNELMIXER_V1_NATIVE_LE);
}

#[test]
fn native_v2_fixture_keeps_rows_and_algorithm_at_exact_offsets() {
    let parameters = ChannelMixerParametersV2::from_native_le(CHANNELMIXER_V2_NATIVE_LE)
        .expect("source-derived Channel Mixer v2 fixture is valid");
    assert_eq!(parameters, ChannelMixerParametersV2::defaults());
    assert_eq!(
        &CHANNELMIXER_V2_NATIVE_LE[CHANNELMIXER_V1_PARAMETER_BYTES..],
        &1_i32.to_le_bytes()
    );
    assert_eq!(parameters.algorithm, ChannelMixerAlgorithm::V2);
    assert_eq!(parameters.to_native_le(), *CHANNELMIXER_V2_NATIVE_LE);
}

#[test]
fn v1_migration_copies_hsl_and_gray_but_suppresses_rgb_when_gray_is_used() {
    let source = ChannelMixerParametersV1 {
        red: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.25],
        green: [7.0, 8.0, 9.0, 10.0, 11.0, 12.0, -0.5],
        blue: [13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 1.5],
    };
    let migrated = migrate_v1_to_v2(source);

    assert_eq!(migrated.algorithm, ChannelMixerAlgorithm::V1);
    assert_eq!(migrated.red[..3], source.red[..3]);
    assert_eq!(migrated.green[..3], source.green[..3]);
    assert_eq!(migrated.blue[..3], source.blue[..3]);
    assert_eq!(
        migrated.red[ChannelMixerDestination::Gray as usize].to_bits(),
        0.25_f32.to_bits()
    );
    assert_eq!(
        migrated.green[ChannelMixerDestination::Gray as usize].to_bits(),
        (-0.5_f32).to_bits()
    );
    assert_eq!(
        migrated.blue[ChannelMixerDestination::Gray as usize].to_bits(),
        1.5_f32.to_bits()
    );
    assert_eq!(migrated.red[3..6], [0.0; 3]);
    assert_eq!(migrated.green[3..6], [0.0; 3]);
    assert_eq!(migrated.blue[3..6], [0.0; 3]);
}

#[test]
fn v1_migration_copies_rgb_only_when_all_gray_coefficients_are_zero() {
    let source = ChannelMixerParametersV1 {
        red: [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, -0.0],
        green: [7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 0.0],
        blue: [13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 0.0],
    };
    let migrated = migrate_v1_to_v2(source);

    assert_eq!(migrated.red[3..6], source.red[3..6]);
    assert_eq!(migrated.green[3..6], source.green[3..6]);
    assert_eq!(migrated.blue[3..6], source.blue[3..6]);
    assert_eq!(
        migrated.red[ChannelMixerDestination::Gray as usize].to_bits(),
        (-0.0_f32).to_bits()
    );
}

#[test]
fn finite_values_outside_annotations_are_accepted_and_invalid_payloads_are_rejected() {
    let parameters = ChannelMixerParametersV2 {
        red: [125.0, -125.0, 0.0, 4.0, 5.0, 6.0, 7.0],
        green: [8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0],
        blue: [15.0, 16.0, 17.0, 18.0, 19.0, 20.0, 21.0],
        algorithm: ChannelMixerAlgorithm::V2,
    };
    let bytes = parameters.to_native_le();
    assert_eq!(
        ChannelMixerParametersV2::from_native_le(&bytes),
        Ok(parameters)
    );

    assert_eq!(
        ChannelMixerParametersV1::from_native_le(&bytes[..CHANNELMIXER_V1_PARAMETER_BYTES - 1]),
        Err(ChannelMixerCodecError::WrongLength {
            expected: CHANNELMIXER_V1_PARAMETER_BYTES,
            actual: CHANNELMIXER_V1_PARAMETER_BYTES - 1,
        })
    );

    let mut nonfinite = bytes;
    nonfinite[4..8].copy_from_slice(&f32::NAN.to_le_bytes());
    assert!(matches!(
        ChannelMixerParametersV2::from_native_le(&nonfinite),
        Err(ChannelMixerCodecError::NonFinite { .. })
    ));

    let mut invalid_algorithm = bytes;
    invalid_algorithm[CHANNELMIXER_V1_PARAMETER_BYTES..].copy_from_slice(&2_i32.to_le_bytes());
    assert_eq!(
        ChannelMixerParametersV2::from_native_le(&invalid_algorithm),
        Err(ChannelMixerCodecError::InvalidAlgorithm { raw: 2 })
    );
}

#[test]
fn versioned_codec_preserves_unknown_versions_verbatim() {
    let bytes = [0xde, 0xad, 0xbe, 0xef];
    let ChannelMixerHistory::Opaque {
        version,
        bytes: stored,
    } = ChannelMixerHistory::decode(99, &bytes).expect("unknown versions stay opaque")
    else {
        panic!("future Channel Mixer versions must not be interpreted");
    };
    assert_eq!(version, 99);
    assert_eq!(stored, bytes);
}

#[test]
fn pending_v1_row_preserves_enabled_order_instance_and_opaque_metadata() {
    let source = history_step(Some(1), Some(0), CHANNELMIXER_V1_NATIVE_LE.to_vec());
    let ChannelMixerHistoryStepDecode::ChannelMixerPendingBlend(decoded) =
        decode_channelmixer_history_step(&source)
    else {
        panic!("native Channel Mixer v1 row must decode as pending core");
    };

    assert_eq!(decoded.source, source);
    assert_eq!(decoded.source.operation.raw_name, b"channelmixer");
    assert_eq!(decoded.source_version, 1);
    assert!(decoded.migrated);
    assert!(!decoded.enabled);
    assert_eq!(decoded.parameters.algorithm, ChannelMixerAlgorithm::V1);
    assert_eq!(
        decoded.canonical_parameters.len(),
        CHANNELMIXER_V2_PARAMETER_BYTES
    );
    assert_eq!(decoded.source.num, 7);
    assert_eq!(decoded.source.multi_priority, Some(3));
    assert_eq!(decoded.source.multi_name.bytes, b"named-instance");
    assert_eq!(decoded.source.multi_name_hand_edited, Some(1));
    assert_eq!(decoded.source.blend_version, Some(13));
    assert_eq!(decoded.source.blend_params.bytes, [0xaa, 0xbb, 0xcc]);
    assert_eq!(decoded.source.instance_id, source.instance_id);
    assert_eq!(
        decoded.execution_blocker.code,
        ChannelMixerHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert!(decoded.execution_blocker.detail.contains("blend/mask"));
    assert!(decoded.execution_blocker.detail.contains("multi-instance"));
    assert!(decoded.execution_blocker.detail.contains("no executable"));
}

#[test]
fn pending_v2_row_retains_enabled_state_and_native_bytes() {
    let source = history_step(Some(2), Some(1), CHANNELMIXER_V2_NATIVE_LE.to_vec());
    let ChannelMixerHistoryStepDecode::ChannelMixerPendingBlend(decoded) =
        decode_channelmixer_history_step(&source)
    else {
        panic!("native Channel Mixer v2 row must decode as pending core");
    };

    assert_eq!(decoded.source, source);
    assert_eq!(decoded.source_version, 2);
    assert!(!decoded.migrated);
    assert!(decoded.enabled);
    assert_eq!(decoded.canonical_parameters, *CHANNELMIXER_V2_NATIVE_LE);
}

#[test]
fn missing_future_malformed_nonfinite_and_invalid_rows_are_preserved_exactly() {
    let cases = [
        (
            history_step(Some(99), Some(1), vec![1, 2, 3]),
            ChannelMixerHistoryDecodeFindingCode::UnsupportedParameterVersion,
        ),
        (
            history_step(
                Some(2),
                Some(1),
                vec![0; CHANNELMIXER_V2_PARAMETER_BYTES - 1],
            ),
            ChannelMixerHistoryDecodeFindingCode::InvalidOperationParameters,
        ),
        (
            history_step(None, Some(1), CHANNELMIXER_V1_NATIVE_LE.to_vec()),
            ChannelMixerHistoryDecodeFindingCode::MissingModuleVersion,
        ),
        (
            history_step(
                Some(i64::from(u16::MAX) + 1),
                Some(1),
                CHANNELMIXER_V1_NATIVE_LE.to_vec(),
            ),
            ChannelMixerHistoryDecodeFindingCode::InvalidModuleVersion,
        ),
        (
            history_step(Some(2), Some(7), CHANNELMIXER_V2_NATIVE_LE.to_vec()),
            ChannelMixerHistoryDecodeFindingCode::InvalidEnabledState,
        ),
    ];
    for (source, code) in cases {
        assert_preserved_exact(&source, code);
    }

    let mut nonfinite = *CHANNELMIXER_V2_NATIVE_LE;
    nonfinite[4..8].copy_from_slice(&f32::INFINITY.to_le_bytes());
    let source = history_step(Some(2), Some(1), nonfinite.to_vec());
    assert_preserved_exact(
        &source,
        ChannelMixerHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    let mut invalid_algorithm = *CHANNELMIXER_V2_NATIVE_LE;
    invalid_algorithm[CHANNELMIXER_V1_PARAMETER_BYTES..].copy_from_slice(&3_i32.to_le_bytes());
    let source = history_step(Some(2), Some(1), invalid_algorithm.to_vec());
    assert_preserved_exact(
        &source,
        ChannelMixerHistoryDecodeFindingCode::InvalidOperationParameters,
    );
}

fn history_step(
    module: Option<i64>,
    enabled: Option<i64>,
    operation_params: Vec<u8>,
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
                operation: Some(b"channelmixer".to_vec()),
                operation_params: Some(operation_params),
                enabled,
                blend_params: Some(vec![0xaa, 0xbb, 0xcc]),
                blend_version: Some(13),
                multi_priority: Some(3),
                multi_name: Some(b"named-instance".to_vec()),
                multi_name_hand_edited: Some(1),
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
    manifest.insert("channelmixer", 2, [1, 2], Some(106));
    manifest
}

fn assert_preserved_exact(source: &CompatHistoryStep, code: ChannelMixerHistoryDecodeFindingCode) {
    let ChannelMixerHistoryStepDecode::Preserved {
        source: preserved,
        finding,
    } = decode_channelmixer_history_step(source)
    else {
        panic!("invalid Channel Mixer rows must remain opaque");
    };
    assert_eq!(finding.code, code);
    assert_eq!(preserved, *source);
    assert_eq!(preserved.enabled, source.enabled);
    assert_eq!(preserved.num, source.num);
    assert_eq!(preserved.module, source.module);
    assert_eq!(preserved.instance_id, source.instance_id);
    assert_eq!(
        preserved.operation_params.bytes,
        source.operation_params.bytes
    );
    assert_eq!(preserved.blend_params.bytes, source.blend_params.bytes);
    assert_eq!(preserved.multi_name.bytes, source.multi_name.bytes);
}
