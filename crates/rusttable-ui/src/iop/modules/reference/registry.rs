//! Presentation projection for the registry operations in this UI slice.

use rusttable_core::Revision;
use rusttable_processing::descriptor::{
    OperationDescriptor, OperationFlags, ParameterDefault, ParameterKind,
};
use rusttable_processing::{DefinitionAvailability, OperationUiAvailability, builtin_registry};

use crate::iop::bloom::{BLOOM_DESCRIPTION, BLOOM_GROUP_KEYS, BLOOM_MODULE_ID, BLOOM_TITLE};
use crate::iop::colorcontrast::{
    COLORCONTRAST_MODULE_ID, COLORCONTRAST_SOURCE_MAP, ColorContrastSourceMap,
};
use crate::iop::colorcorrection::{
    COLORCORRECTION_MODULE_ID, COLORCORRECTION_SOURCE_MAP, ColorCorrectionGridState,
    ColorCorrectionSourceMap,
};
use crate::iop::colorreconstruct::{
    COLORRECONSTRUCTION_DESCRIPTION, COLORRECONSTRUCTION_GROUP_KEYS, COLORRECONSTRUCTION_MODULE_ID,
    COLORRECONSTRUCTION_TITLE,
};
use crate::iop::colorzones::{
    COLORZONES_DESCRIPTION, COLORZONES_GROUP_KEYS, COLORZONES_MODULE_ID, COLORZONES_TITLE,
};
use crate::iop::crop::{
    CROP_ALIASES, CROP_DESCRIPTION, CROP_GROUP_KEYS, CROP_MODULE_ID, CROP_TITLE,
};
use crate::iop::velvia::{VELVIA_MODULE_ID, VELVIA_SOURCE_MAP, VelviaSourceMap};
use crate::iop::vibrance::{VIBRANCE_MODULE_ID, VIBRANCE_SOURCE_MAP, VibranceSourceMap};
use crate::presentation::darkroom_controls::{DarkroomControlValue, DarkroomControlViewModel};

use super::super::{
    DarkroomModuleAvailability, DarkroomModuleError, DarkroomModulePreset, DarkroomModuleSide,
    DarkroomModuleViewModel, DarkroomModulesViewModel,
};

const COLORCORRECTION_PRESETS_UNAVAILABLE_REASON: &str = "Color Correction presets require RGB-display blend state, which the current edit model cannot persist";

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
    let vibrance_deprecated_message = (descriptor.id.compatibility_name == VIBRANCE_MODULE_ID)
        .then_some(VIBRANCE_SOURCE_MAP.deprecated_message());
    let ui_availability = definition.ui_availability();
    let availability = match definition.availability() {
        DefinitionAvailability::Unavailable { reason }
            if descriptor.flags.contains(OperationFlags::DEPRECATED) =>
        {
            DarkroomModuleAvailability::DeprecatedUnavailable {
                reason: reason.clone(),
            }
        }
        DefinitionAvailability::Unavailable { reason } => DarkroomModuleAvailability::Unsupported {
            reason: reason.clone(),
        },
        DefinitionAvailability::Available => match ui_availability {
            OperationUiAvailability::Available
                if descriptor.flags.contains(OperationFlags::DEPRECATED) =>
            {
                DarkroomModuleAvailability::Deprecated {
                    reason: vibrance_deprecated_message
                        .unwrap_or("compatibility operation; shown only by the deprecated filter")
                        .to_owned(),
                }
            }
            OperationUiAvailability::Available => DarkroomModuleAvailability::Supported,
            OperationUiAvailability::PartiallyAvailable {
                reason,
                deferred_responsibilities,
            } => DarkroomModuleAvailability::PartiallySupported {
                reason: reason.clone(),
                deferred_responsibilities: deferred_responsibilities.clone(),
            },
            OperationUiAvailability::Unavailable { reason }
                if descriptor.flags.contains(OperationFlags::DEPRECATED) =>
            {
                DarkroomModuleAvailability::DeprecatedUnavailable {
                    reason: reason.clone(),
                }
            }
            OperationUiAvailability::Unavailable { reason } => {
                DarkroomModuleAvailability::Unsupported {
                    reason: reason.clone(),
                }
            }
        },
    };
    module_from_descriptor(descriptor, availability, ui_availability)
}

fn module_from_descriptor(
    descriptor: &OperationDescriptor,
    availability: DarkroomModuleAvailability,
    ui_availability: &OperationUiAvailability,
) -> DarkroomModuleViewModel {
    let id = descriptor.id.compatibility_name.as_str();
    let colorcorrection_source_map =
        (id == COLORCORRECTION_MODULE_ID).then_some(COLORCORRECTION_SOURCE_MAP);
    let colorcontrast_source_map =
        (id == COLORCONTRAST_MODULE_ID).then_some(COLORCONTRAST_SOURCE_MAP);
    let velvia_source_map = (id == VELVIA_MODULE_ID).then_some(VELVIA_SOURCE_MAP);
    let vibrance_source_map = (id == VIBRANCE_MODULE_ID).then_some(VIBRANCE_SOURCE_MAP);
    let bloom_custom_editor = id == BLOOM_MODULE_ID;
    let colorreconstruct_custom_editor = id == COLORRECONSTRUCTION_MODULE_ID;
    let colorzones_custom_editor = id == COLORZONES_MODULE_ID;
    let crop_editor = id == CROP_MODULE_ID;
    let custom_editor =
        bloom_custom_editor || colorreconstruct_custom_editor || colorzones_custom_editor;
    let mut controls = Vec::new();
    if ui_availability.is_available() && !custom_editor {
        for parameter in &descriptor.parameters {
            if colorcorrection_source_map
                .is_some_and(|source_map| source_map.saturation(&parameter.id).is_none())
                || colorcontrast_source_map
                    .is_some_and(|source_map| source_map.slider(&parameter.id).is_none())
                || vibrance_source_map
                    .is_some_and(|source_map| source_map.slider(&parameter.id).is_none())
            {
                continue;
            }
            controls.extend(control_from_parameter(id, parameter));
        }
    }
    let title = bloom_custom_editor
        .then(|| BLOOM_TITLE.to_owned())
        .or_else(|| colorreconstruct_custom_editor.then(|| COLORRECONSTRUCTION_TITLE.to_owned()))
        .or_else(|| colorzones_custom_editor.then(|| COLORZONES_TITLE.to_owned()))
        .or_else(|| crop_editor.then(|| CROP_TITLE.to_owned()))
        .or_else(|| colorcorrection_source_map.map(|source_map| source_map.title().to_owned()))
        .or_else(|| colorcontrast_source_map.map(|source_map| source_map.title().to_owned()))
        .or_else(|| velvia_source_map.map(|source_map| source_map.title().to_owned()))
        .or_else(|| vibrance_source_map.map(|source_map| source_map.title().to_owned()))
        .unwrap_or_else(|| operation_title(descriptor));
    let group_key = descriptor
        .ui
        .as_ref()
        .map_or_else(|| fallback_group_key(descriptor), |ui| ui.group_key.clone());
    let style_eligible = descriptor.flags.contains(OperationFlags::STYLE_ELIGIBLE);
    let hidden = descriptor.flags.contains(OperationFlags::HIDDEN) || !ui_availability.is_usable();
    let default_enabled = !custom_editor
        && colorcorrection_source_map.is_none_or(ColorCorrectionSourceMap::default_enabled)
        && colorcontrast_source_map.is_none_or(ColorContrastSourceMap::default_enabled)
        && velvia_source_map.is_none_or(VelviaSourceMap::default_enabled)
        && vibrance_source_map.is_none_or(VibranceSourceMap::default_enabled);
    let default_expanded = colorcorrection_source_map
        .is_some_and(ColorCorrectionSourceMap::default_expanded)
        || colorcontrast_source_map.is_some_and(ColorContrastSourceMap::default_expanded)
        || velvia_source_map.is_some_and(VelviaSourceMap::default_expanded)
        || vibrance_source_map.is_some_and(VibranceSourceMap::default_expanded);
    let mut module = DarkroomModuleViewModel::new(
        id,
        title,
        DarkroomModuleSide::Right,
        default_expanded,
        availability.is_supported() && default_enabled,
        !controls.is_empty() || custom_editor,
        Revision::from_u64(0),
        controls,
    )
    .expect("registry descriptor projects to a valid darkroom module")
    .with_availability(availability)
    .with_registry_metadata(group_key, style_eligible, hidden);
    if bloom_custom_editor {
        module = module
            .with_description(BLOOM_DESCRIPTION)
            .with_bloom_custom_editor()
            .with_group_keys(BLOOM_GROUP_KEYS);
    } else if colorreconstruct_custom_editor {
        module = module
            .with_description(COLORRECONSTRUCTION_DESCRIPTION)
            .with_colorreconstruct_custom_editor()
            .with_group_keys(COLORRECONSTRUCTION_GROUP_KEYS);
    } else if colorzones_custom_editor {
        module = module
            .with_description(COLORZONES_DESCRIPTION)
            .with_colorzones_custom_editor()
            .with_group_keys(COLORZONES_GROUP_KEYS);
    } else if crop_editor {
        module = module
            .with_description(CROP_DESCRIPTION)
            .with_group_keys(CROP_GROUP_KEYS)
            .with_aliases(CROP_ALIASES);
    } else if let Some(source_map) = colorcorrection_source_map {
        module = module
            .with_color_correction_grid(ColorCorrectionGridState::DEFAULT)
            .with_group_keys(source_map.group_keys().iter().copied());
    } else if let Some(source_map) = colorcontrast_source_map {
        module = module
            .with_group_keys(source_map.group_keys().iter().copied())
            .with_aliases(source_map.aliases().iter().copied());
    } else if let Some(source_map) = velvia_source_map {
        module = module
            .with_group_keys(source_map.group_keys().iter().copied())
            .with_aliases(source_map.aliases().iter().copied());
    } else if let Some(source_map) = vibrance_source_map {
        module = module
            .with_group_keys(source_map.group_keys().iter().copied())
            .with_aliases(source_map.aliases().iter().copied());
    }
    match id {
        COLORCORRECTION_MODULE_ID => {
            module.with_presets_unavailable(COLORCORRECTION_PRESETS_UNAVAILABLE_REASON)
        }
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
    let (source_label, source_slider, source_step) = if module_id == COLORCORRECTION_MODULE_ID {
        COLORCORRECTION_SOURCE_MAP
            .saturation(&parameter.id)
            .map_or((None, None, None), |source| {
                (
                    Some(source.label().to_owned()),
                    Some(source.slider_presentation()),
                    Some(source.step()),
                )
            })
    } else if module_id == COLORCONTRAST_MODULE_ID {
        COLORCONTRAST_SOURCE_MAP
            .slider(&parameter.id)
            .map_or((None, None, None), |source| {
                (
                    Some(source.label().to_owned()),
                    Some(source.slider_presentation()),
                    Some(source.step()),
                )
            })
    } else if module_id == VELVIA_MODULE_ID {
        VELVIA_SOURCE_MAP
            .slider(&parameter.id)
            .map_or((None, None, None), |source| {
                (
                    Some(source.label().to_owned()),
                    Some(source.slider_presentation()),
                    Some(source.step()),
                )
            })
    } else if module_id == VIBRANCE_MODULE_ID {
        VIBRANCE_SOURCE_MAP
            .slider(&parameter.id)
            .map_or((None, None, None), |source| {
                (
                    Some(source.label().to_owned()),
                    Some(source.slider_presentation()),
                    Some(source.step()),
                )
            })
    } else {
        (None, None, None)
    };
    let label = source_label.unwrap_or_else(|| parameter_label(&parameter.id));
    let result = match (&parameter.kind, &parameter.default) {
        (ParameterKind::Scalar { minimum, maximum }, ParameterDefault::Scalar(default)) => {
            vec![DarkroomControlViewModel::slider(
                control_id,
                label,
                *minimum,
                *maximum,
                source_step.or(parameter.step).unwrap_or(0.01),
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
    result
        .into_iter()
        .filter_map(Result::ok)
        .map(|control| match source_slider {
            Some(source_slider) => control.with_source_mapped_slider(source_slider),
            None => control,
        })
        .collect()
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
    use rusttable_processing::descriptor::{OperationFlags, exposure_descriptor};
    use rusttable_processing::{OperationUiAvailability, builtin_registry};

    use crate::presentation::darkroom_controls::{DarkroomControlKind, DarkroomControlValue};

    use super::modules_from_registry;

    #[test]
    fn registry_projection_keeps_backend_ranges_and_deprecated_visibility_metadata() {
        let modules = modules_from_registry().expect("registry module projection");
        let bloom = modules.module("bloom").expect("bloom");
        assert!(bloom.has_bloom_custom_editor());
        assert!(bloom.bloom_editor_state().is_none());
        assert_eq!(bloom.controls().controls().len(), 0);
        assert_eq!(bloom.title(), "bloom");
        assert_eq!(
            bloom.group_keys().collect::<Vec<_>>(),
            ["group.effect", "group.effects"]
        );
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
        let grain = modules.module("grain").expect("grain");
        assert!(grain.is_style_eligible());
        assert!(!grain.is_favorite());
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
            &OperationUiAvailability::Available,
        );
        assert!(!module.enabled());
        assert!(module.availability().is_unsupported());
        assert_eq!(
            module.status_text(),
            "Unavailable · CPU backend unavailable"
        );
    }

    #[test]
    fn registry_projection_keeps_unavailable_ui_hidden() {
        let module = super::module_from_descriptor(
            &exposure_descriptor(),
            super::DarkroomModuleAvailability::Unsupported {
                reason: "UI not implemented".to_owned(),
            },
            &OperationUiAvailability::Unavailable {
                reason: "UI not implemented".to_owned(),
            },
        );

        assert!(module.is_hidden());
        assert!(module.availability().is_unsupported());
        assert_eq!(module.controls().controls().len(), 0);
        assert_eq!(module.status_text(), "Unavailable · UI not implemented");
    }

    #[test]
    fn crop_projects_exact_metadata_and_fails_closed_without_controls() {
        let modules = modules_from_registry().expect("registry module projection");
        let crop = modules.module("crop").expect("Crop module");

        assert_eq!(crop.title(), "crop");
        assert_eq!(crop.description(), Some("change the framing"));
        assert_eq!(
            crop.group_keys().collect::<Vec<_>>(),
            ["group.basic", "group.technical"]
        );
        assert_eq!(
            crop.aliases().collect::<Vec<_>>(),
            ["reframe", "distortion"]
        );
        assert!(crop.is_hidden());
        assert!(!crop.enabled());
        assert!(!crop.resettable());
        assert!(crop.availability().is_unsupported());
        assert_eq!(crop.controls().controls().len(), 0);
        assert_eq!(
            crop.status_text(),
            "Unavailable · Crop editing requires transformed crop-stage preview context"
        );

        let definition = builtin_registry()
            .definition("rusttable.crop")
            .expect("Crop backend definition");
        assert!(definition.cpu().is_some());
        assert!(!definition.ui_availability().is_usable());
    }

    #[test]
    fn partial_ui_does_not_project_generic_descriptor_controls() {
        let ui_availability = OperationUiAvailability::PartiallyAvailable {
            reason: "custom editor only".to_owned(),
            deferred_responsibilities: vec!["operation.ui.deferred".to_owned()],
        };
        let module = super::module_from_descriptor(
            &exposure_descriptor(),
            super::DarkroomModuleAvailability::PartiallySupported {
                reason: "custom editor only".to_owned(),
                deferred_responsibilities: vec!["operation.ui.deferred".to_owned()],
            },
            &ui_availability,
        );

        assert!(!module.is_hidden());
        assert!(module.availability().is_supported());
        assert!(!module.availability().is_fully_supported());
        assert!(module.availability().is_partial());
        assert_eq!(module.controls().controls().len(), 0);
    }

    #[test]
    fn colorzones_projects_only_the_source_specific_custom_editor_path() {
        let modules = modules_from_registry().expect("registry module projection");
        let colorzones = modules.module("colorzones").expect("Color Zones module");

        assert!(!colorzones.is_hidden());
        assert!(colorzones.availability().is_supported());
        assert!(!colorzones.availability().is_fully_supported());
        assert!(colorzones.availability().is_partial());
        assert_eq!(
            colorzones.availability().reason(),
            Some(
                "the Color Zones custom editor is usable, but native UI responsibilities remain deferred"
            )
        );
        assert_eq!(
            colorzones.status_text(),
            "Partial · the Color Zones custom editor is usable, but native UI responsibilities remain deferred"
        );
        assert!(!colorzones.enabled());
        assert!(colorzones.resettable());
        assert!(colorzones.has_colorzones_custom_editor());
        assert!(colorzones.colorzones_editor_state().is_none());
        assert_eq!(colorzones.controls().controls().len(), 0);
        assert_eq!(colorzones.title(), "color zones");
        assert_eq!(
            colorzones.description(),
            Some("selectively shift hues, chroma and lightness of pixels")
        );
        assert_eq!(
            colorzones.group_keys().collect::<Vec<_>>(),
            ["group.color", "group.grading"]
        );
        let deferred_responsibilities = [
            "iop.colorzones.ui.picker-lifecycle",
            "iop.colorzones.ui.operation-local-histogram",
            "iop.colorzones.ui.display-selection",
            "iop.colorzones.ui.presets",
            "iop.colorzones.ui.global-shortcuts-hold-mode",
            "iop.colorzones.ui.durable-gui-preferences",
            "iop.colorzones.ui.pending-import-materialization",
        ];
        assert_eq!(
            colorzones
                .availability()
                .deferred_responsibilities()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            deferred_responsibilities
        );
        let definition = builtin_registry()
            .definition("rusttable.colorzones")
            .expect("Color Zones backend definition");
        assert!(definition.availability().is_available());
        assert!(!definition.ui_availability().is_available());
        assert!(definition.ui_availability().is_usable());
        assert!(definition.ui_availability().is_partial());
        assert_eq!(
            definition
                .ui_availability()
                .deferred_responsibilities()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            deferred_responsibilities
        );
        assert!(definition.cpu().is_some());
        assert!(definition.gpu().is_some());
    }
}
