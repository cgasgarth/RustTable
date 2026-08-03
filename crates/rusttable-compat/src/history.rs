use std::collections::BTreeMap;

use rusttable_sqlite_native::{DarktableSchema, HistoryRows};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{Finding, FindingCode, Severity, SourceRowKey};

const REFERENCE_OPERATION_CAPABILITIES: &str =
    include_str!("../../../architecture/operation-capabilities.json");

/// Darktable's current persisted blend payload version.
pub const DARKTABLE_BLEND_VERSION: i64 = 14;
/// Size of `dt_develop_blend_params_t` at blend payload version 14.
pub const DARKTABLE_BLEND_PARAMETER_BYTES: usize = 420;

const DEVELOP_BLEND_CS_RGB_SCENE: i32 = 4;
#[expect(
    clippy::excessive_precision,
    reason = "Native Darktable blend defaults retain this exact decimal before f32 serialization."
)]
const SCENE_RGB_JZ_CZ_BOOST: f32 = -6.643_856_19_f32;

/// Returns Darktable's exact v14 identity blend payload in native field order.
///
/// The native initializer starts with `DEVELOP_BLEND_CS_NONE`, then
/// `dt_iop_commit_blend_params` normalizes that field to the module's default
/// colorspace before the payload is persisted. Callers must provide that
/// operation-specific normalized value; every other field remains native.
#[must_use]
pub fn identity_blend_v14_bytes(blend_cst: i32) -> Vec<u8> {
    let mut bytes = vec![0_u8; DARKTABLE_BLEND_PARAMETER_BYTES];
    let mut offset = 0;
    put_u32(&mut bytes, &mut offset, 0); // DEVELOP_MASK_DISABLED
    put_i32(&mut bytes, &mut offset, blend_cst);
    put_u32(&mut bytes, &mut offset, 0x18); // DEVELOP_BLEND_NORMAL2
    put_f32(&mut bytes, &mut offset, 0.0);
    put_f32(&mut bytes, &mut offset, 100.0);
    put_u32(&mut bytes, &mut offset, 0); // DEVELOP_COMBINE_NORM_EXCL
    put_i32(&mut bytes, &mut offset, 0); // mask_id
    put_u32(&mut bytes, &mut offset, 0); // blendif
    put_f32(&mut bytes, &mut offset, 0.0);
    put_u32(&mut bytes, &mut offset, 5); // DEVELOP_MASK_GUIDE_IN_AFTER_BLUR
    for _ in 0..4 {
        put_f32(&mut bytes, &mut offset, 0.0);
    }
    put_u32(&mut bytes, &mut offset, 1); // feather_version
    for _ in 0..2 {
        put_u32(&mut bytes, &mut offset, 0);
    }
    for _ in 0..16 {
        put_f32(&mut bytes, &mut offset, 0.0);
        put_f32(&mut bytes, &mut offset, 0.0);
        put_f32(&mut bytes, &mut offset, 1.0);
        put_f32(&mut bytes, &mut offset, 1.0);
    }
    for index in 0..16 {
        let boost = if blend_cst == DEVELOP_BLEND_CS_RGB_SCENE && matches!(index, 8 | 9 | 12 | 13) {
            SCENE_RGB_JZ_CZ_BOOST
        } else {
            0.0
        };
        put_f32(&mut bytes, &mut offset, boost);
    }
    offset += 20; // raster_mask_source[20]
    put_i32(&mut bytes, &mut offset, 0); // raster_mask_instance
    put_i32(&mut bytes, &mut offset, -1); // INVALID_MASKID
    put_i32(&mut bytes, &mut offset, 0); // gboolean FALSE
    debug_assert_eq!(offset, DARKTABLE_BLEND_PARAMETER_BYTES);
    bytes
}

/// Accepts only a present, byte-for-byte current identity blend payload.
#[must_use]
pub fn is_identity_blend_v14(
    payload: &OpaquePayload,
    version: Option<i64>,
    blend_cst: i32,
) -> bool {
    version == Some(DARKTABLE_BLEND_VERSION)
        && payload.present
        && payload.bytes == identity_blend_v14_bytes(blend_cst)
}

fn put_f32(bytes: &mut [u8], offset: &mut usize, value: f32) {
    bytes[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
    *offset += 4;
}

fn put_u32(bytes: &mut [u8], offset: &mut usize, value: u32) {
    bytes[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
    *offset += 4;
}

fn put_i32(bytes: &mut [u8], offset: &mut usize, value: i32) {
    bytes[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
    *offset += 4;
}

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
    const fn decode(value: Option<i64>) -> Self {
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
    const fn decode(value: i64) -> Self {
        match value {
            0 => Self::Custom,
            1 => Self::Legacy,
            2 => Self::V30,
            3 => Self::V30Jpeg,
            4 => Self::V50,
            5 => Self::V50Jpeg,
            value => Self::Unknown(value),
        }
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
    /// Enabled rows at the latest selected history number for each operation/priority key.
    pub active_rows: Vec<SourceRowKey>,
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
