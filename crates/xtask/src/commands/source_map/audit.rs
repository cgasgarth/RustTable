use std::collections::{BTreeMap, BTreeSet};

use super::super::super::issue_spec::{SectionRole, extract_sections};
use super::super::PINNED_COMMIT;
use super::super::io::body_hash;
use super::super::model::{Finding, IssueInput, IssueRecord, SourceMap};
use super::operations::priority;
use super::policy::is_master_child;

pub(super) fn audit_records(map: &SourceMap, issues: &[IssueInput], parent: u64) -> Vec<Finding> {
    let current = issues
        .iter()
        .map(|issue| (issue.number, issue))
        .collect::<BTreeMap<_, _>>();
    let known = current.keys().copied().collect::<BTreeSet<_>>();
    let mapped = map
        .issues
        .iter()
        .map(|record| record.number)
        .collect::<BTreeSet<_>>();
    let mut findings = Vec::new();
    for issue in issues
        .iter()
        .filter(|issue| !issue.is_pull_request && is_master_child(issue, parent))
        .filter(|issue| !mapped.contains(&issue.number))
    {
        findings.push(Finding::error(
            "missing-map-record",
            Some(issue.number),
            None,
            "master-child issue has no source-map record",
        ));
    }
    for record in &map.issues {
        let Some(issue) = current.get(&record.number) else {
            findings.push(Finding::error(
                "missing-live-issue",
                Some(record.number),
                None,
                "source-map record is absent from the issue snapshot",
            ));
            continue;
        };
        if !is_master_child(issue, parent) {
            findings.push(Finding::error(
                "outside-master-scope",
                Some(issue.number),
                None,
                "source-map record does not declare the configured parent issue",
            ));
            continue;
        }
        findings.extend(audit_issue(issue, record, &known));
    }
    findings
}

fn audit_issue(issue: &IssueInput, record: &IssueRecord, known: &BTreeSet<u64>) -> Vec<Finding> {
    let mut findings = Vec::new();
    if record.body_hash != body_hash(&issue.body) {
        findings.push(Finding::error(
            "body-hash-drift",
            Some(issue.number),
            None,
            "source-map body hash is stale",
        ));
    }
    if issue.state == "open" {
        findings.extend(audit_source_block(issue));
        if record.source_status != "anchored" {
            findings.push(Finding::error(
                "source-not-ready",
                Some(issue.number),
                None,
                format!("source map status is {}", record.source_status),
            ));
        }
    }
    for dependency in &record.dependencies {
        if !known.contains(dependency) {
            findings.push(Finding::error(
                "unknown-dependency",
                Some(issue.number),
                None,
                format!("dependency #{dependency} is absent from snapshot"),
            ));
        }
    }
    if issue.state == "open" && priority(&issue.labels) == "invalid" {
        findings.push(Finding::error(
            "invalid-priority",
            Some(issue.number),
            None,
            "issue has no exactly-one priority label",
        ));
    }
    if issue.state == "open" && issue.milestone.is_none() {
        findings.push(Finding::error(
            "missing-milestone",
            Some(issue.number),
            None,
            "open child issue has no milestone",
        ));
    }
    findings
}

fn audit_source_block(issue: &IssueInput) -> Vec<Finding> {
    let mut findings = Vec::new();
    let Some(block) = source_block(&issue.body) else {
        findings.push(Finding::error(
            "missing-source-block",
            Some(issue.number),
            None,
            "open issue lacks source-anchor section",
        ));
        return findings;
    };
    for required in [PINNED_COMMIT, "Preserve:", "Redesign:", "Excluded:"] {
        if !block.contains(required) && !block.contains(&format!("**{required}")) {
            findings.push(Finding::error(
                "invalid-source-block",
                Some(issue.number),
                None,
                format!("source block lacks {required}"),
            ));
        }
    }
    if !issue.body.contains("## Rust ownership") && !issue.body.contains("Rust ownership:") {
        findings.push(Finding::error(
            "invalid-source-block",
            Some(issue.number),
            None,
            "issue lacks a Rust ownership section",
        ));
    }
    if !issue.body.contains("## Acceptance evidence")
        && !issue.body.contains("## Test matrix")
        && !issue.body.contains("Oracle:")
    {
        findings.push(Finding::error(
            "invalid-source-block",
            Some(issue.number),
            None,
            "issue lacks oracle or acceptance evidence",
        ));
    }
    findings
}

pub(super) fn source_block(body: &str) -> Option<String> {
    let sections = extract_sections(body);
    let section = sections.get(SectionRole::SourceAnchors)?;
    Some(format!(
        "## Upstream darktable source anchors\n\n{}",
        section.content
    ))
}
