use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Inventory {
    pub(super) schema_version: u32,
    pub(super) repository: String,
    pub(super) source_commit: String,
    pub(super) tree_id: String,
    pub(super) tree_sha256: String,
    pub(super) entries: Vec<InventoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct InventoryEntry {
    pub(super) path: String,
    pub(super) mode: String,
    pub(super) object_type: String,
    pub(super) object_id: String,
    pub(super) size_bytes: u64,
    pub(super) class: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct SourceMap {
    pub(super) schema: String,
    pub(super) repository: String,
    pub(super) parent_issue: u64,
    pub(super) source_commit: String,
    pub(super) source_tree_id: String,
    pub(super) inventory_path: String,
    pub(super) issue_snapshot_path: String,
    pub(super) inventory_sha256: String,
    pub(super) derived_input_debt: Vec<String>,
    pub(super) unowned_inventory_paths: Vec<String>,
    pub(super) issues: Vec<IssueRecord>,
    pub(super) responsibilities: Vec<Responsibility>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct IssueRecord {
    pub(super) number: u64,
    pub(super) title: String,
    pub(super) state: String,
    pub(super) priority: String,
    pub(super) milestone: Option<String>,
    pub(super) body_hash: String,
    pub(super) source_anchor_hash: String,
    pub(super) source_status: String,
    pub(super) class: String,
    pub(super) preserve: Vec<String>,
    pub(super) redesign: Vec<String>,
    pub(super) excluded: Vec<String>,
    pub(super) rust_targets: Vec<String>,
    pub(super) dependencies: Vec<u64>,
    pub(super) anchors: Vec<Anchor>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Anchor {
    pub(super) path: String,
    pub(super) blob_id: String,
    pub(super) selector: Selector,
    pub(super) role: String,
    pub(super) evidence: String,
    pub(super) rust_owners: Vec<String>,
    pub(super) oracle_fixtures: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub(super) enum Selector {
    Symbol { name: String },
    Table { name: String },
    ConfigKey { name: String },
    Kernel { name: String },
    Action { name: String },
    Registration { name: String },
    BoundedRange { label: String, start: u64, end: u64 },
    InventoryPath,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct Responsibility {
    pub(super) id: String,
    pub(super) issue: u64,
    pub(super) kind: String,
    pub(super) path: String,
    pub(super) blob_id: String,
    pub(super) selector: Selector,
    pub(super) role: String,
    pub(super) source_commit: String,
    pub(super) rust_owners: Vec<String>,
    pub(super) oracle_fixtures: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct Finding {
    pub(super) code: String,
    pub(super) issue: Option<u64>,
    pub(super) path: Option<String>,
    pub(super) message: String,
}

#[derive(Debug, Clone)]
pub(super) struct SourceTree {
    pub(super) reference: PathBuf,
    pub(super) commit: String,
}

#[derive(Debug, Clone)]
pub(super) struct IssueInput {
    pub(super) number: u64,
    pub(super) title: String,
    pub(super) state: String,
    pub(super) labels: Vec<String>,
    pub(super) milestone: Option<String>,
    pub(super) body: String,
    pub(super) source_anchor_hash: String,
    pub(super) is_pull_request: bool,
}

impl Finding {
    pub(super) fn error(
        code: impl Into<String>,
        issue: Option<u64>,
        path: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: format!("error-{}", code.into()),
            issue,
            path,
            message: message.into(),
        }
    }
}
