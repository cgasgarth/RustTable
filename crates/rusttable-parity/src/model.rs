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
    pub issue_numbers: Vec<u64>,
    pub test_evidence: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub redesign_note: Option<String>,
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
    pub issue_numbers: Vec<u64>,
    #[serde(default)]
    pub test_evidence: Vec<String>,
    #[serde(default)]
    pub redesign_note: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SummaryGroup {
    pub phase: String,
    pub milestone: String,
    pub capability_count: usize,
    pub required: usize,
    pub redesigned: usize,
    pub unsupported: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OverrideFile {
    #[serde(rename = "override")]
    #[serde(default)]
    pub override_entries: Vec<Override>,
}

#[derive(Debug, Clone)]
pub(crate) struct Discovered {
    pub id: String,
    pub reference_path: String,
    pub reference_symbol: String,
    pub category: &'static str,
    pub status: &'static str,
    pub issue_numbers: Vec<u64>,
    pub test_evidence: Vec<String>,
    pub redesign_note: Option<String>,
}
