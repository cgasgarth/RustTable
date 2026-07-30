use rusttable_compat::{
    CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions, HistoryDecoder,
    HistoryLimits,
};
use rusttable_import::darktable::{
    BASICADJ_V1_PARAMETER_BYTES, BasicAdjHistoryDecodeFindingCode, BasicAdjHistoryStepDecode,
    decode_basicadj_import_history_step,
};
use rusttable_sqlite_native::{DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow};

const BASICADJ_V1_NATIVE_LE: &[u8; BASICADJ_V1_PARAMETER_BYTES] =
    include_bytes!("fixtures/basicadj-v1-native-le.bin");

#[test]
fn operation_local_import_leaf_decodes_v1_as_pending_and_retains_opaque_row_data() {
    let source = history_step(Some(1), Some(0), BASICADJ_V1_NATIVE_LE.to_vec(), true, true);
    let BasicAdjHistoryStepDecode::BasicAdjPendingBlend(decoded) =
        decode_basicadj_import_history_step(&source)
    else {
        panic!("native Basic Adjustments v1 must use the operation-local pending leaf");
    };

    assert_eq!(decoded.source, source);
    assert_eq!(decoded.source.operation.raw_name, b"basicadj");
    assert_eq!(decoded.source_version, 1);
    assert!(decoded.migrated);
    assert!(!decoded.enabled);
    assert_eq!(decoded.source.operation_params.bytes, BASICADJ_V1_NATIVE_LE);
    assert_eq!(decoded.source.blend_version, Some(13));
    assert_eq!(decoded.source.blend_params.bytes, [0xaa, 0xbb, 0xcc]);
    assert_eq!(decoded.source.multi_priority, Some(3));
    assert_eq!(decoded.source.multi_name.bytes, b"named-instance");
    assert_eq!(decoded.source.multi_name_hand_edited, Some(1));
    assert_eq!(
        decoded.execution_blocker.code,
        BasicAdjHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
}

#[test]
fn operation_local_import_leaf_preserves_future_and_invalid_rows_exactly() {
    let future = history_step(Some(9), Some(1), vec![0xde, 0xad], true, true);
    let BasicAdjHistoryStepDecode::Preserved {
        source: preserved,
        finding,
    } = decode_basicadj_import_history_step(&future)
    else {
        panic!("future Basic Adjustments versions must remain opaque");
    };
    assert_eq!(
        finding.code,
        BasicAdjHistoryDecodeFindingCode::UnsupportedParameterVersion
    );
    assert_eq!(preserved, future);

    let invalid_enabled =
        history_step(Some(1), Some(9), BASICADJ_V1_NATIVE_LE.to_vec(), true, true);
    let BasicAdjHistoryStepDecode::Preserved {
        source: preserved,
        finding,
    } = decode_basicadj_import_history_step(&invalid_enabled)
    else {
        panic!("invalid enabled state must remain opaque");
    };
    assert_eq!(
        finding.code,
        BasicAdjHistoryDecodeFindingCode::InvalidEnabledState
    );
    assert_eq!(preserved, invalid_enabled);
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
