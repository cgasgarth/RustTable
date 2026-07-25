use rusttable_compat::{
    CompatHistory, CompatHistoryStep, DarktableOperationManifest, HistoryDecodeOptions,
    HistoryDecoder, HistoryLimits, HistoryOrderSource,
};
use rusttable_import::darktable::{
    DarktableHistoryDecodeFindingCode, DarktableHistoryStepDecode, decode_history_step,
};
use rusttable_processing::operations::colorcontrast::{
    ColorContrastConfig, ColorContrastParametersV2,
};
use rusttable_processing::operations::colorcorrection::{
    ColorCorrectionConfig, ColorCorrectionParametersV1,
};
use rusttable_processing::operations::velvia::{
    VelviaConfig, VelviaParametersV1, VelviaParametersV2,
};
use rusttable_processing::operations::vibrance::{VibranceConfig, VibranceParametersV2};
use rusttable_sqlite_native::{
    DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow, RawModuleOrderRow,
};

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
const VIBRANCE_V2_NATIVE_LE: [u8; 4] = [
    0x00, 0x00, 0xc8, 0x41, // amount = 25.0
];

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
    manifest.insert("colorcontrast", 2, [1, 2], None);
    manifest.insert("colorcorrection", 1, [1], None);
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
