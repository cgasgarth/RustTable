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
}

fn default_tolerance_class() -> String {
    "Pointwise".to_owned()
}

#[derive(Debug, Deserialize)]
pub(crate) struct OperationOverrideFile {
    #[serde(rename = "operation", default)]
    pub operations: Vec<OperationOverride>,
}
