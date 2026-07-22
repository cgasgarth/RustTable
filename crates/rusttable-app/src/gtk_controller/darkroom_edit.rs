//! Controller-owned persistence for GTK darkroom module actions.

use std::path::PathBuf;

use rusttable_catalog::EditRepository;
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, FiniteF64, Operation, OperationId, ParameterText, ParameterValue};
use rusttable_processing::builtin_registry;
use rusttable_processing::defringe_compatibility::DefringeMode;
use rusttable_ui::presentation::{DarkroomControlKind, DarkroomControlValue};
use rusttable_ui::{
    DarkroomModuleAction, DarkroomModuleError, DarkroomModuleViewModel, DarkroomModulesViewModel,
    reference_modules,
};

use rusttable_core::{PhotoId, Revision};
use sha2::{Digest, Sha256};

/// Result published after one durable darkroom action.
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomEditOutcome {
    revision: Revision,
    modules: DarkroomModulesViewModel,
}

impl DarkroomEditOutcome {
    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn modules(&self) -> &DarkroomModulesViewModel {
        &self.modules
    }
}

/// Application-side owner of the selected photo's typed operation stack.
#[derive(Debug, Clone)]
pub struct GtkDarkroomEditController {
    catalog_path: Option<PathBuf>,
    selected_photo: Option<PhotoId>,
    modules: Option<DarkroomModulesViewModel>,
}

impl GtkDarkroomEditController {
    #[must_use]
    pub fn new(catalog_path: Option<PathBuf>) -> Self {
        Self {
            catalog_path,
            selected_photo: None,
            modules: None,
        }
    }

    #[must_use]
    pub const fn selected_photo(&self) -> Option<PhotoId> {
        self.selected_photo
    }

    #[must_use]
    pub fn modules(&self) -> Option<&DarkroomModulesViewModel> {
        self.modules.as_ref()
    }

    /// Loads the selected photo's current edit and projects it into GTK controls.
    ///
    /// # Errors
    ///
    /// Returns a typed persistence or projection error when the selected photo
    /// cannot be resolved.
    pub fn select_photo(
        &mut self,
        photo_id: PhotoId,
    ) -> Result<&DarkroomModulesViewModel, DarkroomModuleError> {
        let edit = self.load_edit(photo_id)?;
        let modules = project_edit(&edit)?;
        self.selected_photo = Some(photo_id);
        self.modules = Some(modules);
        self.modules
            .as_ref()
            .ok_or_else(|| persistence_error("darkroom modules were not installed"))
    }

    pub fn clear_selection(&mut self) {
        self.selected_photo = None;
        self.modules = None;
    }

    /// Applies one GTK action through the selected edit's atomic repository transaction.
    ///
    /// # Errors
    ///
    /// # Errors
    ///
    /// Returns a typed module, persistence, or revision error when the action
    /// cannot be applied atomically.
    pub fn apply(
        &mut self,
        action: &DarkroomModuleAction,
    ) -> Result<DarkroomEditOutcome, DarkroomModuleError> {
        let photo_id = self
            .selected_photo
            .ok_or(DarkroomModuleError::NoSelection)?;
        if let Some(module) = self
            .modules
            .as_ref()
            .and_then(|modules| modules.module(action.module_id()))
            && !module.availability().is_supported()
        {
            return Err(DarkroomModuleError::Unsupported {
                module_id: module.id().to_owned(),
                reason: module
                    .availability()
                    .reason()
                    .unwrap_or("registry capability is not qualified")
                    .to_owned(),
            });
        }
        let current = self.load_edit(photo_id)?;
        let expected = action.expected_revision();
        if current.revision() != expected {
            let actual = current.revision();
            self.modules = Some(project_edit(&current)?);
            return Err(DarkroomModuleError::StaleRevision { expected, actual });
        }

        let mut modules = self
            .modules
            .clone()
            .map_or_else(|| project_edit(&current), Ok)?;
        let module = modules.module_mut(action.module_id()).ok_or_else(|| {
            DarkroomModuleError::WrongModule {
                expected: action.module_id().to_owned(),
                actual: "unknown".to_owned(),
            }
        })?;
        let revision = module.apply(action.clone())?;
        if matches!(action, DarkroomModuleAction::Recover { .. }) {
            self.modules = Some(project_edit(&current)?);
            let modules = self
                .modules
                .clone()
                .ok_or_else(|| persistence_error("darkroom modules were not installed"))?;
            return Ok(DarkroomEditOutcome { revision, modules });
        }

        let operations = rewrite_operations(&current, module, action)?;
        let replacement = current
            .revised(operations)
            .map_err(|error| persistence_error(error.to_string()))?;
        let mut repository = self.open_repository()?;
        repository
            .commit_replacement(current.revision(), &replacement)
            .map_err(|error| persistence_error(error.to_string()))?;
        let projected = project_edit(&replacement)?;
        self.modules = Some(projected.clone());
        Ok(DarkroomEditOutcome {
            revision: replacement.revision(),
            modules: projected,
        })
    }

    fn load_edit(&self, photo_id: PhotoId) -> Result<Edit, DarkroomModuleError> {
        let repository = self.open_repository()?;
        repository
            .list()
            .map_err(|error| persistence_error(error.to_string()))?
            .into_iter()
            .filter(|edit| edit.photo_id() == photo_id)
            .max_by_key(|edit| (edit.revision().get(), edit.id().get()))
            .ok_or(DarkroomModuleError::MissingOperation {
                module_id: format!("photo {photo_id}"),
            })
    }

    fn open_repository(&self) -> Result<RedbCatalogRepository, DarkroomModuleError> {
        let path = self
            .catalog_path
            .as_deref()
            .ok_or_else(|| persistence_error("catalog path is unavailable"))?;
        RedbCatalogRepository::open(path).map_err(|error| persistence_error(error.to_string()))
    }
}

fn project_edit(edit: &Edit) -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    let mut modules = reference_modules()?;
    let registry = builtin_registry();
    let module_ids = modules
        .left_modules()
        .map(|module| module.id().to_owned())
        .chain(modules.right_modules().map(|module| module.id().to_owned()))
        .collect::<Vec<_>>();
    for module_id in module_ids {
        let module = modules
            .module_mut(&module_id)
            .expect("module id was collected from the stack");
        let operation = edit.operations().find(|operation| {
            registry
                .definition(operation.key().as_str())
                .is_some_and(|definition| {
                    definition.descriptor().id.compatibility_name == module.id()
                })
        });
        let values = operation
            .map(|operation| control_values(module, operation))
            .transpose()?;
        module.reconcile_operation(
            edit.revision(),
            operation.is_some_and(|operation| {
                Operation::is_enabled(operation) && module.availability().is_supported()
            }),
            values.into_iter().flatten(),
        )?;
    }
    Ok(modules)
}

fn control_values(
    module: &DarkroomModuleViewModel,
    operation: &Operation,
) -> Result<Vec<(String, DarkroomControlValue)>, DarkroomModuleError> {
    module
        .controls()
        .controls()
        .filter_map(|control| {
            let parameter = operation.parameters().find(|(name, _)| {
                control_parameter_id(module.id(), name.as_str()) == control.id().as_str()
            })?;
            Some(parameter_value_to_control(control, parameter.1))
        })
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn parameter_value_to_control(
    control: &rusttable_ui::DarkroomControlViewModel,
    value: &ParameterValue,
) -> Result<(String, DarkroomControlValue), DarkroomModuleError> {
    if control.id().as_str() == "defringe-mode" {
        let ParameterValue::Integer(value) = value else {
            return Err(persistence_error(
                "defringe mode must be a numeric v1 value",
            ));
        };
        let Some(mode) = DefringeMode::from_numeric(*value) else {
            return Err(persistence_error(
                "defringe mode is outside the v1 numeric enum",
            ));
        };
        return Ok((
            control.id().as_str().to_owned(),
            DarkroomControlValue::Choice(mode.index()),
        ));
    }
    let value = match (control.kind(), value) {
        (DarkroomControlKind::Slider, ParameterValue::Scalar(value)) => {
            DarkroomControlValue::Slider(value.get())
        }
        (DarkroomControlKind::Slider, ParameterValue::Integer(value)) => {
            DarkroomControlValue::Slider(*value as f64)
        }
        (DarkroomControlKind::Toggle, ParameterValue::Bool(value)) => {
            DarkroomControlValue::Toggle(*value)
        }
        (DarkroomControlKind::Choice, ParameterValue::Integer(value)) => {
            let Ok(value) = usize::try_from(*value) else {
                return Err(persistence_error("choice index is out of range"));
            };
            DarkroomControlValue::Choice(value)
        }
        (DarkroomControlKind::Choice, ParameterValue::Text(value)) => {
            let Some(index) = control
                .choices()
                .position(|choice| choice.as_str() == value.as_str())
            else {
                return Err(persistence_error(
                    "persisted choice is not in the descriptor",
                ));
            };
            DarkroomControlValue::Choice(index)
        }
        (DarkroomControlKind::Text, ParameterValue::Text(value)) => {
            DarkroomControlValue::Text(value.as_str().to_owned())
        }
        _ => {
            return Err(persistence_error(
                "persisted parameter type mismatches the control",
            ));
        }
    };
    Ok((control.id().as_str().to_owned(), value))
}

fn rewrite_operations(
    edit: &Edit,
    module: &DarkroomModuleViewModel,
    action: &DarkroomModuleAction,
) -> Result<Vec<Operation>, DarkroomModuleError> {
    let registry = builtin_registry();
    let target = edit.operations().find(|operation| {
        registry
            .definition(operation.key().as_str())
            .is_some_and(|definition| definition.descriptor().id.compatibility_name == module.id())
    });
    let Some(target) = target else {
        let definition = registry
            .definitions()
            .iter()
            .find(|definition| definition.descriptor().id.compatibility_name == module.id())
            .ok_or_else(|| DarkroomModuleError::MissingOperation {
                module_id: module.id().to_owned(),
            })?;
        let key = definition.descriptor().id.rust_id.as_str();
        let operation_id = materialized_operation_id(edit, key);
        let operation = registry
            .materialize_operation(key, operation_id)
            .map_err(|error| materialization_error(module.id(), error.to_string()))?;
        let operation = rewrite_target_operation(&operation, module, action)?;
        let mut operations = edit.operations().cloned().collect::<Vec<_>>();
        let insertion = operations
            .iter()
            .position(|candidate| canonical_rank(candidate) > canonical_rank(&operation))
            .unwrap_or(operations.len());
        operations.insert(insertion, operation);
        return Ok(operations);
    };

    edit.operations()
        .map(|operation| {
            if operation.id() != target.id() {
                return Ok(operation.clone());
            }
            let completed = complete_operation_defaults(operation)
                .map_err(|error| materialization_error(module.id(), error.to_string()))?;
            rewrite_target_operation(&completed, module, action)
        })
        .collect()
}

fn complete_operation_defaults(
    operation: &Operation,
) -> Result<Operation, rusttable_processing::OperationMaterializationError> {
    let defaults =
        builtin_registry().materialize_operation(operation.key().as_str(), operation.id())?;
    let mut parameters = defaults
        .parameters()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    for (name, value) in operation.parameters() {
        if let Some((_, default)) = parameters
            .iter_mut()
            .find(|(candidate, _)| candidate == name)
        {
            *default = value.clone();
        } else {
            parameters.push((name.clone(), value.clone()));
        }
    }
    Operation::new_with_opacity(
        operation.id(),
        operation.key().clone(),
        operation.is_enabled(),
        operation.opacity(),
        parameters,
    )
    .map_err(
        |error| rusttable_processing::OperationMaterializationError::OperationBuild {
            key: operation.key().clone(),
            message: error.to_string(),
        },
    )
}

fn rewrite_target_operation(
    operation: &Operation,
    module: &DarkroomModuleViewModel,
    action: &DarkroomModuleAction,
) -> Result<Operation, DarkroomModuleError> {
    let enabled = match action {
        DarkroomModuleAction::Enable { enabled, .. } => *enabled,
        _ => operation.is_enabled(),
    };
    let mut parameters = operation
        .parameters()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    for control in module.controls().controls() {
        let Some((_, value)) = parameters.iter_mut().find(|(name, _)| {
            control_parameter_id(module.id(), name.as_str()) == control.id().as_str()
        }) else {
            continue;
        };
        if let Some(replacement) = parameter_from_control(control, value) {
            *value = replacement;
        }
    }
    Operation::new_with_opacity(
        operation.id(),
        operation.key().clone(),
        enabled,
        operation.opacity(),
        parameters,
    )
    .map_err(|error| persistence_error(error.to_string()))
}

fn materialized_operation_id(edit: &Edit, key: &str) -> OperationId {
    let mut digest = Sha256::new();
    digest.update(b"rusttable.darkroom.materialized-operation.v1\0");
    digest.update(edit.id().get().to_be_bytes());
    digest.update(edit.photo_id().get().to_be_bytes());
    digest.update(key.as_bytes());
    let bytes = digest.finalize();
    let mut id_bytes = [0_u8; 16];
    id_bytes.copy_from_slice(&bytes[..16]);
    let id = u128::from_be_bytes(id_bytes);
    OperationId::new(if id == 0 { 1 } else { id }).expect("materialized operation ID is nonzero")
}

fn canonical_rank(operation: &Operation) -> usize {
    const ORDER: &[&str] = &[
        "invert",
        "temperature",
        "rasterfile",
        "highlights",
        "ashift",
        "rotatepixels",
        "scalepixels",
        "lens",
        "flip",
        "enlargecanvas",
        "clipping",
        "liquify",
        "spots",
        "retouch",
        "exposure",
        "mask_manager",
        "crop",
        "graduatednd",
        "colorin",
        "censorize",
        "primaries",
        "rgbgain",
        "defringe",
        "basicadj",
        "relight",
        "colorcorrection",
        "bloom",
        "shadhi",
        "grain",
        "soften",
        "vignette",
        "colorreconstruct",
        "finalscale",
        "colorout",
        "clahe",
        "dither",
    ];
    let name = builtin_registry()
        .definition(operation.key().as_str())
        .map_or(operation.key().as_str(), |definition| {
            definition.descriptor().id.compatibility_name.as_str()
        });
    ORDER
        .iter()
        .position(|candidate| *candidate == name)
        .unwrap_or(ORDER.len())
}

fn materialization_error(module_id: &str, message: String) -> DarkroomModuleError {
    DarkroomModuleError::Unsupported {
        module_id: module_id.to_owned(),
        reason: message,
    }
}

#[allow(clippy::cast_possible_truncation)]
fn parameter_from_control(
    control: &rusttable_ui::DarkroomControlViewModel,
    existing: &ParameterValue,
) -> Option<ParameterValue> {
    if control.id().as_str() == "defringe-mode" {
        let DarkroomControlValue::Choice(value) = control.value() else {
            return None;
        };
        return DefringeMode::from_numeric(i64::try_from(value).ok()?)
            .map(|mode| ParameterValue::Integer(mode.numeric()));
    }
    match (control.value(), existing) {
        (DarkroomControlValue::Slider(value), ParameterValue::Scalar(_)) => {
            Some(ParameterValue::Scalar(FiniteF64::new(value).ok()?))
        }
        (DarkroomControlValue::Slider(value), ParameterValue::Integer(_)) => {
            Some(ParameterValue::Integer(value as i64))
        }
        (DarkroomControlValue::Toggle(value), ParameterValue::Bool(_)) => {
            Some(ParameterValue::Bool(value))
        }
        (DarkroomControlValue::Choice(value), ParameterValue::Integer(_)) => {
            Some(ParameterValue::Integer(i64::try_from(value).ok()?))
        }
        (DarkroomControlValue::Choice(value), ParameterValue::Text(_)) => control
            .choices()
            .nth(value)
            .and_then(|choice| ParameterText::new(choice.as_str()).ok())
            .map(ParameterValue::Text),
        (DarkroomControlValue::Text(value), ParameterValue::Text(_)) => {
            ParameterText::new(value).ok().map(ParameterValue::Text)
        }
        _ => None,
    }
}

fn control_parameter_id(module_id: &str, parameter: &str) -> String {
    format!("{module_id}-{}", parameter.replace('_', "-"))
}

fn persistence_error(message: impl Into<String>) -> DarkroomModuleError {
    DarkroomModuleError::Persistence {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::{EditId, OperationId, OperationKey, OperationOpacity, ParameterName};

    use super::*;

    fn edit(revision: u64, stops: f64, enabled: bool) -> Edit {
        Edit::from_parts(
            EditId::new(1).expect("edit id"),
            PhotoId::new(2).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(revision),
            [Operation::new_with_opacity(
                OperationId::new(3).expect("operation id"),
                OperationKey::new("rusttable.exposure").expect("operation key"),
                enabled,
                OperationOpacity::ONE,
                [(
                    ParameterName::new("stops").expect("parameter name"),
                    ParameterValue::Scalar(FiniteF64::new(stops).expect("finite")),
                )],
            )
            .expect("operation")],
        )
        .expect("edit")
    }

    #[test]
    fn first_control_on_imported_two_node_edit_materializes_registry_defaults() {
        let original = Edit::from_parts(
            EditId::new(101).expect("edit id"),
            PhotoId::new(202).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(3),
            [
                Operation::new(
                    OperationId::new(11).expect("exposure id"),
                    OperationKey::new("rusttable.exposure").expect("exposure key"),
                    true,
                    [(
                        ParameterName::new("stops").expect("stops"),
                        ParameterValue::Scalar(FiniteF64::new(0.0).expect("finite")),
                    )],
                )
                .expect("exposure"),
                Operation::new(
                    OperationId::new(12).expect("RGB gain id"),
                    OperationKey::new("rusttable.rgb_gain").expect("RGB gain key"),
                    true,
                    [
                        (ParameterName::new("red").expect("red"), scalar(1.0)),
                        (ParameterName::new("green").expect("green"), scalar(1.0)),
                        (ParameterName::new("blue").expect("blue"), scalar(1.0)),
                    ],
                )
                .expect("RGB gain"),
            ],
        )
        .expect("imported edit");
        let mut modules = project_edit(&original).expect("projection");
        let module = modules.module_mut("bloom").expect("bloom module");
        let action = DarkroomModuleAction::Control {
            module_id: "bloom".to_owned(),
            expected_revision: original.revision(),
            id: "bloom-strength".to_owned(),
            value: DarkroomControlValue::Slider(50.0),
        };
        module.apply(action.clone()).expect("first control");

        let operations = rewrite_operations(&original, module, &action).expect("materialization");
        let replacement = original.revised(operations).expect("history revision");
        let operations = replacement.operations().collect::<Vec<_>>();
        assert_eq!(operations.len(), 3);
        assert_eq!(
            operations[0].id(),
            OperationId::new(11).expect("exposure id")
        );
        assert_eq!(
            operations[1].id(),
            OperationId::new(12).expect("RGB gain id")
        );
        let bloom = operations[2];
        assert_eq!(
            bloom.id(),
            materialized_operation_id(&original, "rusttable.bloom")
        );
        assert_eq!(bloom.parameters().count(), 3);
        assert_eq!(
            bloom.parameter(&ParameterName::new("size").expect("size")),
            Some(&scalar(20.0))
        );
        assert_eq!(
            bloom.parameter(&ParameterName::new("threshold").expect("threshold")),
            Some(&scalar(90.0))
        );
        assert_eq!(
            bloom.parameter(&ParameterName::new("strength").expect("strength")),
            Some(&scalar(50.0))
        );
    }

    fn scalar(value: f64) -> ParameterValue {
        ParameterValue::Scalar(FiniteF64::new(value).expect("finite"))
    }

    #[test]
    fn projection_uses_persisted_exposure_values_and_revision() {
        let projected = project_edit(&edit(4, 1.25, true)).expect("projection");
        let exposure = projected.module("exposure").expect("exposure");
        assert_eq!(exposure.revision(), Revision::from_u64(4));
        assert!(exposure.enabled());
        assert_eq!(
            exposure
                .controls()
                .control("exposure-stops")
                .expect("stops")
                .value(),
            DarkroomControlValue::Slider(1.25)
        );
    }

    #[test]
    fn control_action_rewrites_only_the_typed_operation_and_advances_edit_revision() {
        let original = edit(4, 0.0, true);
        let mut modules = project_edit(&original).expect("projection");
        let module = modules.module_mut("exposure").expect("exposure");
        module
            .apply(DarkroomModuleAction::Control {
                module_id: "exposure".to_owned(),
                expected_revision: Revision::from_u64(4),
                id: "exposure-stops".to_owned(),
                value: DarkroomControlValue::Slider(2.0),
            })
            .expect("control action");
        let operations = rewrite_operations(
            &original,
            module,
            &DarkroomModuleAction::Control {
                module_id: "exposure".to_owned(),
                expected_revision: Revision::from_u64(4),
                id: "exposure-stops".to_owned(),
                value: DarkroomControlValue::Slider(2.0),
            },
        )
        .expect("rewrite");
        let replacement = original.revised(operations).expect("revision");
        assert_eq!(replacement.revision(), Revision::from_u64(5));
        let operation = replacement.operations().next().expect("operation");
        assert_eq!(
            operation.parameter(&ParameterName::new("stops").expect("parameter")),
            Some(&ParameterValue::Scalar(
                FiniteF64::new(2.0).expect("finite")
            ))
        );
    }

    #[test]
    fn registry_modules_project_and_persist_non_exposure_actions_through_history() {
        let original = Edit::from_parts(
            EditId::new(4).expect("edit id"),
            PhotoId::new(2).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(4),
            [Operation::new_with_opacity(
                OperationId::new(9).expect("operation id"),
                OperationKey::new("rusttable.bloom").expect("operation key"),
                true,
                OperationOpacity::ONE,
                [
                    (
                        ParameterName::new("size").expect("parameter"),
                        ParameterValue::Scalar(FiniteF64::new(20.0).expect("finite")),
                    ),
                    (
                        ParameterName::new("threshold").expect("parameter"),
                        ParameterValue::Scalar(FiniteF64::new(90.0).expect("finite")),
                    ),
                    (
                        ParameterName::new("strength").expect("parameter"),
                        ParameterValue::Scalar(FiniteF64::new(25.0).expect("finite")),
                    ),
                ],
            )
            .expect("operation")],
        )
        .expect("edit");
        let mut modules = project_edit(&original).expect("registry projection");
        assert_eq!(
            modules.right_modules().len(),
            builtin_registry().definitions().len()
        );
        let module = modules.module_mut("bloom").expect("bloom module");
        module
            .apply(DarkroomModuleAction::Control {
                module_id: "bloom".to_owned(),
                expected_revision: Revision::from_u64(4),
                id: "bloom-strength".to_owned(),
                value: DarkroomControlValue::Slider(50.0),
            })
            .expect("bloom action");
        let operations = rewrite_operations(
            &original,
            module,
            &DarkroomModuleAction::Control {
                module_id: "bloom".to_owned(),
                expected_revision: Revision::from_u64(4),
                id: "bloom-strength".to_owned(),
                value: DarkroomControlValue::Slider(50.0),
            },
        )
        .expect("rewrite");
        let replacement = original.revised(operations).expect("history revision");
        let operation = replacement.operations().next().expect("operation");
        assert_eq!(
            operation.parameter(&ParameterName::new("strength").expect("parameter")),
            Some(&ParameterValue::Scalar(
                FiniteF64::new(50.0).expect("finite")
            ))
        );
    }

    #[test]
    fn censorize_history_round_trip_projects_values() {
        let original = Edit::from_parts(
            EditId::new(5).expect("edit id"),
            PhotoId::new(2).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(8),
            [Operation::new_with_opacity(
                OperationId::new(10).expect("operation id"),
                OperationKey::new("rusttable.censorize").expect("operation key"),
                true,
                OperationOpacity::ONE,
                [
                    (
                        ParameterName::new("radius_1").expect("parameter"),
                        ParameterValue::Scalar(FiniteF64::new(12.0).expect("finite")),
                    ),
                    (
                        ParameterName::new("pixelate").expect("parameter"),
                        ParameterValue::Scalar(FiniteF64::new(24.0).expect("finite")),
                    ),
                    (
                        ParameterName::new("radius_2").expect("parameter"),
                        ParameterValue::Scalar(FiniteF64::new(4.0).expect("finite")),
                    ),
                    (
                        ParameterName::new("noise").expect("parameter"),
                        ParameterValue::Scalar(FiniteF64::new(0.25).expect("finite")),
                    ),
                ],
            )
            .expect("operation")],
        )
        .expect("edit");
        let modules = project_edit(&original).expect("projection");
        let censorize = modules.module("censorize").expect("censorize");
        assert_eq!(censorize.revision(), Revision::from_u64(8));
        assert!(censorize.enabled());
        assert_eq!(
            censorize
                .controls()
                .control("censorize-noise")
                .expect("noise")
                .value(),
            DarkroomControlValue::Slider(0.25)
        );
    }

    #[test]
    fn defringe_imported_numeric_modes_round_trip_through_the_canonical_edit() {
        for mode in [0_i64, 1, 2] {
            let original = Edit::from_parts(
                EditId::new(20 + u128::try_from(mode).expect("mode")).expect("edit id"),
                PhotoId::new(2).expect("photo id"),
                Revision::ZERO,
                Revision::from_u64(9),
                [Operation::new_with_opacity(
                    OperationId::new(30 + u128::try_from(mode).expect("mode"))
                        .expect("operation id"),
                    OperationKey::new("rusttable.defringe").expect("operation key"),
                    true,
                    OperationOpacity::ONE,
                    [
                        (
                            ParameterName::new("radius").expect("radius"),
                            ParameterValue::Scalar(FiniteF64::new(4.0).expect("radius value")),
                        ),
                        (
                            ParameterName::new("threshold").expect("threshold"),
                            ParameterValue::Scalar(FiniteF64::new(20.0).expect("threshold value")),
                        ),
                        (
                            ParameterName::new("mode").expect("mode"),
                            ParameterValue::Integer(mode),
                        ),
                    ],
                )
                .expect("operation")],
            )
            .expect("edit");
            let mut modules = project_edit(&original).expect("projection");
            let defringe = modules.module_mut("defringe").expect("defringe");
            assert_eq!(
                defringe
                    .controls()
                    .control("defringe-mode")
                    .expect("mode control")
                    .value(),
                DarkroomControlValue::Choice(usize::try_from(mode).expect("mode index"))
            );
            let replacement = rewrite_operations(
                &original,
                defringe,
                &DarkroomModuleAction::Control {
                    module_id: "defringe".to_owned(),
                    expected_revision: original.revision(),
                    id: "defringe-mode".to_owned(),
                    value: DarkroomControlValue::Choice(usize::try_from(mode).expect("mode index")),
                },
            )
            .expect("canonical rewrite");
            assert_eq!(
                replacement
                    .first()
                    .expect("defringe operation")
                    .parameter(&ParameterName::new("mode").expect("mode")),
                Some(&ParameterValue::Integer(mode))
            );
        }
    }

    #[test]
    fn clahe_imported_values_project_through_history_and_accept_controls() {
        let original = Edit::from_parts(
            EditId::new(40).expect("edit id"),
            PhotoId::new(2).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(11),
            [Operation::new_with_opacity(
                OperationId::new(41).expect("operation id"),
                OperationKey::new("rusttable.clahe").expect("operation key"),
                true,
                OperationOpacity::ONE,
                [
                    (
                        ParameterName::new("radius").expect("parameter"),
                        ParameterValue::Scalar(FiniteF64::new(128.0).expect("radius")),
                    ),
                    (
                        ParameterName::new("slope").expect("parameter"),
                        ParameterValue::Scalar(FiniteF64::new(2.5).expect("slope")),
                    ),
                ],
            )
            .expect("operation")],
        )
        .expect("edit");

        let mut modules = project_edit(&original).expect("history projection");
        let clahe = modules.module_mut("clahe").expect("CLAHE module");
        assert_eq!(clahe.title(), "Old Local Contrast");
        assert!(!clahe.availability().is_unsupported());
        assert!(clahe.enabled());
        assert_eq!(
            clahe
                .controls()
                .control("clahe-radius")
                .expect("radius")
                .value(),
            DarkroomControlValue::Slider(128.0)
        );
        assert_eq!(
            clahe
                .controls()
                .control("clahe-slope")
                .expect("slope")
                .value(),
            DarkroomControlValue::Slider(2.5)
        );
        clahe
            .apply(DarkroomModuleAction::Control {
                module_id: "clahe".to_owned(),
                expected_revision: original.revision(),
                id: "clahe-radius".to_owned(),
                value: DarkroomControlValue::Slider(64.0),
            })
            .expect("qualified backend accepts actions");
    }
}
