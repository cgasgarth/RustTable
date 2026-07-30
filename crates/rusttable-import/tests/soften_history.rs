use rusttable_compat::soften::SoftenHistoryDecodeFindingCode;
use rusttable_compat::{
    CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions, HistoryDecoder,
    HistoryLimits,
};
use rusttable_import::darktable::{
    DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode, decode_history_step,
};
use rusttable_sqlite_native::{DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow};

const SOFTEN_V1_PARAMETER_BYTES: usize = 16;
const SOFTEN_V1_NATIVE_LE: &[u8; SOFTEN_V1_PARAMETER_BYTES] =
    include_bytes!("../../../fixtures/corpus/assets/operation-soften-params-v1.bin");

#[test]
fn production_import_dispatch_decodes_typed_soften_core_as_pending_blend() {
    let source = history_step(Some(1), Some(0), SOFTEN_V1_NATIVE_LE.to_vec());
    let DarktableHistoryStepDecode::SoftenPendingBlend(decoded) = decode_history_step(&source)
    else {
        panic!("native Soften v1 must use its typed pending-blend dispatch branch");
    };

    assert_eq!(decoded.source, source);
    assert_eq!(decoded.source.operation.raw_name, b"soften");
    assert_eq!(decoded.source_version, 1);
    assert!(!decoded.enabled);
    assert_eq!(decoded.canonical_parameters, *SOFTEN_V1_NATIVE_LE);
    assert_eq!(decoded.source.operation_params.bytes, SOFTEN_V1_NATIVE_LE);
    assert_eq!(decoded.source.instance_id, source.instance_id);
    assert_eq!(decoded.source.blend_params.bytes, [0xaa, 0xbb, 0xcc]);
    assert_eq!(decoded.source.multi_name.bytes, b"named-instance");
    assert_eq!(
        decoded.execution_blocker.code,
        SoftenHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert!(decoded.execution_blocker.detail.contains("blend/mask"));
    assert!(decoded.execution_blocker.detail.contains("multi-instance"));
}

#[test]
fn production_import_dispatch_preserves_unknown_soften_rows_exactly() {
    let source = history_step(Some(2), Some(1), vec![9, 8, 7]);
    let DarktableHistoryStepDecode::Preserved {
        source: preserved,
        finding,
    } = decode_history_step(&source)
    else {
        panic!("unknown Soften versions must remain byte-preserved");
    };
    assert_eq!(
        finding.code,
        DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion
    );
    assert_eq!(preserved, source);
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
                operation: Some(b"soften".to_vec()),
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
    manifest.insert("soften", 1, [1], Some(60));
    manifest
}
