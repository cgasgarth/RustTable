use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const REQUIRED_SECTIONS: [&str; 8] = [
    "Outcome",
    "Fixed decisions",
    "Implementation",
    "Failure and edge behavior",
    "Test matrix",
    "Acceptance evidence",
    "Dependencies",
    "One-PR boundary",
];
const PRIORITIES: [&str; 5] = ["P0", "P1", "P2", "P3", "P4"];
const DEFERRAL_MARKERS: [&str; 6] = [
    "tbd",
    "decide later",
    "choose later",
    "as appropriate",
    "if practical",
    "investigate and decide",
];

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CapabilityCandidate {
    pub id: String,
    pub category: String,
    pub reference_path: String,
    pub reference_symbol: String,
    pub structural_evidence: Vec<String>,
    pub behavioral_evidence: Vec<String>,
    pub acceptance_test_id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CapabilityDeclaration {
    pub capability_id: String,
    pub role: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct IssueInput {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub state_reason: Option<String>,
    pub body: String,
    #[serde(default)]
    pub milestone: Option<u64>,
    pub priority_labels: Vec<String>,
    pub repository: String,
    #[serde(default)]
    pub etag: String,
    #[serde(default)]
    pub replacement_issue: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct IssueSpecification {
    pub capability_id: String,
    pub role: String,
    pub category: String,
    pub title: String,
    pub body: String,
    pub priority: String,
    #[serde(default)]
    pub milestone: Option<u64>,
    #[serde(default)]
    pub compatibility_identity: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct IssueAudit {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub state_reason: Option<String>,
    pub body_sha256: String,
    pub etag: String,
    pub milestone: Option<u64>,
    pub priority_labels: Vec<String>,
    pub status: String,
    pub findings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlannedCreation {
    pub capability_id: String,
    pub role: String,
    pub title: String,
    pub body: String,
    pub priority: String,
    pub milestone: String,
    pub milestone_number: u64,
    pub dependencies: Vec<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlannedUpdate {
    pub issue: u64,
    pub expected_etag: String,
    pub expected_body_sha256: String,
    pub replacement_body: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlannedClosure {
    pub issue: u64,
    pub expected_etag: String,
    pub expected_body_sha256: String,
    pub replacement_issue: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlannedLabelChange {
    pub issue: u64,
    pub expected_etag: String,
    pub expected_body_sha256: String,
    pub labels: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PlannedMilestoneChange {
    pub issue: u64,
    pub expected_etag: String,
    pub expected_body_sha256: String,
    pub milestone: String,
    pub milestone_number: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReconciliationPlan {
    pub schema_version: u32,
    pub repository: String,
    pub parent_issue: u64,
    pub source_hash: String,
    pub plan_sha256: String,
    pub audits: Vec<IssueAudit>,
    pub creations: Vec<PlannedCreation>,
    pub updates: Vec<PlannedUpdate>,
    pub closures: Vec<PlannedClosure>,
    pub label_changes: Vec<PlannedLabelChange>,
    pub milestone_changes: Vec<PlannedMilestoneChange>,
    pub blocked_ambiguities: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HashView<'a> {
    schema_version: u32,
    repository: &'a str,
    parent_issue: u64,
    source_hash: &'a str,
    audits: &'a [IssueAudit],
    creations: &'a [PlannedCreation],
    updates: &'a [PlannedUpdate],
    closures: &'a [PlannedClosure],
    label_changes: &'a [PlannedLabelChange],
    milestone_changes: &'a [PlannedMilestoneChange],
    blocked_ambiguities: &'a [String],
}

impl ReconciliationPlan {
    #[must_use]
    /// Returns the stable byte representation used for plan hashing.
    ///
    /// # Panics
    ///
    /// Panics only if the in-memory plan cannot be serialized by `serde_json`.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(&HashView {
            schema_version: self.schema_version,
            repository: &self.repository,
            parent_issue: self.parent_issue,
            source_hash: &self.source_hash,
            audits: &self.audits,
            creations: &self.creations,
            updates: &self.updates,
            closures: &self.closures,
            label_changes: &self.label_changes,
            milestone_changes: &self.milestone_changes,
            blocked_ambiguities: &self.blocked_ambiguities,
        })
        .expect("reconciliation plan serialization cannot fail")
    }

    /// # Errors
    ///
    /// Returns an error when the stored hash does not match the canonical plan.
    pub fn verify_hash(&self) -> Result<(), String> {
        let actual = digest_bytes(&self.canonical_bytes());
        if actual == self.plan_sha256 {
            Ok(())
        } else {
            Err(format!(
                "issue reconciliation plan hash mismatch: expected {}, got {actual}",
                self.plan_sha256
            ))
        }
    }

    /// # Errors
    ///
    /// Returns an error when the plan hash or any audited issue state differs
    /// from the reviewed live snapshot.
    pub fn validate_for_apply(&self, live: &[IssueInput]) -> Result<(), String> {
        self.verify_hash()?;
        let live_by_number = live
            .iter()
            .map(|issue| (issue.number, issue))
            .collect::<BTreeMap<_, _>>();
        for audit in &self.audits {
            let current = live_by_number
                .get(&audit.number)
                .ok_or_else(|| format!("issue #{} disappeared since plan review", audit.number))?;
            if current.state != audit.state
                || current.state_reason != audit.state_reason
                || current.milestone != audit.milestone
                || current.priority_labels != audit.priority_labels
                || digest_bytes(current.body.as_bytes()) != audit.body_sha256
                || current.etag != audit.etag
            {
                return Err(format!("issue #{} changed since plan review", audit.number));
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn new(
        repository: &str,
        parent_issue: u64,
        candidates: &[CapabilityCandidate],
        existing: &[IssueInput],
        specifications: &[IssueSpecification],
    ) -> Self {
        let mut existing = existing.to_vec();
        existing.sort_by_key(|issue| issue.number);
        let mut candidates = candidates.to_vec();
        candidates.sort_by(|left, right| left.id.cmp(&right.id));
        let mut specifications = specifications.to_vec();
        specifications.sort_by(|left, right| left.capability_id.cmp(&right.capability_id));
        let source_hash =
            digest_json(&(candidates.clone(), existing.clone(), specifications.clone()));
        let audits = audit_existing(&existing, parent_issue, &candidates);
        let mut blocked = audits
            .iter()
            .filter(|audit| audit.state == "open" && audit.status != "ready")
            .flat_map(|audit| {
                audit
                    .findings
                    .iter()
                    .map(move |finding| format!("issue #{}: {finding}", audit.number))
            })
            .collect::<BTreeSet<_>>();
        let mut creations: Vec<PlannedCreation> = Vec::new();
        let mut updates: Vec<PlannedUpdate> = Vec::new();
        let mut closures: Vec<PlannedClosure> = Vec::new();
        let mut label_changes: Vec<PlannedLabelChange> = Vec::new();
        let mut milestone_changes: Vec<PlannedMilestoneChange> = Vec::new();
        let known_ids = candidates
            .iter()
            .map(|candidate| candidate.id.clone())
            .collect::<BTreeSet<_>>();
        for candidate in &candidates {
            let owners = existing
                .iter()
                .filter(|issue| issue_is_owner(issue, &candidate.id, &known_ids))
                .collect::<Vec<_>>();
            let active = owners
                .iter()
                .filter(|issue| issue.state == "open" && issue.replacement_issue.is_none())
                .copied()
                .collect::<Vec<_>>();
            let mut active_roles = BTreeMap::<String, u64>::new();
            for issue in &active {
                let role = issue_role(issue, &candidate.id, &known_ids)
                    .unwrap_or_else(|| "implementation".to_owned());
                if let Some(previous) = active_roles.insert(role.clone(), issue.number) {
                    blocked.insert(format!(
                        "capability {} has multiple open {} owners: #{} and #{}",
                        candidate.id, role, previous, issue.number
                    ));
                }
            }
            let canonical = active.first().copied().or_else(|| {
                owners
                    .iter()
                    .find(|issue| {
                        issue.state == "closed"
                            && issue.state_reason.as_deref() == Some("completed")
                    })
                    .copied()
            });
            for duplicate in existing.iter().filter(|issue| {
                issue.state == "open"
                    && issue.replacement_issue.is_some()
                    && issue_is_owner(issue, &candidate.id, &known_ids)
            }) {
                let replacement = duplicate.replacement_issue.expect("checked above");
                if existing.iter().any(|issue| {
                    issue.number == replacement
                        && (issue.state == "open"
                            || (issue.state == "closed"
                                && issue.state_reason.as_deref() == Some("completed")))
                }) {
                    closures.push(PlannedClosure {
                        issue: duplicate.number,
                        expected_etag: duplicate.etag.clone(),
                        expected_body_sha256: digest_bytes(duplicate.body.as_bytes()),
                        replacement_issue: replacement,
                    });
                }
            }
            if let Some(owner) = canonical {
                if !issue_has_explicit_capability(owner, &candidate.id, &known_ids) {
                    let role = issue_role(owner, &candidate.id, &known_ids)
                        .unwrap_or_else(|| "implementation".to_owned());
                    updates.push(PlannedUpdate {
                        issue: owner.number,
                        expected_etag: owner.etag.clone(),
                        expected_body_sha256: digest_bytes(owner.body.as_bytes()),
                        replacement_body: add_capability_metadata(
                            &owner.body,
                            &candidate.id,
                            &role,
                        ),
                    });
                }
                let expected_milestone = milestone_number(&candidate.category);
                if owner.milestone != Some(expected_milestone) {
                    milestone_changes.push(PlannedMilestoneChange {
                        issue: owner.number,
                        expected_etag: owner.etag.clone(),
                        expected_body_sha256: digest_bytes(owner.body.as_bytes()),
                        milestone: milestone_for_category(&candidate.category).to_owned(),
                        milestone_number: expected_milestone,
                    });
                }
                continue;
            }
            let Some(specification) = specifications
                .iter()
                .find(|spec| spec.capability_id == candidate.id)
            else {
                blocked.insert(format!(
                    "capability {} has no reviewed issue specification",
                    candidate.id
                ));
                continue;
            };
            match validate_specification(specification, parent_issue, &known_ids) {
                Ok(()) => creations.push(PlannedCreation {
                    capability_id: candidate.id.clone(),
                    role: specification.role.clone(),
                    title: specification.title.clone(),
                    body: specification.body.clone(),
                    priority: specification.priority.clone(),
                    milestone: milestone_for(specification),
                    milestone_number: milestone_number(&specification.category),
                    dependencies: specification.dependencies.clone(),
                }),
                Err(findings) => blocked.extend(
                    findings
                        .into_iter()
                        .map(|finding| format!("capability {}: {finding}", candidate.id)),
                ),
            }
        }
        creations.sort_by(|left, right| left.capability_id.cmp(&right.capability_id));
        updates.sort_by_key(|update| update.issue);
        closures.sort_by_key(|closure| closure.issue);
        label_changes.sort_by_key(|change| change.issue);
        milestone_changes.sort_by_key(|change| change.issue);
        let mut plan = Self {
            schema_version: 1,
            repository: repository.to_owned(),
            parent_issue,
            source_hash,
            plan_sha256: String::new(),
            audits,
            creations,
            updates,
            closures,
            label_changes,
            milestone_changes,
            blocked_ambiguities: blocked.into_iter().collect(),
        };
        plan.plan_sha256 = digest_bytes(&plan.canonical_bytes());
        plan
    }
}

/// Builds a deterministic, reviewable reconciliation plan.
///
/// # Errors
///
/// Returns an error when the repository identity or parent issue is invalid.
pub fn build_reconciliation_plan(
    repository: &str,
    parent_issue: u64,
    candidates: &[CapabilityCandidate],
    existing: &[IssueInput],
    specifications: &[IssueSpecification],
) -> Result<ReconciliationPlan, String> {
    if repository.trim().is_empty() || parent_issue == 0 {
        return Err("repository and parent issue are required".to_owned());
    }
    Ok(ReconciliationPlan::new(
        repository,
        parent_issue,
        candidates,
        existing,
        specifications,
    ))
}

/// Parses the strict capability metadata section from an issue body.
///
/// # Errors
///
/// Returns an error for malformed bullets, escaped newlines, duplicate roles,
/// or unknown capability IDs.
pub fn parse_capability_declarations(
    body: &str,
    known_ids: &[String],
) -> Result<Vec<CapabilityDeclaration>, String> {
    if body.contains("\\n") {
        return Err("capability metadata contains literal escaped newlines".to_owned());
    }
    let mut in_section = false;
    let mut declarations: Vec<CapabilityDeclaration> = Vec::new();
    let known = known_ids.iter().collect::<BTreeSet<_>>();
    for line in body.lines() {
        if line.trim() == "## Capabilities" {
            in_section = true;
            continue;
        }
        if in_section && line.trim_start().starts_with("## ") {
            break;
        }
        if !in_section || line.trim().is_empty() {
            continue;
        }
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("- `") else {
            return Err("capability metadata must use '- `id` — role `name`' bullets".to_owned());
        };
        let Some((id, rest)) = rest.split_once("` — role `") else {
            return Err("capability metadata has malformed declaration".to_owned());
        };
        let Some(role) = rest.strip_suffix('`') else {
            return Err("capability metadata has malformed role".to_owned());
        };
        if id.is_empty() || role.is_empty() || !known.contains(&id.to_owned()) {
            return Err(format!(
                "capability metadata references unknown or empty capability {id:?}"
            ));
        }
        let declaration = CapabilityDeclaration {
            capability_id: id.to_owned(),
            role: role.to_owned(),
        };
        if declarations
            .iter()
            .any(|previous| previous.capability_id == declaration.capability_id)
        {
            return Err(format!("duplicate capability declaration for {id}"));
        }
        declarations.push(declaration);
    }
    Ok(declarations)
}

fn audit_existing(
    existing: &[IssueInput],
    parent_issue: u64,
    candidates: &[CapabilityCandidate],
) -> Vec<IssueAudit> {
    let known_ids = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect::<BTreeSet<_>>();
    existing
        .iter()
        .map(|issue| {
            let mut findings = Vec::new();
            let known = known_ids.iter().cloned().collect::<Vec<_>>();
            if issue.body.contains("## Capabilities")
                && parse_capability_declarations(&issue.body, &known).is_err()
            {
                findings.push(
                    "capability metadata is malformed or names an unknown capability".to_owned(),
                );
            }
            let status = if issue.state == "open" {
                if issue.milestone.is_none() {
                    findings.push("missing milestone".to_owned());
                }
                if !issue
                    .body
                    .lines()
                    .any(|line| line.trim() == format!("Parent: #{parent_issue}"))
                {
                    findings.push(format!("missing Parent: #{parent_issue} reference"));
                }
                if issue
                    .priority_labels
                    .iter()
                    .filter(|label| label.starts_with("priority:"))
                    .count()
                    != 1
                {
                    findings.push("expected exactly one priority label".to_owned());
                } else if !issue.priority_labels.iter().any(|label| {
                    label
                        .strip_prefix("priority: ")
                        .is_some_and(|priority| PRIORITIES.contains(&priority))
                }) {
                    findings.push("priority label must be one of P0 through P4".to_owned());
                }
                validate_sections(&issue.body, &mut findings);
                validate_decisions(&issue.body, &mut findings);
                if numeric_title_prefix(&issue.title) {
                    findings.push("numeric title prefix is prohibited".to_owned());
                }
                if findings.is_empty() {
                    "ready"
                } else {
                    classify(&findings)
                }
            } else {
                "closed-audited"
            };
            IssueAudit {
                number: issue.number,
                title: issue.title.clone(),
                state: issue.state.clone(),
                state_reason: issue.state_reason.clone(),
                body_sha256: digest_bytes(issue.body.as_bytes()),
                etag: issue.etag.clone(),
                milestone: issue.milestone,
                priority_labels: issue.priority_labels.clone(),
                status: status.to_owned(),
                findings,
            }
        })
        .collect()
}

fn issue_is_owner(issue: &IssueInput, capability_id: &str, known_ids: &BTreeSet<String>) -> bool {
    if issue.state_reason.as_deref() == Some("not_planned")
        || issue.state_reason.as_deref() == Some("duplicate")
    {
        return false;
    }
    let known = known_ids.iter().cloned().collect::<Vec<_>>();
    if let Ok(declarations) = parse_capability_declarations(&issue.body, &known)
        && (declarations
            .iter()
            .any(|declaration| declaration.capability_id == capability_id)
            || (capability_id.is_empty() && !declarations.is_empty()))
    {
        return true;
    }
    compatibility_alias(&issue.body) == Some(capability_id)
        || (capability_id.is_empty() && compatibility_alias(&issue.body).is_some())
}

fn issue_has_explicit_capability(
    issue: &IssueInput,
    capability_id: &str,
    known_ids: &BTreeSet<String>,
) -> bool {
    let known = known_ids.iter().cloned().collect::<Vec<_>>();
    parse_capability_declarations(&issue.body, &known).is_ok_and(|declarations| {
        declarations
            .iter()
            .any(|declaration| declaration.capability_id == capability_id)
    })
}

fn issue_role(
    issue: &IssueInput,
    capability_id: &str,
    known_ids: &BTreeSet<String>,
) -> Option<String> {
    let known = known_ids.iter().cloned().collect::<Vec<_>>();
    parse_capability_declarations(&issue.body, &known)
        .ok()?
        .into_iter()
        .find(|declaration| declaration.capability_id == capability_id)
        .map(|declaration| declaration.role)
}

fn compatibility_alias(body: &str) -> Option<&'static str> {
    let identity = body.lines().find_map(|line| {
        let marker = "compatibility identity remains exactly";
        let start = line.to_ascii_lowercase().find(marker)? + marker.len();
        let suffix = line.get(start..)?;
        let suffix = suffix.trim_start();
        let suffix = suffix.strip_prefix('`')?;
        suffix.split('`').next()
    })?;
    match identity {
        "clipping" | "crop-and-rotate" => Some("iop.clipping"),
        "overlay" => Some("iop.overlay"),
        "invert" | "film-negative inversion" => Some("iop.invert"),
        _ => None,
    }
}

fn add_capability_metadata(body: &str, capability_id: &str, role: &str) -> String {
    let mut lines = body.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    if let Some(start) = lines
        .iter()
        .position(|line| line.trim() == "## Capabilities")
    {
        let end = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, line)| line.trim_start().starts_with("## "))
            .map_or(lines.len(), |(index, _)| index);
        lines.insert(end, format!("- `{capability_id}` — role `{role}`"));
        return lines.join("\n");
    }
    format!(
        "Parent: #158\n\n## Capabilities\n\n- `{capability_id}` — role `{role}`\n\n{}",
        body.trim_start_matches("Parent: #158").trim()
    )
}

fn validate_specification(
    spec: &IssueSpecification,
    parent_issue: u64,
    known_ids: &BTreeSet<String>,
) -> Result<(), Vec<String>> {
    let mut findings = Vec::new();
    if !known_ids.contains(&spec.capability_id) {
        findings.push(format!("unknown capability {}", spec.capability_id));
    }
    if spec.role.trim().is_empty()
        || spec.category.trim().is_empty()
        || spec.title.trim().is_empty()
    {
        findings.push("capability, role, category, and title are required".to_owned());
    }
    if !PRIORITIES.contains(&spec.priority.as_str()) {
        findings.push(format!("invalid priority {}", spec.priority));
    }
    if spec.body.contains("\\n") {
        findings.push("literal escaped newline is prohibited".to_owned());
    }
    if !spec
        .body
        .lines()
        .any(|line| line.trim() == format!("Parent: #{parent_issue}"))
    {
        findings.push(format!("missing Parent: #{parent_issue} reference"));
    }
    validate_sections(&spec.body, &mut findings);
    validate_decisions(&spec.body, &mut findings);
    let known = known_ids.iter().cloned().collect::<Vec<_>>();
    match parse_capability_declarations(&spec.body, &known) {
        Ok(declarations)
            if declarations.iter().any(|declaration| {
                declaration.capability_id == spec.capability_id && declaration.role == spec.role
            }) => {}
        Ok(_) => findings.push("capability metadata does not match specification role".to_owned()),
        Err(error) => findings.push(error),
    }
    if !spec.body.contains(&format!("`{}`", spec.capability_id)) {
        findings.push("body is not capability-specific".to_owned());
    }
    let sections = headings(&spec.body);
    if let Some(dependencies) = sections.get("Dependencies")
        && !dependencies
            .trim()
            .trim_end_matches('.')
            .eq_ignore_ascii_case("none")
        && !dependencies.contains('#')
    {
        findings.push("Dependencies must be None or direct #<issue> links".to_owned());
    }
    if !spec.dependencies.is_empty()
        && !sections.get("Dependencies").is_some_and(|dependencies| {
            spec.dependencies
                .iter()
                .all(|number| dependencies.contains(&format!("#{number}")))
        })
    {
        findings.push("dependency list differs from the issue body".to_owned());
    }
    if let Some(milestone) = spec.milestone
        && milestone != milestone_number(&spec.category)
    {
        findings.push(format!(
            "milestone {milestone} does not match the category policy ({})",
            milestone_number(&spec.category)
        ));
    }
    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings)
    }
}

fn milestone_for(spec: &IssueSpecification) -> String {
    milestone_for_category(&spec.category).to_owned()
}

fn milestone_for_category(category: &str) -> &'static str {
    match category.to_ascii_lowercase().as_str() {
        "darkroom" | "image operation" | "processing" | "export" | "export/storage"
        | "workflow" => "Processing Pipeline & Color",
        "ui" | "platform" => "Desktop Experience & Iced UI",
        "codec" | "image io" => "Image I/O & Metadata",
        _ => "Foundation & Developer Workflow",
    }
}

fn milestone_number(category: &str) -> u64 {
    match category.to_ascii_lowercase().as_str() {
        "darkroom" | "image operation" | "processing" | "export" | "export/storage"
        | "workflow" => 5,
        "ui" | "platform" => 2,
        "codec" | "image io" => 4,
        _ => 1,
    }
}

fn validate_sections(body: &str, findings: &mut Vec<String>) {
    let sections = headings(body);
    for required in REQUIRED_SECTIONS {
        if sections
            .get(required)
            .is_none_or(|content| content.trim().is_empty())
        {
            findings.push(format!(
                "required section ## {required} is missing or empty"
            ));
        }
    }
}

fn validate_decisions(body: &str, findings: &mut Vec<String>) {
    let lower = body.to_ascii_lowercase();
    for marker in DEFERRAL_MARKERS {
        if lower.contains(marker) {
            findings.push(format!("unresolved decision marker {marker:?}"));
        }
    }
}

fn classify(findings: &[String]) -> &'static str {
    if findings.iter().any(|finding| finding.contains("decision")) {
        "needs-decisions"
    } else if findings.iter().any(|finding| finding.contains("Test")) {
        "needs-tests"
    } else if findings
        .iter()
        .any(|finding| finding.contains("Acceptance"))
    {
        "needs-acceptance-evidence"
    } else {
        "invalid"
    }
}

fn headings(body: &str) -> BTreeMap<String, String> {
    let mut sections = BTreeMap::new();
    let mut current = None;
    let mut content = String::new();
    for line in body.lines() {
        if let Some(title) = line.trim().strip_prefix("## ") {
            if let Some(previous) = current.take() {
                sections.insert(previous, content.trim().to_owned());
            }
            current = Some(title.trim().to_owned());
            content.clear();
        } else if current.is_some() {
            content.push_str(line);
            content.push('\n');
        }
    }
    if let Some(previous) = current {
        sections.insert(previous, content.trim().to_owned());
    }
    sections
}

fn numeric_title_prefix(title: &str) -> bool {
    let Some((prefix, _)) = title.trim_start().split_once(']') else {
        return false;
    };
    let Some(prefix) = prefix.strip_prefix('[') else {
        return false;
    };
    (prefix.len() == 4 && prefix.bytes().all(|byte| byte.is_ascii_digit()))
        || (prefix.len() == 5
            && prefix[..4].bytes().all(|byte| byte.is_ascii_digit())
            && prefix.as_bytes()[4].is_ascii_uppercase())
}

fn digest_json<T: Serialize>(value: &T) -> String {
    digest_bytes(&serde_json::to_vec(value).expect("JSON serialization cannot fail"))
}

fn digest_bytes(bytes: &[u8]) -> String {
    let mut digest = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(&mut digest, "{byte:02x}").expect("writing to a String cannot fail");
    }
    digest
}
