use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSnapshot {
    pub revision: u64,
    pub items: Vec<CatalogItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetadataValue {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditProposal {
    pub item_id: String,
    pub values: Vec<MetadataValue>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportProposal {
    pub item_ids: Vec<String>,
    pub destination: String,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDto {
    pub kind: String,
    pub sequence: u64,
    pub payload: String,
}

impl CatalogSnapshot {
    #[must_use]
    pub const fn bounded(&self, max_items: usize) -> bool {
        self.items.len() <= max_items
    }
}

impl EditProposal {
    #[must_use]
    pub const fn bounded(&self, max_values: usize) -> bool {
        self.values.len() <= max_values
    }
}
