use rusttable_parity::{
    CapabilityCandidate, CapabilityDeclaration, IssueInput, IssueSpecification, ReconciliationPlan,
    build_reconciliation_plan, parse_capability_declarations,
};

fn body(id: &str, identity: &str) -> String {
    format!(
        "Parent: #158\n\n## Capabilities\n\n- `{id}` — role `implementation`\n\n## Outcome\nDeliver {id}.\n\n## Fixed decisions\nCompatibility identity remains exactly `{identity}`.\n\n## Implementation\nImplement {id} in the reviewed RustTable module with a checked plan.\n\n## Failure and edge behavior\nReject invalid input and publish no partial state.\n\n## Test matrix\nUnit, boundary, failure, restart, and parity tests for {id}.\n\n## Acceptance evidence\n`cargo test -p rusttable-processing {id}` produces a deterministic receipt.\n\n## Dependencies\nNone.\n\n## One-PR boundary\nOne PR implements {id} only."
    )
}

fn issue(number: u64, body: String, state: &str, reason: Option<&str>) -> IssueInput {
    IssueInput {
        number,
        title: format!("Issue {number}"),
        state: state.to_owned(),
        state_reason: reason.map(str::to_owned),
        body,
        milestone: Some(5),
        priority_labels: vec!["priority: P2".to_owned()],
        repository: "cgasgarth/RustTable".to_owned(),
        etag: format!("etag-{number}"),
        replacement_issue: None,
    }
}

fn candidate(id: &str, category: &str) -> CapabilityCandidate {
    CapabilityCandidate {
        id: id.to_owned(),
        category: category.to_owned(),
        reference_path: format!("src/iop/{id}.c"),
        reference_symbol: id.to_owned(),
        structural_evidence: vec!["reference-scan:iop".to_owned()],
        behavioral_evidence: vec![format!("parity:{id}")],
        acceptance_test_id: format!("capability.{id}"),
    }
}

fn specification(id: &str, category: &str) -> IssueSpecification {
    IssueSpecification {
        capability_id: id.to_owned(),
        role: "implementation".to_owned(),
        category: category.to_owned(),
        title: format!("Implement {id}"),
        body: body(id, id),
        priority: "P2".to_owned(),
        milestone: None,
        compatibility_identity: Some(id.to_owned()),
        dependencies: Vec::new(),
    }
}

#[test]
fn capability_metadata_is_strict_and_unknown_ids_are_rejected() {
    let parsed = parse_capability_declarations(
        "## Capabilities\n\n- `iop.clipping` — role `implementation`\n",
        &["iop.clipping".to_owned()],
    )
    .expect("metadata");
    assert_eq!(
        parsed,
        vec![CapabilityDeclaration {
            capability_id: "iop.clipping".to_owned(),
            role: "implementation".to_owned(),
        }]
    );
    for malformed in [
        "## Capabilities\\n\\n- `iop.clipping` — role `implementation`",
        "## Capabilities\n\n* `iop.clipping` — role `implementation`",
        "## Capabilities\n\n- `iop.unknown` — role `implementation`",
        "## Capabilities\n\n- `iop.clipping` — role `implementation`\n- `iop.clipping` — role `implementation`",
    ] {
        assert!(parse_capability_declarations(malformed, &["iop.clipping".to_owned()]).is_err());
    }
}

#[test]
fn canonical_owner_wins_over_closed_superseded_duplicate_and_title_similarity() {
    let canonical = issue(380, body("iop.clipping", "clipping"), "open", None);
    let duplicate = IssueInput {
        replacement_issue: Some(380),
        ..issue(
            471,
            body("iop.clipping", "clipping"),
            "closed",
            Some("not_planned"),
        )
    };
    let similarly_titled = issue(
        999,
        "Title-only candidate".to_owned(),
        "closed",
        Some("completed"),
    );
    let plan = build_reconciliation_plan(
        "cgasgarth/RustTable",
        158,
        &[candidate("iop.clipping", "darkroom")],
        &[similarly_titled, duplicate, canonical],
        &[],
    )
    .expect("canonical owner");
    assert!(plan.creations.is_empty());
    assert!(plan.blocked_ambiguities.is_empty());
    assert!(plan.closures.iter().all(|closure| closure.issue != 471));
}

#[test]
fn open_superseded_owner_is_planned_for_closure_instead_of_becoming_canonical() {
    let canonical = issue(380, body("iop.clipping", "clipping"), "open", None);
    let superseded = IssueInput {
        replacement_issue: Some(380),
        ..issue(471, body("iop.clipping", "clipping"), "open", None)
    };
    let plan = build_reconciliation_plan(
        "cgasgarth/RustTable",
        158,
        &[candidate("iop.clipping", "darkroom")],
        &[superseded, canonical],
        &[],
    )
    .expect("supersession plan");
    assert_eq!(
        plan.closures
            .iter()
            .map(|item| item.issue)
            .collect::<Vec<_>>(),
        vec![471]
    );
    assert!(plan.creations.is_empty());
}

#[test]
fn compatibility_identity_is_authoritative_when_capabilities_are_not_yet_tagged() {
    let owner = issue(
        380,
        "Parent: #158\n\n## Fixed decisions\n\nCompatibility identity remains exactly `clipping`.\n"
            .to_owned(),
        "open",
        None,
    );
    let plan = build_reconciliation_plan(
        "cgasgarth/RustTable",
        158,
        &[candidate("iop.clipping", "darkroom")],
        &[owner],
        &[],
    )
    .expect("compatibility owner");
    assert_eq!(plan.updates.len(), 1);
    assert_eq!(plan.updates[0].issue, 380);
    assert!(plan.creations.is_empty());
}

#[test]
fn incomplete_creation_is_blocked_before_any_action_is_planned() {
    let mut spec = specification("iop.new", "darkroom");
    spec.body = "Parent: #158\n\n## Outcome\nplaceholder\n".to_owned();
    let plan = build_reconciliation_plan(
        "cgasgarth/RustTable",
        158,
        &[candidate("iop.new", "darkroom")],
        &[],
        &[spec],
    )
    .expect("plan is reviewable");
    assert!(plan.creations.is_empty());
    assert!(
        plan.blocked_ambiguities
            .iter()
            .any(|finding| finding.contains("required section"))
    );
}

#[test]
fn plan_derives_milestone_and_is_stable_for_reordered_inputs() {
    let mut first = specification("iop.first", "darkroom");
    first.milestone = None;
    let mut second = specification("iop.second", "darkroom");
    second.milestone = None;
    let left = build_reconciliation_plan(
        "cgasgarth/RustTable",
        158,
        &[
            candidate("iop.first", "darkroom"),
            candidate("iop.second", "darkroom"),
        ],
        &[],
        &[first.clone(), second.clone()],
    )
    .expect("left plan");
    let right = build_reconciliation_plan(
        "cgasgarth/RustTable",
        158,
        &[
            candidate("iop.second", "darkroom"),
            candidate("iop.first", "darkroom"),
        ],
        &[],
        &[second, first],
    )
    .expect("right plan");
    assert_eq!(left.canonical_bytes(), right.canonical_bytes());
    assert_eq!(left.creations[0].milestone, "Processing Pipeline & Color");
    ReconciliationPlan::validate_for_apply(&left, &[]).expect("offline apply validation");
}

#[test]
fn optimistic_apply_rejects_missing_or_changed_issue_state() {
    let snapshot = issue(380, body("iop.clipping", "clipping"), "open", None);
    let plan = build_reconciliation_plan(
        "cgasgarth/RustTable",
        158,
        &[candidate("iop.clipping", "darkroom")],
        std::slice::from_ref(&snapshot),
        &[],
    )
    .expect("plan");
    let mut changed = snapshot;
    changed.etag = "changed".to_owned();
    assert!(plan.validate_for_apply(&[changed]).is_err());
    assert!(plan.validate_for_apply(&[]).is_err());
}

#[test]
fn distinct_explicit_roles_allow_one_to_many_ownership() {
    let domain = issue(380, body("iop.clipping", "clipping"), "open", None);
    let mut acceptance = issue(381, body("iop.clipping", "clipping"), "open", None);
    acceptance.body = acceptance
        .body
        .replace("role `implementation`", "role `acceptance`");
    let plan = build_reconciliation_plan(
        "cgasgarth/RustTable",
        158,
        &[candidate("iop.clipping", "darkroom")],
        &[domain, acceptance],
        &[],
    )
    .expect("plan");
    assert!(plan.blocked_ambiguities.is_empty());
    assert!(plan.creations.is_empty());
}
