//! Controller-owned persistence for GTK darkroom module actions.

use std::path::PathBuf;

use rusttable_catalog::EditRepository;
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, FiniteF64, Operation, OperationId, ParameterText, ParameterValue};
use rusttable_processing::builtin_registry;
use rusttable_processing::defringe_compatibility::DefringeMode;
use rusttable_processing::descriptor::OperationFlags;
use rusttable_ui::presentation::{DarkroomControlKind, DarkroomControlValue};
use rusttable_ui::{
    DarkroomModuleAction, DarkroomModuleError, DarkroomModuleViewModel, DarkroomModulesViewModel,
    reference_modules,
};

use rusttable_core::{PhotoId, Revision};
use sha2::{Digest, Sha256};

/// Result published after one darkroom action.
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomEditOutcome {
    revision: Revision,
    modules: DarkroomModulesViewModel,
    processing_changed: bool,
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

    #[must_use]
    pub const fn processing_changed(&self) -> bool {
        self.processing_changed
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
        if matches!(action, DarkroomModuleAction::Disclosure { .. }) {
            let mut modules = self
                .modules
                .clone()
                .ok_or_else(|| persistence_error("darkroom modules were not installed"))?;
            let module = modules
                .module_target_mut(action.module_id(), action.operation_id())
                .ok_or_else(|| DarkroomModuleError::WrongModule {
                    expected: action.module_id().to_owned(),
                    actual: "unknown".to_owned(),
                })?;
            let resolved_action = resolve_action_target(action, module);
            let revision = module.apply(resolved_action)?;
            self.modules = Some(modules.clone());
            return Ok(DarkroomEditOutcome {
                revision,
                modules,
                processing_changed: false,
            });
        }
        if let Some(module) = self
            .modules
            .as_ref()
            .and_then(|modules| modules.module_target(action.module_id(), action.operation_id()))
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
            let projected = project_edit_preserving_disclosure(&current, self.modules.as_ref())?;
            self.modules = Some(projected);
            return Err(DarkroomModuleError::StaleRevision { expected, actual });
        }

        let mut modules = self
            .modules
            .clone()
            .map_or_else(|| project_edit(&current), Ok)?;
        let module = modules
            .module_target_mut(action.module_id(), action.operation_id())
            .ok_or_else(|| DarkroomModuleError::WrongModule {
                expected: action.module_id().to_owned(),
                actual: "unknown".to_owned(),
            })?;
        let resolved_action = resolve_action_target(action, module);
        let revision = module.apply(resolved_action.clone())?;
        if matches!(action, DarkroomModuleAction::Recover { .. }) {
            self.modules = Some(project_edit_preserving_disclosure(
                &current,
                Some(&modules),
            )?);
            let modules = self
                .modules
                .clone()
                .ok_or_else(|| persistence_error("darkroom modules were not installed"))?;
            return Ok(DarkroomEditOutcome {
                revision,
                modules,
                processing_changed: false,
            });
        }

        let operations = rewrite_operations(&current, module, &resolved_action)?;
        let replacement = current
            .revised(operations)
            .map_err(|error| persistence_error(error.to_string()))?;
        let mut repository = self.open_repository()?;
        if let Err(error) = repository.commit_replacement(current.revision(), &replacement) {
            tracing::error!(
                target: "rusttable.gtk.darkroom.edit",
                photo_id = %photo_id,
                edit_id = %current.id(),
                current_revision = %current.revision(),
                expected_revision = %action.expected_revision(),
                requested_revision = %replacement.revision(),
                cause = ?error,
                "darkroom edit persistence failed; keeping the last published edit"
            );
            return Err(persistence_error(error.to_string()));
        }
        let projected = project_edit_preserving_disclosure(&replacement, Some(&modules))?;
        self.modules = Some(projected.clone());
        Ok(DarkroomEditOutcome {
            revision: replacement.revision(),
            modules: projected,
            processing_changed: true,
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
    let templates = reference_modules()?;
    let registry = builtin_registry();
    let templates = templates
        .left_modules()
        .cloned()
        .chain(templates.right_modules().cloned())
        .collect::<Vec<_>>();
    let mut projected = Vec::new();
    for template in templates {
        let definition = registry
            .definitions()
            .iter()
            .find(|definition| definition.descriptor().id.compatibility_name == template.id())
            .expect("registry-backed UI template retains its definition");
        let mut operations = edit
            .operations()
            .filter(|operation| operation_matches_module(operation, template.id()))
            .collect::<Vec<_>>();
        if !definition
            .descriptor()
            .flags
            .contains(OperationFlags::MULTI_INSTANCE)
        {
            operations.truncate(1);
        }
        if operations.is_empty() {
            let mut module = template;
            module.reconcile_operation(edit.revision(), false, [])?;
            projected.push(module);
            continue;
        }
        let instance_count = operations.len();
        for (instance_sequence, operation) in operations.into_iter().enumerate() {
            let mut module = template.clone().with_operation_instance(
                operation.id(),
                instance_sequence,
                instance_count,
            );
            let values = control_values(&module, operation)?;
            let enabled = operation.is_enabled() && module.availability().is_supported();
            module.reconcile_operation(edit.revision(), enabled, values)?;
            projected.push(module);
        }
    }
    DarkroomModulesViewModel::new(projected)
}

fn resolve_action_target(
    action: &DarkroomModuleAction,
    module: &DarkroomModuleViewModel,
) -> DarkroomModuleAction {
    if action.operation_id().is_none() {
        action.clone().with_operation_id(module.operation_id())
    } else {
        action.clone()
    }
}

fn project_edit_preserving_disclosure(
    edit: &Edit,
    previous: Option<&DarkroomModulesViewModel>,
) -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    let mut projected = project_edit(edit)?;
    let Some(previous) = previous else {
        return Ok(projected);
    };
    let module_targets = projected
        .left_modules()
        .map(|module| (module.id().to_owned(), module.operation_id()))
        .chain(
            projected
                .right_modules()
                .map(|module| (module.id().to_owned(), module.operation_id())),
        )
        .collect::<Vec<_>>();
    for (module_id, operation_id) in module_targets {
        let Some(expanded) = previous
            .module_target(&module_id, operation_id)
            .or_else(|| previous.module(&module_id))
            .map(DarkroomModuleViewModel::expanded)
        else {
            continue;
        };
        projected
            .module_target_mut(&module_id, operation_id)
            .expect("projected module target came from the same stack")
            .restore_expanded_presentation(expanded);
    }
    Ok(projected)
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
    if action.module_id() != module.id() {
        return Err(DarkroomModuleError::WrongModule {
            expected: module.id().to_owned(),
            actual: action.module_id().to_owned(),
        });
    }
    if action.operation_id() != module.operation_id() {
        return Err(DarkroomModuleError::WrongOperation {
            module_id: module.id().to_owned(),
            expected: module.operation_id(),
            actual: action.operation_id(),
        });
    }
    let registry = builtin_registry();
    let target = if let Some(operation_id) = module.operation_id() {
        let operation = edit
            .operations()
            .find(|operation| operation.id() == operation_id)
            .ok_or_else(|| DarkroomModuleError::MissingOperation {
                module_id: format!("{} operation {operation_id}", module.id()),
            })?;
        if !operation_matches_module(operation, module.id()) {
            return Err(DarkroomModuleError::WrongOperation {
                module_id: module.id().to_owned(),
                expected: Some(operation_id),
                actual: Some(operation.id()),
            });
        }
        Some(operation)
    } else {
        let mut matches = edit
            .operations()
            .filter(|operation| operation_matches_module(operation, module.id()));
        let target = matches.next();
        if matches.next().is_some() {
            return Err(DarkroomModuleError::WrongOperation {
                module_id: module.id().to_owned(),
                expected: None,
                actual: target.map(Operation::id),
            });
        }
        target
    };
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
        DarkroomModuleAction::Reset { .. } => true,
        _ => operation.is_enabled(),
    };
    let base = if matches!(action, DarkroomModuleAction::Reset { .. }) {
        builtin_registry()
            .materialize_operation(operation.key().as_str(), operation.id())
            .map_err(|error| materialization_error(module.id(), error.to_string()))?
    } else {
        operation.clone()
    };
    let mut parameters = base
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
        base.id(),
        base.key().clone(),
        enabled,
        base.opacity(),
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
        "shadhi",
        "relight",
        "colorcorrection",
        "colorcontrast",
        // Native order keeps Color Contrast immediately before Velvia, with
        // Vibrance as the next operation when it is eventually registered.
        "velvia",
        "bloom",
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

fn operation_matches_module(operation: &Operation, module_id: &str) -> bool {
    builtin_registry()
        .definition(operation.key().as_str())
        .is_some_and(|definition| definition.descriptor().id.compatibility_name == module_id)
}

fn persistence_error(message: impl Into<String>) -> DarkroomModuleError {
    DarkroomModuleError::Persistence {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        sync::atomic::{AtomicU64, Ordering},
    };

    use rusttable_core::{EditId, OperationId, OperationKey, OperationOpacity, ParameterName};

    use super::*;

    static TEST_CATALOG_ID: AtomicU64 = AtomicU64::new(0);

    struct TestCatalog {
        path: PathBuf,
    }

    impl TestCatalog {
        fn seed(edit: &Edit) -> Self {
            let id = TEST_CATALOG_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rusttable-darkroom-edit-reset-{}-{id}.redb",
                std::process::id()
            ));
            let _ = fs::remove_file(&path);
            let mut repository =
                RedbCatalogRepository::open(&path).expect("open reset test catalog");
            repository.commit_new(edit).expect("seed reset test edit");
            drop(repository);
            Self { path }
        }

        fn load(&self, edit_id: EditId) -> Edit {
            RedbCatalogRepository::open(&self.path)
                .expect("reopen reset test catalog")
                .find_by_edit_id(edit_id)
                .expect("read reset test edit")
                .expect("persisted reset test edit")
        }
    }

    impl Drop for TestCatalog {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

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
            operation_id: None,
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

    #[test]
    fn enabling_velvia_materializes_defaults_in_source_relative_order() {
        let registry = builtin_registry();
        let original = Edit::from_parts(
            EditId::new(301).expect("edit id"),
            PhotoId::new(302).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(7),
            [
                registry
                    .materialize_operation(
                        "rusttable.shadhi",
                        OperationId::new(70).expect("shadhi id"),
                    )
                    .expect("shadhi defaults"),
                registry
                    .materialize_operation(
                        "rusttable.relight",
                        OperationId::new(71).expect("relight id"),
                    )
                    .expect("relight defaults"),
                registry
                    .materialize_operation(
                        "rusttable.colorcorrection",
                        OperationId::new(72).expect("color correction id"),
                    )
                    .expect("color correction defaults"),
                registry
                    .materialize_operation(
                        "rusttable.bloom",
                        OperationId::new(73).expect("bloom id"),
                    )
                    .expect("bloom defaults"),
            ],
        )
        .expect("source-relative edit");
        let mut modules = project_edit(&original).expect("projection");
        let velvia = modules.module_mut("velvia").expect("Velvia module");
        let action = DarkroomModuleAction::Enable {
            module_id: "velvia".to_owned(),
            operation_id: None,
            expected_revision: original.revision(),
            enabled: true,
        };
        velvia.apply(action.clone()).expect("enable Velvia");

        let operations =
            rewrite_operations(&original, velvia, &action).expect("materialize Velvia");
        let keys = operations
            .iter()
            .map(|operation| operation.key().as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            [
                "rusttable.shadhi",
                "rusttable.relight",
                "rusttable.colorcorrection",
                "rusttable.velvia",
                "rusttable.bloom"
            ]
        );
        let velvia = &operations[3];
        assert!(velvia.is_enabled());
        assert_eq!(
            velvia.parameter(&ParameterName::new("strength").expect("strength")),
            Some(&scalar(25.0))
        );
        assert_eq!(
            velvia.parameter(&ParameterName::new("bias").expect("bias")),
            Some(&scalar(1.0))
        );
    }

    #[test]
    fn enabling_colorcontrast_materializes_hidden_defaults_immediately_before_velvia() {
        let registry = builtin_registry();
        let original = Edit::from_parts(
            EditId::new(311).expect("edit id"),
            PhotoId::new(312).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(7),
            [
                registry
                    .materialize_operation(
                        "rusttable.colorcorrection",
                        OperationId::new(80).expect("color correction id"),
                    )
                    .expect("color correction defaults"),
                registry
                    .materialize_operation(
                        "rusttable.velvia",
                        OperationId::new(81).expect("Velvia id"),
                    )
                    .expect("Velvia defaults"),
                registry
                    .materialize_operation(
                        "rusttable.bloom",
                        OperationId::new(82).expect("bloom id"),
                    )
                    .expect("bloom defaults"),
            ],
        )
        .expect("source-relative edit");
        let mut modules = project_edit(&original).expect("projection");
        let colorcontrast = modules
            .module_mut("colorcontrast")
            .expect("Color Contrast module");
        let action = DarkroomModuleAction::Enable {
            module_id: "colorcontrast".to_owned(),
            operation_id: None,
            expected_revision: original.revision(),
            enabled: true,
        };
        colorcontrast
            .apply(action.clone())
            .expect("enable Color Contrast");

        let operations = rewrite_operations(&original, colorcontrast, &action)
            .expect("materialize Color Contrast");
        assert_eq!(
            operations
                .iter()
                .map(|operation| operation.key().as_str())
                .collect::<Vec<_>>(),
            [
                "rusttable.colorcorrection",
                "rusttable.colorcontrast",
                "rusttable.velvia",
                "rusttable.bloom",
            ]
        );
        let colorcontrast = &operations[1];
        assert!(colorcontrast.is_enabled());
        assert_eq!(colorcontrast.parameters().count(), 5);
        for (name, expected) in [
            ("a_steepness", 1.0),
            ("a_offset", 0.0),
            ("b_steepness", 1.0),
            ("b_offset", 0.0),
        ] {
            assert_eq!(
                colorcontrast.parameter(&ParameterName::new(name).expect("parameter name")),
                Some(&scalar(expected))
            );
        }
        assert_eq!(
            colorcontrast.parameter(&ParameterName::new("unbound").expect("unbound")),
            Some(&ParameterValue::Integer(1))
        );
    }

    #[test]
    fn colorcontrast_rewrite_preserves_hidden_and_out_of_range_native_values() {
        let original = Edit::from_parts(
            EditId::new(321).expect("edit id"),
            PhotoId::new(322).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(9),
            [Operation::new_with_opacity(
                OperationId::new(323).expect("Color Contrast id"),
                OperationKey::new("rusttable.colorcontrast").expect("Color Contrast key"),
                true,
                OperationOpacity::ONE,
                [
                    (
                        ParameterName::new("a_steepness").expect("a steepness"),
                        scalar(5.5),
                    ),
                    (
                        ParameterName::new("a_offset").expect("a offset"),
                        scalar(0.75),
                    ),
                    (
                        ParameterName::new("b_steepness").expect("b steepness"),
                        scalar(-0.5),
                    ),
                    (
                        ParameterName::new("b_offset").expect("b offset"),
                        scalar(-0.25),
                    ),
                    (
                        ParameterName::new("unbound").expect("unbound"),
                        ParameterValue::Integer(-9),
                    ),
                ],
            )
            .expect("Color Contrast operation")],
        )
        .expect("persisted Color Contrast edit");
        let mut modules = project_edit(&original).expect("finite native values project");
        let colorcontrast = modules
            .module_mut("colorcontrast")
            .expect("Color Contrast module");
        assert_eq!(
            colorcontrast
                .controls()
                .control("colorcontrast-a-steepness")
                .expect("a* steepness")
                .value(),
            DarkroomControlValue::Slider(5.5)
        );
        assert_eq!(
            colorcontrast
                .controls()
                .control("colorcontrast-b-steepness")
                .expect("b* steepness")
                .value(),
            DarkroomControlValue::Slider(-0.5)
        );
        assert!(
            colorcontrast
                .controls()
                .control("colorcontrast-a-offset")
                .is_none()
        );
        assert!(
            colorcontrast
                .controls()
                .control("colorcontrast-b-offset")
                .is_none()
        );
        assert!(
            colorcontrast
                .controls()
                .control("colorcontrast-unbound")
                .is_none()
        );
        let action = DarkroomModuleAction::Enable {
            module_id: "colorcontrast".to_owned(),
            operation_id: Some(OperationId::new(323).expect("Color Contrast id")),
            expected_revision: original.revision(),
            enabled: false,
        };
        colorcontrast
            .apply(action.clone())
            .expect("disable Color Contrast");

        let rewritten = rewrite_operations(&original, colorcontrast, &action)
            .expect("preserve native Color Contrast values");
        let colorcontrast = rewritten.first().expect("Color Contrast operation");
        assert!(!colorcontrast.is_enabled());
        for (name, expected) in [
            ("a_steepness", 5.5),
            ("a_offset", 0.75),
            ("b_steepness", -0.5),
            ("b_offset", -0.25),
        ] {
            assert_eq!(
                colorcontrast.parameter(&ParameterName::new(name).expect("parameter name")),
                Some(&scalar(expected))
            );
        }
        assert_eq!(
            colorcontrast.parameter(&ParameterName::new("unbound").expect("unbound")),
            Some(&ParameterValue::Integer(-9))
        );
    }

    #[test]
    fn two_colorcontrast_instances_project_independently_in_persisted_order() {
        let first_id = OperationId::new(501).expect("first Color Contrast id");
        let second_id = OperationId::new(777).expect("second Color Contrast id");
        let original = Edit::from_parts(
            EditId::new(490).expect("edit id"),
            PhotoId::new(491).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(12),
            [
                colorcontrast_operation(first_id, 1.25, 0.1, 1.5, 0.2, 1, 0.8),
                colorcontrast_operation(second_id, 2.25, -0.3, 2.5, -0.4, 0, 0.6),
            ],
        )
        .expect("multi-instance Color Contrast edit");

        let modules = project_edit(&original).expect("multi-instance projection");
        assert!(
            modules.module("colorcontrast").is_none(),
            "compatibility-only lookup is ambiguous"
        );
        assert!(
            modules.module_target("velvia", Some(second_id)).is_none(),
            "a forged compatibility-name/operation-ID pair must not resolve"
        );
        let instances = modules.instances("colorcontrast").collect::<Vec<_>>();
        assert_eq!(instances.len(), 2);
        assert_eq!(instances[0].operation_id(), Some(first_id));
        assert_eq!(instances[1].operation_id(), Some(second_id));
        assert_eq!(
            instances[0]
                .controls()
                .control("colorcontrast-a-steepness")
                .expect("first a* steepness")
                .value(),
            DarkroomControlValue::Slider(1.25)
        );
        assert_eq!(
            instances[1]
                .controls()
                .control("colorcontrast-a-steepness")
                .expect("second a* steepness")
                .value(),
            DarkroomControlValue::Slider(2.25)
        );
        assert_eq!(
            instances[0].widget_id(),
            format!("colorcontrast-instance-{first_id}")
        );
        assert_eq!(
            instances[1].widget_id(),
            format!("colorcontrast-instance-{second_id}")
        );
    }

    #[test]
    fn reset_targets_second_colorcontrast_and_restores_full_defaults_and_opacity() {
        let first_id = OperationId::new(601).expect("first Color Contrast id");
        let second_id = OperationId::new(602).expect("second Color Contrast id");
        let original = Edit::from_parts(
            EditId::new(590).expect("edit id"),
            PhotoId::new(591).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(18),
            [
                colorcontrast_operation(first_id, 1.2, 0.11, 1.3, 0.12, 0, 0.4),
                colorcontrast_operation(second_id, 3.2, -0.21, 3.3, -0.22, -9, 0.3),
            ],
        )
        .expect("multi-instance Color Contrast edit");
        let before = original.operations().cloned().collect::<Vec<_>>();
        let mut modules = project_edit(&original).expect("multi-instance projection");
        let action = DarkroomModuleAction::Reset {
            module_id: "colorcontrast".to_owned(),
            operation_id: Some(second_id),
            expected_revision: original.revision(),
        };
        let second = modules
            .module_target_mut("colorcontrast", Some(second_id))
            .expect("second Color Contrast target");
        second
            .apply(action.clone())
            .expect("reset second Color Contrast UI model");

        let rewritten =
            rewrite_operations(&original, second, &action).expect("reset exact operation");
        assert_eq!(
            rewritten.iter().map(Operation::id).collect::<Vec<_>>(),
            [first_id, second_id]
        );
        assert_eq!(
            rewritten[0], before[0],
            "the first instance must remain byte-for-byte unchanged"
        );
        let reset = &rewritten[1];
        assert!(reset.is_enabled());
        assert_eq!(reset.opacity(), OperationOpacity::ONE);
        for (name, expected) in [
            ("a_steepness", 1.0),
            ("a_offset", 0.0),
            ("b_steepness", 1.0),
            ("b_offset", 0.0),
        ] {
            assert_eq!(
                reset.parameter(&ParameterName::new(name).expect("parameter name")),
                Some(&scalar(expected))
            );
        }
        assert_eq!(
            reset.parameter(&ParameterName::new("unbound").expect("unbound")),
            Some(&ParameterValue::Integer(1))
        );
    }

    #[test]
    fn controller_reset_enables_existing_disabled_colorcontrast_and_restores_native_state() {
        let registry = builtin_registry();
        let upstream_id = OperationId::new(611).expect("color correction id");
        let target_id = OperationId::new(612).expect("Color Contrast id");
        let downstream_id = OperationId::new(613).expect("Velvia id");
        let original = Edit::from_parts(
            EditId::new(610).expect("edit id"),
            PhotoId::new(610).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(30),
            [
                registry
                    .materialize_operation("rusttable.colorcorrection", upstream_id)
                    .expect("color correction defaults"),
                colorcontrast_operation_with_enabled(
                    target_id, false, 3.2, -0.21, 3.3, -0.22, -9, 0.3,
                ),
                registry
                    .materialize_operation("rusttable.velvia", downstream_id)
                    .expect("Velvia defaults"),
            ],
        )
        .expect("disabled Color Contrast edit");
        let before = original.operations().cloned().collect::<Vec<_>>();
        let catalog = TestCatalog::seed(&original);
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()));
        let selected = controller
            .select_photo(original.photo_id())
            .expect("select disabled Color Contrast");
        assert!(
            !selected
                .module_target("colorcontrast", Some(target_id))
                .expect("persisted Color Contrast module")
                .enabled()
        );

        let outcome = controller
            .apply(&DarkroomModuleAction::Reset {
                module_id: "colorcontrast".to_owned(),
                operation_id: Some(target_id),
                expected_revision: original.revision(),
            })
            .expect("reset existing Color Contrast through production controller");

        assert!(outcome.processing_changed());
        assert_eq!(outcome.revision(), Revision::from_u64(31));
        let projected = outcome
            .modules()
            .module_target("colorcontrast", Some(target_id))
            .expect("reprojected Color Contrast");
        assert!(projected.enabled());
        for id in ["colorcontrast-a-steepness", "colorcontrast-b-steepness"] {
            assert_eq!(
                projected
                    .controls()
                    .control(id)
                    .expect("reset Color Contrast control")
                    .value(),
                DarkroomControlValue::Slider(1.0)
            );
        }

        let persisted = catalog.load(original.id());
        assert_eq!(persisted.revision(), outcome.revision());
        let operations = persisted.operations().collect::<Vec<_>>();
        assert_eq!(
            operations
                .iter()
                .map(|operation| operation.id())
                .collect::<Vec<_>>(),
            [upstream_id, target_id, downstream_id]
        );
        assert_eq!(operations[0], &before[0]);
        assert_eq!(operations[2], &before[2]);
        assert_native_colorcontrast_reset(operations[1], target_id);
    }

    #[test]
    fn controller_reset_materializes_absent_colorcontrast_enabled_in_native_order() {
        let registry = builtin_registry();
        let upstream_id = OperationId::new(621).expect("color correction id");
        let downstream_id = OperationId::new(622).expect("Velvia id");
        let trailing_id = OperationId::new(623).expect("bloom id");
        let original = Edit::from_parts(
            EditId::new(620).expect("edit id"),
            PhotoId::new(620).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(40),
            [
                registry
                    .materialize_operation("rusttable.colorcorrection", upstream_id)
                    .expect("color correction defaults"),
                registry
                    .materialize_operation("rusttable.velvia", downstream_id)
                    .expect("Velvia defaults"),
                registry
                    .materialize_operation("rusttable.bloom", trailing_id)
                    .expect("bloom defaults"),
            ],
        )
        .expect("edit without Color Contrast");
        let target_id = materialized_operation_id(&original, "rusttable.colorcontrast");
        let before = original.operations().cloned().collect::<Vec<_>>();
        let catalog = TestCatalog::seed(&original);
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()));
        let selected = controller
            .select_photo(original.photo_id())
            .expect("select edit without Color Contrast");
        let template = selected
            .module("colorcontrast")
            .expect("absent Color Contrast template");
        assert_eq!(template.operation_id(), None);
        assert!(!template.enabled());

        let outcome = controller
            .apply(&DarkroomModuleAction::Reset {
                module_id: "colorcontrast".to_owned(),
                operation_id: None,
                expected_revision: original.revision(),
            })
            .expect("reset absent Color Contrast through production controller");

        assert!(outcome.processing_changed());
        assert_eq!(outcome.revision(), Revision::from_u64(41));
        let projected = outcome
            .modules()
            .module_target("colorcontrast", Some(target_id))
            .expect("materialized Color Contrast module");
        assert!(projected.enabled());
        let persisted = catalog.load(original.id());
        let operations = persisted.operations().collect::<Vec<_>>();
        assert_eq!(
            operations
                .iter()
                .map(|operation| operation.key().as_str())
                .collect::<Vec<_>>(),
            [
                "rusttable.colorcorrection",
                "rusttable.colorcontrast",
                "rusttable.velvia",
                "rusttable.bloom",
            ]
        );
        assert_eq!(
            operations
                .iter()
                .map(|operation| operation.id())
                .collect::<Vec<_>>(),
            [upstream_id, target_id, downstream_id, trailing_id]
        );
        assert_eq!(operations[0], &before[0]);
        assert_eq!(operations[2], &before[1]);
        assert_eq!(operations[3], &before[2]);
        assert_native_colorcontrast_reset(operations[1], target_id);
    }

    #[test]
    fn multi_instance_disclosure_is_preserved_per_operation_id() {
        let first_id = OperationId::new(701).expect("first Color Contrast id");
        let second_id = OperationId::new(702).expect("second Color Contrast id");
        let original = Edit::from_parts(
            EditId::new(690).expect("edit id"),
            PhotoId::new(691).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(20),
            [
                colorcontrast_operation(first_id, 1.1, 0.0, 1.2, 0.0, 1, 1.0),
                colorcontrast_operation(second_id, 2.1, 0.0, 2.2, 0.0, 1, 1.0),
            ],
        )
        .expect("multi-instance Color Contrast edit");
        let mut modules = project_edit(&original).expect("multi-instance projection");
        modules
            .module_target_mut("colorcontrast", Some(second_id))
            .expect("second Color Contrast target")
            .apply(DarkroomModuleAction::Disclosure {
                module_id: "colorcontrast".to_owned(),
                operation_id: Some(second_id),
                expected_revision: original.revision(),
                expanded: true,
            })
            .expect("expand second instance");
        let replacement = original
            .revised(original.operations().cloned().collect::<Vec<_>>())
            .expect("replacement edit");

        let projected =
            project_edit_preserving_disclosure(&replacement, Some(&modules)).expect("reprojection");
        assert!(
            !projected
                .module_target("colorcontrast", Some(first_id))
                .expect("first instance")
                .expanded()
        );
        assert!(
            projected
                .module_target("colorcontrast", Some(second_id))
                .expect("second instance")
                .expanded()
        );
    }

    fn colorcontrast_operation(
        id: OperationId,
        a_steepness: f64,
        a_offset: f64,
        b_steepness: f64,
        b_offset: f64,
        unbound: i64,
        opacity: f64,
    ) -> Operation {
        colorcontrast_operation_with_enabled(
            id,
            true,
            a_steepness,
            a_offset,
            b_steepness,
            b_offset,
            unbound,
            opacity,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn colorcontrast_operation_with_enabled(
        id: OperationId,
        enabled: bool,
        a_steepness: f64,
        a_offset: f64,
        b_steepness: f64,
        b_offset: f64,
        unbound: i64,
        opacity: f64,
    ) -> Operation {
        Operation::new_with_opacity(
            id,
            OperationKey::new("rusttable.colorcontrast").expect("Color Contrast key"),
            enabled,
            OperationOpacity::new(opacity).expect("valid opacity"),
            [
                (
                    ParameterName::new("a_steepness").expect("a steepness"),
                    scalar(a_steepness),
                ),
                (
                    ParameterName::new("a_offset").expect("a offset"),
                    scalar(a_offset),
                ),
                (
                    ParameterName::new("b_steepness").expect("b steepness"),
                    scalar(b_steepness),
                ),
                (
                    ParameterName::new("b_offset").expect("b offset"),
                    scalar(b_offset),
                ),
                (
                    ParameterName::new("unbound").expect("unbound"),
                    ParameterValue::Integer(unbound),
                ),
            ],
        )
        .expect("Color Contrast operation")
    }

    fn assert_native_colorcontrast_reset(operation: &Operation, expected_id: OperationId) {
        assert_eq!(operation.id(), expected_id);
        assert_eq!(operation.key().as_str(), "rusttable.colorcontrast");
        assert!(operation.is_enabled());
        assert_eq!(operation.opacity(), OperationOpacity::ONE);
        assert_eq!(operation.parameters().count(), 5);
        for (name, expected) in [
            ("a_steepness", 1.0),
            ("a_offset", 0.0),
            ("b_steepness", 1.0),
            ("b_offset", 0.0),
        ] {
            assert_eq!(
                operation.parameter(&ParameterName::new(name).expect("parameter name")),
                Some(&scalar(expected))
            );
        }
        assert_eq!(
            operation.parameter(&ParameterName::new("unbound").expect("unbound")),
            Some(&ParameterValue::Integer(1))
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
    fn disclosure_of_absent_velvia_is_presentation_only() {
        let modules = project_edit(&edit(4, 0.0, true)).expect("projection");
        let mut controller = GtkDarkroomEditController {
            catalog_path: None,
            selected_photo: Some(PhotoId::new(2).expect("photo id")),
            modules: Some(modules),
        };

        let outcome = controller
            .apply(&DarkroomModuleAction::Disclosure {
                module_id: "velvia".to_owned(),
                operation_id: None,
                expected_revision: Revision::from_u64(4),
                expanded: true,
            })
            .expect("presentation-only disclosure needs no catalog write");

        assert_eq!(outcome.revision(), Revision::from_u64(4));
        assert!(!outcome.processing_changed());
        let velvia = outcome.modules().module("velvia").expect("Velvia module");
        assert!(velvia.expanded());
        assert!(!velvia.enabled());
    }

    #[test]
    fn replacement_projection_preserves_velvia_disclosure_state() {
        let original = edit(4, 0.0, true);
        let mut modules = project_edit(&original).expect("projection");
        modules
            .module_mut("velvia")
            .expect("Velvia module")
            .apply(DarkroomModuleAction::Disclosure {
                module_id: "velvia".to_owned(),
                operation_id: None,
                expected_revision: original.revision(),
                expanded: true,
            })
            .expect("expand Velvia");
        let replacement = original
            .revised(original.operations().cloned().collect::<Vec<_>>())
            .expect("processing replacement");

        let projected =
            project_edit_preserving_disclosure(&replacement, Some(&modules)).expect("reprojection");
        let velvia = projected.module("velvia").expect("Velvia module");
        assert!(velvia.expanded());
        assert!(!velvia.enabled());
        assert_eq!(velvia.revision(), replacement.revision());
    }

    #[test]
    fn persisted_velvia_outside_ui_bounds_projects_and_survives_enable_action() {
        let original = Edit::from_parts(
            EditId::new(401).expect("edit id"),
            PhotoId::new(402).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(9),
            [Operation::new_with_opacity(
                OperationId::new(403).expect("Velvia id"),
                OperationKey::new("rusttable.velvia").expect("Velvia key"),
                true,
                OperationOpacity::ONE,
                [
                    (
                        ParameterName::new("strength").expect("strength"),
                        scalar(101.0),
                    ),
                    (ParameterName::new("bias").expect("bias"), scalar(-0.01)),
                ],
            )
            .expect("Velvia operation")],
        )
        .expect("persisted Velvia edit");
        let mut modules = project_edit(&original).expect("finite native values project");
        let velvia = modules.module_mut("velvia").expect("Velvia module");
        assert_eq!(
            velvia
                .controls()
                .control("velvia-strength")
                .expect("strength")
                .value(),
            DarkroomControlValue::Slider(101.0)
        );
        assert_eq!(
            velvia
                .controls()
                .control("velvia-bias")
                .expect("bias")
                .value(),
            DarkroomControlValue::Slider(-0.01)
        );
        let action = DarkroomModuleAction::Enable {
            module_id: "velvia".to_owned(),
            operation_id: Some(OperationId::new(403).expect("Velvia id")),
            expected_revision: original.revision(),
            enabled: false,
        };
        velvia.apply(action.clone()).expect("disable Velvia");

        let rewritten =
            rewrite_operations(&original, velvia, &action).expect("preserve native values");
        let velvia = rewritten.first().expect("Velvia operation");
        assert!(!velvia.is_enabled());
        assert_eq!(
            velvia.parameter(&ParameterName::new("strength").expect("strength")),
            Some(&scalar(101.0))
        );
        assert_eq!(
            velvia.parameter(&ParameterName::new("bias").expect("bias")),
            Some(&scalar(-0.01))
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
                operation_id: Some(OperationId::new(3).expect("exposure id")),
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
                operation_id: Some(OperationId::new(3).expect("exposure id")),
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
                operation_id: Some(OperationId::new(9).expect("bloom id")),
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
                operation_id: Some(OperationId::new(9).expect("bloom id")),
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
                    operation_id: Some(
                        OperationId::new(30 + u128::try_from(mode).expect("mode"))
                            .expect("operation id"),
                    ),
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
                operation_id: Some(OperationId::new(41).expect("operation id")),
                expected_revision: original.revision(),
                id: "clahe-radius".to_owned(),
                value: DarkroomControlValue::Slider(64.0),
            })
            .expect("qualified backend accepts actions");
    }
}
