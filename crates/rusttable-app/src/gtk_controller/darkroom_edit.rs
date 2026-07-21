//! Controller-owned persistence for GTK darkroom module actions.

use std::path::PathBuf;

use rusttable_catalog::EditRepository;
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, FiniteF64, Operation, ParameterText, ParameterValue};
use rusttable_processing::builtin_registry;
use rusttable_ui::presentation::{DarkroomControlKind, DarkroomControlValue};
use rusttable_ui::{
    DarkroomModuleAction, DarkroomModuleError, DarkroomModuleViewModel, DarkroomModulesViewModel,
    reference_modules,
};

use rusttable_core::{PhotoId, Revision};

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
            operation.is_some_and(Operation::is_enabled),
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
    let operation = edit.operations().find(|operation| {
        builtin_registry()
            .definition(operation.key().as_str())
            .is_some_and(|definition| definition.descriptor().id.compatibility_name == module.id())
    });
    let Some(target) = operation else {
        return Err(DarkroomModuleError::MissingOperation {
            module_id: module.id().to_owned(),
        });
    };
    edit.operations()
        .map(|operation| {
            if operation.id() != target.id() {
                return Ok(operation.clone());
            }
            let enabled = match action {
                DarkroomModuleAction::Enable { enabled, .. } => *enabled,
                _ => operation.is_enabled(),
            };
            let parameters = operation
                .parameters()
                .map(|(name, value)| {
                    let control_id = control_parameter_id(module.id(), name.as_str());
                    let replacement = module
                        .controls()
                        .control(&control_id)
                        .and_then(|control| parameter_from_control(control, value));
                    (name.clone(), replacement.unwrap_or_else(|| value.clone()))
                })
                .collect::<Vec<_>>();
            Operation::new_with_opacity(
                operation.id(),
                operation.key().clone(),
                enabled,
                operation.opacity(),
                parameters,
            )
            .map_err(|error| persistence_error(error.to_string()))
        })
        .collect()
}

#[allow(clippy::cast_possible_truncation)]
fn parameter_from_control(
    control: &rusttable_ui::DarkroomControlViewModel,
    existing: &ParameterValue,
) -> Option<ParameterValue> {
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
}
