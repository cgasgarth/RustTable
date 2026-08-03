use rusttable_compat::{
    DarktableOperationManifest, FindingCode, HistoryDecodeOptions, HistoryDecoder, HistoryLimits,
    HistoryOrderSource, OperationCompatibility, Severity,
};
use rusttable_sqlite_native::{
    DarktableSchema, HistoryRows, RawHistoryHashRow, RawHistoryRow, RawImageHistoryRow,
    RawModuleOrderRow,
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

fn v30_rows(priorities: &[i64]) -> HistoryRows {
    let mut rows = fixture_rows();
    let template = rows.history[1].clone();
    rows.history = priorities
        .iter()
        .enumerate()
        .map(|(ordinal, priority)| {
            let source_ordinal = u64::try_from(ordinal).expect("fixture ordinal fits u64");
            let history_ordinal = i64::try_from(ordinal).expect("fixture ordinal fits i64");
            RawHistoryRow {
                source_row: 100 + source_ordinal,
                num: history_ordinal,
                multi_priority: Some(*priority),
                multi_name: Some(format!("temperature-{priority}").into_bytes()),
                ..template.clone()
            }
        })
        .collect();
    rows.images[0].history_end =
        Some(i64::try_from(priorities.len()).expect("fixture length fits i64"));
    rows.module_orders[0].version = Some(2);
    rows.module_orders[0].operation_list = None;
    rows
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

#[test]
fn v30_without_list_proves_a_single_default_instance() {
    let image = decoder()
        .decode(DarktableSchema::new(57, 13), v30_rows(&[0]))
        .pop()
        .expect("fixture has one image");

    assert_eq!(
        image.order_source,
        Some(HistoryOrderSource::BuiltInModuleOrder)
    );
    assert!(image.order_proven);
    assert!(image.executable);
    assert!(
        !image
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::ModuleOrderConflict)
    );
}

#[test]
fn v30_without_list_rejects_multiple_instances_but_orders_them_deterministically() {
    let image = decoder()
        .decode(DarktableSchema::new(57, 13), v30_rows(&[1, 0]))
        .pop()
        .expect("fixture has one image");
    let ordered_priorities = image
        .operation_order
        .iter()
        .map(|id| {
            image
                .instances
                .iter()
                .find(|instance| instance.id == *id)
                .expect("order references a decoded instance")
                .multi_priority
        })
        .collect::<Vec<_>>();

    assert_eq!(ordered_priorities, [Some(0), Some(1)]);
    assert!(!image.order_proven);
    assert!(!image.executable);
    assert!(image.findings.iter().any(|finding| {
        finding.code == FindingCode::ModuleOrderConflict && finding.severity == Severity::Blocking
    }));
}

#[test]
fn v30_without_list_rejects_a_lone_nonzero_instance() {
    let image = decoder()
        .decode(DarktableSchema::new(57, 13), v30_rows(&[1]))
        .pop()
        .expect("fixture has one image");

    assert_eq!(image.operation_order.len(), 1);
    assert!(!image.order_proven);
    assert!(!image.executable);
    assert!(image.findings.iter().any(|finding| {
        finding.code == FindingCode::ModuleOrderConflict && finding.severity == Severity::Blocking
    }));
}

#[test]
fn every_native_builtin_order_version_proves_its_source_order() {
    let cases = [
        (1, ["basecurve", "bilateral"]),
        (2, ["bilateral", "basecurve"]),
        (3, ["colorin", "denoiseprofile"]),
        (4, ["finalscale", "colorout"]),
        (5, ["finalscale", "colorout"]),
    ];
    for (version, expected) in cases {
        let image = decoder()
            .decode(
                DarktableSchema::new(57, 13),
                builtin_rows(version, &expected),
            )
            .pop()
            .expect("fixture has one image");

        assert!(image.order_proven, "order version {version} must be proven");
        assert!(
            image.executable,
            "order version {version} must be executable"
        );
        let names = image
            .operation_order
            .iter()
            .map(|id| {
                image
                    .instances
                    .iter()
                    .find(|instance| instance.id == *id)
                    .and_then(|instance| instance.operation.name.as_deref())
                    .expect("built-in operation is known")
            })
            .collect::<Vec<_>>();
        assert_eq!(names, expected, "wrong native order for version {version}");
    }
}

#[test]
fn selected_repeated_rows_only_activate_the_latest_instance_row() {
    let mut rows = builtin_rows(2, &["basecurve", "basecurve"]);
    rows.history[0].source_row = 20;
    rows.history[1].source_row = 21;
    let image = decoder()
        .decode(DarktableSchema::new(57, 13), rows)
        .pop()
        .expect("fixture has one image");

    assert!(image.executable);
    assert_eq!(
        image.selection.active_rows,
        [rusttable_compat::SourceRowKey::new("main.history", 21)]
    );
}

#[test]
fn history_hash_uses_the_latest_enabled_selected_row() {
    let mut rows = builtin_rows(2, &["basecurve", "basecurve"]);
    rows.history[0].operation_params = Some(vec![1]);
    rows.history[0].blend_params = Some(vec![2]);
    rows.history[1].operation_params = Some(vec![9]);
    rows.history[1].blend_params = Some(vec![8]);
    let mut digest = md5::Context::new();
    digest.consume(b"basecurve");
    digest.consume([9]);
    digest.consume([8]);
    digest.consume(2_i32.to_ne_bytes());
    rows.hashes = vec![RawHistoryHashRow {
        source_row: 102,
        image_id: 7,
        basic_hash: None,
        auto_hash: None,
        current_hash: Some(digest.finalize().0.to_vec()),
        mipmap_hash: None,
    }];

    let image = decoder()
        .decode(DarktableSchema::new(57, 13), rows)
        .pop()
        .expect("fixture has one image");
    assert_eq!(
        image
            .history_hash
            .expect("hash row is decoded")
            .current_matches,
        Some(true)
    );
}

#[test]
fn a_newer_disabled_row_supersedes_an_older_enabled_row() {
    let mut rows = builtin_rows(2, &["basecurve", "basecurve"]);
    rows.history[0].enabled = Some(1);
    rows.history[1].enabled = Some(0);
    let image = decoder()
        .decode(DarktableSchema::new(57, 13), rows)
        .pop()
        .expect("fixture has one image");

    assert!(image.executable);
    assert!(image.selection.active_rows.is_empty());
}

#[test]
fn renamed_selected_rows_share_one_instance_and_keep_the_latest_name() {
    let mut rows = builtin_rows(2, &["basecurve", "basecurve"]);
    rows.history[0].source_row = 20;
    rows.history[1].source_row = 21;
    rows.history[0].multi_name = Some(b"old-name".to_vec());
    rows.history[1].multi_name = Some(b"renamed".to_vec());
    let image = decoder()
        .decode(DarktableSchema::new(57, 13), rows)
        .pop()
        .expect("fixture has one image");

    assert!(image.executable);
    assert_eq!(image.instances.len(), 1);
    assert_eq!(image.steps[0].multi_name.bytes, b"old-name");
    assert_eq!(image.steps[1].multi_name.bytes, b"renamed");
    assert_eq!(image.steps[0].instance_id, image.steps[1].instance_id);
    assert_eq!(image.instances[0].multi_name.bytes, b"renamed");
    assert_eq!(
        image.instances[0].multi_name_display.as_deref(),
        Some("renamed")
    );
    assert_eq!(
        image.instances[0].history_sources,
        [
            rusttable_compat::SourceRowKey::new("main.history", 20),
            rusttable_compat::SourceRowKey::new("main.history", 21),
        ]
    );
    assert_eq!(
        image.selection.active_rows,
        [rusttable_compat::SourceRowKey::new("main.history", 21)]
    );
    assert!(
        !image
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::DuplicateInstanceKey)
    );
}

#[test]
fn missing_module_order_does_not_prove_conflicting_physical_history_order() {
    let mut rows = builtin_rows(2, &["tonecurve", "basecurve"]);
    rows.module_orders.clear();
    let image = decoder()
        .decode(DarktableSchema::new(57, 13), rows)
        .pop()
        .expect("fixture has one image");

    assert_eq!(image.order_source, None);
    assert!(!image.order_proven);
    assert!(!image.executable);
}

#[test]
fn custom_module_order_normalizes_null_priority_to_base_instance() {
    let mut rows = builtin_rows(0, &["basecurve"]);
    rows.history[0].multi_priority = None;
    rows.module_orders[0].operation_list = Some(b"basecurve,0".to_vec());
    let image = decoder()
        .decode(DarktableSchema::new(57, 13), rows)
        .pop()
        .expect("fixture has one image");

    assert!(image.order_proven);
    assert!(image.executable);
    assert_eq!(image.steps[0].multi_priority, None);
}

#[test]
fn history_hash_includes_the_history_end_boundary_row() {
    let mut rows = builtin_rows(2, &["basecurve", "basecurve"]);
    rows.images[0].history_end = Some(1);
    rows.history[0].operation_params = Some(vec![1]);
    rows.history[0].blend_params = Some(vec![2]);
    rows.history[1].operation_params = Some(vec![9]);
    rows.history[1].blend_params = Some(vec![8]);

    let mut digest = md5::Context::new();
    digest.consume(b"basecurve");
    digest.consume([9]);
    digest.consume([8]);
    digest.consume(2_i32.to_ne_bytes());
    let expected = digest.finalize().0;
    rows.hashes = vec![RawHistoryHashRow {
        source_row: 102,
        image_id: 7,
        basic_hash: None,
        auto_hash: None,
        current_hash: Some(expected.to_vec()),
        mipmap_hash: None,
    }];

    let image = decoder()
        .decode(DarktableSchema::new(57, 13), rows)
        .pop()
        .expect("fixture has one image");
    assert!(!image.steps[1].selected);
    let hash = image.history_hash.expect("hash row is decoded");
    assert_eq!(hash.current.bytes, expected);
    assert_eq!(hash.current_matches, Some(true));
}

#[test]
fn blocking_findings_remain_non_executable_when_findings_are_hidden_or_limited() {
    for max_findings in [0, 1] {
        let image = HistoryDecoder::new(HistoryDecodeOptions {
            limits: HistoryLimits {
                max_findings,
                max_payload_bytes: 0,
                ..HistoryLimits::default()
            },
            manifest: DarktableOperationManifest::reference(),
        })
        .decode(DarktableSchema::new(57, 13), fixture_rows())
        .pop()
        .expect("fixture has one image");

        assert!(
            !image.executable,
            "max_findings={max_findings} hid a blocker"
        );
        if max_findings == 0 {
            assert!(image.findings.is_empty());
        }
    }
}

fn builtin_rows(version: i64, operations: &[&str]) -> HistoryRows {
    let history = operations
        .iter()
        .enumerate()
        .map(|(ordinal, operation)| RawHistoryRow {
            source_row: u64::try_from(ordinal).expect("source row fits u64"),
            image_id: 7,
            num: i64::try_from(ordinal).expect("history number fits i64"),
            module: Some(1),
            operation: Some(operation.as_bytes().to_vec()),
            operation_params: Some(Vec::new()),
            enabled: Some(1),
            blend_params: Some(Vec::new()),
            blend_version: Some(1),
            multi_priority: Some(0),
            multi_name: Some(Vec::new()),
            multi_name_hand_edited: Some(0),
        })
        .collect::<Vec<_>>();
    HistoryRows {
        history,
        images: vec![RawImageHistoryRow {
            source_row: 100,
            image_id: 7,
            history_end: Some(i64::try_from(operations.len()).expect("history end fits i64")),
        }],
        module_orders: vec![RawModuleOrderRow {
            source_row: 101,
            image_id: 7,
            version: Some(version),
            operation_list: None,
        }],
        hashes: Vec::new(),
    }
}
