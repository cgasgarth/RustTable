use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use super::super::super::issue_spec::{SectionRole, extract_sections};
use super::super::super::{Result, report};
use super::super::git_inventory::{
    build_inventory, compare_inventory, validate_anchors, validate_inventory,
};
use super::super::io::{
    body_hash, format_findings, format_findings_value, parse_toml, read_json, read_json_path,
    read_toml, write_json, write_toml,
};
use super::super::model::{
    Anchor, Finding, Inventory, IssueInput, IssueRecord, Responsibility, Selector, SourceMap,
    SourceTree,
};
use super::super::{ISSUE_MAP_SCHEMA, PINNED_COMMIT, REPOSITORY};
use super::audit::audit_records;
use super::policy::{debt_finding, is_master_child, strict_audit_issue_numbers};
use crate::cli::{
    MigrationCommand, SourceMapApplyArgs, SourceMapCommand, SourceMapDriftArgs,
    SourceMapGenerateArgs, SourceMapVerifyArgs,
};
use crate::process::ProcessRunner;
use crate::root::RepositoryRoot;

#[cfg(test)]
#[path = "operations_tests.rs"]
mod tests;

pub(crate) fn run(
    root: &RepositoryRoot,
    command: &MigrationCommand,
    runner: &ProcessRunner,
) -> Result {
    let MigrationCommand::SourceMap { command } = command;
    match command {
        SourceMapCommand::Generate(args) => generate(root, args, runner),
        SourceMapCommand::Verify(args) => verify(root, args, runner),
        SourceMapCommand::AuditIssues(args) => audit_issues(root, args, runner),
        SourceMapCommand::Drift(args) => drift(root, args, runner),
        SourceMapCommand::Apply(args) => apply(root, args, runner),
    }
}

fn generate(root: &RepositoryRoot, args: &SourceMapGenerateArgs, runner: &ProcessRunner) -> Result {
    if args.commit != PINNED_COMMIT {
        return Err(format!(
            "source-map generate is baseline-only; use drift for {}, expected {PINNED_COMMIT}",
            args.commit
        ));
    }
    let tree = SourceTree::open_baseline(&args.reference, runner)?;
    let inventory = build_inventory(&tree, runner)?;
    let issues = load_issues(root, args.api_fixture.as_deref(), runner)?;
    // Capability and operation manifests are separate derived inputs.  They
    // are deliberately not consumed here: a stale manifest must never be
    // relabeled as pinned while the canonical source inventory is generated.
    // `cargo xtask parity verify` is the fail-closed consumer for those inputs.
    let derived_input_debt = derived_input_debt(root);
    let map = build_map(
        &inventory,
        &issues,
        &args.inventory,
        &args.issue_snapshot,
        derived_input_debt,
    );
    write_json(root, &args.inventory, &inventory)?;
    write_json(root, &args.issue_snapshot, &issue_snapshot(&issues))?;
    write_toml(root, &args.map, &map)?;
    let plan = build_plan(&map, &issues, &args.map);
    write_json(root, &args.plan, &plan)?;
    Ok(report(
        root,
        "migration.source-map.generate",
        serde_json::json!({
            "schema": ISSUE_MAP_SCHEMA,
            "source_commit": PINNED_COMMIT,
            "inventory_entries": inventory.entries.len(),
            "issue_records": map.issues.len(),
            "responsibilities": map.responsibilities.len(),
            "derived_input_debt": map.derived_input_debt.len(),
            "plan_operations": plan["operations"].as_array().map_or(0, Vec::len),
            "inputs": { "capabilities": args.capabilities, "issue_index": args.issue_index },
            "outputs": { "map": args.map, "inventory": args.inventory, "issue_snapshot": args.issue_snapshot, "plan": args.plan },
        }),
    ))
}

fn verify(root: &RepositoryRoot, args: &SourceMapVerifyArgs, runner: &ProcessRunner) -> Result {
    let reference = args
        .reference
        .as_ref()
        .ok_or_else(|| "source-map verify requires --reference".to_owned())?;
    let commit = args.commit.as_deref().unwrap_or(PINNED_COMMIT);
    if commit != PINNED_COMMIT {
        return Err(format!(
            "source-map verify is baseline-only; use drift for {commit}, expected {PINNED_COMMIT}"
        ));
    }
    let inventory: Inventory = read_json(root, &args.inventory)?;
    let map: SourceMap = read_toml(root, &args.map)?;
    validate_inventory(&inventory)?;
    validate_map(&map, &inventory, args.issue)?;
    let mut findings = Vec::new();
    if args.issue.is_none() {
        findings.extend(
            map.derived_input_debt
                .iter()
                .cloned()
                .map(|message| Finding::error("stale-derived-input", None, None, message)),
        );
    }
    let tree = SourceTree::open_baseline(reference, runner)?;
    let actual = build_inventory(&tree, runner)?;
    findings.extend(compare_inventory(&inventory, &actual));
    findings.extend(validate_anchors(
        &map, &inventory, &tree, runner, args.issue,
    )?);
    if !findings.is_empty() {
        return Err(format_findings("source-map verify", &findings));
    }
    Ok(report(
        root,
        "migration.source-map.verify",
        serde_json::json!({
            "schema": ISSUE_MAP_SCHEMA, "source_commit": map.source_commit,
            "inventory_entries": inventory.entries.len(), "issue_records": map.issues.len(),
            "issue": args.issue, "status": "verified",
        }),
    ))
}

fn audit_issues(
    root: &RepositoryRoot,
    args: &crate::cli::SourceMapAuditArgs,
    runner: &ProcessRunner,
) -> Result {
    let inventory: Inventory = read_json(root, &args.inventory)?;
    let map: SourceMap = read_toml(root, &args.map)?;
    validate_inventory(&inventory)?;
    validate_map(&map, &inventory, None)?;
    let issues = load_issues(root, args.api_fixture.as_deref(), runner)?;
    let strict_issues = strict_audit_issue_numbers(&map, &issues, args.parent);
    let mut findings = audit_records(&map, &issues, args.parent)
        .into_iter()
        .map(|finding| {
            if finding
                .issue
                .is_some_and(|issue| !strict_issues.contains(&issue))
            {
                debt_finding(finding)
            } else {
                finding
            }
        })
        .collect::<Vec<_>>();
    findings.extend(
        map.derived_input_debt.iter().cloned().map(|message| {
            debt_finding(Finding::error("stale-derived-input", None, None, message))
        }),
    );
    findings.sort_by(|left, right| {
        left.issue
            .cmp(&right.issue)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.message.cmp(&right.message))
    });
    let debt_findings = findings
        .iter()
        .filter(|finding| finding.code.starts_with("debt-"))
        .cloned()
        .collect::<Vec<_>>();
    let debt_issues = debt_findings
        .iter()
        .filter_map(|finding| finding.issue)
        .collect::<BTreeSet<_>>();
    let fatal_finding_count = findings.len().saturating_sub(debt_findings.len());
    let debt_report = serde_json::json!({
        "schema": "rusttable.source-map-migration-debt.v1",
        "repository": REPOSITORY,
        "parent_issue": args.parent,
        "linked_issue": 570,
        "source_commit": map.source_commit,
        "issue_snapshot": args.issue_snapshot,
        "issue_count": issues.len(),
        "strict_issue_count": strict_issues.len(),
        "unowned_inventory_entry_count": map.unowned_inventory_paths.len(),
        "fatal_finding_count": fatal_finding_count,
        "debt_finding_count": debt_findings.len(),
        "debt_issue_count": debt_issues.len(),
        "findings": debt_findings,
    });
    write_json(root, &args.debt_report, &debt_report)?;
    let value = serde_json::json!({ "schema": "rusttable.source-map-audit.v1", "parent_issue": args.parent,
        "source_commit": map.source_commit, "issue_count": issues.len(), "finding_count": findings.len(),
        "fatal_finding_count": fatal_finding_count, "migration_debt_finding_count": debt_findings.len(),
        "unowned_inventory_entry_count": map.unowned_inventory_paths.len(),
        "migration_debt_report": args.debt_report, "status": if fatal_finding_count > 0 { "blocked-by-strict-findings" } else if debt_findings.is_empty() { "verified" } else { "blocked-by-migration-debt" },
        "findings": findings });
    if value["findings"].as_array().is_some_and(|items| {
        items.iter().any(|finding| {
            finding["code"]
                .as_str()
                .is_some_and(|code| code.starts_with("error-"))
        })
    }) {
        return Err(format_findings_value("source-map audit-issues", &value));
    }
    Ok(report(root, "migration.source-map.audit-issues", value))
}

fn drift(root: &RepositoryRoot, args: &SourceMapDriftArgs, runner: &ProcessRunner) -> Result {
    let inventory: Inventory = read_json(root, &args.inventory)?;
    let map: SourceMap = read_toml(root, &args.map)?;
    validate_inventory(&inventory)?;
    validate_map(&map, &inventory, None)?;
    let accepted = SourceTree::open_baseline(&args.reference, runner)?;
    let candidate = SourceTree::open_commit(&args.reference, &args.candidate_commit, runner)?;
    let old = build_inventory(&accepted, runner)?;
    let new = build_inventory(&candidate, runner)?;
    let changed = compare_inventory(&old, &new);
    let impacted = changed
        .iter()
        .filter_map(|finding| finding.path.as_deref())
        .flat_map(|path| {
            map.responsibilities
                .iter()
                .filter(move |item| item.path == path)
                .map(|item| item.issue)
        })
        .collect::<BTreeSet<_>>();
    Ok(report(
        root,
        "migration.source-map.drift",
        serde_json::json!({
            "schema": "rusttable.source-map-drift.v1", "accepted_commit": PINNED_COMMIT,
            "candidate_commit": args.candidate_commit, "changed_entries": changed, "impacted_issues": impacted,
        }),
    ))
}

fn apply(root: &RepositoryRoot, args: &SourceMapApplyArgs, runner: &ProcessRunner) -> Result {
    let plan: Value = read_json(root, &args.plan)?;
    let operations = plan["operations"]
        .as_array()
        .ok_or_else(|| "source-map apply: plan.operations must be an array".to_owned())?;
    let issues = load_issues(root, args.api_fixture.as_deref(), runner)?;
    let by_number = issues
        .iter()
        .map(|issue| (issue.number, issue))
        .collect::<BTreeMap<_, _>>();
    let mut updates = Vec::new();
    for operation in operations {
        let number = operation["issue"]
            .as_u64()
            .ok_or_else(|| "source-map apply: operation.issue is required".to_owned())?;
        let expected = operation["expected_body_hash"]
            .as_str()
            .ok_or_else(|| format!("source-map apply: #{number} expected hash missing"))?;
        let replacement = operation["replacement_body"]
            .as_str()
            .ok_or_else(|| format!("source-map apply: #{number} replacement body missing"))?;
        let issue = by_number
            .get(&number)
            .ok_or_else(|| format!("source-map apply: #{number} absent from current snapshot"))?;
        let actual = body_hash(&issue.body);
        if actual != expected {
            return Err(format!(
                "source-map apply: #{number} body changed; expected {expected}, found {actual}"
            ));
        }
        updates.push((number, replacement.to_owned()));
    }
    if args.confirm && !updates.is_empty() {
        super::super::super::github::apply_issue_spec_updates(runner, &updates)?;
    }
    Ok(report(
        root,
        "migration.source-map.apply",
        serde_json::json!({ "schema": "rusttable.source-map-apply.v1", "confirmed": args.confirm, "operations": updates.len(), "status": if args.confirm { "applied" } else { "validated" } }),
    ))
}

fn validate_map(
    map: &SourceMap,
    inventory: &Inventory,
    selected: Option<u64>,
) -> std::result::Result<(), String> {
    if map.schema != ISSUE_MAP_SCHEMA || map.repository != REPOSITORY || map.parent_issue != 158 {
        return Err("source map schema, repository, or parent is invalid".to_owned());
    }
    if map.source_commit != PINNED_COMMIT {
        return Err(format!(
            "source map uses {}, expected {PINNED_COMMIT}",
            map.source_commit
        ));
    }
    if map.source_tree_id != inventory.tree_id {
        return Err("source map tree identity does not match inventory".to_owned());
    }
    if map.inventory_sha256 != inventory.tree_sha256 {
        return Err("source map inventory checksum does not match inventory".to_owned());
    }
    if map.derived_input_debt.iter().any(String::is_empty) {
        return Err("source map has an empty derived-input debt record".to_owned());
    }
    let mut numbers = BTreeSet::new();
    if let Some(number) = selected
        && !map.issues.iter().any(|record| record.number == number)
    {
        return Err(format!("source map does not contain issue #{number}"));
    }
    let paths = inventory
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    let map_numbers = map
        .issues
        .iter()
        .map(|record| record.number)
        .collect::<BTreeSet<_>>();
    for record in &map.issues {
        if !numbers.insert(record.number) {
            return Err(format!("duplicate issue #{} in source map", record.number));
        }
        if selected.is_some_and(|number| record.number != number) {
            continue;
        }
        if record.preserve.is_empty() || record.redesign.is_empty() || record.excluded.is_empty() {
            return Err(format!(
                "issue #{} lacks Preserve/Redesign/Excluded",
                record.number
            ));
        }
        let mut anchors = BTreeSet::new();
        for anchor in &record.anchors {
            let inventory_entry = inventory
                .entries
                .iter()
                .find(|entry| entry.path == anchor.path);
            if anchor.path.contains('*')
                || anchor.path.contains("..")
                || !selector_is_valid(&anchor.selector)
                || anchor.blob_id.is_empty()
                || anchor.rust_owners.is_empty()
                || anchor.oracle_fixtures.is_empty()
                || !paths.contains(anchor.path.as_str())
                || inventory_entry.is_none_or(|entry| entry.object_id != anchor.blob_id)
                || !anchors.insert((&anchor.path, &anchor.selector))
            {
                return Err(format!(
                    "issue #{} has an invalid or non-unique anchor",
                    record.number
                ));
            }
        }
    }
    for responsibility in &map.responsibilities {
        let inventory_entry = inventory
            .entries
            .iter()
            .find(|entry| entry.path == responsibility.path);
        if !map_numbers.contains(&responsibility.issue)
            || responsibility.source_commit != PINNED_COMMIT
            || !paths.contains(responsibility.path.as_str())
            || !selector_is_valid(&responsibility.selector)
            || responsibility.blob_id.is_empty()
            || responsibility.rust_owners.is_empty()
            || responsibility.oracle_fixtures.is_empty()
            || inventory_entry.is_none_or(|entry| entry.object_id != responsibility.blob_id)
        {
            return Err(format!("invalid responsibility {}", responsibility.id));
        }
    }
    validate_inventory_coverage(map, &paths)
}

fn validate_inventory_coverage(
    map: &SourceMap,
    paths: &BTreeSet<&str>,
) -> std::result::Result<(), String> {
    let unowned = map
        .unowned_inventory_paths
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if unowned.len() != map.unowned_inventory_paths.len()
        || map
            .unowned_inventory_paths
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
        || !unowned.iter().all(|path| paths.contains(path))
    {
        return Err("source map has invalid unowned inventory debt paths".to_owned());
    }
    let owned = map
        .responsibilities
        .iter()
        .map(|responsibility| responsibility.path.as_str())
        .collect::<BTreeSet<_>>();
    if paths
        .iter()
        .any(|path| !owned.contains(path) && !unowned.contains(path))
        || owned.iter().any(|path| !paths.contains(path))
    {
        return Err("source map inventory coverage is incomplete".to_owned());
    }
    Ok(())
}

fn selector_is_valid(selector: &Selector) -> bool {
    match selector {
        Selector::Symbol { name }
        | Selector::Table { name }
        | Selector::ConfigKey { name }
        | Selector::Kernel { name }
        | Selector::Action { name }
        | Selector::Registration { name } => !name.is_empty(),
        Selector::BoundedRange { label, start, end } => {
            !label.is_empty() && *start >= 1 && start <= end
        }
        Selector::InventoryPath => true,
    }
}

fn validate_manifest_commit(value: &Value, path: &str, require_root: bool) -> Result<()> {
    if require_root {
        let root = if path.ends_with("capabilities.toml") {
            value["source_commit"].as_str()
        } else {
            value["reference"]["source_commit"].as_str()
        };
        if root != Some(PINNED_COMMIT) {
            return Err(format!(
                "rejecting stale or incomplete {path}: required canonical source_commit {PINNED_COMMIT}"
            ));
        }
    }
    let mut commits = Vec::new();
    collect_source_commits(value, &mut commits);
    if commits.is_empty() || commits.iter().any(|commit| commit != PINNED_COMMIT) {
        return Err(format!(
            "rejecting stale or incomplete {path}: every source_commit must equal {PINNED_COMMIT}"
        ));
    }
    Ok(())
}

fn derived_input_debt(root: &RepositoryRoot) -> Vec<String> {
    [
        ("architecture/darktable-capabilities.toml", true),
        ("architecture/darktable-operations.toml", true),
        ("architecture/operation-overrides.toml", false),
        ("architecture/darktable-issue-index.toml", false),
    ]
    .into_iter()
    .filter_map(|(path, require_root)| {
        let result = parse_toml(root, path)
            .and_then(|value| validate_manifest_commit(&value, path, require_root));
        result.err().map(|error| format!("{path}: {error}"))
    })
    .collect()
}

fn collect_source_commits(value: &Value, commits: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key == "source_commit"
                    && let Some(commit) = child.as_str()
                {
                    commits.push(commit.to_owned());
                }
                collect_source_commits(child, commits);
            }
        }
        Value::Array(values) => values
            .iter()
            .for_each(|child| collect_source_commits(child, commits)),
        _ => {}
    }
}

fn build_map(
    inventory: &Inventory,
    issues: &[IssueInput],
    inventory_path: &Path,
    snapshot_path: &Path,
    derived_input_debt: Vec<String>,
) -> SourceMap {
    let mut records = issues
        .iter()
        .filter(|issue| !issue.is_pull_request && is_master_child(issue, 158))
        .map(|issue| build_issue_record(issue, inventory))
        .collect::<Vec<_>>();
    if let Some(issue) = issues.iter().find(|issue| issue.number == 555) {
        let mut qualification = self_record(inventory);
        qualification.title.clone_from(&issue.title);
        qualification.state.clone_from(&issue.state);
        qualification.milestone.clone_from(&issue.milestone);
        qualification.body_hash = body_hash(&issue.body);
        qualification
            .source_anchor_hash
            .clone_from(&issue.source_anchor_hash);
        if let Some(record) = records.iter_mut().find(|record| record.number == 555) {
            *record = qualification;
        } else {
            records.push(qualification);
        }
    }
    if !records.iter().any(|record| record.number == 555) {
        records.push(self_record(inventory));
    }
    records.sort_by_key(|record| record.number);
    let responsibilities = responsibilities_from_records(&records);
    let owned_paths = responsibilities
        .iter()
        .map(|item| item.path.as_str())
        .collect::<BTreeSet<_>>();
    let unowned_inventory_paths = inventory
        .entries
        .iter()
        .filter(|entry| !owned_paths.contains(entry.path.as_str()))
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();
    SourceMap {
        schema: ISSUE_MAP_SCHEMA.to_owned(),
        repository: REPOSITORY.to_owned(),
        parent_issue: 158,
        source_commit: PINNED_COMMIT.to_owned(),
        source_tree_id: inventory.tree_id.clone(),
        inventory_path: inventory_path.display().to_string(),
        issue_snapshot_path: snapshot_path.display().to_string(),
        inventory_sha256: inventory.tree_sha256.clone(),
        derived_input_debt,
        unowned_inventory_paths,
        issues: records,
        responsibilities,
    }
}

fn build_issue_record(issue: &IssueInput, inventory: &Inventory) -> IssueRecord {
    let anchors = super::anchors::anchors_from_body(issue.number, &issue.body, inventory);
    let rust_targets = if anchors.is_empty() {
        vec!["issue-specific Rust owner from issue body".to_owned()]
    } else {
        anchors
            .iter()
            .flat_map(|anchor| anchor.rust_owners.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    };
    IssueRecord {
        number: issue.number,
        title: issue.title.clone(),
        state: issue.state.clone(),
        priority: priority(&issue.labels),
        milestone: issue.milestone.clone(),
        body_hash: body_hash(&issue.body),
        source_anchor_hash: issue.source_anchor_hash.clone(),
        source_status: if anchors.is_empty() || issue.source_anchor_hash.is_empty() {
            "blocked-needs-reviewed-source-block".to_owned()
        } else if anchors
            .iter()
            .any(|anchor| matches!(anchor.selector, Selector::InventoryPath))
        {
            "blocked-needs-typed-anchors".to_owned()
        } else {
            "anchored".to_owned()
        },
        class: if issue.number == 555 {
            "no-direct-analogue".to_owned()
        } else {
            "source-owned".to_owned()
        },
        preserve: decision(
            issue,
            "Preserve",
            "Named upstream responsibilities and behavior parity",
        ),
        redesign: decision(
            issue,
            "Redesign",
            "Rust-owned typed contracts and machine-readable ownership",
        ),
        excluded: decision(
            issue,
            "Excluded",
            "Copying or compiling upstream C/C++/OpenCL implementation",
        ),
        rust_targets,
        dependencies: dependencies(&issue.body),
        anchors,
    }
}

fn responsibilities_from_records(records: &[IssueRecord]) -> Vec<Responsibility> {
    let mut responsibilities = records
        .iter()
        .filter(|record| record.source_status == "anchored")
        .flat_map(|record| {
            record
                .anchors
                .iter()
                .enumerate()
                .map(move |(index, anchor)| Responsibility {
                    id: format!("issue-{}-anchor-{index}", record.number),
                    issue: record.number,
                    kind: record.class.clone(),
                    path: anchor.path.clone(),
                    blob_id: anchor.blob_id.clone(),
                    selector: anchor.selector.clone(),
                    role: anchor.role.clone(),
                    source_commit: PINNED_COMMIT.to_owned(),
                    rust_owners: anchor.rust_owners.clone(),
                    oracle_fixtures: anchor.oracle_fixtures.clone(),
                })
        })
        .collect::<Vec<_>>();
    responsibilities.sort_by(|left, right| left.id.cmp(&right.id));
    responsibilities
}

fn decision(issue: &IssueInput, heading: &str, default: &str) -> Vec<String> {
    if issue.number == 555 {
        vec![default.to_owned()]
    } else {
        super::anchors::decision_or_default(&issue.body, heading, default)
    }
}

fn self_record(inventory: &Inventory) -> IssueRecord {
    let paths = [
        "CMakeLists.txt",
        "DefineOptions.cmake",
        "src/main.c",
        "src/common/darktable.c",
        "src/common/database.c",
        "src/imageio/imageio.c",
        "src/develop/develop.c",
        "data/kernels/programs.conf",
        "src/lua/init.c",
    ];
    let available = inventory
        .entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    let anchors = paths
        .iter()
        .filter(|path| available.contains(**path))
        .map(|path| Anchor {
            path: (*path).to_owned(),
            selector: Selector::BoundedRange {
                label: "complete-pinned-tree-inventory".to_owned(),
                start: 1,
                end: 1,
            },
            role: "reference-qualification".to_owned(),
            evidence: "complete-pinned-tree-inventory".to_owned(),
            blob_id: inventory
                .entries
                .iter()
                .find(|entry| entry.path == *path)
                .map(|entry| entry.object_id.clone())
                .unwrap_or_default(),
            rust_owners: vec!["crates/xtask".to_owned()],
            oracle_fixtures: vec!["darktable-source-inventory".to_owned()],
        })
        .collect();
    IssueRecord {
        number: 555,
        title: "Make every migration issue source-anchored to the pinned darktable tree".to_owned(),
        state: "open".to_owned(),
        priority: "priority: P0".to_owned(),
        milestone: Some("Foundation & Developer Workflow".to_owned()),
        body_hash: String::new(),
        source_anchor_hash: String::new(),
        source_status: "anchored".to_owned(),
        class: "no-direct-analogue".to_owned(),
        preserve: vec!["Complete responsibility discovery and exact source identity".to_owned()],
        redesign: vec!["Typed inventory, ownership map, drift, and readiness tooling".to_owned()],
        excluded: vec!["Product algorithm or UI implementation".to_owned()],
        rust_targets: vec!["crates/xtask".to_owned(), "architecture/".to_owned()],
        dependencies: vec![492, 495, 515, 518],
        anchors,
    }
}

fn build_plan(map: &SourceMap, issues: &[IssueInput], map_path: &Path) -> Value {
    let current = issues
        .iter()
        .map(|issue| (issue.number, issue))
        .collect::<BTreeMap<_, _>>();
    let operations = map
        .issues
        .iter()
        .filter(|record| record.number != 555 && record.state == "open")
        .filter_map(|record| current.get(&record.number).map(|issue| (record, *issue)))
        .filter(|(_, issue)| !issue.body.contains("## Upstream darktable source anchors"))
        .map(|(record, issue)| serde_json::json!({ "issue": record.number, "expected_body_hash": body_hash(&issue.body), "replacement_body": format!("{}\n\n{}", issue.body.trim_end(), render_source_block(record)), "map": map_path }))
        .collect::<Vec<_>>();
    serde_json::json!({ "schema": "rusttable.source-map-apply-plan.v1", "source_commit": PINNED_COMMIT, "operations": operations })
}

fn render_source_block(record: &IssueRecord) -> String {
    let anchors = record
        .anchors
        .iter()
        .map(|anchor| {
            format!(
                "- `{}` — `{:?}` ({})",
                anchor.path, anchor.selector, anchor.role
            )
        })
        .collect::<Vec<_>>();
    format!(
        "## Upstream darktable source anchors\n\nCanonical source: `darktable-org/darktable@{PINNED_COMMIT}`.\n\nClassification: `{}`.\n\nAnchors:\n{}\n\n**Preserve:** {}\n\n**Redesign:** {}\n\n**Excluded:** {}\n\nRust owners: {}.\n\nOracle: pinned source inventory and declared acceptance evidence.",
        record.class,
        if anchors.is_empty() {
            "- No reviewed anchor yet; source readiness is blocked.".to_owned()
        } else {
            anchors.join("\n")
        },
        record.preserve.join("; "),
        record.redesign.join("; "),
        record.excluded.join("; "),
        record.rust_targets.join(", ")
    )
}

pub(super) fn priority(labels: &[String]) -> String {
    let values = labels
        .iter()
        .filter(|label| label.starts_with("priority:"))
        .collect::<Vec<_>>();
    if values.len() == 1
        && matches!(
            values[0].as_str(),
            "priority: P0" | "priority: P1" | "priority: P2" | "priority: P3" | "priority: P4"
        )
    {
        values[0].clone()
    } else {
        "invalid".to_owned()
    }
}

#[cfg(test)]
pub(super) fn source_block(body: &str) -> Option<String> {
    super::audit::source_block(body)
}

fn dependencies(body: &str) -> Vec<u64> {
    body.lines()
        .filter(|line| line.to_ascii_lowercase().contains("depends on"))
        .flat_map(|line| line.split('#').skip(1))
        .filter_map(|token| {
            token
                .split(|character: char| !character.is_ascii_digit())
                .next()
        })
        .filter_map(|token| token.parse::<u64>().ok())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn load_issues(
    root: &RepositoryRoot,
    fixture: Option<&Path>,
    runner: &ProcessRunner,
) -> Result<Vec<IssueInput>> {
    let values = match fixture {
        Some(path) => {
            let value = read_json_path(root, path)?;
            value["issues"]
                .as_array()
                .cloned()
                .or_else(|| value.as_array().cloned())
                .ok_or_else(|| "issue fixture must contain an issues array".to_owned())?
        }
        None => super::super::super::github::issue_spec_values(runner)?,
    };
    let mut issues = values.iter().filter_map(parse_issue).collect::<Vec<_>>();
    issues.sort_by_key(|issue| issue.number);
    issues.dedup_by_key(|issue| issue.number);
    Ok(issues)
}

fn parse_issue(value: &Value) -> Option<IssueInput> {
    Some(IssueInput {
        number: value["number"].as_u64()?,
        title: value["title"].as_str()?.to_owned(),
        state: value["state"].as_str().unwrap_or("open").to_owned(),
        labels: value["labels"]
            .as_array()
            .map(|labels| {
                labels
                    .iter()
                    .filter_map(|label| label.as_str().or_else(|| label["name"].as_str()))
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        milestone: value["milestone"]["title"]
            .as_str()
            .or_else(|| value["milestone"].as_str())
            .map(ToOwned::to_owned),
        body: value["body"].as_str().unwrap_or_default().to_owned(),
        source_anchor_hash: extract_sections(value["body"].as_str().unwrap_or_default())
            .get(SectionRole::SourceAnchors)
            .map_or_else(String::new, |section| section.content_hash.clone()),
        is_pull_request: value.get("pull_request").is_some(),
    })
}

fn issue_snapshot(issues: &[IssueInput]) -> Value {
    serde_json::json!({ "schema": "rusttable.issue-spec-snapshot.v1", "repository": "cgasgarth/RustTable", "parent_issue": 158, "issues": issues.iter().map(|issue| serde_json::json!({ "number": issue.number, "title": issue.title, "body": issue.body, "body_hash": body_hash(&issue.body), "source_anchor_hash": issue.source_anchor_hash, "labels": issue.labels, "milestone": issue.milestone, "state": issue.state, "is_pull_request": issue.is_pull_request })).collect::<Vec<_>>() })
}
