use std::collections::BTreeSet;

use super::{
    CompatHistoryHash, CompatHistoryStep, CompatModuleInstance, CompatModuleOrder,
    DARKTABLE_ORDER_RULES, DarktableOperationManifest, EnabledState, Finding, FindingCode,
    HistoryLimits, HistoryOrderSource, HistoryRows, ModuleInstanceId, ModuleOrderEntry,
    ModuleOrderRule, ModuleOrderVersion, OpaquePayload, Severity, SourceRowKey, finding,
};

pub(super) fn decode_module_order(
    image_id: i64,
    rows: &HistoryRows,
    limits: &HistoryLimits,
    findings: &mut Vec<Finding>,
    manifest: &DarktableOperationManifest,
) -> Option<CompatModuleOrder> {
    let records = rows
        .module_orders
        .iter()
        .filter(|row| row.image_id == image_id)
        .collect::<Vec<_>>();
    let raw = records.first()?;
    if records.len() > 1 {
        finding(
            findings,
            FindingCode::MultipleModuleOrderRows,
            Severity::Blocking,
            raw.source_row,
            "multiple module_order rows provide competing order evidence",
        );
    }
    let Some(version) = ModuleOrderVersion::decode(raw.version) else {
        finding(
            findings,
            FindingCode::InvalidModuleOrderVersion,
            Severity::Blocking,
            raw.source_row,
            "module_order.version is NULL",
        );
        return None;
    };
    if matches!(version, ModuleOrderVersion::Unknown(_)) {
        finding(
            findings,
            FindingCode::InvalidModuleOrderVersion,
            Severity::Blocking,
            raw.source_row,
            "module_order.version is not a supported Darktable order kind",
        );
    }
    let raw_list = OpaquePayload::from_optional(raw.operation_list.as_deref());
    let entries = if raw.operation_list.is_some() {
        parse_order_list(
            raw.operation_list.as_deref().unwrap_or_default(),
            limits.max_module_order_entries,
            raw.source_row,
            findings,
            manifest,
        )
    } else {
        Vec::new()
    };
    if matches!(version, ModuleOrderVersion::Custom) && raw.operation_list.is_none() {
        finding(
            findings,
            FindingCode::InvalidModuleOrderList,
            Severity::Blocking,
            raw.source_row,
            "custom module order has no serialized operation list",
        );
    }
    Some(CompatModuleOrder {
        source: SourceRowKey::new("main.module_order", raw.source_row),
        version,
        raw_list,
        entries,
        rules: DARKTABLE_ORDER_RULES
            .iter()
            .map(|(before, after)| ModuleOrderRule {
                before: before.as_bytes().to_vec(),
                after: after.as_bytes().to_vec(),
            })
            .collect(),
    })
}

fn parse_order_list(
    raw: &[u8],
    max_entries: usize,
    source_row: u64,
    findings: &mut Vec<Finding>,
    manifest: &DarktableOperationManifest,
) -> Vec<ModuleOrderEntry> {
    let Ok(text) = std::str::from_utf8(raw) else {
        finding(
            findings,
            FindingCode::InvalidModuleOrderList,
            Severity::Blocking,
            source_row,
            "module_order.iop_list is not valid UTF-8",
        );
        return Vec::new();
    };
    let tokens = text.split(',').collect::<Vec<_>>();
    if tokens.len() % 2 != 0 || tokens.iter().any(|token| token.is_empty()) {
        finding(
            findings,
            FindingCode::InvalidModuleOrderList,
            Severity::Blocking,
            source_row,
            "module_order.iop_list is not operation,instance pairs",
        );
        return Vec::new();
    }
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for (ordinal, pair) in tokens.chunks(2).enumerate() {
        if ordinal >= max_entries {
            finding(
                findings,
                FindingCode::HistoryOrderLimit,
                Severity::Blocking,
                source_row,
                "module_order entry limit truncated the order projection",
            );
            break;
        }
        let operation = pair[0].as_bytes().to_vec();
        let Ok(instance) = pair[1].parse::<i64>() else {
            finding(
                findings,
                FindingCode::InvalidModuleOrderList,
                Severity::Blocking,
                source_row,
                "module_order instance is not an integer",
            );
            continue;
        };
        if !seen.insert((operation.clone(), instance)) {
            finding(
                findings,
                FindingCode::DuplicateModuleOrderEntry,
                Severity::Blocking,
                source_row,
                "module_order repeats an operation instance",
            );
        }
        if manifest.get(pair[0]).is_none() {
            finding(
                findings,
                FindingCode::UnknownModuleOrderOperation,
                Severity::Blocking,
                source_row,
                "module_order contains an operation absent from the manifest",
            );
        }
        entries.push(ModuleOrderEntry {
            ordinal,
            operation,
            instance,
        });
    }
    entries
}

pub(super) fn order_instances(
    instances: &[CompatModuleInstance],
    steps: &[CompatHistoryStep],
    module_order: Option<&CompatModuleOrder>,
    manifest: &DarktableOperationManifest,
    findings: &mut Vec<Finding>,
) -> (Vec<ModuleInstanceId>, Option<HistoryOrderSource>, bool) {
    let Some(order) = module_order else {
        let mut ordered = instances.to_vec();
        ordered.sort_by_key(|instance| first_step_number(instance.id, steps));
        let proven = !findings.iter().any(|finding| {
            finding.severity == Severity::Blocking
                && matches!(
                    finding.code,
                    FindingCode::InvalidHistoryNumber
                        | FindingCode::DuplicateHistoryNumber
                        | FindingCode::HistoryNumberGap
                )
        });
        return (
            ordered.into_iter().map(|instance| instance.id).collect(),
            proven.then_some(HistoryOrderSource::HistoryNumbers),
            proven,
        );
    };
    if !order.entries.is_empty() {
        let mut result = Vec::new();
        for entry in &order.entries {
            if let Some(instance) = instances.iter().find(|instance| {
                instance.operation.raw_name == entry.operation
                    && instance.multi_priority == Some(entry.instance)
            }) && !result.contains(&instance.id)
            {
                result.push(instance.id);
            }
        }
        for instance in instances {
            if !result.contains(&instance.id) {
                finding(
                    findings,
                    FindingCode::MissingModuleOrderEntry,
                    Severity::Blocking,
                    instance.first_source.row(),
                    "history module instance is absent from serialized module_order",
                );
            }
        }
        check_order_rules(
            &result,
            instances,
            &order.rules,
            findings,
            order.source.row(),
        );
        return (
            result,
            Some(if matches!(order.version, ModuleOrderVersion::Custom) {
                HistoryOrderSource::CustomModuleOrder
            } else {
                HistoryOrderSource::BuiltInModuleOrder
            }),
            !findings
                .iter()
                .any(|finding| finding.code == FindingCode::MissingModuleOrderEntry),
        );
    }
    let (result, proven) = built_in_order(instances, manifest);
    if !proven {
        finding(
            findings,
            FindingCode::ModuleOrderConflict,
            Severity::Blocking,
            order.source.row(),
            "built-in module_order version has no complete manifest order proof",
        );
    }
    check_order_rules(
        &result,
        instances,
        &order.rules,
        findings,
        order.source.row(),
    );
    (result, Some(HistoryOrderSource::BuiltInModuleOrder), proven)
}

fn built_in_order(
    instances: &[CompatModuleInstance],
    manifest: &DarktableOperationManifest,
) -> (Vec<ModuleInstanceId>, bool) {
    let mut ordered = instances.to_vec();
    ordered.sort_by_key(|instance| {
        (
            instance
                .operation
                .name
                .as_deref()
                .and_then(|name| manifest.get(name))
                .and_then(|entry| entry.default_order)
                .unwrap_or(i64::MAX),
            instance.multi_priority.unwrap_or(i64::MAX),
            instance.first_source.row(),
        )
    });
    let proven = ordered.iter().all(|instance| {
        instance.operation.name.as_deref().is_some_and(|name| {
            manifest
                .get(name)
                .is_some_and(|entry| entry.default_order.is_some())
        })
    });
    (
        ordered.into_iter().map(|instance| instance.id).collect(),
        proven,
    )
}

fn check_order_rules(
    order: &[ModuleInstanceId],
    instances: &[CompatModuleInstance],
    rules: &[ModuleOrderRule],
    findings: &mut Vec<Finding>,
    source_row: u64,
) {
    for rule in rules {
        let before = order.iter().position(|id| {
            instances
                .iter()
                .find(|instance| instance.id == *id)
                .is_some_and(|instance| instance.operation.raw_name == rule.before)
        });
        let after = order.iter().position(|id| {
            instances
                .iter()
                .find(|instance| instance.id == *id)
                .is_some_and(|instance| instance.operation.raw_name == rule.after)
        });
        if let (Some(before), Some(after)) = (before, after)
            && before >= after
        {
            finding(
                findings,
                FindingCode::ModuleOrderConflict,
                Severity::Blocking,
                source_row,
                "module_order violates a persisted Darktable operation-order rule",
            );
        }
    }
}

fn first_step_number(id: ModuleInstanceId, steps: &[CompatHistoryStep]) -> i64 {
    steps
        .iter()
        .filter(|step| step.instance_id == id)
        .map(|step| step.num)
        .min()
        .unwrap_or(i64::MAX)
}

pub(super) fn decode_hash(
    image_id: i64,
    rows: &HistoryRows,
    steps: &[CompatHistoryStep],
    module_order: Option<&CompatModuleOrder>,
    history_end: Option<i64>,
    findings: &mut Vec<Finding>,
) -> Option<CompatHistoryHash> {
    let records = rows
        .hashes
        .iter()
        .filter(|row| row.image_id == image_id)
        .collect::<Vec<_>>();
    let raw = records.first()?;
    if records.len() > 1 {
        finding(
            findings,
            FindingCode::MultipleHistoryHashRows,
            Severity::Blocking,
            raw.source_row,
            "multiple history_hash rows provide competing hash state",
        );
    }
    let hash = CompatHistoryHash {
        source: SourceRowKey::new("main.history_hash", raw.source_row),
        basic: OpaquePayload::from_optional(raw.basic_hash.as_deref()),
        auto: OpaquePayload::from_optional(raw.auto_hash.as_deref()),
        current: OpaquePayload::from_optional(raw.current_hash.as_deref()),
        mipmap: OpaquePayload::from_optional(raw.mipmap_hash.as_deref()),
        current_matches: None,
    };
    let expected = darktable_hash(steps, module_order, history_end);
    let current_matches = raw
        .current_hash
        .as_deref()
        .map(|current| current == expected.as_slice());
    if current_matches == Some(false) {
        finding(
            findings,
            FindingCode::HistoryHashMismatch,
            Severity::Warning,
            raw.source_row,
            "history_hash.current_hash disagrees with decoded selected history evidence",
        );
    }
    Some(CompatHistoryHash {
        current_matches,
        ..hash
    })
}

fn darktable_hash(
    steps: &[CompatHistoryStep],
    module_order: Option<&CompatModuleOrder>,
    history_end: Option<i64>,
) -> [u8; 16] {
    let mut digest = md5::Context::new();
    let endpoint = history_end.unwrap_or(i64::MAX);
    let mut seen = BTreeSet::new();
    let mut selected = steps
        .iter()
        .filter(|step| {
            step.num < endpoint
                && matches!(step.enabled, EnabledState::Enabled)
                && seen.insert((step.operation.raw_name.clone(), step.multi_priority))
        })
        .collect::<Vec<_>>();
    selected.sort_by_key(|step| step.num);
    for step in selected {
        digest.consume(&step.operation.raw_name);
        digest.consume(&step.operation_params.bytes);
        digest.consume(&step.blend_params.bytes);
    }
    if let Some(order) = module_order {
        let version = match order.version {
            ModuleOrderVersion::Custom => 0_i32,
            ModuleOrderVersion::Legacy => 1,
            ModuleOrderVersion::V30 => 2,
            ModuleOrderVersion::V30Jpeg => 3,
            ModuleOrderVersion::V50 => 4,
            ModuleOrderVersion::V50Jpeg => 5,
            ModuleOrderVersion::Unknown(value) => {
                i32::try_from(value).unwrap_or(if value.is_negative() {
                    i32::MIN
                } else {
                    i32::MAX
                })
            }
        };
        digest.consume(version.to_ne_bytes());
        if matches!(order.version, ModuleOrderVersion::Custom) {
            digest.consume(&order.raw_list.bytes);
        }
    }
    digest.finalize().0
}
