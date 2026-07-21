//! Presentation projection for the registry operations in this UI slice.

use rusttable_core::Revision;
use rusttable_processing::descriptor::{
    OperationDescriptor, OperationFlags, ParameterDefault, ParameterKind,
};
use rusttable_processing::{DefinitionAvailability, builtin_registry};

use crate::presentation::darkroom_controls::{DarkroomControlError, DarkroomControlViewModel};

use super::super::{
    DarkroomModuleAvailability, DarkroomModuleError, DarkroomModuleSide, DarkroomModuleViewModel,
    DarkroomModulesViewModel,
};

// This is presentation order only; descriptors, parameters, ranges, and flags all come from the
// processing registry below.
const RAIL_OPERATION_IDS: [&str; 4] = [
    "rusttable.bloom",
    "rusttable.soften",
    "rusttable.invert",
    "rusttable.dither",
];

pub(super) fn modules_from_registry() -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    let registry = builtin_registry();
    let modules = RAIL_OPERATION_IDS
        .iter()
        .filter_map(|id| registry.definition(id))
        .map(module_from_definition)
        .collect::<Result<Vec<_>, _>>()?;
    DarkroomModulesViewModel::new(modules)
}

fn module_from_definition(
    definition: &rusttable_processing::OperationDefinition,
) -> Result<DarkroomModuleViewModel, DarkroomModuleError> {
    let descriptor = definition.descriptor();
    let id = descriptor.id.compatibility_name.as_str();
    let mut controls = Vec::new();
    for parameter in &descriptor.parameters {
        if let Some(control) = control_from_parameter(id, parameter) {
            controls.push(control?);
        }
    }
    let title = operation_title(descriptor);
    let availability = match definition.availability() {
        DefinitionAvailability::Available
            if descriptor.flags.contains(OperationFlags::DEPRECATED) =>
        {
            DarkroomModuleAvailability::Deprecated {
                reason: "compatibility operation; shown only by the deprecated filter".to_owned(),
            }
        }
        DefinitionAvailability::Available => DarkroomModuleAvailability::Supported,
        DefinitionAvailability::Unavailable { reason } => DarkroomModuleAvailability::Unsupported {
            reason: reason.clone(),
        },
    };
    DarkroomModuleViewModel::new(
        id,
        title,
        DarkroomModuleSide::Right,
        false,
        availability.is_supported(),
        !controls.is_empty(),
        Revision::from_u64(0),
        controls,
    )
    .map(|module| module.with_availability(availability))
    .map_err(|error| DarkroomModuleError::Control(DarkroomControlError::Validation(error)))
}

#[allow(clippy::cast_precision_loss)]
fn control_from_parameter(
    module_id: &str,
    parameter: &rusttable_processing::descriptor::ParameterDescriptor,
) -> Option<Result<DarkroomControlViewModel, DarkroomModuleError>> {
    let control_id = format!("{module_id}-{}", parameter.id);
    let label = parameter_label(&parameter.id);
    let result = match (&parameter.kind, &parameter.default) {
        (ParameterKind::Scalar { minimum, maximum }, ParameterDefault::Scalar(default)) => {
            DarkroomControlViewModel::slider(
                control_id,
                label,
                *minimum,
                *maximum,
                parameter.step.unwrap_or(0.01),
                *default,
                *default,
            )
        }
        // Integer descriptors are projected into GTK's existing f64 slider boundary. The
        // processing registry remains integer-typed; this cast is presentation-only.
        (ParameterKind::Integer { minimum, maximum }, ParameterDefault::Integer(default)) => {
            DarkroomControlViewModel::slider(
                control_id,
                label,
                *minimum as f64,
                *maximum as f64,
                parameter.step.unwrap_or(1.0),
                *default as f64,
                *default as f64,
            )
        }
        (ParameterKind::Bool, ParameterDefault::Bool(default)) => {
            DarkroomControlViewModel::toggle(control_id, label, *default, *default)
        }
        (ParameterKind::Enum { tags }, ParameterDefault::Enum(default)) => {
            let selected = tags.iter().position(|tag| tag == default)?;
            DarkroomControlViewModel::choice(control_id, label, tags.iter(), selected)
        }
        _ => return None,
    };
    Some(
        result
            .map_err(|error| DarkroomModuleError::Control(DarkroomControlError::Validation(error))),
    )
}

fn operation_title(descriptor: &OperationDescriptor) -> String {
    let mut title = title_case(&descriptor.id.compatibility_name);
    if descriptor
        .capability
        .modes
        .iter()
        .any(|mode| mode == "posterize")
    {
        title.push_str(" or posterize");
    }
    if descriptor.id.compatibility_name == "invert" {
        title.push_str(" / fill light");
    }
    title
}

fn parameter_label(id: &str) -> String {
    title_case(id)
}

fn title_case(value: &str) -> String {
    value
        .split(['-', '_', '.'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                first.to_uppercase().chain(chars).collect()
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use rusttable_processing::builtin_registry;
    use rusttable_processing::descriptor::OperationFlags;

    use super::modules_from_registry;

    #[test]
    fn registry_projection_keeps_backend_ranges_and_deprecated_visibility_metadata() {
        let modules = modules_from_registry().expect("registry module projection");
        let bloom = modules.module("bloom").expect("bloom");
        let size = bloom.controls().control("bloom-size").expect("bloom size");
        assert!((size.slider_spec().expect("slider").maximum() - 100.0).abs() < f64::EPSILON);
        assert!(modules.module("soften").is_some());
        assert!(modules.module("dither").is_some());
        let invert = modules.module("invert").expect("invert");
        assert!(invert.availability().is_deprecated());
        assert_eq!(invert.title(), "Invert / fill light");
    }

    #[test]
    fn registry_projection_does_not_duplicate_descriptor_parameter_definitions() {
        let registry_ids = builtin_registry()
            .definitions()
            .iter()
            .map(|definition| definition.descriptor().id.rust_id.as_str())
            .collect::<Vec<_>>();
        for id in super::RAIL_OPERATION_IDS {
            assert!(registry_ids.contains(&id));
        }
        assert!(
            builtin_registry()
                .definition("rusttable.invert")
                .expect("invert")
                .descriptor()
                .flags
                .contains(OperationFlags::HIDDEN)
        );
    }
}
