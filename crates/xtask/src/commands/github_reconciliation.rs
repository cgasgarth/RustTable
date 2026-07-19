use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use super::github::{
    FixtureApi, GitHubApi, ISSUE_SPEC_PARENT, ReadApi, TARGET_REPOSITORY, WriteApi, is_child_issue,
    issue_numbers_in_text,
};
use super::report;
use crate::cli::IssueReconciliationArgs;
use crate::process::ProcessRunner;
use crate::root::RepositoryRoot;

type Result<T = crate::output::Report, E = String> = std::result::Result<T, E>;

#[derive(serde::Deserialize)]
struct SpecificationFile {
    #[serde(rename = "specification")]
    specifications: Vec<rusttable_parity::IssueSpecification>,
}

#[derive(Debug, Default, serde::Deserialize, serde::Serialize)]
struct ApplyReceipt {
    schema_version: u32,
    plan_sha256: String,
    applied_actions: BTreeSet<String>,
}

fn receipt_path(plan: &Path) -> std::path::PathBuf {
    plan.with_extension("receipt.json")
}

fn load_receipt(path: &Path, plan_sha256: &str) -> Result<ApplyReceipt, String> {
    if !path.exists() {
        return Ok(ApplyReceipt {
            schema_version: 1,
            plan_sha256: plan_sha256.to_owned(),
            applied_actions: BTreeSet::new(),
        });
    }
    let receipt: ApplyReceipt = serde_json::from_str(
        &fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))?;
    if receipt.schema_version != 1 || receipt.plan_sha256 != plan_sha256 {
        return Err("issue reconciliation receipt does not match the plan".to_owned());
    }
    Ok(receipt)
}

fn save_receipt(path: &Path, receipt: &ApplyReceipt) -> Result<(), String> {
    let rendered = serde_json::to_vec_pretty(receipt)
        .map_err(|error| format!("serialize {}: {error}", path.display()))?;
    fs::write(path, rendered).map_err(|error| format!("write {}: {error}", path.display()))
}

fn action_key(kind: &str, issue: u64) -> String {
    format!("{kind}:{issue}")
}

fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut digest, "{byte:02x}").expect("writing to a String cannot fail");
    }
    digest
}

pub(crate) fn plan_issue_reconciliation(
    root: &RepositoryRoot,
    arguments: &IssueReconciliationArgs,
    runner: &ProcessRunner,
) -> Result {
    let api: Box<dyn ReadApi> = match &arguments.api_fixture {
        Some(path) => Box::new(FixtureApi::read(&root.join(path))?),
        None => Box::new(GitHubApi::from_environment(runner)?),
    };
    let inputs = reconciliation_inputs(api.as_ref())?;
    let candidates = reconciliation_candidates(root)?;
    let specifications = reconciliation_specifications(root, arguments)?;
    let plan = rusttable_parity::build_reconciliation_plan(
        TARGET_REPOSITORY,
        ISSUE_SPEC_PARENT,
        &candidates,
        &inputs,
        &specifications,
    )?;
    let path = root.join(&arguments.plan);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let rendered = serde_json::to_vec_pretty(&plan)
        .map_err(|error| format!("serialize issue reconciliation plan: {error}"))?;
    fs::write(&path, rendered).map_err(|error| format!("write {}: {error}", path.display()))?;
    Ok(report(
        root,
        "parity.plan-issue-reconciliation",
        serde_json::json!({
            "plan": path,
            "plan_sha256": plan.plan_sha256,
            "issues": plan.audits.len(),
            "creations": plan.creations.len(),
            "updates": plan.updates.len(),
            "closures": plan.closures.len(),
            "label_changes": plan.label_changes.len(),
            "milestone_changes": plan.milestone_changes.len(),
            "blocked_ambiguities": plan.blocked_ambiguities,
        }),
    ))
}

#[allow(clippy::too_many_lines)]
pub(crate) fn apply_issue_reconciliation(
    root: &RepositoryRoot,
    arguments: &IssueReconciliationArgs,
    runner: &ProcessRunner,
) -> Result {
    if !arguments.confirm {
        return Err("issue reconciliation apply requires --confirm".to_owned());
    }
    if arguments.api_fixture.is_some() {
        return Err("issue reconciliation apply cannot use an API fixture".to_owned());
    }
    let path = root.join(&arguments.plan);
    let plan: rusttable_parity::ReconciliationPlan = serde_json::from_str(
        &fs::read_to_string(&path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))?;
    plan.verify_hash()?;
    if plan.repository != TARGET_REPOSITORY || plan.parent_issue != ISSUE_SPEC_PARENT {
        return Err("issue reconciliation plan targets the wrong repository or parent".to_owned());
    }
    if !plan.blocked_ambiguities.is_empty() {
        return Err(format!(
            "issue reconciliation plan is blocked by {} ambiguity findings",
            plan.blocked_ambiguities.len()
        ));
    }
    let receipt_file = receipt_path(&path);
    let mut receipt = load_receipt(&receipt_file, &plan.plan_sha256)?;
    let resumed = !receipt.applied_actions.is_empty();
    let api = GitHubApi::from_environment(runner)?;
    let inputs = reconciliation_inputs(&api)?;
    let candidates = reconciliation_candidates(root)?;
    let specifications = reconciliation_specifications(root, arguments)?;
    let current = rusttable_parity::build_reconciliation_plan(
        TARGET_REPOSITORY,
        ISSUE_SPEC_PARENT,
        &candidates,
        &inputs,
        &specifications,
    )?;
    if !resumed
        && (current.source_hash != plan.source_hash || current.audits.len() != plan.audits.len())
    {
        return Err("issue reconciliation plan is stale: live issue snapshot changed".to_owned());
    }
    if !resumed {
        plan.validate_for_apply(&inputs)?;
    }
    let mut applied = 0usize;

    for action in &plan.label_changes {
        let key = action_key("label", action.issue);
        if receipt.applied_actions.contains(&key) {
            continue;
        }
        let current = current_input(&inputs, action.issue)?;
        assert_unchanged(
            current,
            action.issue,
            &action.expected_body_sha256,
            &action.expected_etag,
        )?;
        api.update_issue(
            TARGET_REPOSITORY,
            action.issue,
            serde_json::json!({ "labels": action.labels }),
        )?;
        receipt.applied_actions.insert(key);
        save_receipt(&receipt_file, &receipt)?;
        applied += 1;
    }
    for action in &plan.milestone_changes {
        let key = action_key("milestone", action.issue);
        if receipt.applied_actions.contains(&key) {
            continue;
        }
        let current = current_input(&inputs, action.issue)?;
        assert_unchanged(
            current,
            action.issue,
            &action.expected_body_sha256,
            &action.expected_etag,
        )?;
        api.update_issue(
            TARGET_REPOSITORY,
            action.issue,
            serde_json::json!({ "milestone": action.milestone_number }),
        )?;
        receipt.applied_actions.insert(key);
        save_receipt(&receipt_file, &receipt)?;
        applied += 1;
    }
    for action in &plan.closures {
        let key = action_key("closure", action.issue);
        if receipt.applied_actions.contains(&key) {
            continue;
        }
        let current = current_input(&inputs, action.issue)?;
        assert_unchanged(
            current,
            action.issue,
            &action.expected_body_sha256,
            &action.expected_etag,
        )?;
        api.update_issue(
            TARGET_REPOSITORY,
            action.issue,
            serde_json::json!({
                "state": "closed",
                "state_reason": "duplicate",
                "body": format!("{}\n\n## Replacement\n\nSuperseded by #{}.", current.body.trim_end(), action.replacement_issue),
            }),
        )?;
        receipt.applied_actions.insert(key);
        save_receipt(&receipt_file, &receipt)?;
        applied += 1;
    }
    for action in &plan.updates {
        let key = action_key("update", action.issue);
        if receipt.applied_actions.contains(&key) {
            continue;
        }
        let current = current_input(&inputs, action.issue)?;
        assert_unchanged(
            current,
            action.issue,
            &action.expected_body_sha256,
            &action.expected_etag,
        )?;
        api.update_issue(
            TARGET_REPOSITORY,
            action.issue,
            serde_json::json!({ "body": action.replacement_body }),
        )?;
        receipt.applied_actions.insert(key);
        save_receipt(&receipt_file, &receipt)?;
        applied += 1;
    }
    for action in &plan.creations {
        let key = format!("create:{}", action.capability_id);
        if receipt.applied_actions.contains(&key) {
            continue;
        }
        if !current
            .creations
            .iter()
            .any(|creation| creation.capability_id == action.capability_id)
        {
            receipt.applied_actions.insert(key);
            save_receipt(&receipt_file, &receipt)?;
            continue;
        }
        api.create_issue(
            TARGET_REPOSITORY,
            serde_json::json!({
                "title": action.title,
                "body": action.body,
                "labels": [format!("priority: {}", action.priority)],
                "milestone": action.milestone_number,
            }),
        )?;
        receipt.applied_actions.insert(key);
        save_receipt(&receipt_file, &receipt)?;
        applied += 1;
    }
    Ok(report(
        root,
        "parity.apply-issue-reconciliation",
        serde_json::json!({
            "plan": path,
            "receipt": receipt_file,
            "plan_sha256": plan.plan_sha256,
            "applied": applied,
            "total_applied": receipt.applied_actions.len(),
        }),
    ))
}
fn current_input(
    inputs: &[rusttable_parity::IssueInput],
    issue: u64,
) -> Result<&rusttable_parity::IssueInput, String> {
    inputs
        .iter()
        .find(|input| input.number == issue)
        .ok_or_else(|| format!("issue reconciliation action references missing issue #{issue}"))
}

fn assert_unchanged(
    current: &rusttable_parity::IssueInput,
    issue: u64,
    expected_body_sha256: &str,
    expected_etag: &str,
) -> Result<(), String> {
    if digest(current.body.as_bytes()) != expected_body_sha256 || current.etag != expected_etag {
        return Err(format!("issue #{issue} changed since plan review"));
    }
    Ok(())
}

fn reconciliation_inputs(api: &dyn ReadApi) -> Result<Vec<rusttable_parity::IssueInput>, String> {
    let mut issues = api
        .issues(TARGET_REPOSITORY)?
        .into_iter()
        .filter(|issue| !issue.is_pull_request && is_child_issue(issue))
        .map(|issue| {
            let etag = digest(format!("{}\n{}", issue.state, issue.body).as_bytes());
            let replacement_issue = issue_numbers_in_text(&issue.body)
                .into_iter()
                .find(|_| issue.body.to_ascii_lowercase().contains("superseded"));
            rusttable_parity::IssueInput {
                number: issue.number,
                title: issue.title,
                state: issue.state,
                state_reason: issue.state_reason,
                body: issue.body,
                milestone: issue.milestone,
                priority_labels: issue.labels,
                repository: issue.repository,
                etag,
                replacement_issue,
            }
        })
        .collect::<Vec<_>>();
    issues.sort_by_key(|issue| issue.number);
    Ok(issues)
}

fn reconciliation_candidates(
    root: &RepositoryRoot,
) -> Result<Vec<rusttable_parity::CapabilityCandidate>, String> {
    let path = root.join("architecture/darktable-capabilities.toml");
    let manifest = rusttable_parity::parse_manifest(
        &fs::read_to_string(&path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))?;
    Ok(manifest
        .capabilities
        .into_iter()
        .map(|capability| rusttable_parity::CapabilityCandidate {
            id: capability.id,
            category: capability.category,
            reference_path: capability.reference_path,
            reference_symbol: capability.reference_symbol,
            structural_evidence: capability.structural_evidence,
            behavioral_evidence: capability.behavioral_evidence,
            acceptance_test_id: capability.acceptance_test_id,
        })
        .collect())
}

fn reconciliation_specifications(
    root: &RepositoryRoot,
    arguments: &IssueReconciliationArgs,
) -> Result<Vec<rusttable_parity::IssueSpecification>, String> {
    let Some(relative) = &arguments.specifications else {
        return Ok(Vec::new());
    };
    let path = root.join(relative);
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("read issue specifications {}: {error}", path.display()))?;
    if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
        return serde_json::from_str(&source)
            .map_err(|error| format!("parse issue specifications {}: {error}", path.display()));
    }
    toml::from_str::<SpecificationFile>(&source)
        .map(|file| file.specifications)
        .map_err(|error| format!("parse issue specifications {}: {error}", path.display()))
}
