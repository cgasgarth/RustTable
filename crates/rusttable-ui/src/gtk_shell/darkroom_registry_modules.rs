//! Presentation projection for the registry operations in this UI slice.

use rusttable_core::Revision;
use rusttable_processing::descriptor::{
    OperationDescriptor, OperationFlags, ParameterDefault, ParameterKind, graduatednd_descriptor,
    vignette_descriptor,
};
use rusttable_processing::{DefinitionAvailability, builtin_registry};

use crate::presentation::darkroom_controls::{DarkroomControlValue, DarkroomControlViewModel};

use super::super::{
    DarkroomModuleAvailability, DarkroomModuleError, DarkroomModulePreset, DarkroomModuleSide,
    DarkroomModuleViewModel, DarkroomModulesViewModel,
};

// This is presentation order only; descriptors, parameters, ranges, and flags all come from the
// processing registry below.
const RAIL_OPERATION_IDS: [&str; 4] = [
    "rusttable.bloom",
    "rusttable.soften",
    "rusttable.invert",
    "rusttable.dither",
];

const DARKTABLE_OPERATION_IDS: [&str; 2] = ["graduatednd", "vignette"];

pub(super) fn modules_from_registry() -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    debug_assert_eq!(DARKTABLE_OPERATION_IDS.len(), 2);
    let registry = builtin_registry();
    let mut modules = RAIL_OPERATION_IDS
        .iter()
        .filter_map(|id| registry.definition(id))
        .map(module_from_definition)
        .collect::<Vec<_>>();
    modules.extend([
        module_from_descriptor(
            &graduatednd_descriptor(),
            DarkroomModuleAvailability::Unsupported {
                reason: "processing evaluator is not included in this UI slice".to_owned(),
            },
        ),
        module_from_descriptor(
            &vignette_descriptor(),
            DarkroomModuleAvailability::Unsupported {
                reason: "processing evaluator is not included in this UI slice".to_owned(),
            },
        ),
    ]);
    DarkroomModulesViewModel::new(modules)
}

fn module_from_definition(
    definition: &rusttable_processing::OperationDefinition,
) -> DarkroomModuleViewModel {
    let descriptor = definition.descriptor();
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
    module_from_descriptor(descriptor, availability)
}

fn module_from_descriptor(
    descriptor: &OperationDescriptor,
    availability: DarkroomModuleAvailability,
) -> DarkroomModuleViewModel {
    let id = descriptor.id.compatibility_name.as_str();
    let mut controls = Vec::new();
    for parameter in &descriptor.parameters {
        controls.extend(control_from_parameter(id, parameter));
    }
    let title = operation_title(descriptor);
    let module = DarkroomModuleViewModel::new(
        id,
        title,
        DarkroomModuleSide::Right,
        false,
        availability.is_supported(),
        !controls.is_empty(),
        Revision::from_u64(0),
        controls,
    )
    .expect("registry descriptor projects to a valid darkroom module")
    .with_availability(availability);
    if id == "graduatednd" {
        module.with_presets(graduatednd_presets())
    } else if id == "vignette" {
        module.with_presets(vec![DarkroomModulePreset::new(
            "lomo",
            "lomo",
            vec![
                (
                    "vignette-scale".to_owned(),
                    DarkroomControlValue::Slider(40.0),
                ),
                (
                    "vignette-falloff_scale".to_owned(),
                    DarkroomControlValue::Slider(100.0),
                ),
                (
                    "vignette-brightness".to_owned(),
                    DarkroomControlValue::Slider(-1.0),
                ),
                (
                    "vignette-saturation".to_owned(),
                    DarkroomControlValue::Slider(0.5),
                ),
            ],
        )])
    } else {
        module
    }
}

#[allow(clippy::cast_precision_loss)]
fn control_from_parameter(
    module_id: &str,
    parameter: &rusttable_processing::descriptor::ParameterDescriptor,
) -> Vec<DarkroomControlViewModel> {
    let control_id = format!("{module_id}-{}", parameter.id);
    let label = parameter_label(&parameter.id);
    let result = match (&parameter.kind, &parameter.default) {
        (ParameterKind::Scalar { minimum, maximum }, ParameterDefault::Scalar(default)) => {
            vec![DarkroomControlViewModel::slider(
                control_id,
                label,
                *minimum,
                *maximum,
                parameter.step.unwrap_or(0.01),
                *default,
                *default,
            )]
        }
        // Integer descriptors are projected into GTK's existing f64 slider boundary. The
        // processing registry remains integer-typed; this cast is presentation-only.
        (ParameterKind::Integer { minimum, maximum }, ParameterDefault::Integer(default)) => {
            vec![DarkroomControlViewModel::slider(
                control_id,
                label,
                *minimum as f64,
                *maximum as f64,
                parameter.step.unwrap_or(1.0),
                *default as f64,
                *default as f64,
            )]
        }
        (ParameterKind::Bool, ParameterDefault::Bool(default)) => {
            vec![DarkroomControlViewModel::toggle(
                control_id, label, *default, *default,
            )]
        }
        (ParameterKind::Enum { tags }, ParameterDefault::Enum(default)) => {
            let Some(selected) = tags.iter().position(|tag| tag == default) else {
                return Vec::new();
            };
            vec![DarkroomControlViewModel::choice(
                control_id,
                label,
                tags.iter(),
                selected,
            )]
        }
        (
            ParameterKind::Vector {
                dimensions,
                minimum,
                maximum,
            },
            ParameterDefault::Vector(defaults),
        ) if defaults.len() == usize::from(*dimensions) => defaults
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let axis = ["x", "y", "z", "w"]
                    .get(index)
                    .copied()
                    .unwrap_or("component");
                DarkroomControlViewModel::slider(
                    format!("{module_id}-{}-{axis}", parameter.id),
                    format!("{label} {axis}"),
                    *minimum,
                    *maximum,
                    parameter.step.unwrap_or(0.01),
                    *value,
                    *value,
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    result.into_iter().filter_map(Result::ok).collect()
}

fn graduatednd_presets() -> Vec<DarkroomModulePreset> {
    [
        ("nd2-soft", "neutral gray | ND2 (soft)", 1.0, 0.0, 0.0, 0.0),
        ("nd4-soft", "neutral gray | ND4 (soft)", 2.0, 0.0, 0.0, 0.0),
        ("nd8-soft", "neutral gray | ND8 (soft)", 3.0, 0.0, 0.0, 0.0),
        ("nd2-hard", "neutral gray | ND2 (hard)", 1.0, 75.0, 0.0, 0.0),
        ("nd4-hard", "neutral gray | ND4 (hard)", 2.0, 75.0, 0.0, 0.0),
        ("nd8-hard", "neutral gray | ND8 (hard)", 3.0, 75.0, 0.0, 0.0),
        (
            "orange-nd2-soft",
            "tinted | orange ND2 (soft)",
            1.0,
            0.0,
            0.102_439,
            0.8,
        ),
        (
            "yellow-nd2-soft",
            "tinted | yellow ND2 (soft)",
            1.0,
            0.0,
            0.151_220,
            0.5,
        ),
        (
            "purple-nd2-soft",
            "tinted | purple ND2 (soft)",
            1.0,
            0.0,
            0.824_390,
            0.5,
        ),
        (
            "green-nd2-soft",
            "tinted | green ND2 (soft)",
            1.0,
            0.0,
            0.302_439,
            0.5,
        ),
        (
            "red-nd2-soft",
            "tinted | red ND2 (soft)",
            1.0,
            0.0,
            0.0,
            0.5,
        ),
        (
            "blue-nd2-soft",
            "tinted | blue ND2 (soft)",
            1.0,
            0.0,
            0.663_415,
            0.5,
        ),
        (
            "brown-nd4-soft",
            "tinted | brown ND4 (soft)",
            2.0,
            0.0,
            0.082_927,
            0.25,
        ),
    ]
    .into_iter()
    .map(|(id, label, density, hardness, hue, saturation)| {
        DarkroomModulePreset::new(
            id,
            label,
            vec![
                (
                    "graduatednd-density".to_owned(),
                    DarkroomControlValue::Slider(density),
                ),
                (
                    "graduatednd-hardness".to_owned(),
                    DarkroomControlValue::Slider(hardness),
                ),
                (
                    "graduatednd-rotation".to_owned(),
                    DarkroomControlValue::Slider(0.0),
                ),
                (
                    "graduatednd-offset".to_owned(),
                    DarkroomControlValue::Slider(50.0),
                ),
                (
                    "graduatednd-hue".to_owned(),
                    DarkroomControlValue::Slider(hue),
                ),
                (
                    "graduatednd-saturation".to_owned(),
                    DarkroomControlValue::Slider(saturation),
                ),
            ],
        )
    })
    .collect()
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
