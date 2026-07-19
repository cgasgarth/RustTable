use std::collections::BTreeSet;

use super::super::model::{Finding, IssueInput, SourceMap};
use super::operations::priority;

pub(super) fn strict_audit_issue_numbers(
    map: &SourceMap,
    issues: &[IssueInput],
    parent: u64,
) -> BTreeSet<u64> {
    let mapped = map
        .issues
        .iter()
        .map(|record| record.number)
        .collect::<BTreeSet<_>>();
    issues
        .iter()
        .filter(|issue| {
            !issue.is_pull_request
                && issue.number != parent
                && mapped.contains(&issue.number)
                && issue.state == "open"
                && (matches!(
                    priority(&issue.labels).as_str(),
                    "priority: P0" | "priority: P1"
                ) || map.issues.iter().any(|record| {
                    record.number == issue.number && record.source_status == "anchored"
                }))
        })
        .map(|issue| issue.number)
        .collect()
}

pub(super) fn is_master_child(issue: &IssueInput, parent: u64) -> bool {
    issue.body.lines().any(|line| {
        line.trim()
            .eq_ignore_ascii_case(&format!("parent: #{parent}"))
    })
}

pub(super) fn debt_finding(mut finding: Finding) -> Finding {
    finding.code = finding.code.replacen("error-", "debt-", 1);
    finding
}

#[cfg(test)]
mod tests {
    use super::debt_finding;
    use crate::commands::source_map::model::Finding;

    #[test]
    fn legacy_findings_are_explicit_migration_debt() {
        let finding = Finding::error("missing-source-block", Some(557), None, "legacy");
        assert_eq!(debt_finding(finding).code, "debt-missing-source-block");
    }
}
