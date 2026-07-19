use std::collections::BTreeMap;

use super::checks::{
    add_finding, check_acceptance, check_capabilities, check_class_contract, check_decisions,
    check_dependencies, check_failure_contract, check_one_pr, check_structure, check_tests,
    classify_issue, refresh_audit_result, remediation_for, status_for,
};
use super::extract::{
    canonical_body_hash, canonical_spec_hash, dependency_cycles, extract_sections,
    normalized_body_fingerprint, parse_dependencies,
};
use super::helpers::{is_generic_placeholder, parse_capabilities};
use super::types::{
    AuditOptions, Finding, FindingCode, IssueAudit, IssueSnapshot, IssueSpecPolicy, ReferenceIndex,
    SCHEMA, SectionRole,
};

impl IssueSpecPolicy {
    pub fn new(references: ReferenceIndex) -> Self {
        Self { references }
    }

    pub fn audit(&self, issue: &IssueSnapshot) -> IssueAudit {
        let options = AuditOptions {
            capability_owner: self.references.capability_owners.contains(&issue.number),
        };
        audit_issue(issue, self, options)
    }

    #[expect(
        dead_code,
        reason = "kept for capability-owner and policy-specific audits"
    )]
    pub fn audit_with_options(&self, issue: &IssueSnapshot, options: AuditOptions) -> IssueAudit {
        audit_issue(issue, self, options)
    }

    pub fn audit_all(&self, issues: &[IssueSnapshot]) -> Vec<IssueAudit> {
        let mut audits: Vec<_> = issues.iter().map(|issue| self.audit(issue)).collect();
        let mut fingerprints: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (index, issue) in issues.iter().enumerate() {
            fingerprints
                .entry(normalized_body_fingerprint(&issue.body))
                .or_default()
                .push(index);
        }
        for indexes in fingerprints.values().filter(|indexes| indexes.len() > 1) {
            if !indexes
                .iter()
                .any(|index| is_generic_placeholder(&issues[*index].body))
            {
                continue;
            }
            for index in indexes {
                add_finding(
                    &mut audits[*index].findings,
                    Finding::new(
                        FindingCode::CopiedGenericBody,
                        None,
                        None,
                        "body is copied generic specification text shared by unrelated issues",
                        Some(normalized_body_fingerprint(&issues[*index].body)),
                    ),
                );
                refresh_audit_result(&mut audits[*index]);
            }
        }
        let cycles = dependency_cycles(&self.references.dependency_graph);
        for cycle in cycles {
            for issue in &cycle {
                if let Some(audit) = audits.iter_mut().find(|audit| audit.issue == *issue) {
                    add_finding(
                        &mut audit.findings,
                        Finding::new(
                            FindingCode::DependencyCycle,
                            Some(SectionRole::Dependencies),
                            None,
                            "dependency graph contains a cycle",
                            Some(
                                cycle
                                    .iter()
                                    .map(u64::to_string)
                                    .collect::<Vec<_>>()
                                    .join(" -> "),
                            ),
                        ),
                    );
                    refresh_audit_result(audit);
                }
            }
        }
        audits
    }
}

pub fn audit_issue(
    issue: &IssueSnapshot,
    policy: &IssueSpecPolicy,
    options: AuditOptions,
) -> IssueAudit {
    let sections = extract_sections(&issue.body);
    let class = classify_issue(issue, &sections);
    let dependencies = sections.get(SectionRole::Dependencies).map_or_else(
        || parse_dependencies(""),
        |section| parse_dependencies(&section.content),
    );
    let capabilities = sections
        .get(SectionRole::Capabilities)
        .map_or_else(Vec::new, |section| parse_capabilities(&section.content));
    let mut findings = Vec::new();
    check_structure(&sections, &mut findings);
    check_decisions(&sections, &mut findings);
    check_failure_contract(&sections, &issue.body, &mut findings);
    check_tests(&sections, &issue.body, &mut findings);
    check_acceptance(&sections, &mut findings);
    check_dependencies(
        issue.number,
        &sections,
        &dependencies,
        policy,
        &mut findings,
    );
    check_one_pr(&sections, &mut findings);
    check_capabilities(
        issue,
        options,
        &sections,
        &capabilities,
        policy,
        &mut findings,
    );
    check_class_contract(issue, class, &sections, &mut findings);
    if sections.literal_escaped_newline {
        add_finding(
            &mut findings,
            Finding::new(
                FindingCode::LiteralEscapedNewline,
                None,
                None,
                "issue body contains literal escaped newlines instead of Markdown line breaks",
                Some(r"\n".to_owned()),
            ),
        );
    }
    for duplicate in &sections.duplicates {
        add_finding(
            &mut findings,
            Finding::new(
                FindingCode::DuplicateSection,
                Some(duplicate.role),
                Some(duplicate.duplicate_line),
                format!("section {} appears more than once", duplicate.role.as_str()),
                Some(format!("first line {}", duplicate.first_line)),
            ),
        );
    }
    let spec_hash = canonical_spec_hash(&sections);
    let body_hash = canonical_body_hash(&issue.body);
    let source_anchor_hash = sections
        .get(SectionRole::SourceAnchors)
        .map_or_else(String::new, |section| section.content_hash.clone());
    let status = status_for(&sections, &findings);
    let remediation = remediation_for(issue.number, status, &findings);
    IssueAudit {
        schema: SCHEMA,
        issue: issue.number,
        title: issue.title.clone(),
        class,
        status,
        body_hash,
        source_anchor_hash,
        spec_hash,
        sections,
        dependencies,
        capabilities,
        findings,
        remediation,
    }
}
