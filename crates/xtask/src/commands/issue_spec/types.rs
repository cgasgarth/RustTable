use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

pub const SCHEMA: &str = "rusttable.issue-spec.v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SpecificationStatus {
    ReadyV2,
    ReadyLegacyEquivalent,
    NeedsDecisions,
    NeedsFailureContract,
    NeedsTests,
    NeedsAcceptanceEvidence,
    NeedsDependencies,
    NeedsOnePrBoundary,
    DuplicateOrSuperseded,
    Invalid,
}

impl SpecificationStatus {
    pub const fn is_ready(self) -> bool {
        matches!(self, Self::ReadyV2 | Self::ReadyLegacyEquivalent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SectionRole {
    Outcome,
    FixedDecisions,
    ImplementationScope,
    FailureAndEdgeBehavior,
    TestMatrix,
    AcceptanceEvidence,
    Dependencies,
    OnePrBoundary,
    Capabilities,
    QualificationDecision,
    ObservedDefects,
    PostCompletionAudit,
}

impl SectionRole {
    pub const REQUIRED: [Self; 8] = [
        Self::Outcome,
        Self::FixedDecisions,
        Self::ImplementationScope,
        Self::FailureAndEdgeBehavior,
        Self::TestMatrix,
        Self::AcceptanceEvidence,
        Self::Dependencies,
        Self::OnePrBoundary,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Outcome => "Outcome",
            Self::FixedDecisions => "Fixed decisions",
            Self::ImplementationScope => "Implementation scope",
            Self::FailureAndEdgeBehavior => "Failure and edge behavior",
            Self::TestMatrix => "Test matrix",
            Self::AcceptanceEvidence => "Acceptance evidence",
            Self::Dependencies => "Dependencies",
            Self::OnePrBoundary => "One-PR boundary",
            Self::Capabilities => "Capabilities",
            Self::QualificationDecision => "Qualification decision",
            Self::ObservedDefects => "Observed defects",
            Self::PostCompletionAudit => "Post-completion audit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SectionSource {
    V2,
    LegacyEquivalent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalSection {
    pub role: SectionRole,
    pub heading: String,
    pub content: String,
    pub line: usize,
    pub source: SectionSource,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DuplicateHeading {
    pub role: SectionRole,
    pub first_line: usize,
    pub duplicate_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionExtraction {
    pub sections: BTreeMap<SectionRole, CanonicalSection>,
    pub duplicates: Vec<DuplicateHeading>,
    pub legacy_equivalent: bool,
    pub literal_escaped_newline: bool,
}

impl SectionExtraction {
    pub fn get(&self, role: SectionRole) -> Option<&CanonicalSection> {
        self.sections.get(&role)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueSnapshot {
    pub number: u64,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    #[serde(default = "default_issue_state")]
    pub state: String,
    #[serde(default)]
    pub state_reason: Option<String>,
    #[serde(default)]
    pub is_pull_request: bool,
}

fn default_issue_state() -> String {
    "open".to_owned()
}

impl IssueSnapshot {
    #[allow(dead_code)]
    pub fn new(number: u64, title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            number,
            title: title.into(),
            body: body.into(),
            labels: Vec::new(),
            milestone: None,
            state: default_issue_state(),
            state_reason: None,
            is_pull_request: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum FindingCode {
    MissingRequiredSection,
    EmptySection,
    BoilerplateSection,
    DuplicateSection,
    LiteralEscapedNewline,
    UnresolvedDecision,
    UnnamedTechnicalDecision,
    MissingFailureBehavior,
    MissingCancellationBehavior,
    MissingRestartBehavior,
    MissingPlatformBehavior,
    MissingTestCase,
    VagueTestMatrix,
    VagueAcceptanceEvidence,
    InvalidDependencyReference,
    UnknownDependency,
    SupersededDependency,
    DependencyCycle,
    MissingOnePrScope,
    MissingOnePrExclusions,
    MultiplePrScope,
    MissingCapabilityMetadata,
    MalformedCapability,
    MultipleCapabilityOwners,
    UnknownCapabilityOwner,
    SupersededCapabilityOwner,
    CopiedGenericBody,
    MissingClassDecision,
    MissingQualificationContract,
    MissingObservedDefects,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum FindingSeverity {
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub code: FindingCode,
    pub severity: FindingSeverity,
    pub role: Option<SectionRole>,
    pub line: Option<usize>,
    pub message: String,
    pub evidence: Option<String>,
}

impl Finding {
    pub(crate) fn new(
        code: FindingCode,
        role: Option<SectionRole>,
        line: Option<usize>,
        message: impl Into<String>,
        evidence: Option<String>,
    ) -> Self {
        Self {
            code,
            severity: FindingSeverity::Error,
            role,
            line,
            message: message.into(),
            evidence,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum RemediationKind {
    AddSection,
    ReplaceBoilerplate,
    ResolveDecision,
    SpecifyFailureContract,
    AddTestMatrix,
    AddAcceptanceEvidence,
    RepairDependencies,
    NarrowPrBoundary,
    RepairCapabilities,
    ReReviewSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemediationStep {
    pub kind: RemediationKind,
    pub finding: FindingCode,
    pub instruction: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remediation {
    pub issue: u64,
    pub status: SpecificationStatus,
    pub steps: Vec<RemediationStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencySpec {
    pub issue: u64,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyParse {
    pub references: Vec<DependencySpec>,
    pub explicit_none: bool,
    pub invalid_numeric_tokens: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum CapabilityRole {
    Owner,
    Adapter,
    Consumer,
    Compatibility,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityMetadata {
    pub name: String,
    pub role: CapabilityRole,
    pub owner_issue: Option<u64>,
    pub line: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum IssueClass {
    Foundation,
    DomainService,
    ImageOperation,
    Codec,
    ExportDestination,
    PlatformAdapter,
    Ui,
    ScriptingExtension,
    Corrective,
    Qualification,
    ReleasePackage,
}

impl IssueClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Foundation => "foundation",
            Self::DomainService => "domain/service",
            Self::ImageOperation => "image operation",
            Self::Codec => "codec",
            Self::ExportDestination => "export destination",
            Self::PlatformAdapter => "platform adapter",
            Self::Ui => "UI",
            Self::ScriptingExtension => "scripting/extension",
            Self::Corrective => "corrective",
            Self::Qualification => "qualification",
            Self::ReleasePackage => "release/package",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueAudit {
    pub schema: &'static str,
    pub issue: u64,
    pub title: String,
    pub class: IssueClass,
    pub status: SpecificationStatus,
    pub body_hash: String,
    pub spec_hash: String,
    pub sections: SectionExtraction,
    pub dependencies: DependencyParse,
    pub capabilities: Vec<CapabilityMetadata>,
    pub findings: Vec<Finding>,
    pub remediation: Remediation,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReferenceIndex {
    pub known_issues: BTreeSet<u64>,
    pub superseded: BTreeMap<u64, u64>,
    pub dependency_graph: BTreeMap<u64, BTreeSet<u64>>,
    pub capability_owners: BTreeSet<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AuditOptions {
    pub capability_owner: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IssueSpecPolicy {
    pub references: ReferenceIndex,
}

impl fmt::Display for SpecificationStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ReadyV2 => "ReadyV2",
            Self::ReadyLegacyEquivalent => "ReadyLegacyEquivalent",
            Self::NeedsDecisions => "NeedsDecisions",
            Self::NeedsFailureContract => "NeedsFailureContract",
            Self::NeedsTests => "NeedsTests",
            Self::NeedsAcceptanceEvidence => "NeedsAcceptanceEvidence",
            Self::NeedsDependencies => "NeedsDependencies",
            Self::NeedsOnePrBoundary => "NeedsOnePrBoundary",
            Self::DuplicateOrSuperseded => "DuplicateOrSuperseded",
            Self::Invalid => "Invalid",
        })
    }
}
