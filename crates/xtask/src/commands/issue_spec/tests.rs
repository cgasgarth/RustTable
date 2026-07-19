use super::extract::{
    canonical_body_hash, canonical_spec_hash, extract_sections, parse_dependencies,
};
use super::types::*;

const VALID: &str = r"# Outcome

Users receive a deterministic image receipt for every completed edit.

## Fixed decisions

- Algorithm: SHA-256 over normalized UTF-8.
- Format: JSON receipt with stable field ordering.
- Default: reject non-finite values.

## Implementation scope

Implement the parser in crates/xtask and keep the policy data model independent of GitHub transport.

## Failure and edge behavior

Malformed input returns a typed error; cancellation is reported and restart resumes from the last receipt. Platform differences are explicit for Windows and macOS.

## Test matrix

Positive valid input, negative malformed input, boundary empty and maximum input, cancellation/restart, platform, and parity cases.

## Acceptance evidence

Run `cargo test -p xtask issue_spec` and inspect the deterministic JSON receipt artifact with exit code 0.

## Dependencies

None

## One-PR boundary

One PR implements this parser and policy module. Do not change CLI wiring, workflows, or product capabilities.
";

#[test]
fn v2_body_is_ready_and_hashes_are_stable() {
    let issue = IssueSnapshot::new(518, "semantic issue specification", VALID);
    let policy = IssueSpecPolicy::default();
    let audit = policy.audit(&issue);
    assert_eq!(audit.status, SpecificationStatus::ReadyV2);
    assert!(audit.findings.is_empty(), "{:?}", audit.findings);
    assert_eq!(audit.body_hash, canonical_body_hash(VALID));
    assert_eq!(audit.spec_hash, canonical_spec_hash(&audit.sections));
}

#[test]
fn legacy_aliases_are_explicitly_tracked() {
    let body = VALID
        .replace("# Outcome", "# Goal")
        .replace("## Implementation scope", "## Required implementation")
        .replace("## Acceptance evidence", "## Acceptance criteria");
    let extraction = extract_sections(&body);
    assert!(extraction.legacy_equivalent);
    assert_eq!(
        extraction.get(SectionRole::Outcome).unwrap().source,
        SectionSource::LegacyEquivalent
    );
    assert_eq!(
        extraction
            .get(SectionRole::ImplementationScope)
            .unwrap()
            .source,
        SectionSource::LegacyEquivalent
    );
    assert_eq!(
        extraction
            .get(SectionRole::AcceptanceEvidence)
            .unwrap()
            .source,
        SectionSource::LegacyEquivalent
    );
    assert_eq!(
        IssueSpecPolicy::default()
            .audit(&IssueSnapshot::new(1, "legacy", &body))
            .status,
        SpecificationStatus::ReadyLegacyEquivalent
    );
}

#[test]
fn decision_deferrals_and_unnamed_protocols_are_blocked() {
    let body = VALID.replace("Algorithm: SHA-256", "Algorithm: decide later");
    let audit = IssueSpecPolicy::default().audit(&IssueSnapshot::new(1, "bad decisions", body));
    assert_eq!(audit.status, SpecificationStatus::NeedsDecisions);
    assert!(
        audit
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::UnresolvedDecision)
    );
}

#[test]
fn dependencies_require_github_references_or_none() {
    let parsed = parse_dependencies("Depends on 446 and #492");
    assert_eq!(parsed.references[0].issue, 492);
    assert_eq!(parsed.invalid_numeric_tokens, vec!["446"]);
    let parsed = parse_dependencies("None, but also #446");
    assert!(parsed.explicit_none);
    assert_eq!(parsed.references.len(), 1);
}

#[test]
fn capabilities_require_names_and_roles_for_multiple_records() {
    let body = VALID.replace(
        "## One-PR boundary",
        "## Capabilities\n\n- id: operation.one\n- id: operation.two\n\n## One-PR boundary",
    );
    let mut issue = IssueSnapshot::new(1, "capability", body);
    issue.labels.push("capability".to_owned());
    let audit = IssueSpecPolicy::default().audit(&issue);
    assert!(
        audit
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::MultipleCapabilityOwners)
    );
}

#[test]
fn duplicate_headings_and_literal_newlines_are_invalid() {
    let body = format!("{VALID}\n## Outcome\nA second outcome.\nLiteral \\n");
    let audit = IssueSpecPolicy::default().audit(&IssueSnapshot::new(1, "malformed", body));
    assert_eq!(audit.status, SpecificationStatus::Invalid);
    assert!(
        audit
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::DuplicateSection)
    );
    assert!(
        audit
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::LiteralEscapedNewline)
    );
}

#[test]
fn normalized_hash_ignores_line_endings_bom_and_trailing_space() {
    let variant = format!("\u{feff}{}\r\n", VALID.replace('\n', "\r\n"));
    assert_eq!(canonical_body_hash(VALID), canonical_body_hash(&variant));
}

#[test]
fn reference_index_reports_unknown_and_superseded_dependencies() {
    let mut references = ReferenceIndex::default();
    references.known_issues.insert(446);
    references.superseded.insert(492, 518);
    let body = VALID.replace("None", "#446 and #492");
    let audit = IssueSpecPolicy::new(references).audit(&IssueSnapshot::new(1, "refs", body));
    assert_eq!(audit.status, SpecificationStatus::DuplicateOrSuperseded);
    assert!(
        audit
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::UnknownDependency)
    );
    assert!(
        audit
            .findings
            .iter()
            .any(|finding| finding.code == FindingCode::SupersededDependency)
    );
    assert!(!audit.remediation.steps.is_empty());
}
