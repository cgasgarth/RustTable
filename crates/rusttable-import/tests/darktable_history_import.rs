use rusttable_compat::{
    CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions, HistoryDecoder,
    HistoryLimits,
};
use rusttable_import::darktable::{
    DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode, decode_history_step,
};
use rusttable_processing::operations::velvia::{
    VelviaConfig, VelviaParametersV1, VelviaParametersV2,
};
use rusttable_sqlite_native::{DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow};

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
    manifest.insert("velvia", 2, [1, 2], Some(57));
    manifest
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
