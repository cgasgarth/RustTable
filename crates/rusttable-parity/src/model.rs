use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema_version: u32,
    pub source_commit: String,
    pub capabilities: Vec<Capability>,
    #[serde(default)]
    pub summary: Vec<SummaryGroup>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Capability {
    pub id: String,
    pub reference_path: String,
    pub reference_symbol: String,
    pub category: String,
    pub status: String,
    pub ownership: Vec<IssueOwnership>,
    pub structural_evidence: Vec<String>,
    pub behavioral_evidence: Vec<String>,
    pub acceptance_test_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redesign_note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct IssueOwnership {
    pub issue_number: u64,
    pub role: String,
    pub milestone: String,
    pub priority: String,
}

fn default_override_category() -> String {
    "cross-cutting".to_owned()
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Override {
    pub id: String,
    pub reference_path: String,
    #[serde(default)]
    pub reference_symbol: Option<String>,
    #[serde(default = "default_override_category")]
    pub category: String,
    pub status: String,
    #[serde(default)]
    pub behavioral_evidence: Vec<String>,
    pub acceptance_test_id: String,
    #[serde(default)]
    pub redesign_note: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SummaryGroup {
    pub priority: String,
    pub milestone: String,
    pub capability_count: usize,
    pub required: usize,
    pub redesigned: usize,
    pub unsupported: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CapabilityReceipt {
    pub schema_version: u32,
    pub manifest_sha256: String,
    pub capability_count: usize,
    pub ownership_count: usize,
    pub issue_count: usize,
    pub unmapped_capabilities: Vec<String>,
    pub multiply_mapped_capabilities: Vec<String>,
    pub stale_issue_numbers: Vec<u64>,
    pub unsupported_capabilities: Vec<String>,
    pub evidence_incomplete_capabilities: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OverrideFile {
    #[serde(rename = "override")]
    #[serde(default)]
    pub override_entries: Vec<Override>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct IssueIndex {
    pub schema_version: u32,
    pub repository: String,
    pub parent_issue: u64,
    #[serde(default)]
    pub issue_body_contract_version: u32,
    #[serde(default)]
    pub source_snapshot: String,
    pub capability_ids: Vec<String>,
    pub issues: Vec<IssueRecord>,
    pub ownership: Vec<OwnershipRecord>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct IssueRecord {
    pub number: u64,
    pub title: String,
    pub state: String,
    #[serde(default)]
    pub state_reason: Option<String>,
    pub milestone: String,
    pub priority: String,
    pub parent_issue: u64,
    #[serde(default)]
    pub replacement_issue: Option<u64>,
    pub capability_ids: Vec<String>,
    #[serde(default)]
    pub superseded_capability_ids: Vec<String>,
    #[serde(default)]
    pub canonical_owner: bool,
    #[serde(default)]
    pub issue_body_contract_version: u32,
    #[serde(default)]
    pub body_completeness: String,
    #[serde(default)]
    pub operation_identity: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct OwnershipRecord {
    pub capability_id: String,
    pub issue_number: u64,
    pub role: String,
    pub category: String,
    pub status: String,
    pub structural_evidence: Vec<String>,
    pub behavioral_evidence: Vec<String>,
    pub acceptance_test_id: String,
    #[serde(default)]
    pub canonical_owner: bool,
    #[serde(default)]
    pub replacement_chain: Vec<u64>,
    #[serde(default)]
    pub redesign_note: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct Discovered {
    pub id: String,
    pub reference_path: String,
    pub reference_symbol: String,
    pub category: String,
    pub status: String,
    pub ownership: Vec<IssueOwnership>,
    pub structural_evidence: Vec<String>,
    pub behavioral_evidence: Vec<String>,
    pub acceptance_test_id: String,
    pub redesign_note: Option<String>,
}
