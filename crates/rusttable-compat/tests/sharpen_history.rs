use rusttable_compat::{
    CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions, HistoryDecoder,
    HistoryLimits,
};
use rusttable_sqlite_native::{DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow};

use rusttable_compat::sharpen::{
    SHARPEN_V1_BUILTIN_PRESET_APPLICABILITY, SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE,
    SHARPEN_V1_PARAMETER_BYTES, SharpenBuiltinPresetApplicability, SharpenHistoryDecodeFindingCode,
    SharpenHistoryStepDecode, SharpenParametersV1, decode_sharpen_history_step,
};

#[test]
fn sharpen_v1_preserves_native_field_order_and_builtin_raw_preset() {
    // `src/iop/sharpen.c:42-47` declares radius, amount, threshold in this
    // order. Lines 98-108 register the raw-only preset as (2.0, 0.5, 0.5)
    // with `sizeof(dt_iop_sharpen_params_t)` and no conversion.
    let parameters = SharpenParametersV1::from_native_le(&SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE)
        .expect("native preset has three finite f32 fields");
    assert_eq!(
        SHARPEN_V1_BUILTIN_PRESET_APPLICABILITY,
        SharpenBuiltinPresetApplicability::RawOnly
    );
    assert_eq!(parameters.radius.to_bits(), 2.0_f32.to_bits());
    assert_eq!(parameters.amount.to_bits(), 0.5_f32.to_bits());
    assert_eq!(parameters.threshold.to_bits(), 0.5_f32.to_bits());
    assert_eq!(
        parameters.to_native_le(),
        SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE
    );

    let source = history_step(
        Some(1),
        Some(0),
        SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE.to_vec(),
    );
    let SharpenHistoryStepDecode::SharpenPendingBlend(decoded) =
        decode_sharpen_history_step(&source)
    else {
        panic!("native Sharpen v1 row must decode as typed pending core");
    };

    assert_eq!(decoded.source, source);
    assert_eq!(decoded.source.operation.raw_name, b"sharpen");
    assert_eq!(decoded.source_version, 1);
    assert!(!decoded.enabled);
    assert_eq!(
        decoded.canonical_parameters,
        SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE
    );
    assert_eq!(decoded.parameters, parameters);
    assert_eq!(
        decoded.execution_blocker.code,
        SharpenHistoryDecodeFindingCode::OpaqueBlendSemantics
    );
    assert!(decoded.execution_blocker.detail.contains("blend/mask"));
    assert!(decoded.execution_blocker.detail.contains("multi-instance"));
    assert!(decoded.execution_blocker.detail.contains("no executable"));
    assert_eq!(decoded.source.blend_params.bytes, [0xaa, 0xbb]);
    assert_eq!(decoded.source.multi_name.bytes, b"base");
}

#[test]
fn sharpen_v1_accepts_finite_native_values_without_ui_clamping() {
    let parameters = SharpenParametersV1 {
        radius: 125.0,
        amount: -0.25,
        threshold: 1_000.0,
    };
    let payload = parameters.to_native_le();
    let source = history_step(Some(1), Some(1), payload.to_vec());
    let SharpenHistoryStepDecode::SharpenPendingBlend(decoded) =
        decode_sharpen_history_step(&source)
    else {
        panic!("finite native values remain valid even outside UI slider bounds");
    };
    assert_eq!(decoded.parameters, parameters);
    assert_eq!(decoded.canonical_parameters, payload);
}

#[test]
fn sharpen_unknown_malformed_nonfinite_and_invalid_state_rows_remain_exact() {
    let unknown = history_step(Some(2), Some(1), vec![9, 8, 7]);
    assert_preserved_exact(
        &unknown,
        SharpenHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );

    let future = history_step(Some(99), Some(0), vec![1, 2, 3, 4]);
    assert_preserved_exact(
        &future,
        SharpenHistoryDecodeFindingCode::UnsupportedParameterVersion,
    );

    let malformed = history_step(Some(1), Some(1), vec![0; SHARPEN_V1_PARAMETER_BYTES - 1]);
    assert_preserved_exact(
        &malformed,
        SharpenHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    let mut nonfinite = SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE;
    nonfinite[4..8].copy_from_slice(&f32::NAN.to_le_bytes());
    let nonfinite = history_step(Some(1), Some(1), nonfinite.to_vec());
    assert_preserved_exact(
        &nonfinite,
        SharpenHistoryDecodeFindingCode::InvalidOperationParameters,
    );

    let invalid_enabled = history_step(
        Some(1),
        Some(7),
        SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE.to_vec(),
    );
    assert_preserved_exact(
        &invalid_enabled,
        SharpenHistoryDecodeFindingCode::InvalidEnabledState,
    );

    let missing_version = history_step(None, Some(1), SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE.to_vec());
    assert_preserved_exact(
        &missing_version,
        SharpenHistoryDecodeFindingCode::MissingModuleVersion,
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
                num: 0,
                module,
                operation: Some(b"sharpen".to_vec()),
                operation_params: Some(operation_params),
                enabled,
                blend_params: Some(vec![0xaa, 0xbb]),
                blend_version: Some(13),
                multi_priority: Some(0),
                multi_name: Some(b"base".to_vec()),
                multi_name_hand_edited: Some(0),
            }],
            images: vec![RawImageHistoryRow {
                source_row: 11,
                image_id: 7,
                history_end: Some(1),
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
    manifest.insert("sharpen", 1, [1], Some(60));
    manifest
}

fn assert_preserved_exact(source: &CompatHistoryStep, code: SharpenHistoryDecodeFindingCode) {
    let SharpenHistoryStepDecode::Preserved {
        source: preserved,
        finding,
    } = decode_sharpen_history_step(source)
    else {
        panic!("row must remain opaque");
    };
    assert_eq!(finding.code, code);
    assert_eq!(preserved, *source);
    assert_eq!(preserved.enabled, source.enabled);
    assert_eq!(preserved.module, source.module);
    assert_eq!(
        preserved.operation_params.bytes,
        source.operation_params.bytes
    );
}
