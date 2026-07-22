use std::collections::BTreeMap;

use rusttable_sqlite_native::{DarktableSchema, HistoryRows};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{Finding, FindingCode, Severity, SourceRowKey};

const REFERENCE_OPERATION_CAPABILITIES: &str =
    include_str!("../../../architecture/operation-capabilities.json");

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestOperation {
    current_version: u32,
    parameter_versions: Vec<u32>,
    default_order: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DarktableOperationManifest {
    operations: BTreeMap<String, ManifestOperation>,
}

#[derive(Debug, Deserialize)]
struct ManifestFile {
    entries: Vec<ManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    identity: String,
    compatibility_name: String,
    descriptor_version: u32,
    parameter_versions: Vec<u32>,
    order: Option<i64>,
}

impl DarktableOperationManifest {
    /// Loads the pinned #495 operation manifest shipped with `RustTable`.
    #[must_use]
    pub fn reference() -> Self {
        let Ok(file) = serde_json::from_str::<ManifestFile>(REFERENCE_OPERATION_CAPABILITIES)
        else {
            return Self::default();
        };
        let mut operations = BTreeMap::new();
        for entry in file.entries {
            if !entry.identity.starts_with("darktable:") {
                continue;
            }
            operations.insert(
                entry.compatibility_name,
                ManifestOperation {
                    current_version: entry.descriptor_version,
                    parameter_versions: entry.parameter_versions,
                    default_order: entry.order,
                },
            );
        }
        Self { operations }
    }

    /// Creates an empty manifest for tests or a caller-owned source manifest.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            operations: BTreeMap::new(),
        }
    }

    /// Adds one exact compatibility name and its supported parameter versions.
    pub fn insert(
        &mut self,
        name: impl Into<String>,
        current_version: u32,
        parameter_versions: impl IntoIterator<Item = u32>,
        default_order: Option<i64>,
    ) {
        self.operations.insert(
            name.into(),
            ManifestOperation {
                current_version,
                parameter_versions: parameter_versions.into_iter().collect(),
                default_order,
            },
        );
    }

    fn get(&self, name: &str) -> Option<&ManifestOperation> {
        self.operations.get(name)
    }
}

impl Default for HistoryDecodeOptions {
    fn default() -> Self {
        Self {
            limits: HistoryLimits::default(),
            manifest: DarktableOperationManifest::reference(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryLimits {
    pub max_rows: usize,
    pub max_payload_bytes: usize,
    pub max_name_bytes: usize,
    pub max_module_order_entries: usize,
    pub max_findings: usize,
}

impl Default for HistoryLimits {
    fn default() -> Self {
        Self {
            max_rows: 100_000,
            max_payload_bytes: 64 * 1024 * 1024,
            max_name_bytes: 16 * 1024,
            max_module_order_entries: 100_000,
            max_findings: 100_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryDecodeOptions {
    pub limits: HistoryLimits,
    pub manifest: DarktableOperationManifest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpaquePayload {
    pub present: bool,
    pub bytes: Vec<u8>,
    pub sha256: [u8; 32],
}

impl OpaquePayload {
    #[must_use]
    pub fn from_optional(value: Option<&[u8]>) -> Self {
        let bytes = value.unwrap_or_default().to_vec();
        Self {
            present: value.is_some(),
            sha256: Sha256::digest(&bytes).into(),
            bytes,
        }
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationCompatibility {
    Known {
        current_version: u32,
        parameter_versions: Vec<u32>,
    },
    Unknown,
    InvalidName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationIdentity {
    pub raw_name: Vec<u8>,
    pub name: Option<String>,
    pub compatibility: OperationCompatibility,
}

impl OperationIdentity {
    fn decode(raw_name: Vec<u8>, manifest: &DarktableOperationManifest) -> Self {
        let Ok(name) = String::from_utf8(raw_name.clone()) else {
            return Self {
                raw_name,
                name: None,
                compatibility: OperationCompatibility::InvalidName,
            };
        };
        let compatibility = manifest.get(&name).map_or_else(
            || OperationCompatibility::Unknown,
            |entry| OperationCompatibility::Known {
                current_version: entry.current_version,
                parameter_versions: entry.parameter_versions.clone(),
            },
        );
        Self {
            raw_name,
            name: Some(name),
            compatibility,
        }
    }

    #[must_use]
    pub const fn is_known(&self) -> bool {
        matches!(self.compatibility, OperationCompatibility::Known { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnabledState {
    Enabled,
    Disabled,
    Missing,
    Invalid(i64),
}

impl EnabledState {
    fn decode(value: Option<i64>) -> Self {
        match value {
            Some(0) => Self::Disabled,
            Some(1) => Self::Enabled,
            Some(value) => Self::Invalid(value),
            None => Self::Missing,
        }
    }

    #[must_use]
    pub const fn selected(self) -> bool {
        matches!(self, Self::Enabled | Self::Disabled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleInstanceId([u8; 32]);

impl ModuleInstanceId {
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatHistoryStep {
    pub source: SourceRowKey,
    pub image_id: i64,
    pub num: i64,
    pub module: Option<i64>,
    pub operation: OperationIdentity,
    pub operation_params: OpaquePayload,
    pub enabled: EnabledState,
    pub selected: bool,
    pub blend_params: OpaquePayload,
    pub blend_version: Option<i64>,
    pub multi_priority: Option<i64>,
    pub multi_name: OpaquePayload,
    pub multi_name_hand_edited: Option<i64>,
    pub instance_id: ModuleInstanceId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatModuleInstance {
    pub id: ModuleInstanceId,
    pub operation: OperationIdentity,
    pub multi_priority: Option<i64>,
    pub multi_name: OpaquePayload,
    pub multi_name_display: Option<String>,
    pub multi_name_hand_edited: Option<i64>,
    pub first_source: SourceRowKey,
    pub history_sources: Vec<SourceRowKey>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ModuleOrderVersion {
    Custom,
    Legacy,
    V30,
    V30Jpeg,
    V50,
    V50Jpeg,
    Unknown(i64),
}

impl ModuleOrderVersion {
    fn decode(value: Option<i64>) -> Option<Self> {
        value.map(|value| match value {
            0 => Self::Custom,
            1 => Self::Legacy,
            2 => Self::V30,
            3 => Self::V30Jpeg,
            4 => Self::V50,
            5 => Self::V50Jpeg,
            value => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleOrderEntry {
    pub ordinal: usize,
    pub operation: Vec<u8>,
    pub instance: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleOrderRule {
    pub before: Vec<u8>,
    pub after: Vec<u8>,
}

/// Darktable's persisted order constraints from `src/common/iop_order.c`.
pub const DARKTABLE_ORDER_RULES: &[(&str, &str)] = &[
    ("rawprepare", "invert"),
    ("invert", "temperature"),
    ("temperature", "highlights"),
    ("highlights", "cacorrect"),
    ("cacorrect", "hotpixels"),
    ("hotpixels", "rawdenoise"),
    ("rawdenoise", "demosaic"),
    ("demosaic", "colorin"),
    ("colorin", "colorout"),
    ("colorout", "gamma"),
    ("flip", "crop"),
    ("flip", "clipping"),
    ("ashift", "clipping"),
    ("colorin", "channelmixerrgb"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatModuleOrder {
    pub source: SourceRowKey,
    pub version: ModuleOrderVersion,
    pub raw_list: OpaquePayload,
    pub entries: Vec<ModuleOrderEntry>,
    pub rules: Vec<ModuleOrderRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryOrderSource {
    CustomModuleOrder,
    BuiltInModuleOrder,
    HistoryNumbers,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySelection {
    pub history_end: Option<i64>,
    pub selected_rows: Vec<SourceRowKey>,
    pub redo_rows: Vec<SourceRowKey>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatHistoryHash {
    pub source: SourceRowKey,
    pub basic: OpaquePayload,
    pub auto: OpaquePayload,
    pub current: OpaquePayload,
    pub mipmap: OpaquePayload,
    pub current_matches: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatHistory {
    pub schema: DarktableSchema,
    pub image_id: i64,
    pub steps: Vec<CompatHistoryStep>,
    pub instances: Vec<CompatModuleInstance>,
    pub selection: HistorySelection,
    pub module_order: Option<CompatModuleOrder>,
    pub history_hash: Option<CompatHistoryHash>,
    pub operation_order: Vec<ModuleInstanceId>,
    pub order_source: Option<HistoryOrderSource>,
    pub order_proven: bool,
    pub executable: bool,
    pub findings: Vec<Finding>,
}

pub struct HistoryDecoder {
    options: HistoryDecodeOptions,
}

fn finding(
    findings: &mut Vec<Finding>,
    code: FindingCode,
    severity: Severity,
    row: u64,
    detail: impl Into<String>,
) {
    findings.push(Finding {
        code,
        severity,
        source: Some(SourceRowKey::new("main.history", row)),
        detail: detail.into(),
    });
}

#[path = "history/decoder.rs"]
mod decoder;
#[path = "history/order.rs"]
mod order;
