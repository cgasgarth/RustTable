use rusttable_compat::{
    CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions, HistoryDecoder,
    HistoryLimits,
};
use rusttable_sqlite_native::{
    DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow, RawModuleOrderRow,
};

use rusttable_compat::sharpen::{
    SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE, SHARPEN_V1_PARAMETER_BYTES,
    SharpenHistoryDecodeFindingCode, SharpenParametersV1,
};
use rusttable_import::darktable::{
    DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode, decode_history_step,
};

const SHARPEN_V1_NATIVE_LE: &[u8; SHARPEN_V1_PARAMETER_BYTES] =
    include_bytes!("fixtures/sharpen-v1-native-le.bin");

#[test]
fn import_helper_decodes_fixture_and_preserves_history_order_and_instance_identity() {
    // `src/iop/sharpen.c:42-47` declares radius, amount, threshold in this
    // order. Lines 98-108 register the raw-only built-in preset as (2.0,
    // 0.5, 0.5), which is the exact little-endian fixture below.
    assert_eq!(SHARPEN_V1_NATIVE_LE, &SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE);
    let preset = SharpenParametersV1::from_native_le(SHARPEN_V1_NATIVE_LE)
        .expect("native built-in preset has three finite f32 values");
    assert_eq!(preset.radius.to_bits(), 2.0_f32.to_bits());
    assert_eq!(preset.amount.to_bits(), 0.5_f32.to_bits());
    assert_eq!(preset.threshold.to_bits(), 0.5_f32.to_bits());

    let rows = ordered_rows();
    let history = decode_history(rows.clone());
    assert!(history.order_proven);
    assert_eq!(history.steps.len(), 2);
    // The decoder sorts persisted history by `num`, not by source row/input
    // order, while the custom module_order row supplies the execution order.
    assert_eq!(history.steps[0].num, 0);
    assert_eq!(history.steps[1].num, 1);
    assert_eq!(ordered_source_rows(&history), [42, 41]);
    assert_ne!(history.steps[0].instance_id, history.steps[1].instance_id);
    assert_eq!(history.steps[0].operation.raw_name, b"sharpen");
    assert_eq!(history.steps[1].operation.raw_name, b"sharpen");

    let repeated = decode_history(rows);
    assert_eq!(
        history
            .steps
            .iter()
            .map(|step| step.instance_id)
            .collect::<Vec<_>>(),
        repeated
            .steps
            .iter()
            .map(|step| step.instance_id)
            .collect::<Vec<_>>()
    );

    let first = decode_history_step(&history.steps[0]);
    let DarktableHistoryStepDecode::SharpenPendingBlend(first) = first else {
        panic!("native Sharpen v1 fixture must decode as typed pending core");
    };
    assert_eq!(first.source.operation_params.bytes, SHARPEN_V1_NATIVE_LE);
    assert_eq!(first.source_version, 1);
    assert!(!first.enabled);
    assert_eq!(first.canonical_parameters, *SHARPEN_V1_NATIVE_LE);
    assert_eq!(first.parameters, preset);
    assert_eq!(
        first.execution_blocker.code,
        SharpenHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert!(first.execution_blocker.detail.contains("blend/mask"));
    assert!(first.execution_blocker.detail.contains("multi-instance"));
    assert_eq!(first.source.instance_id, history.steps[0].instance_id);
    assert_eq!(first.source.blend_params.bytes, [0xaa, 0xbb]);
}

#[test]
fn import_helper_preserves_future_malformed_nonfinite_and_enabled_state_rows() {
    let cases = [
        (
            Some(2),
            Some(1),
            vec![9, 8, 7],
            DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
        ),
        (
            Some(1),
            Some(1),
            vec![0; SHARPEN_V1_PARAMETER_BYTES - 1],
            DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
        ),
        (
            Some(1),
            Some(7),
            SHARPEN_V1_NATIVE_LE.to_vec(),
            DarktableHistoryDecodeFindingCode::InvalidEnabledState,
        ),
    ];
    for (module, enabled, payload, code) in cases {
        let source = single_step(module, enabled, payload);
        assert_preserved_exact(&source, code);
    }

    let mut nonfinite = *SHARPEN_V1_NATIVE_LE;
    nonfinite[8..12].copy_from_slice(&f32::INFINITY.to_le_bytes());
    let source = single_step(Some(1), Some(1), nonfinite.to_vec());
    assert_preserved_exact(
        &source,
        DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
    );
}

fn decode_history(rows: HistoryRows) -> rusttable_compat::CompatHistory {
    HistoryDecoder::new(HistoryDecodeOptions {
        limits: HistoryLimits::default(),
        manifest: manifest(),
    })
    .decode(DarktableSchema::new(57, 13), rows)
    .remove(0)
}

fn ordered_rows() -> HistoryRows {
    HistoryRows {
        history: vec![
            raw_history_row(
                42,
                1,
                1,
                Some(1),
                Some(1),
                SharpenParametersV1 {
                    radius: 1.25,
                    amount: 0.75,
                    threshold: 3.5,
                }
                .to_native_le()
                .to_vec(),
                b"detail",
            ),
            raw_history_row(
                41,
                0,
                0,
                Some(1),
                Some(0),
                SHARPEN_V1_NATIVE_LE.to_vec(),
                b"base",
            ),
        ],
        images: vec![RawImageHistoryRow {
            source_row: 11,
            image_id: 7,
            history_end: Some(2),
        }],
        module_orders: vec![RawModuleOrderRow {
            source_row: 12,
            image_id: 7,
            version: Some(0),
            operation_list: Some(b"sharpen,1,sharpen,0".to_vec()),
        }],
        ..HistoryRows::default()
    }
}

fn single_step(
    module: Option<i64>,
    enabled: Option<i64>,
    operation_params: Vec<u8>,
) -> CompatHistoryStep {
    decode_history(HistoryRows {
        history: vec![raw_history_row(
            41,
            0,
            0,
            module,
            enabled,
            operation_params,
            b"base",
        )],
        images: vec![RawImageHistoryRow {
            source_row: 11,
            image_id: 7,
            history_end: Some(1),
        }],
        ..HistoryRows::default()
    })
    .steps
    .remove(0)
}

fn raw_history_row(
    source_row: u64,
    num: i64,
    priority: i64,
    module: Option<i64>,
    enabled: Option<i64>,
    operation_params: Vec<u8>,
    multi_name: &[u8],
) -> RawHistoryRow {
    RawHistoryRow {
        source_row,
        image_id: 7,
        num,
        module,
        operation: Some(b"sharpen".to_vec()),
        operation_params: Some(operation_params),
        enabled,
        blend_params: Some(vec![0xaa, 0xbb]),
        blend_version: Some(13),
        multi_priority: Some(priority),
        multi_name: Some(multi_name.to_vec()),
        multi_name_hand_edited: Some(0),
    }
}

fn manifest() -> DarktableOperationManifest {
    let mut manifest = DarktableOperationManifest::new();
    manifest.insert("sharpen", 1, [1], Some(60));
    manifest
}

fn ordered_source_rows(history: &rusttable_compat::CompatHistory) -> Vec<u64> {
    history
        .operation_order
        .iter()
        .map(|instance_id| {
            history
                .instances
                .iter()
                .find(|instance| instance.id == *instance_id)
                .expect("ordered Sharpen instance exists")
                .first_source
                .row()
        })
        .collect()
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
    assert_eq!(
        preserved.operation_params.bytes,
        source.operation_params.bytes
    );
    assert_eq!(preserved.enabled, source.enabled);
    assert_eq!(preserved.module, source.module);
}
