use rusttable_compat::{
    DarktableOperationManifest, FindingCode, HistoryDecodeOptions, HistoryDecoder, HistoryLimits,
    HistoryOrderSource, OperationCompatibility, Severity,
};
use rusttable_sqlite_native::{
    DarktableSchema, HistoryRows, RawHistoryRow, RawImageHistoryRow, RawModuleOrderRow,
};

fn fixture_rows() -> HistoryRows {
    HistoryRows {
        history: vec![
            RawHistoryRow {
                source_row: 22,
                image_id: 7,
                num: 1,
                module: Some(1),
                operation: Some(b"exposure".to_vec()),
                operation_params: Some(vec![9, 8, 7]),
                enabled: Some(1),
                blend_params: Some(vec![4, 3]),
                blend_version: Some(2),
                multi_priority: Some(0),
                multi_name: Some(b"base".to_vec()),
                multi_name_hand_edited: Some(1),
            },
            RawHistoryRow {
                source_row: 21,
                image_id: 7,
                num: 0,
                module: Some(1),
                operation: Some(b"temperature".to_vec()),
                operation_params: Some(vec![1, 2, 3, 4]),
                enabled: Some(1),
                blend_params: Some(vec![5]),
                blend_version: Some(1),
                multi_priority: Some(0),
                multi_name: Some(b"base".to_vec()),
                multi_name_hand_edited: Some(0),
            },
            RawHistoryRow {
                source_row: 23,
                image_id: 7,
                num: 2,
                module: Some(1),
                operation: Some(b"temperature".to_vec()),
                operation_params: Some(vec![6]),
                enabled: Some(0),
                blend_params: None,
                blend_version: Some(1),
                multi_priority: Some(1),
                multi_name: Some(b"second".to_vec()),
                multi_name_hand_edited: Some(1),
            },
        ],
        images: vec![RawImageHistoryRow {
            source_row: 4,
            image_id: 7,
            history_end: Some(2),
        }],
        module_orders: vec![RawModuleOrderRow {
            source_row: 5,
            image_id: 7,
            version: Some(0),
            operation_list: Some(b"temperature,0,exposure,0,temperature,1".to_vec()),
        }],
        hashes: Vec::new(),
    }
}

fn decoder() -> HistoryDecoder {
    HistoryDecoder::new(HistoryDecodeOptions {
        limits: HistoryLimits::default(),
        manifest: DarktableOperationManifest::reference(),
    })
}

#[test]
fn history_rows_preserve_opaque_payloads_instances_and_redo_tail() {
    let image = decoder()
        .decode(DarktableSchema::new(57, 13), fixture_rows())
        .pop()
        .expect("fixture has one image");

    assert_eq!(image.steps.len(), 3);
    assert_eq!(image.steps[0].source.row(), 21);
    assert!(image.steps[0].selected);
    assert!(!image.steps[2].selected);
    assert_eq!(image.steps[1].operation_params.bytes, [9, 8, 7]);
    assert_eq!(image.steps[1].blend_params.sha256.len(), 32);
    assert_eq!(image.instances.len(), 3);
    assert_eq!(
        image.instances[0].multi_name_display.as_deref(),
        Some("base")
    );
    assert_eq!(image.selection.selected_rows.len(), 2);
    assert_eq!(image.selection.redo_rows.len(), 1);
    assert_eq!(
        image.order_source,
        Some(HistoryOrderSource::CustomModuleOrder)
    );
    assert!(image.order_proven);
    assert!(image.executable);
    assert!(matches!(
        image.steps[0].operation.compatibility,
        OperationCompatibility::Known {
            current_version: 4,
            ..
        }
    ));
}

#[test]
fn history_decode_is_independent_of_physical_row_order() {
    let mut shuffled = fixture_rows();
    shuffled.history.reverse();
    shuffled.module_orders.reverse();
    assert_eq!(
        decoder().decode(DarktableSchema::new(57, 13), fixture_rows()),
        decoder().decode(DarktableSchema::new(57, 13), shuffled)
    );
}

#[test]
fn unknown_operation_and_order_conflict_are_preserved_and_blocking() {
    let mut rows = fixture_rows();
    rows.history[0].operation = Some(b"future_operation".to_vec());
    rows.module_orders[0].operation_list = Some(b"future_operation,0,exposure,0".to_vec());
    let image = decoder()
        .decode(DarktableSchema::new(57, 13), rows)
        .pop()
        .expect("fixture has one image");
    assert!(!image.executable);
    assert!(image.findings.iter().any(|finding| {
        finding.code == FindingCode::UnknownOperation && finding.severity == Severity::Blocking
    }));
    assert!(
        image
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::UnknownModuleOrderOperation)
    );
}

#[test]
fn limits_stop_executable_projection() {
    let options = HistoryDecodeOptions {
        limits: HistoryLimits {
            max_payload_bytes: 2,
            ..HistoryLimits::default()
        },
        manifest: DarktableOperationManifest::reference(),
    };
    let image = HistoryDecoder::new(options)
        .decode(DarktableSchema::new(57, 13), fixture_rows())
        .pop()
        .expect("fixture has one image");
    assert!(!image.executable);
    assert!(
        image
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::HistoryPayloadLimit)
    );
}
