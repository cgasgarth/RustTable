mod model;
mod registry;
mod source;
mod validate;

pub mod generated;

pub use model::{
    BindingReflection, BindingResourceKind, FeaturePlan, NumericalClass, NumericalMetadata,
    OverrideReflection, ParameterReflection, REFLECTION_SCHEMA, SHADER_SCHEMA, ShaderEntry,
    ShaderEntryId, ShaderError, ShaderIdentity, ShaderManifest, ShaderReflection, SourceSpanAlias,
};
pub use registry::ShaderRegistry;
pub use source::{ExpandedShaderSource, expand_template, validate_source_alias};

pub fn validate_checked_in() -> Result<&'static ShaderRegistry, ShaderError> {
    let registry = ShaderRegistry::checked_in();
    registry.verify_checked_in_outputs()?;
    Ok(registry)
}
