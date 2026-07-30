use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use super::{
    CompatHistory, CompatHistoryStep, CompatModuleInstance, DarktableSchema, EnabledState, Finding,
    FindingCode, HistoryDecodeOptions, HistoryDecoder, HistoryRows, HistorySelection,
    ModuleInstanceId, OpaquePayload, OperationCompatibility, OperationIdentity, Severity,
    SourceRowKey, finding, order,
};

impl HistoryDecoder {
    #[must_use]
    pub const fn new(options: HistoryDecodeOptions) -> Self {
        Self { options }
    }

    #[must_use]
    pub fn decode(&self, schema: DarktableSchema, mut rows: HistoryRows) -> Vec<CompatHistory> {
        rows.history
            .sort_by_key(|row| (row.image_id, row.num, row.source_row));
        rows.images
            .sort_by_key(|row| (row.image_id, row.source_row));
        rows.module_orders
            .sort_by_key(|row| (row.image_id, row.source_row));
        rows.hashes
            .sort_by_key(|row| (row.image_id, row.source_row));
        let mut image_ids = BTreeSet::new();
        image_ids.extend(rows.history.iter().map(|row| row.image_id));
        image_ids.extend(rows.images.iter().map(|row| row.image_id));
        image_ids.extend(rows.module_orders.iter().map(|row| row.image_id));
        image_ids.extend(rows.hashes.iter().map(|row| row.image_id));
        image_ids
            .into_iter()
            .map(|image_id| self.decode_image(schema, image_id, &rows))
            .collect()
    }

    #[allow(clippy::too_many_lines)]
    fn decode_image(
        &self,
        schema: DarktableSchema,
        image_id: i64,
        rows: &HistoryRows,
    ) -> CompatHistory {
        let mut findings = Vec::new();
        let image_rows = rows
            .images
            .iter()
            .filter(|row| row.image_id == image_id)
            .collect::<Vec<_>>();
        let history_end = image_rows.first().and_then(|row| row.history_end);
        if image_rows.len() > 1 {
            finding(
                &mut findings,
                FindingCode::MultipleHistoryEndpoints,
                Severity::Blocking,
                image_rows[1].source_row,
                "multiple images.history rows provide competing history endpoints",
            );
        }
        if image_rows.is_empty() {
            finding(
                &mut findings,
                FindingCode::MissingHistoryImage,
                Severity::Blocking,
                0,
                "history-related rows reference an image absent from main.images",
            );
        }
        if history_end.is_none() {
            finding(
                &mut findings,
                FindingCode::MissingHistoryEnd,
                Severity::Warning,
                image_rows.first().map_or(0, |row| row.source_row),
                "history endpoint is unavailable in this source schema or row",
            );
        } else if history_end.is_some_and(|end| end < 0) {
            finding(
                &mut findings,
                FindingCode::InvalidHistoryEnd,
                Severity::Blocking,
                image_rows[0].source_row,
                "history_end must not be negative",
            );
        }

        let mut raw_history = rows
            .history
            .iter()
            .filter(|row| row.image_id == image_id)
            .cloned()
            .collect::<Vec<_>>();
        if raw_history.len() > self.options.limits.max_rows {
            raw_history.truncate(self.options.limits.max_rows);
            finding(
                &mut findings,
                FindingCode::HistoryRowLimit,
                Severity::Blocking,
                0,
                "history row limit truncated the compatibility projection",
            );
        }
        let mut steps = Vec::with_capacity(raw_history.len());
        let mut payload_bytes = 0_usize;
        let mut name_bytes = 0_usize;
        let mut instance_sources = BTreeMap::<(Vec<u8>, i64, Vec<u8>), SourceRowKey>::new();
        let mut instance_ids = BTreeMap::<(Vec<u8>, i64, Vec<u8>), ModuleInstanceId>::new();
        let mut seen_instance_keys = BTreeMap::<(Vec<u8>, i64), Vec<u8>>::new();
        for raw in raw_history {
            let source = SourceRowKey::new("main.history", raw.source_row);
            let operation_raw = raw.operation.clone().unwrap_or_default();
            if raw.operation.is_none() {
                finding(
                    &mut findings,
                    FindingCode::MissingOperation,
                    Severity::Blocking,
                    raw.source_row,
                    "history.operation is NULL",
                );
            }
            let operation =
                OperationIdentity::decode(operation_raw.clone(), &self.options.manifest);
            match operation.compatibility {
                OperationCompatibility::InvalidName => finding(
                    &mut findings,
                    FindingCode::InvalidOperationName,
                    Severity::Blocking,
                    raw.source_row,
                    "history.operation is not valid UTF-8",
                ),
                OperationCompatibility::Unknown => finding(
                    &mut findings,
                    FindingCode::UnknownOperation,
                    Severity::Blocking,
                    raw.source_row,
                    "operation is absent from the pinned compatibility manifest",
                ),
                OperationCompatibility::Known { .. } => {}
            }
            let priority = raw.multi_priority.unwrap_or(0);
            if raw.multi_priority.is_some_and(|value| value < 0) {
                finding(
                    &mut findings,
                    FindingCode::InvalidMultiPriority,
                    Severity::Blocking,
                    raw.source_row,
                    "multi_priority must not be negative",
                );
            }
            let name = raw.multi_name.clone().unwrap_or_default();
            name_bytes = name_bytes.saturating_add(name.len());
            if name_bytes > self.options.limits.max_name_bytes {
                finding(
                    &mut findings,
                    FindingCode::HistoryNameLimit,
                    Severity::Blocking,
                    raw.source_row,
                    "history instance-name bytes exceeded the configured limit",
                );
            }
            if String::from_utf8(name.clone()).is_err() {
                finding(
                    &mut findings,
                    FindingCode::InvalidMultiName,
                    Severity::Warning,
                    raw.source_row,
                    "multi_name is retained as bytes but is not valid UTF-8",
                );
            }
            let instance_key = (operation_raw.clone(), priority, name.clone());
            let instance_key_without_name = (operation_raw.clone(), priority);
            if let Some(previous_name) =
                seen_instance_keys.insert(instance_key_without_name, name.clone())
                && previous_name != name
            {
                finding(
                    &mut findings,
                    FindingCode::DuplicateInstanceKey,
                    Severity::Blocking,
                    raw.source_row,
                    "operation and multi_priority identify more than one multi_name",
                );
            }
            let first_source = *instance_sources
                .entry(instance_key.clone())
                .or_insert(source);
            let instance_id = *instance_ids.entry(instance_key).or_insert_with(|| {
                module_instance_id(
                    schema,
                    image_id,
                    &operation_raw,
                    priority,
                    &name,
                    first_source,
                )
            });
            let operation_params = OpaquePayload::from_optional(raw.operation_params.as_deref());
            let blend_params = OpaquePayload::from_optional(raw.blend_params.as_deref());
            payload_bytes = payload_bytes
                .saturating_add(operation_params.len())
                .saturating_add(blend_params.len());
            if payload_bytes > self.options.limits.max_payload_bytes {
                finding(
                    &mut findings,
                    FindingCode::HistoryPayloadLimit,
                    Severity::Blocking,
                    raw.source_row,
                    "history parameter/blend bytes exceeded the configured limit",
                );
            }
            let enabled = EnabledState::decode(raw.enabled);
            if matches!(enabled, EnabledState::Missing | EnabledState::Invalid(_)) {
                finding(
                    &mut findings,
                    FindingCode::InvalidEnabled,
                    Severity::Blocking,
                    raw.source_row,
                    "history.enabled is missing or not 0/1",
                );
            }
            let selected = history_end.is_some_and(|end| raw.num >= 0 && raw.num < end);
            steps.push(CompatHistoryStep {
                source,
                image_id,
                num: raw.num,
                module: raw.module,
                operation,
                operation_params,
                enabled,
                selected,
                blend_params,
                blend_version: raw.blend_version,
                multi_priority: raw.multi_priority,
                multi_name: OpaquePayload::from_optional(raw.multi_name.as_deref()),
                multi_name_hand_edited: raw.multi_name_hand_edited,
                instance_id,
            });
        }
        validate_numbers(&steps, &mut findings);
        validate_history_end(history_end, &steps, &mut findings);
        let instances = build_instances(&steps, &instance_ids, &instance_sources);
        let selection = HistorySelection {
            history_end,
            selected_rows: steps
                .iter()
                .filter(|step| step.selected)
                .map(|step| step.source)
                .collect(),
            redo_rows: steps
                .iter()
                .filter(|step| !step.selected)
                .map(|step| step.source)
                .collect(),
        };
        let module_order = order::decode_module_order(
            image_id,
            rows,
            &self.options.limits,
            &mut findings,
            &self.options.manifest,
        );
        let (operation_order, order_source, order_proven) = order::order_instances(
            &instances,
            &steps,
            module_order.as_ref(),
            &self.options.manifest,
            &mut findings,
        );
        let history_hash = order::decode_hash(
            image_id,
            rows,
            &steps,
            module_order.as_ref(),
            history_end,
            &mut findings,
        );
        findings.truncate(self.options.limits.max_findings);
        let executable = order_proven
            && !findings
                .iter()
                .any(|finding| finding.severity == Severity::Blocking);
        CompatHistory {
            schema,
            image_id,
            steps,
            instances,
            selection,
            module_order,
            history_hash,
            operation_order,
            order_source,
            order_proven,
            executable,
            findings,
        }
    }
}

fn module_instance_id(
    schema: DarktableSchema,
    image_id: i64,
    operation: &[u8],
    priority: i64,
    name: &[u8],
    source: SourceRowKey,
) -> ModuleInstanceId {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.darktable.history.instance.v1");
    hasher.update(schema.library().to_le_bytes());
    hasher.update(schema.data().to_le_bytes());
    hasher.update(image_id.to_le_bytes());
    update_bytes(&mut hasher, operation);
    hasher.update(priority.to_le_bytes());
    update_bytes(&mut hasher, name);
    hasher.update(source.row().to_le_bytes());
    ModuleInstanceId(hasher.finalize().into())
}

fn update_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn validate_numbers(steps: &[CompatHistoryStep], findings: &mut Vec<Finding>) {
    let mut numbers = BTreeSet::new();
    for step in steps {
        if step.num < 0 {
            finding(
                findings,
                FindingCode::InvalidHistoryNumber,
                Severity::Blocking,
                step.source.row(),
                "history.num must not be negative",
            );
        }
        if !numbers.insert(step.num) {
            finding(
                findings,
                FindingCode::DuplicateHistoryNumber,
                Severity::Blocking,
                step.source.row(),
                "history.num is duplicated for this image",
            );
        }
    }
    if let (Some(first), Some(last)) = (numbers.first(), numbers.last())
        && (*first != 0 || *last - *first + 1 != i64::try_from(numbers.len()).unwrap_or(-1))
    {
        finding(
            findings,
            FindingCode::HistoryNumberGap,
            Severity::Blocking,
            0,
            "history.num has a gap or does not start at zero",
        );
    }
}

fn validate_history_end(
    history_end: Option<i64>,
    steps: &[CompatHistoryStep],
    findings: &mut Vec<Finding>,
) {
    let Some(history_end) = history_end else {
        return;
    };
    let max_end = steps
        .iter()
        .map(|step| step.num)
        .max()
        .and_then(|num| num.checked_add(1))
        .unwrap_or(0);
    if history_end > max_end {
        finding(
            findings,
            FindingCode::InvalidHistoryEnd,
            Severity::Blocking,
            0,
            "history_end is beyond the last persisted history row",
        );
    }
}

fn build_instances(
    steps: &[CompatHistoryStep],
    instance_ids: &BTreeMap<(Vec<u8>, i64, Vec<u8>), ModuleInstanceId>,
    instance_sources: &BTreeMap<(Vec<u8>, i64, Vec<u8>), SourceRowKey>,
) -> Vec<CompatModuleInstance> {
    let mut grouped = BTreeMap::<ModuleInstanceId, Vec<&CompatHistoryStep>>::new();
    for step in steps {
        grouped.entry(step.instance_id).or_default().push(step);
    }
    let mut instances = grouped
        .into_iter()
        .map(|(id, mut grouped_steps)| {
            grouped_steps.sort_by_key(|step| step.source.row());
            let first = grouped_steps[0];
            CompatModuleInstance {
                id,
                operation: first.operation.clone(),
                multi_priority: first.multi_priority,
                multi_name: first.multi_name.clone(),
                multi_name_display: String::from_utf8(first.multi_name.bytes.clone()).ok(),
                multi_name_hand_edited: first.multi_name_hand_edited,
                first_source: first.source,
                history_sources: grouped_steps.iter().map(|step| step.source).collect(),
            }
        })
        .collect::<Vec<_>>();
    let _ = (instance_ids, instance_sources);
    instances.sort_by_key(|instance| (instance.first_source.row(), instance.id));
    instances
}
