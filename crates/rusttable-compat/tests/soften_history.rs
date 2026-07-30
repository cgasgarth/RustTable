use rusttable_compat::soften::{
    SOFTEN_V1_DEFAULT_NATIVE_LE, SOFTEN_V1_PARAMETER_BYTES, SoftenHistoryDecodeFindingCode,
    SoftenHistoryStepDecode, SoftenParametersV1, decode_soften_history_step,
};
use rusttable_compat::{
    CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions, HistoryDecoder,
    HistoryLimits,
};
use rusttable_sqlite_native::{DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow};

const SOFTEN_V1_NATIVE_LE: &[u8; SOFTEN_V1_PARAMETER_BYTES] =
    include_bytes!("../../../fixtures/corpus/assets/operation-soften-params-v1.bin");

#[test]
fn soften_v1_fixture_preserves_native_field_order_and_pending_row_metadata() {
    assert_eq!(SOFTEN_V1_NATIVE_LE, &SOFTEN_V1_DEFAULT_NATIVE_LE);
    let parameters = SoftenParametersV1::from_native_le(SOFTEN_V1_NATIVE_LE)
        .expect("native Soften defaults have four finite f32 values");
    assert_eq!(parameters.size.to_bits(), 50.0_f32.to_bits());
    assert_eq!(parameters.saturation.to_bits(), 100.0_f32.to_bits());
    assert_eq!(parameters.brightness.to_bits(), 0.33_f32.to_bits());
    assert_eq!(parameters.amount.to_bits(), 50.0_f32.to_bits());
    assert_eq!(parameters.to_native_le(), *SOFTEN_V1_NATIVE_LE);

    let source = history_step(Some(1), Some(0), SOFTEN_V1_NATIVE_LE.to_vec());
    let SoftenHistoryStepDecode::SoftenPendingBlend(decoded) = decode_soften_history_step(&source)
    else {
        panic!("native Soften v1 row must decode as typed pending core");
    };

    assert_eq!(decoded.source, source);
    assert_eq!(decoded.source.operation.raw_name, b"soften");
    assert_eq!(decoded.source_version, 1);
    assert!(!decoded.enabled);
    assert_eq!(decoded.parameters, parameters);
    assert_eq!(decoded.canonical_parameters, *SOFTEN_V1_NATIVE_LE);
    assert_eq!(decoded.source.num, 7);
    assert_eq!(decoded.source.multi_priority, Some(3));
    assert_eq!(decoded.source.multi_name.bytes, b"named-instance");
    assert_eq!(decoded.source.multi_name_hand_edited, Some(1));
    assert_eq!(decoded.source.blend_version, Some(13));
    assert_eq!(decoded.source.blend_params.bytes, [0xaa, 0xbb, 0xcc]);
    assert_eq!(decoded.source.instance_id, source.instance_id);
    assert_eq!(
        decoded.execution_blocker.code,
        SoftenHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert!(decoded.execution_blocker.detail.contains("blend/mask"));
    assert!(decoded.execution_blocker.detail.contains("multi-instance"));
    assert!(decoded.execution_blocker.detail.contains("no executable"));
}

#[test]
fn soften_v1_accepts_finite_native_values_without_ui_clamping() {
    let parameters = SoftenParametersV1 {
        size: 125.0,
        saturation: -25.0,
        brightness: 8.0,
        amount: -0.5,
    };
    let source = history_step(Some(1), Some(1), parameters.to_native_le().to_vec());
    let SoftenHistoryStepDecode::SoftenPendingBlend(decoded) = decode_soften_history_step(&source)
    else {
        panic!("finite native values remain valid even outside UI slider bounds");
    };
    assert_eq!(decoded.parameters, parameters);
    assert_eq!(decoded.canonical_parameters, parameters.to_native_le());
}

#[test]
fn soften_unknown_malformed_nonfinite_and_invalid_state_rows_remain_exact() {
    let unknown = history_step(Some(2), Some(1), vec![9, 8, 7]);
    assert_preserved_exact(
        &unknown,
        SoftenHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );

    let future = history_step(Some(99), Some(0), vec![1, 2, 3, 4]);
    assert_preserved_exact(
        &future,
        SoftenHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );

    let malformed = history_step(Some(1), Some(1), vec![0; SOFTEN_V1_PARAMETER_BYTES - 1]);
    assert_preserved_exact(
        &malformed,
        SoftenHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    let mut nonfinite = *SOFTEN_V1_NATIVE_LE;
    nonfinite[8..12].copy_from_slice(&f32::NAN.to_le_bytes());
    let nonfinite = history_step(Some(1), Some(1), nonfinite.to_vec());
    assert_preserved_exact(
        &nonfinite,
        SoftenHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    let invalid_enabled = history_step(Some(1), Some(7), SOFTEN_V1_NATIVE_LE.to_vec());
    assert_preserved_exact(
        &invalid_enabled,
        SoftenHistoryDecodeFindingCode::InvalidEnabledState,
    );

    let missing_version = history_step(None, Some(1), SOFTEN_V1_NATIVE_LE.to_vec());
    assert_preserved_exact(
        &missing_version,
        SoftenHistoryDecodeFindingCode::MissingModuleVersion,
    );

    let invalid_version = history_step(
        Some(i64::from(u16::MAX) + 1),
        Some(1),
        SOFTEN_V1_NATIVE_LE.to_vec(),
    );
    assert_preserved_exact(
        &invalid_version,
        SoftenHistoryDecodeFindingCode::InvalidModuleVersion,
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

fn assert_preserved_exact(source: &CompatHistoryStep, code: SoftenHistoryDecodeFindingCode) {
    let SoftenHistoryStepDecode::Preserved {
        source: preserved,
        finding,
    } = decode_soften_history_step(source)
    else {
        panic!("row must remain opaque");
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
