use rusttable_compat::channelmixer::{
    CHANNELMIXER_V1_PARAMETER_BYTES, CHANNELMIXER_V2_PARAMETER_BYTES, ChannelMixerAlgorithm,
    ChannelMixerHistoryDecodeFindingCode, ChannelMixerHistoryStepDecode,
    decode_channelmixer_history_step,
};
use rusttable_compat::{
    CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions, HistoryDecoder,
    HistoryLimits,
};
use rusttable_import::darktable::{DarktableHistoryStepDecode, decode_history_step};
use rusttable_sqlite_native::{DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow};

const CHANNELMIXER_V1_NATIVE_LE: &[u8; CHANNELMIXER_V1_PARAMETER_BYTES] =
    include_bytes!("../../../fixtures/corpus/assets/operation-channelmixer-params-v1.bin");
const CHANNELMIXER_V2_NATIVE_LE: &[u8; CHANNELMIXER_V2_PARAMETER_BYTES] =
    include_bytes!("../../../fixtures/corpus/assets/operation-channelmixer-params-v2.bin");

#[test]
fn import_leaf_decodes_v1_to_canonical_v2_but_keeps_row_pending() {
    let source = history_step(Some(1), Some(0), CHANNELMIXER_V1_NATIVE_LE.to_vec());
    let ChannelMixerHistoryStepDecode::ChannelMixerPendingBlend(decoded) =
        decode_channelmixer_history_step(&source)
    else {
        panic!("Channel Mixer v1 must use the typed pending-blend leaf");
    };

    assert_eq!(decoded.source, source);
    assert_eq!(decoded.source_version, 1);
    assert!(decoded.migrated);
    assert!(!decoded.enabled);
    assert_eq!(decoded.parameters.algorithm, ChannelMixerAlgorithm::V1);
    assert_eq!(
        decoded.canonical_parameters.len(),
        CHANNELMIXER_V2_PARAMETER_BYTES
    );
    assert_eq!(
        decoded.source.operation_params.bytes,
        CHANNELMIXER_V1_NATIVE_LE
    );
    assert_eq!(decoded.source.blend_params.bytes, [0xaa, 0xbb, 0xcc]);
    assert_eq!(decoded.source.multi_name.bytes, b"named-instance");
    assert_eq!(decoded.source.multi_priority, Some(3));
    assert_eq!(decoded.source.instance_id, source.instance_id);
    assert_eq!(
        decoded.execution_blocker.code,
        ChannelMixerHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
}

#[test]
fn production_history_dispatch_keeps_channelmixer_pending_and_preserved() {
    let source = history_step(Some(2), Some(1), CHANNELMIXER_V2_NATIVE_LE.to_vec());
    let DarktableHistoryStepDecode::ChannelMixerPendingBlend(decoded) =
        decode_history_step(&source)
    else {
        panic!("production history dispatch must route Channel Mixer to its pending leaf");
    };
    assert_eq!(decoded.source, source);
    assert_eq!(decoded.source_version, 2);

    let future = history_step(Some(9), Some(1), vec![0xde, 0xad]);
    let DarktableHistoryStepDecode::Preserved {
        source: preserved,
        finding,
    } = decode_history_step(&future)
    else {
        panic!("production history dispatch must preserve unknown Channel Mixer versions");
    };
    assert_eq!(preserved, future);
    assert_eq!(
        finding.code,
        rusttable_import::darktable::DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion
    );
}

#[test]
fn import_leaf_decodes_v2_without_rewriting_native_bytes() {
    let source = history_step(Some(2), Some(1), CHANNELMIXER_V2_NATIVE_LE.to_vec());
    let ChannelMixerHistoryStepDecode::ChannelMixerPendingBlend(decoded) =
        decode_channelmixer_history_step(&source)
    else {
        panic!("Channel Mixer v2 must use the typed pending-blend leaf");
    };

    assert_eq!(decoded.source, source);
    assert_eq!(decoded.source_version, 2);
    assert!(!decoded.migrated);
    assert!(decoded.enabled);
    assert_eq!(decoded.canonical_parameters, *CHANNELMIXER_V2_NATIVE_LE);
}

#[test]
fn import_leaf_preserves_future_malformed_nonfinite_and_invalid_rows() {
    let cases = [
        (
            history_step(Some(3), Some(1), vec![1, 2, 3]),
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
            history_step(Some(2), Some(9), CHANNELMIXER_V2_NATIVE_LE.to_vec()),
            ChannelMixerHistoryDecodeFindingCode::InvalidEnabledState,
        ),
    ];
    for (source, code) in cases {
        assert_preserved(&source, code);
    }

    let mut nonfinite = *CHANNELMIXER_V2_NATIVE_LE;
    nonfinite[4..8].copy_from_slice(&f32::NAN.to_le_bytes());
    let source = history_step(Some(2), Some(1), nonfinite.to_vec());
    assert_preserved(
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

fn assert_preserved(source: &CompatHistoryStep, code: ChannelMixerHistoryDecodeFindingCode) {
    let ChannelMixerHistoryStepDecode::Preserved {
        source: preserved,
        finding,
    } = decode_channelmixer_history_step(source)
    else {
        panic!("unsupported Channel Mixer rows must remain byte-preserved");
    };
    assert_eq!(finding.code, code);
    assert_eq!(preserved, *source);
    assert_eq!(
        preserved.operation_params.bytes,
        source.operation_params.bytes
    );
    assert_eq!(preserved.blend_params.bytes, source.blend_params.bytes);
    assert_eq!(preserved.multi_name.bytes, source.multi_name.bytes);
    assert_eq!(preserved.instance_id, source.instance_id);
}
