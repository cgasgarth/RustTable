use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cli::{GithubCommand, IssueSpecArgs};
use crate::commands::{Result, report};
use crate::process::ProcessRunner;
use crate::root::RepositoryRoot;

use super::super::github;
use super::extract_sections;
use super::types::{
    IssueAudit, IssueSnapshot, IssueSpecPolicy, ReferenceIndex, SCHEMA, SectionRole,
    SpecificationStatus,
};
use super::{canonical_body_hash, parse_dependencies};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct IssueSpecSnapshotFile {
    schema: String,
    repository: String,
    parent_issue: u64,
    issues: Vec<IssueSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct SourceMapFile {
    parent_issue: u64,
    source_commit: String,
    issues: Vec<SourceMapIssue>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct SourceMapIssue {
    number: u64,
    body_hash: String,
    source_anchor_hash: String,
    source_status: String,
}

const PINNED_DARKTABLE_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct IssueSpecPolicyFile {
    #[serde(default = "default_policy_schema")]
    schema: String,
    #[serde(default = "default_repository")]
    repository: String,
    #[serde(default = "default_parent_issue")]
    parent_issue: u64,
    #[serde(default)]
    superseded: BTreeMap<String, u64>,
    #[serde(default)]
    capability_owner_issues: Vec<u64>,
}

fn default_policy_schema() -> String {
    "rusttable.issue-spec-policy.v1".to_owned()
}

fn default_repository() -> String {
    "cgasgarth/RustTable".to_owned()
}

const fn default_parent_issue() -> u64 {
    158
}

pub(crate) fn run(
    root: &RepositoryRoot,
    command: &GithubCommand,
    arguments: &IssueSpecArgs,
    runner: &ProcessRunner,
) -> Result {
    match command {
        GithubCommand::RefreshIssueSpecSnapshot(_) => refresh_snapshot(root, arguments, runner),
        GithubCommand::VerifyIssueSpecs(_) => verify_issue_specs(root, arguments),
        GithubCommand::ReadyIssues(_) => ready_issues(root, arguments),
        GithubCommand::ApplyIssueSpecPlan(_) => apply_plan(root, arguments, runner),
        _ => Err("issue-spec command dispatch received an unrelated GitHub command".to_owned()),
    }
}

fn refresh_snapshot(
    root: &RepositoryRoot,
    arguments: &IssueSpecArgs,
    runner: &ProcessRunner,
) -> Result {
    let values = match &arguments.api_fixture {
        Some(path) => read_issue_values_fixture(&root.join(path))?,
        None => github::issue_spec_values(runner)?,
    };
    let mut issues = values
        .iter()
        .map(issue_from_value)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    issues.sort_by_key(|issue| issue.number);
    issues.dedup_by_key(|issue| issue.number);
    let snapshot = IssueSpecSnapshotFile {
        schema: "rusttable.issue-spec-snapshot.v1".to_owned(),
        repository: default_repository(),
        parent_issue: default_parent_issue(),
        issues,
    };
    let path = root.join(&arguments.snapshot);
    write_json(&path, &snapshot)?;
    Ok(report(
        root,
        "github.refresh-issue-spec-snapshot",
        serde_json::json!({
            "schema": snapshot.schema,
            "repository": snapshot.repository,
            "parent_issue": snapshot.parent_issue,
            "issue_count": snapshot.issues.len(),
            "path": arguments.snapshot,
            "source": if arguments.api_fixture.is_some() { "fixture" } else { "github" },
        }),
    ))
}

fn verify_issue_specs(root: &RepositoryRoot, arguments: &IssueSpecArgs) -> Result {
    let snapshot = read_snapshot(&root.join(&arguments.snapshot))?;
    for issue in &snapshot.issues {
        let actual_body = canonical_body_hash(&issue.body);
        if actual_body != issue.body_hash {
            return Err(format!(
                "issue-spec snapshot body drift for #{}: expected {}, found {}",
                issue.number, issue.body_hash, actual_body
            ));
        }
        let actual = extract_sections(&issue.body)
            .get(SectionRole::SourceAnchors)
            .map_or_else(String::new, |section| section.content_hash.clone());
        if actual != issue.source_anchor_hash {
            return Err(format!(
                "issue-spec snapshot source block drift for #{}: expected {}, found {}",
                issue.number, issue.source_anchor_hash, actual
            ));
        }
    }
    let policy = read_policy(&root.join(&arguments.policy), &snapshot)?;
    let audits = policy.audit_all(&open_child_issues(&snapshot));
    let ready = audits
        .iter()
        .filter(|audit| audit.status.is_ready())
        .count();
    let blocked = audits.len().saturating_sub(ready);
    let p0_p1_blocked = audits
        .iter()
        .filter(|audit| {
            snapshot
                .issues
                .iter()
                .find(|issue| issue.number == audit.issue)
                .and_then(|issue| priority_label(issue).ok())
                .is_some_and(|priority| matches!(priority, "priority: P0" | "priority: P1"))
                && !audit.status.is_ready()
        })
        .count();
    Ok(report(
        root,
        "github.verify-issue-specs",
        serde_json::json!({
            "schema": SCHEMA,
            "snapshot": arguments.snapshot,
            "issue_count": audits.len(),
            "ready_count": ready,
            "blocked_count": blocked,
            "p0_p1_blocked_count": p0_p1_blocked,
            "audits": audits,
        }),
    ))
}

fn ready_issues(root: &RepositoryRoot, arguments: &IssueSpecArgs) -> Result {
    let snapshot = read_snapshot(&root.join(&arguments.snapshot))?;
    let policy = read_policy(&root.join(&arguments.policy), &snapshot)?;
    let source_map = read_source_map(root, snapshot.parent_issue)?;
    let issues = open_child_issues(&snapshot);
    let audits = policy.audit_all(&issues);
    let audit_by_number = audits
        .into_iter()
        .map(|audit| (audit.issue, audit))
        .collect::<BTreeMap<_, _>>();
    let by_number = snapshot
        .issues
        .iter()
        .map(|issue| (issue.number, issue))
        .collect::<BTreeMap<_, _>>();
    let mut entries = issues
        .iter()
        .map(|issue| {
            ready_entry(
                issue,
                &audit_by_number[&issue.number],
                &source_map,
                &by_number,
            )
        })
        .collect::<Vec<_>>();
    let selected = entries
        .iter()
        .filter(|entry| entry.ready)
        .min_by(|left, right| {
            priority_rank(&left.priority)
                .cmp(&priority_rank(&right.priority))
                .then_with(|| left.issue.cmp(&right.issue))
        })
        .map(|entry| entry.issue);
    if let Some(selected_issue) = selected {
        let selected_priority = entries
            .iter()
            .find(|entry| entry.issue == selected_issue)
            .map(|entry| entry.priority.clone())
            .unwrap_or_default();
        for entry in &mut entries {
            if entry.ready && entry.issue != selected_issue {
                entry.blockers.push(format!(
                    "lower priority than selected {selected_priority} issue #{selected_issue}"
                ));
                entry.ready = false;
            }
        }
    }
    entries.sort_by(|left, right| {
        priority_rank(&left.priority)
            .cmp(&priority_rank(&right.priority))
            .then_with(|| left.issue.cmp(&right.issue))
    });
    Ok(report(
        root,
        "github.ready-issues",
        serde_json::json!({
            "schema": "rusttable.issue-ready-queue.v1",
            "repository": default_repository(),
            "parent_issue": default_parent_issue(),
            "selected_issue": selected,
            "issues": entries,
        }),
    ))
}

fn ready_entry(
    issue: &IssueSnapshot,
    audit: &IssueAudit,
    source_map: &BTreeMap<u64, SourceMapIssue>,
    by_number: &BTreeMap<u64, &IssueSnapshot>,
) -> IssueReadyEntry {
    let mut blockers = Vec::new();
    if !audit.status.is_ready() {
        blockers.push(format!("spec: {}", audit.status));
    }
    let source_status = source_status(issue.number, audit, source_map, &mut blockers);
    dependency_blockers(audit, by_number, &mut blockers);
    IssueReadyEntry {
        issue: issue.number,
        title: issue.title.clone(),
        priority: priority_label(issue).unwrap_or("invalid").to_owned(),
        status: audit.status,
        source_status,
        dependencies: audit
            .dependencies
            .references
            .iter()
            .map(|dependency| dependency.issue)
            .collect(),
        ready: blockers.is_empty(),
        blockers,
    }
}

fn source_status(
    issue: u64,
    audit: &IssueAudit,
    source_map: &BTreeMap<u64, SourceMapIssue>,
    blockers: &mut Vec<String>,
) -> String {
    match source_map.get(&issue) {
        Some(source) if source.body_hash != audit.body_hash => {
            blockers.push("source map body hash is stale".to_owned());
            "stale-body-hash".to_owned()
        }
        Some(source) if source.source_anchor_hash != audit.source_anchor_hash => {
            blockers.push("source map anchor hash is stale".to_owned());
            "stale-anchor-hash".to_owned()
        }
        Some(source) if source.source_status != "anchored" => {
            blockers.push(format!("source: {}", source.source_status));
            source.source_status.clone()
        }
        Some(source) => source.source_status.clone(),
        None => {
            blockers.push("source map record is missing".to_owned());
            "missing".to_owned()
        }
    }
}

fn dependency_blockers(
    audit: &IssueAudit,
    by_number: &BTreeMap<u64, &IssueSnapshot>,
    blockers: &mut Vec<String>,
) {
    for dependency in &audit.dependencies.references {
        if let Some(target) = by_number.get(&dependency.issue) {
            if target.state == "open" {
                blockers.push(format!("depends on open #{}", dependency.issue));
            } else if matches!(
                target.state_reason.as_deref(),
                Some("not planned" | "duplicate")
            ) {
                blockers.push(format!(
                    "depends on #{} closed as {}",
                    dependency.issue,
                    target.state_reason.as_deref().unwrap_or("unresolved")
                ));
            }
        } else {
            blockers.push(format!("unknown dependency #{}", dependency.issue));
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct IssueReadyEntry {
    issue: u64,
    title: String,
    priority: String,
    status: SpecificationStatus,
    source_status: String,
    dependencies: Vec<u64>,
    ready: bool,
    blockers: Vec<String>,
}

fn read_source_map(
    root: &RepositoryRoot,
    expected_parent: u64,
) -> Result<BTreeMap<u64, SourceMapIssue>> {
    let path = root.join("architecture/darktable-source-map.toml");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("source map {}: {error}", path.display()))?;
    let map: SourceMapFile =
        toml::from_str(&text).map_err(|error| format!("source map {}: {error}", path.display()))?;
    if map.parent_issue != expected_parent || map.source_commit != PINNED_DARKTABLE_COMMIT {
        return Err("source map has the wrong parent or canonical source commit".to_owned());
    }
    let mut issues = BTreeMap::new();
    for issue in map.issues {
        if issues.insert(issue.number, issue).is_some() {
            return Err("source map has duplicate issue records".to_owned());
        }
    }
    Ok(issues)
}

#[expect(
    clippy::too_many_lines,
    reason = "apply is a fail-closed, ordered optimistic-concurrency transaction"
)]
fn apply_plan(root: &RepositoryRoot, arguments: &IssueSpecArgs, runner: &ProcessRunner) -> Result {
    let path = arguments
        .reviewed_plan
        .as_ref()
        .ok_or_else(|| "apply-issue-spec-plan: --reviewed-plan is required".to_owned())?;
    let plan: Value = serde_json::from_str(
        &fs::read_to_string(root.join(path))
            .map_err(|error| format!("reviewed plan {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("reviewed plan {}: invalid JSON: {error}", path.display()))?;
    let operations = plan
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| "reviewed plan: operations array is required".to_owned())?;
    if operations.iter().any(|operation| {
        operation.get("issue").and_then(Value::as_u64).is_none()
            || operation
                .get("expected_body_hash")
                .and_then(Value::as_str)
                .is_none()
            || operation
                .get("replacement_body")
                .and_then(Value::as_str)
                .is_none()
    }) {
        return Err(
            "reviewed plan: every operation requires issue, expected_body_hash, and replacement_body"
                .to_owned(),
        );
    }
    let snapshot = read_snapshot(&root.join(&arguments.snapshot))?;
    let policy = read_policy(&root.join(&arguments.policy), &snapshot)?;
    let mut proposed = snapshot.issues.clone();
    let mut updates = Vec::new();
    let mut seen = BTreeSet::new();
    for operation in operations {
        let issue_number = operation
            .get("issue")
            .and_then(Value::as_u64)
            .ok_or_else(|| "reviewed plan: issue must be an integer".to_owned())?;
        if !seen.insert(issue_number) {
            return Err(format!(
                "reviewed plan: issue #{issue_number} appears more than once"
            ));
        }
        let expected_hash = operation
            .get("expected_body_hash")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("reviewed plan: issue #{issue_number} expected_body_hash is missing")
            })?;
        let replacement = operation
            .get("replacement_body")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("reviewed plan: issue #{issue_number} replacement_body is missing")
            })?;
        let issue = proposed
            .iter_mut()
            .find(|issue| issue.number == issue_number)
            .ok_or_else(|| {
                format!("reviewed plan: issue #{issue_number} is absent from snapshot")
            })?;
        let actual_hash = canonical_body_hash(&issue.body);
        if actual_hash != expected_hash {
            return Err(format!(
                "reviewed plan: issue #{issue_number} body drifted (expected {expected_hash}, found {actual_hash})"
            ));
        }
        if canonical_body_hash(replacement) != actual_hash {
            replacement.clone_into(&mut issue.body);
            updates.push((issue_number, replacement.to_owned()));
        }
    }
    let audits = policy.audit_all(&open_child_issues(&IssueSpecSnapshotFile {
        issues: proposed,
        ..snapshot.clone()
    }));
    let invalid = audits
        .iter()
        .filter(|audit| seen.contains(&audit.issue) && !audit.findings.is_empty())
        .map(|audit| audit.issue)
        .collect::<Vec<_>>();
    if !invalid.is_empty() {
        return Err(format!(
            "reviewed plan: replacement bodies fail semantic validation for {invalid:?}"
        ));
    }
    if arguments.api_fixture.is_none() && !updates.is_empty() {
        github::apply_issue_spec_updates(runner, &updates)?;
    }
    Ok(report(
        root,
        "github.apply-issue-spec-plan",
        serde_json::json!({
            "schema": "rusttable.issue-spec-apply-plan.v1",
            "operations": operations.len(),
            "applied": if arguments.api_fixture.is_some() { 0 } else { updates.len() },
            "status": if arguments.api_fixture.is_some() { "validated-fixture" } else { "applied" },
            "issue_numbers": updates.iter().map(|(issue, _)| issue).collect::<Vec<_>>(),
        }),
    ))
}

fn open_child_issues(snapshot: &IssueSpecSnapshotFile) -> Vec<IssueSnapshot> {
    snapshot
        .issues
        .iter()
        .filter(|issue| {
            issue.state == "open"
                && !issue.is_pull_request
                && issue
                    .body
                    .lines()
                    .any(|line| line.trim() == format!("Parent: #{}", snapshot.parent_issue))
        })
        .cloned()
        .collect()
}

fn read_snapshot(path: &Path) -> std::result::Result<IssueSpecSnapshotFile, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("issue-spec snapshot {}: {error}", path.display()))?;
    let snapshot: IssueSpecSnapshotFile = serde_json::from_str(&source).map_err(|error| {
        format!(
            "issue-spec snapshot {}: invalid JSON: {error}",
            path.display()
        )
    })?;
    if snapshot.schema != "rusttable.issue-spec-snapshot.v1" {
        return Err(format!(
            "issue-spec snapshot {}: unsupported schema {}",
            path.display(),
            snapshot.schema
        ));
    }
    if snapshot.repository != default_repository()
        || snapshot.parent_issue != default_parent_issue()
    {
        return Err(format!(
            "issue-spec snapshot {}: repository or parent does not match {} / #{}",
            path.display(),
            default_repository(),
            default_parent_issue()
        ));
    }
    Ok(snapshot)
}

fn read_policy(
    path: &Path,
    snapshot: &IssueSpecSnapshotFile,
) -> std::result::Result<IssueSpecPolicy, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("issue-spec policy {}: {error}", path.display()))?;
    let document: IssueSpecPolicyFile = toml::from_str(&source).map_err(|error| {
        format!(
            "issue-spec policy {}: invalid TOML: {error}",
            path.display()
        )
    })?;
    if document.schema != "rusttable.issue-spec-policy.v1"
        || document.repository != snapshot.repository
        || document.parent_issue != snapshot.parent_issue
    {
        return Err(format!(
            "issue-spec policy {}: schema, repository, or parent does not match snapshot",
            path.display()
        ));
    }
    let references = ReferenceIndex {
        known_issues: snapshot.issues.iter().map(|issue| issue.number).collect(),
        superseded: document
            .superseded
            .into_iter()
            .filter_map(|(issue, replacement)| issue.parse().ok().map(|issue| (issue, replacement)))
            .collect(),
        dependency_graph: snapshot
            .issues
            .iter()
            .map(|issue| {
                (
                    issue.number,
                    extract_sections(&issue.body)
                        .get(SectionRole::Dependencies)
                        .map(|section| {
                            parse_dependencies(&section.content)
                                .references
                                .into_iter()
                                .map(|reference| reference.issue)
                                .collect()
                        })
                        .unwrap_or_default(),
                )
            })
            .collect(),
        capability_owners: document.capability_owner_issues.into_iter().collect(),
    };
    Ok(IssueSpecPolicy::new(references))
}

fn read_issue_values_fixture(path: &Path) -> std::result::Result<Vec<Value>, String> {
    let source = fs::read_to_string(path)
        .map_err(|error| format!("issue-spec API fixture {}: {error}", path.display()))?;
    let value: Value = serde_json::from_str(&source).map_err(|error| {
        format!(
            "issue-spec API fixture {}: invalid JSON: {error}",
            path.display()
        )
    })?;
    value
        .get("issues")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| value.as_array().cloned())
        .ok_or_else(|| "issue-spec API fixture: issues array is required".to_owned())
}

fn issue_from_value(value: &Value) -> std::result::Result<IssueSnapshot, String> {
    let number = value
        .get("number")
        .and_then(Value::as_u64)
        .ok_or_else(|| "issue-spec issue: number is required".to_owned())?;
    let title = value
        .get("title")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("issue-spec issue #{number}: title is required"))?;
    let body = value
        .get("body")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let labels = value
        .get("labels")
        .and_then(Value::as_array)
        .map(|labels| {
            labels
                .iter()
                .filter_map(|label| label.as_str().or_else(|| label.get("name")?.as_str()))
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let milestone = value.get("milestone").and_then(|milestone| {
        milestone
            .get("title")
            .and_then(Value::as_str)
            .or_else(|| milestone.as_str())
            .map(ToOwned::to_owned)
    });
    Ok(IssueSnapshot {
        number,
        title: title.to_owned(),
        body: body.to_owned(),
        labels,
        milestone,
        state: value
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("open")
            .to_owned(),
        state_reason: value
            .get("state_reason")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        is_pull_request: value.get("pull_request").is_some(),
        source_anchor_hash: extract_sections(body)
            .get(SectionRole::SourceAnchors)
            .map_or_else(String::new, |section| section.content_hash.clone()),
        body_hash: canonical_body_hash(body),
    })
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::result::Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize {}: {error}", path.display()))?;
    text.push('\n');
    fs::write(path, text).map_err(|error| format!("write {}: {error}", path.display()))
}

fn priority_label(issue: &IssueSnapshot) -> std::result::Result<&str, String> {
    let labels = issue
        .labels
        .iter()
        .filter(|label| label.starts_with("priority:"))
        .collect::<Vec<_>>();
    if labels.len() != 1
        || !matches!(
            labels[0].as_str(),
            "priority: P0" | "priority: P1" | "priority: P2" | "priority: P3" | "priority: P4"
        )
    {
        return Err(format!("issue #{}: invalid priority label", issue.number));
    }
    Ok(labels[0].as_str())
}

fn priority_rank(priority: &str) -> usize {
    [
        "priority: P0",
        "priority: P1",
        "priority: P2",
        "priority: P3",
        "priority: P4",
    ]
    .iter()
    .position(|candidate| *candidate == priority)
    .unwrap_or(usize::MAX)
}
