//! Presentation projection for the registry operations in this UI slice.

use rusttable_core::Revision;
use rusttable_processing::descriptor::{
    OperationDescriptor, OperationFlags, ParameterDefault, ParameterKind,
};
use rusttable_processing::{DefinitionAvailability, builtin_registry};

use crate::presentation::darkroom_controls::{DarkroomControlValue, DarkroomControlViewModel};

use super::super::{
    DarkroomModuleAvailability, DarkroomModuleError, DarkroomModulePreset, DarkroomModuleSide,
    DarkroomModuleViewModel, DarkroomModulesViewModel,
};

pub(super) fn modules_from_registry() -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    let registry = builtin_registry();
    let modules = registry
        .definitions_in_declaration_order()
        .into_iter()
        .map(module_from_definition)
        .collect::<Vec<_>>();
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
    let group_key = descriptor
        .ui
        .as_ref()
        .map_or_else(|| fallback_group_key(descriptor), |ui| ui.group_key.clone());
    let favorite = descriptor.flags.contains(OperationFlags::STYLE_ELIGIBLE);
    let hidden = descriptor.flags.contains(OperationFlags::HIDDEN);
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
    .with_availability(availability)
    .with_registry_metadata(group_key, favorite, hidden);
    match id {
        "graduatednd" => module.with_presets(graduatednd_presets()),
        "relight" => module.with_presets(relight_presets()),
        "vignette" => module.with_presets(vignette_presets()),
        _ => module,
    }
}

#[allow(clippy::cast_precision_loss)]
fn control_from_parameter(
    module_id: &str,
    parameter: &rusttable_processing::descriptor::ParameterDescriptor,
) -> Vec<DarkroomControlViewModel> {
    let control_id = format!("{module_id}-{}", ui_parameter_id(&parameter.id));
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
                    format!("{module_id}-{}-{axis}", ui_parameter_id(&parameter.id)),
                    format!("{label} {axis}"),
                    *minimum,
                    *maximum,
                    parameter.step.unwrap_or(0.01),
                    *value,
                    *value,
                )
            })
            .collect(),
        (ParameterKind::Text { maximum_bytes }, ParameterDefault::Text(default)) => {
            vec![DarkroomControlViewModel::text(
                control_id,
                label,
                default.clone(),
                default.clone(),
                usize::from(*maximum_bytes),
            )]
        }
        _ => Vec::new(),
    };
    result.into_iter().filter_map(Result::ok).collect()
}

fn ui_parameter_id(parameter_id: &str) -> String {
    parameter_id.replace('_', "-")
}

fn graduatednd_presets() -> Vec<DarkroomModulePreset> {
    rusttable_processing::operations::graduatednd::presets()
        .iter()
        .map(|preset| {
            let parameters = preset.parameters;
            DarkroomModulePreset::new(
                preset.name,
                preset.name,
                [
                    ("graduatednd-density", parameters.density),
                    ("graduatednd-hardness", parameters.hardness),
                    ("graduatednd-rotation", parameters.rotation),
                    ("graduatednd-offset", parameters.offset),
                    ("graduatednd-hue", parameters.hue),
                    ("graduatednd-saturation", parameters.saturation),
                ]
                .into_iter()
                .map(|(id, value)| {
                    (
                        id.to_owned(),
                        DarkroomControlValue::Slider(f64::from(value)),
                    )
                })
                .collect(),
            )
        })
        .collect()
}

fn relight_presets() -> Vec<DarkroomModulePreset> {
    rusttable_processing::operations::relight::presets()
        .iter()
        .map(|preset| {
            let parameters = preset.parameters;
            DarkroomModulePreset::new(
                preset.name,
                preset.name,
                [
                    ("relight-ev", parameters.ev),
                    ("relight-center", parameters.center),
                    ("relight-width", parameters.width),
                ]
                .into_iter()
                .map(|(id, value)| {
                    (
                        id.to_owned(),
                        DarkroomControlValue::Slider(f64::from(value)),
                    )
                })
                .collect(),
            )
        })
        .collect()
}

fn vignette_presets() -> Vec<DarkroomModulePreset> {
    rusttable_processing::operations::vignette::presets()
        .iter()
        .map(|preset| {
            let parameters = preset.parameters;
            DarkroomModulePreset::new(
                preset.name,
                preset.name,
                [
                    ("vignette-scale", parameters.scale),
                    ("vignette-falloff-scale", parameters.falloff_scale),
                    ("vignette-brightness", parameters.brightness),
                    ("vignette-saturation", parameters.saturation),
                    ("vignette-center-x", parameters.center[0]),
                    ("vignette-center-y", parameters.center[1]),
                    ("vignette-whratio", parameters.whratio),
                    ("vignette-shape", parameters.shape),
                ]
                .into_iter()
                .map(|(id, value)| {
                    (
                        id.to_owned(),
                        DarkroomControlValue::Slider(f64::from(value)),
                    )
                })
                .chain([
                    (
                        "vignette-autoratio".to_owned(),
                        DarkroomControlValue::Toggle(parameters.autoratio),
                    ),
                    (
                        "vignette-unbound".to_owned(),
                        DarkroomControlValue::Toggle(parameters.unbound),
                    ),
                    (
                        "vignette-dithering".to_owned(),
                        DarkroomControlValue::Slider(f64::from(parameters.dithering as u8)),
                    ),
                ])
                .collect(),
            )
        })
        .collect()
}

fn operation_title(descriptor: &OperationDescriptor) -> String {
    let mut title = descriptor.ui.as_ref().map_or_else(
        || title_case(&descriptor.id.compatibility_name),
        |ui| {
            title_case(
                ui.label_key
                    .strip_prefix("operation.")
                    .unwrap_or(&ui.label_key),
            )
        },
    );
    if descriptor
        .capability
        .modes
        .iter()
        .any(|mode| mode == "posterize")
    {
        title.push_str(" or posterize");
    }
    title
}

fn fallback_group_key(descriptor: &OperationDescriptor) -> String {
    if descriptor.flags.contains(OperationFlags::GEOMETRY) {
        "group.corrective".to_owned()
    } else if descriptor.flags.contains(OperationFlags::COLOR) {
        "group.color".to_owned()
    } else {
        match descriptor.stage.as_str() {
            "input-color" | "output-color" => "group.color".to_owned(),
            "display-linear" => "group.effects".to_owned(),
            _ => "group.basic".to_owned(),
        }
    }
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
    use rusttable_processing::descriptor::{OperationFlags, exposure_descriptor};

    use crate::presentation::darkroom_controls::{DarkroomControlKind, DarkroomControlValue};

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
        assert_eq!(invert.title(), "Invert");
        assert!(modules.module("temperature").is_some());
        assert!(modules.module("lenscorrection").is_some());
        assert!(modules.module("colorin").is_some());
        assert!(modules.module("colorout").is_some());
        let colorin = modules.module("colorin").expect("colorin");
        let input_profile = colorin
            .controls()
            .control("colorin-input-profile")
            .expect("input profile");
        assert_eq!(input_profile.kind(), DarkroomControlKind::Text);
        assert_eq!(
            input_profile.value(),
            DarkroomControlValue::Text("builtin:srgb".to_owned())
        );
        assert!(modules.module("graduatednd").is_some());
        assert!(modules.module("vignette").is_some());
        assert!(modules.module("grain").expect("grain").is_favorite());
        assert!(
            modules
                .module("finalscale")
                .expect("finalscale")
                .is_hidden()
        );
    }

    #[test]
    fn registry_projection_does_not_duplicate_descriptor_parameter_definitions() {
        let registry_ids = builtin_registry()
            .definitions()
            .iter()
            .map(|definition| definition.descriptor().id.rust_id.as_str())
            .collect::<Vec<_>>();
        let module_ids = modules_from_registry()
            .expect("modules")
            .right_modules()
            .map(|module| {
                *registry_ids
                    .iter()
                    .find(|rust_id| {
                        builtin_registry()
                            .definition(rust_id)
                            .is_some_and(|definition| {
                                definition.descriptor().id.compatibility_name == module.id()
                            })
                    })
                    .expect("module has registry identity")
            })
            .collect::<Vec<_>>();
        assert_eq!(module_ids.len(), registry_ids.len());
        assert!(
            builtin_registry()
                .definition("rusttable.invert")
                .expect("invert")
                .descriptor()
                .flags
                .contains(OperationFlags::HIDDEN)
        );
    }

    #[test]
    fn registry_projection_keeps_unavailable_operations_truthful() {
        let module = super::module_from_descriptor(
            &exposure_descriptor(),
            super::DarkroomModuleAvailability::Unsupported {
                reason: "CPU backend unavailable".to_owned(),
            },
        );
        assert!(!module.enabled());
        assert!(module.availability().is_unsupported());
        assert_eq!(
            module.status_text(),
            "Unavailable · CPU backend unavailable"
        );
    }
}
