use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct OperationManifest {
    pub schema_version: u32,
    pub reference: ReferenceIdentity,
    pub history: HistoryCompatibility,
    pub operations: Vec<Operation>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReferenceIdentity {
    pub source_commit: String,
    pub build_version: String,
    pub executable_hash: String,
    pub data_bundle_hash: String,
    pub target_triple: String,
    pub c_abi_model: String,
    pub build_option_hash: String,
    #[serde(default)]
    pub canonical_identity: String,
    #[serde(default)]
    pub identity_hash: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub executable_sha256: String,
    #[serde(default)]
    pub data_dir_sha256: String,
    #[serde(default)]
    pub opencl_bundle_sha256: String,
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub architecture: String,
    #[serde(default)]
    pub build_options_hash: String,
    #[serde(default)]
    pub compiler: String,
    #[serde(default)]
    pub native_library_identity: String,
    #[serde(default)]
    pub cli_reference_hash: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Evidence {
    pub source_commit: String,
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub line_start: Option<u32>,
    #[serde(default)]
    pub line_end: Option<u32>,
    #[serde(default)]
    pub fixture_id: Option<String>,
    pub reason: String,
    pub reviewer: String,
    pub evidence_hash: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HistoryCompatibility {
    pub database_table: String,
    pub database_fields: Vec<String>,
    pub xmp_fields: Vec<String>,
    pub enabled_rule: String,
    pub instance_rule: String,
    pub blend_rule: String,
    pub ordering_rule: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Operation {
    pub name: String,
    pub reference_path: String,
    pub module_version: u32,
    pub parameter_size: usize,
    pub parameter_layout_hash: String,
    pub default_enabled: bool,
    pub default_order: usize,
    pub group: String,
    pub tags: Vec<String>,
    pub multi_instance: bool,
    pub supports_blend_masks: bool,
    pub input_color_space: String,
    pub output_color_space: String,
    pub roi_behavior: String,
    pub tiling_requirement: String,
    pub cpu_implementation: String,
    pub opencl_programs: Vec<String>,
    pub opencl_kernels: Vec<String>,
    #[serde(default)]
    pub parameter_versions: Vec<ParameterVersion>,
    #[serde(default)]
    pub migrations: Vec<ParameterMigration>,
    pub preset_sources: Vec<String>,
    pub owning_issue_number: u64,
    #[serde(default)]
    pub evidence: Vec<OperationEvidence>,
    #[serde(default = "default_tolerance_class")]
    pub tolerance_class: String,
    #[serde(default)]
    pub abi_layouts: Vec<AbiLayout>,
    #[serde(default)]
    pub codec: Option<ParameterCodec>,
    #[serde(default)]
    pub color_contract: ColorContract,
    #[serde(default)]
    pub capability_contract: CapabilityContract,
    #[serde(default)]
    pub roi_contract: RoiContract,
    #[serde(default)]
    pub tiling_contract: TilingContract,
    #[serde(default)]
    pub opencl_resolution: Vec<OpenclProgramResolution>,
    #[serde(default)]
    pub presets: Vec<PresetRecord>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct OperationEvidence {
    pub field: String,
    #[serde(flatten)]
    pub evidence: Evidence,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ParameterVersion {
    pub version: u32,
    pub byte_size: usize,
    pub layout_hash: String,
    pub decoder: String,
    pub opaque_blocking: bool,
    pub fixture_id: String,
    pub evidence: Evidence,
    #[serde(default)]
    pub abi_layouts: Vec<AbiLayout>,
    #[serde(default)]
    pub codec: Option<ParameterCodec>,
    #[serde(default)]
    pub target_codecs: Vec<TargetCodec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ParameterMigration {
    pub from_version: u32,
    pub to_version: u32,
    pub strategy: String,
    pub fixture_id: String,
    pub evidence: Evidence,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct OperationOverride {
    pub name: String,
    #[serde(default)]
    pub module_version: Option<u32>,
    #[serde(default)]
    pub parameter_size: Option<usize>,
    #[serde(default)]
    pub parameter_layout_hash: Option<String>,
    #[serde(default)]
    pub parameter_decoder: Option<String>,
    #[serde(default)]
    pub default_enabled: Option<bool>,
    #[serde(default)]
    pub default_order: Option<usize>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub multi_instance: Option<bool>,
    #[serde(default)]
    pub supports_blend_masks: Option<bool>,
    #[serde(default)]
    pub input_color_space: Option<String>,
    #[serde(default)]
    pub output_color_space: Option<String>,
    #[serde(default)]
    pub roi_behavior: Option<String>,
    #[serde(default)]
    pub tiling_requirement: Option<String>,
    #[serde(default)]
    pub cpu_implementation: Option<String>,
    #[serde(default)]
    pub opencl_programs: Option<Vec<String>>,
    #[serde(default)]
    pub opencl_kernels: Option<Vec<String>>,
    #[serde(default)]
    pub parameter_versions: Option<Vec<ParameterVersion>>,
    #[serde(default)]
    pub migrations: Option<Vec<ParameterMigration>>,
    #[serde(default)]
    pub preset_sources: Option<Vec<String>>,
    #[serde(default)]
    pub owning_issue_number: Option<u64>,
    #[serde(default)]
    pub evidence: Option<Vec<OperationEvidence>>,
    #[serde(default)]
    pub tolerance_class: Option<String>,
    #[serde(default)]
    pub abi_layouts: Option<Vec<AbiLayout>>,
    #[serde(default)]
    pub codec: Option<ParameterCodec>,
    #[serde(default)]
    pub color_contract: Option<ColorContract>,
    #[serde(default)]
    pub capability_contract: Option<CapabilityContract>,
    #[serde(default)]
    pub roi_contract: Option<RoiContract>,
    #[serde(default)]
    pub tiling_contract: Option<TilingContract>,
    #[serde(default)]
    pub opencl_resolution: Option<Vec<OpenclProgramResolution>>,
    #[serde(default)]
    pub presets: Option<Vec<PresetRecord>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AbiLayout {
    pub target: String,
    pub c_abi_model: String,
    pub endianness: String,
    pub pointer_width: u16,
    pub fields: Vec<FieldLayout>,
    #[serde(default)]
    pub padding: Vec<PaddingInterval>,
    pub total_size: usize,
    pub alignment: usize,
    pub layout_hash: String,
    #[serde(default)]
    pub difference_from: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct FieldLayout {
    pub name: String,
    pub type_name: String,
    #[serde(default)]
    pub enum_identity: Option<String>,
    #[serde(default)]
    pub enum_value: Option<i64>,
    #[serde(default)]
    pub array_extent: Option<usize>,
    pub offset: usize,
    pub size: usize,
    pub alignment: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PaddingInterval {
    pub offset: usize,
    pub size: usize,
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ParameterCodec {
    pub byte_size: usize,
    pub decoder: String,
    pub encoder: String,
    pub byte_order: String,
    pub fields: Vec<CodecField>,
    pub preserves_padding: bool,
    pub format: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TargetCodec {
    pub target: String,
    pub codec: ParameterCodec,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CodecField {
    pub name: String,
    pub kind: String,
    pub offset: usize,
    pub size: usize,
    #[serde(default)]
    pub array_extent: Option<usize>,
    #[serde(default)]
    pub enum_values: Vec<EnumValue>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct EnumValue {
    pub name: String,
    pub value: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct ColorContract {
    pub input: CallbackResult,
    pub output: CallbackResult,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CallbackResult {
    pub mode: String,
    pub value: String,
    #[serde(default)]
    pub predicate: Option<String>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

impl Default for CallbackResult {
    fn default() -> Self {
        Self {
            mode: "unresolved".to_owned(),
            value: "unknown".to_owned(),
            predicate: None,
            evidence: Vec::new(),
        }
    }
}

/// These independent booleans preserve the distinct capability dimensions
/// required by the manifest contract; replacing them with one enum would lose
/// valid combinations such as consuming and publishing a raster mask.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, Default)]
pub struct CapabilityContract {
    pub supports_shared_blending: bool,
    pub supports_drawn_masks: bool,
    pub publishes_raster_mask: bool,
    pub consumes_raster_mask: bool,
    #[serde(default)]
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RoiContract {
    pub behavior: String,
    pub overlap: String,
    pub full_analysis: String,
    pub geometry: String,
    pub fast_pipe: String,
    pub scale: String,
}

impl Default for RoiContract {
    fn default() -> Self {
        Self {
            behavior: "unresolved".to_owned(),
            overlap: "unresolved".to_owned(),
            full_analysis: "unresolved".to_owned(),
            geometry: "unresolved".to_owned(),
            fast_pipe: "unresolved".to_owned(),
            scale: "unresolved".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TilingContract {
    pub class: String,
    #[serde(default)]
    pub tile_width: Option<usize>,
    #[serde(default)]
    pub tile_height: Option<usize>,
    pub overlap: usize,
}

impl Default for TilingContract {
    fn default() -> Self {
        Self {
            class: "unresolved".to_owned(),
            tile_width: None,
            tile_height: None,
            overlap: 0,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct OpenclProgramResolution {
    pub program: String,
    pub registry_index: usize,
    pub source_path: String,
    pub kernels: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct PresetRecord {
    pub identity: String,
    pub parameter_version: u32,
    pub payload_hex: String,
    pub auto_apply: String,
    pub format: String,
    pub source_path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub evidence: Evidence,
}

fn default_tolerance_class() -> String {
    "Pointwise".to_owned()
}

#[derive(Debug, Deserialize)]
pub(crate) struct OperationOverrideFile {
    #[serde(rename = "operation", default)]
    pub operations: Vec<OperationOverride>,
}
