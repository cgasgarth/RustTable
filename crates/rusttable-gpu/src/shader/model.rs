use std::fmt;

use rusttable_core::numerics::{ImplementationNumerics, NumericalContract, ToleranceClass};
use serde::{Deserialize, Serialize};

pub const SHADER_SCHEMA: &str = "rusttable.shader.v1";
pub const REFLECTION_SCHEMA: &str = "rusttable.shader-reflection.v1";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ShaderEntryId {
    pub program_id: String,
    pub entry_point_id: String,
}

impl ShaderEntryId {
    pub fn new(program_id: impl Into<String>, entry_point_id: impl Into<String>) -> Self {
        Self {
            program_id: program_id.into(),
            entry_point_id: entry_point_id.into(),
        }
    }

    #[must_use]
    pub fn stable_name(&self) -> String {
        format!("{}.{}", self.program_id, self.entry_point_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderIdentity {
    pub program_id: String,
    pub program_version: u16,
    pub entry_point_id: String,
    pub entry_point_version: u16,
    pub source_tree_hash: String,
    pub generated_wgsl_hash: String,
    pub reflection_schema: String,
    pub numerical_class: NumericalClass,
    pub feature_plan: FeaturePlan,
    pub owner_operation_ids: Vec<String>,
    pub owner_kernel_ids: Vec<String>,
    pub canonical_cpu_reference: String,
    pub implementation_version: u16,
    pub implementation_numerics: ImplementationNumerics,
}

impl ShaderIdentity {
    #[must_use]
    pub fn entry_id(&self) -> ShaderEntryId {
        ShaderEntryId::new(&self.program_id, &self.entry_point_id)
    }

    #[must_use]
    pub fn cache_identity(&self) -> String {
        let mut fields = vec![
            self.program_id.clone(),
            self.program_version.to_string(),
            self.entry_point_id.clone(),
            self.entry_point_version.to_string(),
            self.source_tree_hash.clone(),
            self.generated_wgsl_hash.clone(),
            self.reflection_schema.clone(),
            format!("{:?}", self.numerical_class),
            format!("{:?}", self.feature_plan),
            self.canonical_cpu_reference.clone(),
            self.implementation_version.to_string(),
            self.implementation_numerics.contract().stable_id(),
            self.implementation_numerics
                .implementation_hash()
                .to_owned(),
        ];
        fields.extend(self.owner_operation_ids.iter().cloned());
        fields.extend(self.owner_kernel_ids.iter().cloned());
        fields.join("|")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NumericalClass {
    F32Point,
    F32Neighborhood,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeaturePlan {
    CoreCompute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct NumericalMetadata {
    pub uses_f32: bool,
    pub uses_f16: bool,
    pub contraction_assumption: String,
    pub transcendental_operations: Vec<String>,
    pub texture_filtering: bool,
    pub sampling: bool,
    pub atomics: bool,
    pub reductions: bool,
    pub subnormal_policy: String,
    pub non_finite_policy: String,
    pub schema_3_tolerance_class: String,
    pub canonical_cpu_reference: String,
    pub contract: NumericalContract,
    pub tolerance: ToleranceClass,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpanAlias {
    pub source_alias: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindingResourceKind {
    StorageBuffer,
    UniformBuffer,
    Sampler,
    Texture,
    StorageTexture,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BindingReflection {
    pub group: u32,
    pub binding: u32,
    pub name: String,
    pub resource: BindingResourceKind,
    pub access: String,
    pub address_space: String,
    pub type_description: String,
    pub minimum_binding_size: u32,
    pub dynamic_offset: bool,
    pub dynamic_offset_alignment: u32,
    pub format: Option<String>,
    pub dimension: Option<String>,
    pub source: SourceSpanAlias,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterReflection {
    pub name: String,
    pub scalar_type: String,
    pub offset: u32,
    pub size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverrideReflection {
    pub name: String,
    pub id: Option<u16>,
    pub scalar_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShaderReflection {
    pub schema: String,
    pub entry_point: String,
    pub stage: String,
    pub bindings: Vec<BindingReflection>,
    pub parameters: Vec<ParameterReflection>,
    pub overrides: Vec<OverrideReflection>,
    pub workgroup_size: [u32; 3],
    pub required_capabilities: Vec<String>,
    pub source_spans: Vec<SourceSpanAlias>,
    pub numerical: NumericalMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderEntry {
    pub identity: ShaderIdentity,
    pub source_alias: String,
    pub expanded_source: String,
    pub reflection: ShaderReflection,
}

impl ShaderEntry {
    #[must_use]
    pub fn id(&self) -> ShaderEntryId {
        self.identity.entry_id()
    }

    #[must_use]
    pub fn cache_key(&self) -> String {
        self.identity.cache_identity()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShaderManifest {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaderError {
    InvalidSourceAlias(String),
    SourceNotFound(String),
    IncludeTraversal(String),
    IncludeCycle(String),
    IncludeDepth,
    IncludeCount,
    ExpansionTooLarge,
    UnknownSubstitution(String),
    InvalidSubstitution(String),
    ForbiddenSourceConstruct(String),
    InvalidUtf8(String),
    Parse {
        alias: String,
        line: u32,
        column: u32,
    },
    Validation {
        alias: String,
        line: u32,
        column: u32,
    },
    Reflection(String),
    DuplicateIdentity(String),
    MissingOwner(String),
    MissingTolerance(String),
    ManifestDrift,
    GeneratedBindingsDrift,
    Io(String),
}

impl fmt::Display for ShaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSourceAlias(value) => {
                write!(formatter, "invalid shader source alias: {value}")
            }
            Self::SourceNotFound(value) => write!(formatter, "shader source not found: {value}"),
            Self::IncludeTraversal(value) => {
                write!(formatter, "shader include traversal rejected: {value}")
            }
            Self::IncludeCycle(value) => write!(formatter, "shader include cycle: {value}"),
            Self::IncludeDepth => formatter.write_str("shader include depth exceeded"),
            Self::IncludeCount => formatter.write_str("shader include count exceeded"),
            Self::ExpansionTooLarge => {
                formatter.write_str("shader expansion exceeded the bounded source budget")
            }
            Self::UnknownSubstitution(value) => {
                write!(formatter, "unknown shader substitution: {value}")
            }
            Self::InvalidSubstitution(value) => {
                write!(formatter, "invalid shader substitution: {value}")
            }
            Self::ForbiddenSourceConstruct(value) => {
                write!(formatter, "forbidden shader source construct: {value}")
            }
            Self::InvalidUtf8(value) => write!(formatter, "shader source is not UTF-8: {value}"),
            Self::Parse {
                alias,
                line,
                column,
            } => write!(formatter, "WGSL parse error at {alias}:{line}:{column}"),
            Self::Validation {
                alias,
                line,
                column,
            } => write!(
                formatter,
                "WGSL validation error at {alias}:{line}:{column}"
            ),
            Self::Reflection(value) => write!(formatter, "shader reflection error: {value}"),
            Self::DuplicateIdentity(value) => {
                write!(formatter, "duplicate shader identity: {value}")
            }
            Self::MissingOwner(value) => write!(formatter, "shader owner is missing: {value}"),
            Self::MissingTolerance(value) => {
                write!(formatter, "shader tolerance is missing: {value}")
            }
            Self::ManifestDrift => formatter.write_str("checked-in shader manifest is stale"),
            Self::GeneratedBindingsDrift => {
                formatter.write_str("checked-in shader bindings are stale")
            }
            Self::Io(value) => write!(formatter, "shader IO error: {value}"),
        }
    }
}

impl std::error::Error for ShaderError {}

pub(crate) fn hex(bytes: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
