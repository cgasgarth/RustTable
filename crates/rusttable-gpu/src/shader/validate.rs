use std::collections::BTreeSet;

use naga::proc::Layouter;
use naga::{AddressSpace, ArraySize, ImageClass, ScalarKind, StorageAccess, TypeInner};

use super::model::{
    BindingReflection, BindingResourceKind, FeaturePlan, NumericalMetadata, OverrideReflection,
    ParameterReflection, ShaderError, ShaderReflection, SourceSpanAlias,
};

#[allow(clippy::too_many_lines)]
pub(crate) fn validate_and_reflect(
    alias: &str,
    source: &str,
    line_aliases: &[SourceSpanAlias],
    entry_name: &str,
    numerical: NumericalMetadata,
) -> Result<ShaderReflection, ShaderError> {
    let module = naga::front::wgsl::parse_str(source).map_err(|error| {
        let (line, column) = error.location(source).map_or((1, 1), |location| {
            (location.line_number, location.line_position)
        });
        ShaderError::Parse {
            alias: alias.to_owned(),
            line,
            column,
        }
    })?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .map_err(|_error| ShaderError::Validation {
        alias: alias.to_owned(),
        line: 1,
        column: 1,
    })?;
    let entry = module
        .entry_points
        .first()
        .filter(|entry| entry.name == entry_name)
        .or_else(|| {
            module
                .entry_points
                .iter()
                .find(|entry| entry.name == entry_name)
        })
        .ok_or_else(|| ShaderError::Reflection("module has no entry point".to_owned()))?;
    let mut layouter = Layouter::default();
    layouter
        .update(module.to_ctx())
        .map_err(|error| ShaderError::Reflection(error.to_string()))?;
    let mut bindings = Vec::new();
    let mut source_spans = Vec::new();
    let mut parameter_records = Vec::new();
    for (_, global) in module.global_variables.iter() {
        let Some(binding) = global.binding else {
            continue;
        };
        let name = global.name.clone().unwrap_or_else(|| "unnamed".to_owned());
        let (resource, address_space, access) = resource_kind(global.space);
        let binding_type_description = type_description(&module, global.ty);
        let minimum_binding_size = layouter[global.ty].size;
        let source = source_span(source, line_aliases, binding.group, binding.binding, &name);
        source_spans.push(source.clone());
        bindings.push(BindingReflection {
            group: binding.group,
            binding: binding.binding,
            name,
            resource,
            access,
            address_space,
            type_description: binding_type_description,
            minimum_binding_size,
            dynamic_offset: false,
            dynamic_offset_alignment: 256,
            format: image_format(&module, global.ty),
            dimension: image_dimension(&module, global.ty),
            source,
        });
        if matches!(global.space, AddressSpace::Uniform)
            && let TypeInner::Struct { members, .. } = &module.types[global.ty].inner
        {
            for member in members {
                let name = member.name.clone().unwrap_or_else(|| "unnamed".to_owned());
                let size = layouter[member.ty].size;
                parameter_records.push(ParameterReflection {
                    name,
                    scalar_type: type_description(&module, member.ty),
                    offset: member.offset,
                    size,
                });
            }
        }
    }
    bindings.sort_by_key(|binding| (binding.group, binding.binding));
    let mut overrides = Vec::new();
    for (_, override_value) in module.overrides.iter() {
        overrides.push(OverrideReflection {
            name: override_value
                .name
                .clone()
                .unwrap_or_else(|| "unnamed".to_owned()),
            id: override_value.id,
            scalar_type: type_description(&module, override_value.ty),
        });
    }
    let required_capabilities = required_capabilities(&module, &bindings);
    Ok(ShaderReflection {
        schema: super::model::REFLECTION_SCHEMA.to_owned(),
        entry_point: entry.name.clone(),
        stage: format!("{:?}", entry.stage),
        bindings,
        parameters: parameter_records,
        overrides,
        workgroup_size: entry.workgroup_size,
        required_capabilities,
        source_spans,
        numerical,
    })
}

fn resource_kind(space: AddressSpace) -> (BindingResourceKind, String, String) {
    match space {
        AddressSpace::Storage { access } => (
            BindingResourceKind::StorageBuffer,
            "storage".to_owned(),
            storage_access(access),
        ),
        AddressSpace::Uniform => (
            BindingResourceKind::UniformBuffer,
            "uniform".to_owned(),
            "read".to_owned(),
        ),
        AddressSpace::Handle => (
            BindingResourceKind::Other,
            "handle".to_owned(),
            "read".to_owned(),
        ),
        other => (
            BindingResourceKind::Other,
            format!("{other:?}").to_lowercase(),
            "read".to_owned(),
        ),
    }
}

fn storage_access(access: StorageAccess) -> String {
    match (
        access.contains(StorageAccess::LOAD),
        access.contains(StorageAccess::STORE),
    ) {
        (true, true) => "read_write".to_owned(),
        (true, false) => "read".to_owned(),
        (false, true) => "write".to_owned(),
        (false, false) => "none".to_owned(),
    }
}

fn type_description(module: &naga::Module, handle: naga::Handle<naga::Type>) -> String {
    match &module.types[handle].inner {
        TypeInner::Scalar(scalar) => scalar_description(scalar.kind, scalar.width),
        TypeInner::Vector { size, scalar } => {
            format!(
                "vec{}<{}>",
                u8::from(*size),
                scalar_description(scalar.kind, scalar.width)
            )
        }
        TypeInner::Array { base, size, .. } => {
            let length = match size {
                ArraySize::Constant(value) => value.get().to_string(),
                ArraySize::Pending(_) => "override".to_owned(),
                ArraySize::Dynamic => "runtime".to_owned(),
            };
            format!("array<{}, {length}>", type_description(module, *base))
        }
        TypeInner::Struct { .. } => "struct".to_owned(),
        TypeInner::Image { .. } => "texture".to_owned(),
        TypeInner::Sampler { .. } => "sampler".to_owned(),
        other => format!("{other:?}"),
    }
}

fn scalar_description(kind: ScalarKind, width: u8) -> String {
    let prefix = match kind {
        ScalarKind::Sint => "i",
        ScalarKind::Uint => "u",
        ScalarKind::Float => "f",
        ScalarKind::Bool => "bool",
        ScalarKind::AbstractInt => "abstract-i",
        ScalarKind::AbstractFloat => "abstract-f",
    };
    if matches!(kind, ScalarKind::Bool) {
        prefix.to_owned()
    } else {
        format!("{prefix}{width}")
    }
}

fn source_span(
    source: &str,
    line_aliases: &[SourceSpanAlias],
    group: u32,
    binding: u32,
    name: &str,
) -> SourceSpanAlias {
    for (index, line) in source.lines().enumerate() {
        if line.contains(&format!("@group({group})"))
            && line.contains(&format!("@binding({binding})"))
            && line.contains(name)
            && let Some(alias) = line_aliases.get(index)
        {
            return alias.clone();
        }
    }
    SourceSpanAlias {
        source_alias: "shaders/point.wgsl".to_owned(),
        line: 1,
        column: 1,
    }
}

fn image_format(module: &naga::Module, handle: naga::Handle<naga::Type>) -> Option<String> {
    match module.types[handle].inner {
        TypeInner::Image {
            class: ImageClass::Storage { format, .. },
            ..
        } => Some(format!("{format:?}")),
        _ => None,
    }
}

fn image_dimension(module: &naga::Module, handle: naga::Handle<naga::Type>) -> Option<String> {
    match module.types[handle].inner {
        TypeInner::Image { dim, .. } => Some(format!("{dim:?}")),
        _ => None,
    }
}

fn required_capabilities(module: &naga::Module, bindings: &[BindingReflection]) -> Vec<String> {
    let mut capabilities = BTreeSet::from([format!("{:?}", FeaturePlan::CoreCompute)]);
    if module.overrides.is_empty() {
        capabilities.insert("NoOverrides".to_owned());
    }
    if bindings
        .iter()
        .any(|binding| binding.access == "read_write")
    {
        capabilities.insert("StorageBufferReadWrite".to_owned());
    }
    capabilities.into_iter().collect()
}
