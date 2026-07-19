use super::helpers::{contains_any, has_named_value, is_boilerplate};
use super::types::{
    AuditOptions, CapabilityMetadata, CapabilityRole, DependencyParse, Finding, FindingCode,
    IssueAudit, IssueClass, IssueSnapshot, IssueSpecPolicy, Remediation, RemediationKind,
    RemediationStep, SectionExtraction, SectionRole, SpecificationStatus,
};

pub(super) fn check_structure(sections: &SectionExtraction, findings: &mut Vec<Finding>) {
    for role in SectionRole::REQUIRED {
        let Some(section) = sections.get(role) else {
            let status_role = match role {
                SectionRole::FixedDecisions => "decision",
                SectionRole::FailureAndEdgeBehavior => "failure",
                SectionRole::TestMatrix => "test",
                SectionRole::AcceptanceEvidence => "acceptance",
                SectionRole::Dependencies => "dependency",
                SectionRole::OnePrBoundary => "one-PR",
                _ => role.as_str(),
            };
            add_finding(
                findings,
                Finding::new(
                    FindingCode::MissingRequiredSection,
                    Some(role),
                    None,
                    format!("required {status_role} section is missing"),
                    None,
                ),
            );
            continue;
        };
        if section.content.is_empty() {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::EmptySection,
                    Some(role),
                    Some(section.line),
                    format!("{} section is empty", role.as_str()),
                    None,
                ),
            );
        } else if matches!(
            role,
            SectionRole::Outcome | SectionRole::FixedDecisions | SectionRole::ImplementationScope
        ) && is_boilerplate(&section.content)
        {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::BoilerplateSection,
                    Some(role),
                    Some(section.line),
                    format!("{} section is boilerplate-only", role.as_str()),
                    Some(section.content.clone()),
                ),
            );
        }
    }
}

pub(super) fn check_decisions(sections: &SectionExtraction, findings: &mut Vec<Finding>) {
    let Some(section) = sections.get(SectionRole::FixedDecisions) else {
        return;
    };
    let lower = section.content.to_lowercase();
    for pattern in [
        "tbd",
        "todo",
        "to be determined",
        "choose later",
        "decide later",
        "investigate and decide",
        "as appropriate",
        "if practical",
        "open question",
    ] {
        if lower.contains(pattern) && !is_prohibition_or_rule(&lower, pattern) {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::UnresolvedDecision,
                    Some(SectionRole::FixedDecisions),
                    Some(section.line),
                    "fixed decisions contain an unresolved deferral",
                    Some(pattern.to_owned()),
                ),
            );
        }
    }
    for term in ["algorithm", "protocol", "format", "framework", "default"] {
        if lower.contains(term) && !has_named_value(&lower, term) {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::UnnamedTechnicalDecision,
                    Some(SectionRole::FixedDecisions),
                    Some(section.line),
                    format!("decision names {term} but does not name its selected value"),
                    Some(term.to_owned()),
                ),
            );
        }
    }
}

pub(super) fn check_failure_contract(
    sections: &SectionExtraction,
    body: &str,
    findings: &mut Vec<Finding>,
) {
    let Some(section) = sections.get(SectionRole::FailureAndEdgeBehavior) else {
        return;
    };
    let lower = section.content.to_lowercase();
    if !contains_any(
        &lower,
        &[
            "fail",
            "error",
            "reject",
            "invalid",
            "unsupported",
            "fallback",
        ],
    ) {
        add_finding(
            findings,
            Finding::new(
                FindingCode::MissingFailureBehavior,
                Some(SectionRole::FailureAndEdgeBehavior),
                Some(section.line),
                "failure and edge behavior does not state an observable failure outcome",
                None,
            ),
        );
    }
    let body_lower = behavioral_text(sections, body);
    for (terms, code, label) in [
        (
            &["cancel", "abort"][..],
            FindingCode::MissingCancellationBehavior,
            "cancellation",
        ),
        (
            &["restart", "resume"][..],
            FindingCode::MissingRestartBehavior,
            "restart",
        ),
        (
            &["platform", "windows", "macos", "linux"][..],
            FindingCode::MissingPlatformBehavior,
            "platform",
        ),
    ] {
        if contains_any(&body_lower, terms) && !contains_any(&lower, terms) {
            add_finding(
                findings,
                Finding::new(
                    code,
                    Some(SectionRole::FailureAndEdgeBehavior),
                    Some(section.line),
                    format!(
                        "implementation mentions {label} behavior but its failure contract does not define it"
                    ),
                    None,
                ),
            );
        }
    }
}

pub(super) fn check_tests(sections: &SectionExtraction, body: &str, findings: &mut Vec<Finding>) {
    let Some(section) = sections.get(SectionRole::TestMatrix) else {
        return;
    };
    let lower = section.content.to_lowercase();
    for (terms, label) in [
        (
            &["positive", "success", "valid", "complete", "pass", "golden"][..],
            "positive",
        ),
        (
            &[
                "negative",
                "failure",
                "invalid",
                "reject",
                "missing",
                "malformed",
                "omitted",
            ][..],
            "negative",
        ),
        (
            &["boundary", "limit", "empty", "zero", "maximum", "edge"][..],
            "boundary",
        ),
    ] {
        if !contains_any(&lower, terms) {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::MissingTestCase,
                    Some(SectionRole::TestMatrix),
                    Some(section.line),
                    format!("test matrix lacks a {label} case"),
                    None,
                ),
            );
        }
    }
    let body_lower = behavioral_text(sections, body);
    for (terms, label) in [
        (&["cancel", "abort"][..], "cancellation"),
        (&["restart", "resume"][..], "restart"),
        (&["platform", "windows", "macos", "linux"][..], "platform"),
        (
            &[
                "parity",
                "darktable compatibility",
                "reference implementation",
            ][..],
            "parity",
        ),
    ] {
        if contains_any(&body_lower, terms)
            && !contains_any(&lower, terms)
            && !contains_any(&lower, &["not applicable", "n/a"])
        {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::MissingTestCase,
                    Some(SectionRole::TestMatrix),
                    Some(section.line),
                    format!("test matrix does not cover {label} behavior"),
                    None,
                ),
            );
        }
    }
}

pub(super) fn check_acceptance(sections: &SectionExtraction, findings: &mut Vec<Finding>) {
    let Some(section) = sections.get(SectionRole::AcceptanceEvidence) else {
        return;
    };
    let lower = section.content.to_lowercase();
    let executable = contains_any(
        &lower,
        &[
            "cargo ",
            "command",
            "receipt",
            "artifact",
            "exit code",
            "metric",
            "assert",
            "`",
        ],
    );
    if !executable || (lower.contains("tests pass") && lower.split_whitespace().count() < 12) {
        add_finding(
            findings,
            Finding::new(
                FindingCode::VagueAcceptanceEvidence,
                Some(SectionRole::AcceptanceEvidence),
                Some(section.line),
                "acceptance evidence must name an executable command or measurable receipt/artifact",
                None,
            ),
        );
    }
}

fn behavioral_text(sections: &SectionExtraction, body: &str) -> String {
    let _ = body;
    [
        sections
            .get(SectionRole::Outcome)
            .map(|section| section.content.as_str()),
        sections
            .get(SectionRole::ImplementationScope)
            .map(|section| section.content.as_str()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n")
    .to_lowercase()
    .lines()
    .filter(|line| {
        !line.contains("when implementation")
            && !line.contains("where applicable")
            && !line.contains("as applicable")
            && !line.contains("class policies")
            && !line.contains("platform adapter")
    })
    .collect::<Vec<_>>()
    .join("\n")
}

fn is_prohibition_or_rule(text: &str, pattern: &str) -> bool {
    text.lines().any(|line| {
        (line.contains("no unresolved") && line.contains(pattern))
            || (line.contains("must not contain") && line.contains(pattern))
            || (line.contains("without") && line.contains(pattern))
            || (line.contains("does not pass") && line.contains(pattern))
            || (line.contains("not pass") && line.contains(pattern))
    })
}

pub(super) fn check_dependencies(
    issue: u64,
    sections: &SectionExtraction,
    dependencies: &DependencyParse,
    policy: &IssueSpecPolicy,
    findings: &mut Vec<Finding>,
) {
    let Some(section) = sections.get(SectionRole::Dependencies) else {
        return;
    };
    if !dependencies.invalid_numeric_tokens.is_empty()
        || (dependencies.references.is_empty() && !dependencies.explicit_none)
        || (dependencies.explicit_none && !dependencies.references.is_empty())
    {
        add_finding(
            findings,
            Finding::new(
                FindingCode::InvalidDependencyReference,
                Some(SectionRole::Dependencies),
                Some(section.line),
                "dependencies must contain GitHub issue references or the literal None, but not both",
                Some(dependencies.invalid_numeric_tokens.join(", ")),
            ),
        );
    }
    for dependency in &dependencies.references {
        if dependency.issue == issue {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::DependencyCycle,
                    Some(SectionRole::Dependencies),
                    Some(dependency.line),
                    "issue cannot depend on itself",
                    Some(format!("#{}", dependency.issue)),
                ),
            );
        }
        if !policy.references.known_issues.is_empty()
            && !policy.references.known_issues.contains(&dependency.issue)
        {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::UnknownDependency,
                    Some(SectionRole::Dependencies),
                    Some(dependency.line),
                    format!(
                        "dependency #{} is absent from the reviewed issue index",
                        dependency.issue
                    ),
                    None,
                ),
            );
        }
        if let Some(replacement) = policy.references.superseded.get(&dependency.issue) {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::SupersededDependency,
                    Some(SectionRole::Dependencies),
                    Some(dependency.line),
                    format!(
                        "dependency #{} is superseded by #{}",
                        dependency.issue, replacement
                    ),
                    None,
                ),
            );
        }
    }
}

pub(super) fn check_one_pr(sections: &SectionExtraction, findings: &mut Vec<Finding>) {
    let Some(section) = sections.get(SectionRole::OnePrBoundary) else {
        return;
    };
    let lower = section.content.to_lowercase();
    if !contains_any(
        &lower,
        &[
            "one pr",
            "one pull request",
            "single pr",
            "single pull request",
        ],
    ) {
        add_finding(
            findings,
            Finding::new(
                FindingCode::MissingOnePrScope,
                Some(SectionRole::OnePrBoundary),
                Some(section.line),
                "one-PR boundary must state the single-PR scope",
                None,
            ),
        );
    }
    if !contains_any(
        &lower,
        &[
            "do not",
            "exclude",
            "out of scope",
            "not include",
            "without",
        ],
    ) {
        add_finding(
            findings,
            Finding::new(
                FindingCode::MissingOnePrExclusions,
                Some(SectionRole::OnePrBoundary),
                Some(section.line),
                "one-PR boundary must state explicit exclusions",
                None,
            ),
        );
    }
    if contains_any(
        &lower,
        &["multiple pr", "several pr", "separate prs", "split across"],
    ) {
        add_finding(
            findings,
            Finding::new(
                FindingCode::MultiplePrScope,
                Some(SectionRole::OnePrBoundary),
                Some(section.line),
                "one-PR boundary describes multiple pull requests",
                None,
            ),
        );
    }
}

pub(super) fn check_capabilities(
    issue: &IssueSnapshot,
    options: AuditOptions,
    sections: &SectionExtraction,
    capabilities: &[CapabilityMetadata],
    policy: &IssueSpecPolicy,
    findings: &mut Vec<Finding>,
) {
    let required = options.capability_owner
        || policy.references.capability_owners.contains(&issue.number)
        || issue.labels.iter().any(|label| {
            let label = label.to_lowercase();
            label == "capability" || label == "darktable capability"
        });
    if required && sections.get(SectionRole::Capabilities).is_none() {
        add_finding(
            findings,
            Finding::new(
                FindingCode::MissingCapabilityMetadata,
                Some(SectionRole::Capabilities),
                None,
                "issue owns a capability but has no Capabilities metadata section",
                None,
            ),
        );
    }
    if let Some(section) = sections.get(SectionRole::Capabilities) {
        if capabilities.is_empty() && !section.content.eq_ignore_ascii_case("none") {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::MalformedCapability,
                    Some(SectionRole::Capabilities),
                    Some(section.line),
                    "Capabilities metadata has no parseable capability name",
                    Some(section.content.clone()),
                ),
            );
        }
        if capabilities.len() > 1
            && capabilities
                .iter()
                .all(|capability| capability.role == CapabilityRole::Unknown)
        {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::MultipleCapabilityOwners,
                    Some(SectionRole::Capabilities),
                    Some(section.line),
                    "multiple capability records require explicit roles",
                    None,
                ),
            );
        }
        for capability in capabilities {
            let Some(owner) = capability.owner_issue else {
                continue;
            };
            if !policy.references.known_issues.is_empty()
                && !policy.references.known_issues.contains(&owner)
            {
                add_finding(
                    findings,
                    Finding::new(
                        FindingCode::UnknownCapabilityOwner,
                        Some(SectionRole::Capabilities),
                        Some(capability.line),
                        format!(
                            "capability owner #{owner} is absent from the reviewed issue index"
                        ),
                        None,
                    ),
                );
            }
            if let Some(replacement) = policy.references.superseded.get(&owner) {
                add_finding(
                    findings,
                    Finding::new(
                        FindingCode::SupersededCapabilityOwner,
                        Some(SectionRole::Capabilities),
                        Some(capability.line),
                        format!("capability owner #{owner} is superseded by #{replacement}"),
                        None,
                    ),
                );
            }
        }
    }
}

pub(super) fn classify_issue(issue: &IssueSnapshot, sections: &SectionExtraction) -> IssueClass {
    let identity = format!(
        "{}\n{}\n{}",
        issue.title,
        issue.milestone.as_deref().unwrap_or_default(),
        issue.labels.join(" ")
    )
    .to_lowercase();
    if contains_any(
        &identity,
        &["qualification", "benchmark corpus", "hypothesis"],
    ) || sections.get(SectionRole::QualificationDecision).is_some()
    {
        return IssueClass::Qualification;
    }
    if contains_any(
        &identity,
        &[
            "corrective",
            "regression",
            "repair ",
            "defect",
            "superseded",
        ],
    ) || sections.get(SectionRole::ObservedDefects).is_some()
    {
        return IssueClass::Corrective;
    }
    if contains_any(
        &identity,
        &["release", "package", "installer", "distribution"],
    ) {
        return IssueClass::ReleasePackage;
    }
    if contains_any(&identity, &["lua", "wasm", "extension", "scripting"]) {
        return IssueClass::ScriptingExtension;
    }
    if sections.get(SectionRole::Capabilities).is_some()
        || contains_any(
            &identity,
            &["iop.", "image operation", "pixelpipe", "capability"],
        )
    {
        return IssueClass::ImageOperation;
    }
    if contains_any(
        &identity,
        &["jpeg", "png", "tiff", "codec", "decoder", "format."],
    ) {
        return IssueClass::Codec;
    }
    if contains_any(&identity, &["export", "storage", "destination"]) {
        return IssueClass::ExportDestination;
    }
    if contains_any(
        &identity,
        &[
            "windows",
            "macos",
            "linux",
            "platform adapter",
            "cross-platform",
        ],
    ) {
        return IssueClass::PlatformAdapter;
    }
    if contains_any(
        &identity,
        &["iced", "ui", "user interface", "view", "widget"],
    ) {
        return IssueClass::Ui;
    }
    if contains_any(&identity, &["foundation", "workflow", "governance"]) {
        return IssueClass::Foundation;
    }
    IssueClass::DomainService
}

pub(super) fn check_class_contract(
    _issue: &IssueSnapshot,
    class: IssueClass,
    sections: &SectionExtraction,
    findings: &mut Vec<Finding>,
) {
    let decisions = sections
        .get(SectionRole::FixedDecisions)
        .map(|section| section.content.to_lowercase())
        .unwrap_or_default();
    let tests = sections
        .get(SectionRole::TestMatrix)
        .map(|section| section.content.to_lowercase())
        .unwrap_or_default();
    let evidence = sections
        .get(SectionRole::AcceptanceEvidence)
        .map(|section| section.content.to_lowercase())
        .unwrap_or_default();
    let required = match class {
        IssueClass::Foundation => ["command", "repository", "policy"],
        IssueClass::ImageOperation => ["algorithm", "default", "parity"],
        IssueClass::Codec => ["format", "signature", "unsupported"],
        IssueClass::ExportDestination => ["destination", "atomic", "collision"],
        IssueClass::PlatformAdapter => ["platform", "fallback", "unsupported"],
        IssueClass::Ui => ["accessibility", "keyboard", "state"],
        IssueClass::ScriptingExtension => ["capability", "permission", "version"],
        IssueClass::ReleasePackage => ["artifact", "install", "rollback"],
        IssueClass::DomainService | IssueClass::Corrective | IssueClass::Qualification => {
            ["", "", ""]
        }
    };
    for term in required.into_iter().filter(|term| !term.is_empty()) {
        if !decisions.contains(term) && !tests.contains(term) && !evidence.contains(term) {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::MissingClassDecision,
                    Some(SectionRole::FixedDecisions),
                    sections
                        .get(SectionRole::FixedDecisions)
                        .map(|section| section.line),
                    format!(
                        "{} issue must name class-specific contract term {term}",
                        class.as_str()
                    ),
                    Some(term.to_owned()),
                ),
            );
        }
    }
    if class == IssueClass::Qualification {
        let Some(section) = sections.get(SectionRole::QualificationDecision) else {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::MissingQualificationContract,
                    None,
                    None,
                    "qualification issues require a Qualification decision section",
                    None,
                ),
            );
            return;
        };
        let lower = section.content.to_lowercase();
        if !contains_any(&lower, &["hypothesis", "candidate"])
            || !contains_any(&lower, &["benchmark", "corpus"])
            || !contains_any(&lower, &["decision rule", "threshold", "rule"])
            || !contains_any(&lower, &["owner", "follow-up", "follow up"])
        {
            add_finding(
                findings,
                Finding::new(
                    FindingCode::MissingQualificationContract,
                    Some(SectionRole::QualificationDecision),
                    Some(section.line),
                    "qualification decision must name hypotheses/candidates, benchmark/corpus, decision rule, bounded output, and follow-up owner",
                    None,
                ),
            );
        }
    }
    if class == IssueClass::Corrective && sections.get(SectionRole::ObservedDefects).is_none() {
        add_finding(
            findings,
            Finding::new(
                FindingCode::MissingObservedDefects,
                None,
                None,
                "corrective issues require Observed defects evidence and predecessor issue/PR",
                None,
            ),
        );
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "status precedence is the versioned semantic contract"
)]
pub(super) fn status_for(
    sections: &SectionExtraction,
    findings: &[Finding],
) -> SpecificationStatus {
    for (role, status) in [
        (
            SectionRole::FixedDecisions,
            SpecificationStatus::NeedsDecisions,
        ),
        (
            SectionRole::FailureAndEdgeBehavior,
            SpecificationStatus::NeedsFailureContract,
        ),
        (SectionRole::TestMatrix, SpecificationStatus::NeedsTests),
        (
            SectionRole::AcceptanceEvidence,
            SpecificationStatus::NeedsAcceptanceEvidence,
        ),
        (
            SectionRole::Dependencies,
            SpecificationStatus::NeedsDependencies,
        ),
        (
            SectionRole::OnePrBoundary,
            SpecificationStatus::NeedsOnePrBoundary,
        ),
    ] {
        if findings.iter().any(|finding| {
            finding.code == FindingCode::MissingRequiredSection && finding.role == Some(role)
        }) {
            return status;
        }
    }
    if findings.iter().any(|finding| {
        matches!(
            finding.code,
            FindingCode::CopiedGenericBody
                | FindingCode::SupersededDependency
                | FindingCode::SupersededCapabilityOwner
        )
    }) {
        return SpecificationStatus::DuplicateOrSuperseded;
    }
    if findings.iter().any(|finding| {
        matches!(
            finding.code,
            FindingCode::DuplicateSection
                | FindingCode::LiteralEscapedNewline
                | FindingCode::InvalidDependencyReference
                | FindingCode::UnknownDependency
                | FindingCode::DependencyCycle
                | FindingCode::MalformedCapability
                | FindingCode::MultipleCapabilityOwners
                | FindingCode::UnknownCapabilityOwner
                | FindingCode::MissingRequiredSection
                | FindingCode::EmptySection
                | FindingCode::BoilerplateSection
        )
    }) {
        return SpecificationStatus::Invalid;
    }
    for (code, status) in [
        (
            FindingCode::UnresolvedDecision,
            SpecificationStatus::NeedsDecisions,
        ),
        (
            FindingCode::UnnamedTechnicalDecision,
            SpecificationStatus::NeedsDecisions,
        ),
        (
            FindingCode::MissingFailureBehavior,
            SpecificationStatus::NeedsFailureContract,
        ),
        (
            FindingCode::MissingCancellationBehavior,
            SpecificationStatus::NeedsFailureContract,
        ),
        (
            FindingCode::MissingRestartBehavior,
            SpecificationStatus::NeedsFailureContract,
        ),
        (
            FindingCode::MissingPlatformBehavior,
            SpecificationStatus::NeedsFailureContract,
        ),
        (
            FindingCode::MissingTestCase,
            SpecificationStatus::NeedsTests,
        ),
        (
            FindingCode::VagueTestMatrix,
            SpecificationStatus::NeedsTests,
        ),
        (
            FindingCode::VagueAcceptanceEvidence,
            SpecificationStatus::NeedsAcceptanceEvidence,
        ),
        (
            FindingCode::MissingOnePrScope,
            SpecificationStatus::NeedsOnePrBoundary,
        ),
        (
            FindingCode::MissingOnePrExclusions,
            SpecificationStatus::NeedsOnePrBoundary,
        ),
        (
            FindingCode::MultiplePrScope,
            SpecificationStatus::NeedsOnePrBoundary,
        ),
        (
            FindingCode::MissingCapabilityMetadata,
            SpecificationStatus::NeedsDecisions,
        ),
        (
            FindingCode::MissingClassDecision,
            SpecificationStatus::NeedsDecisions,
        ),
        (
            FindingCode::MissingQualificationContract,
            SpecificationStatus::NeedsDecisions,
        ),
        (
            FindingCode::MissingObservedDefects,
            SpecificationStatus::NeedsAcceptanceEvidence,
        ),
    ] {
        if findings.iter().any(|finding| finding.code == code) {
            return status;
        }
    }
    if sections.legacy_equivalent {
        SpecificationStatus::ReadyLegacyEquivalent
    } else {
        SpecificationStatus::ReadyV2
    }
}

pub(super) fn remediation_for(
    issue: u64,
    status: SpecificationStatus,
    findings: &[Finding],
) -> Remediation {
    let mut steps = Vec::new();
    for finding in findings {
        let (kind, instruction) = match finding.code {
            FindingCode::MissingRequiredSection | FindingCode::EmptySection => (
                RemediationKind::AddSection,
                format!("add complete content for {}", finding.role.map_or("the required role", SectionRole::as_str)),
            ),
            FindingCode::BoilerplateSection | FindingCode::CopiedGenericBody => (
                RemediationKind::ReplaceBoilerplate,
                "replace generic text with issue-specific decisions, boundaries, and evidence".to_owned(),
            ),
            FindingCode::UnresolvedDecision | FindingCode::UnnamedTechnicalDecision => (
                RemediationKind::ResolveDecision,
                "name the selected algorithm, protocol, format, framework, or default and its rationale".to_owned(),
            ),
            FindingCode::MissingClassDecision | FindingCode::MissingQualificationContract => (
                RemediationKind::ResolveDecision,
                "add the class-specific decision contract and bounded evidence required by the issue type".to_owned(),
            ),
            FindingCode::MissingObservedDefects => (
                RemediationKind::AddAcceptanceEvidence,
                "record observed defect evidence and the predecessor issue or pull request".to_owned(),
            ),
            FindingCode::MissingFailureBehavior
            | FindingCode::MissingCancellationBehavior
            | FindingCode::MissingRestartBehavior
            | FindingCode::MissingPlatformBehavior => (
                RemediationKind::SpecifyFailureContract,
                "define observable failure, cancellation, restart, and platform behavior where applicable".to_owned(),
            ),
            FindingCode::MissingTestCase | FindingCode::VagueTestMatrix => (
                RemediationKind::AddTestMatrix,
                "add positive, negative, boundary, and applicable lifecycle/platform/parity cases".to_owned(),
            ),
            FindingCode::VagueAcceptanceEvidence => (
                RemediationKind::AddAcceptanceEvidence,
                "name executable commands and measurable receipts or artifacts".to_owned(),
            ),
            FindingCode::InvalidDependencyReference
            | FindingCode::UnknownDependency
            | FindingCode::SupersededDependency
            | FindingCode::DependencyCycle => (
                RemediationKind::RepairDependencies,
                "replace dependencies with current GitHub issue references or None and remove cycles".to_owned(),
            ),
            FindingCode::MissingOnePrScope
            | FindingCode::MissingOnePrExclusions
            | FindingCode::MultiplePrScope => (
                RemediationKind::NarrowPrBoundary,
                "state one coherent PR scope and explicit exclusions".to_owned(),
            ),
            FindingCode::MissingCapabilityMetadata
            | FindingCode::MalformedCapability
            | FindingCode::MultipleCapabilityOwners
            | FindingCode::UnknownCapabilityOwner
            | FindingCode::SupersededCapabilityOwner => (
                RemediationKind::RepairCapabilities,
                "declare capability names, roles, and current owner issue references".to_owned(),
            ),
            FindingCode::DuplicateSection | FindingCode::LiteralEscapedNewline => (
                RemediationKind::ReReviewSnapshot,
                "repair Markdown structure, then regenerate and review the snapshot hash".to_owned(),
            ),
        };
        steps.push(RemediationStep {
            kind,
            finding: finding.code.clone(),
            instruction,
        });
    }
    steps.sort_by_key(|step| (step.kind, step.finding.clone()));
    steps.dedup_by(|left, right| left.kind == right.kind && left.finding == right.finding);
    Remediation {
        issue,
        status,
        steps,
    }
}

pub(super) fn refresh_audit_result(audit: &mut IssueAudit) {
    audit.findings.sort_by_key(|finding| {
        (
            finding.code.clone(),
            finding.role,
            finding.line,
            finding.message.clone(),
        )
    });
    audit.status = status_for(&audit.sections, &audit.findings);
    audit.remediation = remediation_for(audit.issue, audit.status, &audit.findings);
}

pub(super) fn add_finding(findings: &mut Vec<Finding>, finding: Finding) {
    findings.push(finding);
    findings.sort_by_key(|finding| {
        (
            finding.code.clone(),
            finding.role,
            finding.line,
            finding.message.clone(),
        )
    });
    findings.dedup_by(|left, right| {
        left.code == right.code && left.role == right.role && left.line == right.line
    });
}
