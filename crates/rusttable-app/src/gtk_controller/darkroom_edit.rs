//! Controller-owned persistence for GTK darkroom module actions.

use std::path::PathBuf;

use rusttable_catalog::{EditRepository, EditRepositoryError};
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, FiniteF64, Operation, OperationId, ParameterText, ParameterValue};
use rusttable_processing::builtin_registry;
use rusttable_processing::defringe_compatibility::DefringeMode;
use rusttable_processing::descriptor::OperationFlags;
use rusttable_ui::iop::bloom::{BLOOM_MODULE_ID, BloomEditorState, BloomGtkState};
use rusttable_ui::iop::colorcorrection::{COLORCORRECTION_MODULE_ID, ColorCorrectionGridState};
use rusttable_ui::iop::colorzones::{COLORZONES_MODULE_ID, ColorZonesGtkState};
use rusttable_ui::presentation::{DarkroomControlKind, DarkroomControlValue};
use rusttable_ui::{
    DarkroomModuleAction, DarkroomModuleError, DarkroomModuleViewModel, DarkroomModulesViewModel,
    reference_modules,
};

use rusttable_core::{PhotoId, Revision};
use sha2::{Digest, Sha256};

use super::colorzones_edit::{
    ColorZonesEditAction, ColorZonesEditError, ColorZonesGuiPreferences, apply_colorzones_edit,
    colorzones_snapshots, reconcile_colorzones_snapshot,
};

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
    colorzones_gui_preferences: ColorZonesGuiPreferences,
}

impl GtkDarkroomEditController {
    #[must_use]
    pub fn new(catalog_path: Option<PathBuf>) -> Self {
        Self {
            catalog_path,
            selected_photo: None,
            modules: None,
            colorzones_gui_preferences: ColorZonesGuiPreferences::default(),
        }
    }

    #[must_use]
    pub fn with_colorzones_gui_preferences(
        mut self,
        preferences: ColorZonesGuiPreferences,
    ) -> Self {
        self.colorzones_gui_preferences = preferences;
        self
    }

    #[must_use]
    pub const fn colorzones_gui_preferences(&self) -> ColorZonesGuiPreferences {
        self.colorzones_gui_preferences
    }

    #[must_use]
    pub const fn selected_photo(&self) -> Option<PhotoId> {
        self.selected_photo
    }

    #[must_use]
    pub fn modules(&self) -> Option<&DarkroomModulesViewModel> {
        self.modules.as_ref()
    }

    /// Loads exact persisted Color Zones instances, or the current default
    /// snapshot when the selected edit has no instance.
    ///
    /// # Errors
    ///
    /// Returns a shared darkroom error when no photo is selected, persistence
    /// fails, or canonical Color Zones parameters are invalid.
    pub fn colorzones_snapshots(&self) -> Result<Vec<ColorZonesGtkState>, DarkroomModuleError> {
        let photo_id = self
            .selected_photo
            .ok_or(DarkroomModuleError::NoSelection)?;
        let edit = self.load_edit(photo_id)?;
        colorzones_snapshots(&edit, self.colorzones_gui_preferences).map_err(colorzones_edit_error)
    }

    /// Reloads controller truth for a mounted Color Zones leaf after rejection.
    /// The rejected action is never replayed.
    ///
    /// # Errors
    ///
    /// Returns a shared darkroom error when current persisted state cannot be
    /// loaded or projected.
    pub fn reconcile_colorzones_snapshot(
        &self,
        preferred_target: OperationId,
    ) -> Result<ColorZonesGtkState, DarkroomModuleError> {
        let photo_id = self
            .selected_photo
            .ok_or(DarkroomModuleError::NoSelection)?;
        let edit = self.load_edit(photo_id)?;
        reconcile_colorzones_snapshot(&edit, preferred_target, self.colorzones_gui_preferences)
            .map_err(colorzones_edit_error)
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
        let modules = project_edit_with_preferences(&edit, self.colorzones_gui_preferences)?;
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

    /// Updates durable Color Zones presentation state and rebuilds controller
    /// projections without creating or revising an image edit.
    ///
    /// # Errors
    ///
    /// Returns a shared darkroom error if the selected edit cannot be reloaded.
    pub fn update_colorzones_gui_preferences(
        &mut self,
        preferences: ColorZonesGuiPreferences,
    ) -> Result<DarkroomEditOutcome, DarkroomModuleError> {
        let photo_id = self
            .selected_photo
            .ok_or(DarkroomModuleError::NoSelection)?;
        let edit = self.load_edit(photo_id)?;
        let projected = project_edit_preserving_disclosure_with_preferences(
            &edit,
            self.modules.as_ref(),
            preferences,
        )?;
        self.colorzones_gui_preferences = preferences;
        self.modules = Some(projected.clone());
        Ok(DarkroomEditOutcome {
            revision: edit.revision(),
            modules: projected,
            processing_changed: false,
        })
    }

    /// Applies one GTK action through the selected edit's atomic repository transaction.
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
            let projected = project_edit_preserving_disclosure_with_preferences(
                &current,
                self.modules.as_ref(),
                self.colorzones_gui_preferences,
            )?;
            self.modules = Some(projected);
            return Err(DarkroomModuleError::StaleRevision { expected, actual });
        }

        let mut modules = self.modules.clone().map_or_else(
            || project_edit_with_preferences(&current, self.colorzones_gui_preferences),
            Ok,
        )?;
        let module = modules
            .module_target_mut(action.module_id(), action.operation_id())
            .ok_or_else(|| DarkroomModuleError::WrongModule {
                expected: action.module_id().to_owned(),
                actual: "unknown".to_owned(),
            })?;
        let resolved_action = resolve_action_target(action, module);
        let revision = module.apply(resolved_action.clone())?;
        if matches!(action, DarkroomModuleAction::Recover { .. }) {
            self.modules = Some(project_edit_preserving_disclosure_with_preferences(
                &current,
                Some(&modules),
                self.colorzones_gui_preferences,
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
        let projected = project_edit_preserving_disclosure_with_preferences(
            &replacement,
            Some(&modules),
            self.colorzones_gui_preferences,
        )?;
        self.modules = Some(projected.clone());
        Ok(DarkroomEditOutcome {
            revision: replacement.revision(),
            modules: projected,
            processing_changed: true,
        })
    }

    /// Applies one source-shaped Color Zones editor action through the selected
    /// edit's atomic repository transaction.
    ///
    /// The generic GTK module remains hidden until its source-derived widget is
    /// ported; this routing seam exists for that editor without exposing generic
    /// descriptor controls or decoding opaque imported history rows.
    ///
    /// # Errors
    ///
    /// Returns a shared darkroom module error for stale revisions, invalid exact
    /// targets, checked editor mutations, projection, or persistence failures.
    pub fn apply_colorzones(
        &mut self,
        action: &ColorZonesEditAction,
    ) -> Result<DarkroomEditOutcome, DarkroomModuleError> {
        let photo_id = self
            .selected_photo
            .ok_or(DarkroomModuleError::NoSelection)?;
        let current = self.load_edit(photo_id)?;
        let applied = match apply_colorzones_edit(&current, action) {
            Ok(applied) => applied,
            Err(ColorZonesEditError::StaleRevision { expected, actual }) => {
                let projected = project_edit_preserving_disclosure_with_preferences(
                    &current,
                    self.modules.as_ref(),
                    self.colorzones_gui_preferences,
                )?;
                self.modules = Some(projected);
                return Err(DarkroomModuleError::StaleRevision { expected, actual });
            }
            Err(error) => return Err(colorzones_edit_error(error)),
        };
        if !applied.changed() {
            let projected = project_edit_preserving_disclosure_with_preferences(
                &current,
                self.modules.as_ref(),
                self.colorzones_gui_preferences,
            )?;
            self.modules = Some(projected.clone());
            return Ok(DarkroomEditOutcome {
                revision: current.revision(),
                modules: projected,
                processing_changed: false,
            });
        }
        let replacement = applied.into_edit();
        let mut repository = self.open_repository()?;
        if let Err(error) = repository.commit_replacement(current.revision(), &replacement) {
            tracing::error!(
                target: "rusttable.gtk.darkroom.colorzones.edit",
                photo_id = %photo_id,
                edit_id = %current.id(),
                current_revision = %current.revision(),
                requested_revision = %replacement.revision(),
                cause = ?error,
                "Color Zones edit persistence failed; keeping the last published edit"
            );
            if matches!(&error, EditRepositoryError::EditRevisionConflict { .. }) {
                drop(repository);
                let latest = self.load_edit(photo_id)?;
                let projected = project_edit_preserving_disclosure_with_preferences(
                    &latest,
                    self.modules.as_ref(),
                    self.colorzones_gui_preferences,
                )?;
                self.modules = Some(projected);
                return Err(DarkroomModuleError::StaleRevision {
                    expected: action.expected_revision(),
                    actual: latest.revision(),
                });
            }
            return Err(persistence_error(error.to_string()));
        }
        let projected = project_edit_preserving_disclosure_with_preferences(
            &replacement,
            self.modules.as_ref(),
            self.colorzones_gui_preferences,
        )?;
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

#[cfg(test)]
fn project_edit(edit: &Edit) -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    project_edit_with_preferences(edit, ColorZonesGuiPreferences::default())
}

fn project_edit_with_preferences(
    edit: &Edit,
    colorzones_preferences: ColorZonesGuiPreferences,
) -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    let templates = reference_modules()?;
    let registry = builtin_registry();
    let templates = templates
        .left_modules()
        .cloned()
        .chain(templates.right_modules().cloned())
        .collect::<Vec<_>>();
    let mut projected = Vec::new();
    for template in templates {
        if template.id() == BLOOM_MODULE_ID {
            let operations = edit
                .operations()
                .filter(|operation| operation_matches_module(operation, BLOOM_MODULE_ID))
                .collect::<Vec<_>>();
            if operations.is_empty() {
                let state = BloomGtkState::new(
                    materialized_operation_id(edit, "rusttable.bloom"),
                    edit.revision(),
                    BloomEditorState::default(),
                    false,
                    template.availability().is_supported(),
                    true,
                );
                projected.push(template.with_bloom_editor_state(state));
                continue;
            }
            let instance_count = operations.len();
            for (instance_sequence, operation) in operations.into_iter().enumerate() {
                let parameters = bloom_parameters(operation)?;
                let state = BloomGtkState::new(
                    operation.id(),
                    edit.revision(),
                    BloomEditorState::new(parameters)
                        .map_err(|error| persistence_error(error.to_string()))?,
                    operation.is_enabled() && template.availability().is_supported(),
                    template.availability().is_supported(),
                    false,
                );
                projected.push(
                    template
                        .clone()
                        .with_operation_instance(operation.id(), instance_sequence, instance_count)
                        .with_bloom_editor_state(state),
                );
            }
            continue;
        }
        if template.id() == COLORZONES_MODULE_ID {
            let states = colorzones_snapshots(edit, colorzones_preferences)
                .map_err(colorzones_edit_error)?;
            let instance_count = states.len();
            for (instance_sequence, state) in states.into_iter().enumerate() {
                let module = if state.materialization_required() {
                    template.clone()
                } else {
                    template.clone().with_operation_instance(
                        state.operation_id(),
                        instance_sequence,
                        instance_count,
                    )
                };
                projected.push(module.with_colorzones_editor_state(state));
            }
            continue;
        }
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
            if module.color_correction_grid().is_some() {
                module.reconcile_color_correction_grid(
                    edit.revision(),
                    color_correction_grid_from_operation(operation)?,
                )?;
            }
            projected.push(module);
        }
    }
    DarkroomModulesViewModel::new(projected)
}

fn bloom_parameters(
    operation: &Operation,
) -> Result<rusttable_processing::operations::bloom::BloomParametersV1, DarkroomModuleError> {
    let compiled = rusttable_processing::ProcessingOperation::compile(operation)
        .map_err(|error| persistence_error(error.to_string()))?;
    let rusttable_processing::ProcessingOperationKind::Bloom { config } = compiled.kind() else {
        return Err(persistence_error(
            "Bloom operation compiled to a different kind",
        ));
    };
    Ok(
        rusttable_processing::operations::bloom::BloomParametersV1::new(
            config.size(),
            config.threshold(),
            config.strength(),
        ),
    )
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

#[cfg(test)]
fn project_edit_preserving_disclosure(
    edit: &Edit,
    previous: Option<&DarkroomModulesViewModel>,
) -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    let preferences = previous
        .and_then(|modules| {
            modules
                .left_modules()
                .chain(modules.right_modules())
                .find_map(DarkroomModuleViewModel::colorzones_editor_state)
        })
        .map_or_else(ColorZonesGuiPreferences::default, |state| {
            ColorZonesGuiPreferences::new(state.editor().output_channel(), state.graph_height())
        });
    project_edit_preserving_disclosure_with_preferences(edit, previous, preferences)
}

fn project_edit_preserving_disclosure_with_preferences(
    edit: &Edit,
    previous: Option<&DarkroomModulesViewModel>,
    preferences: ColorZonesGuiPreferences,
) -> Result<DarkroomModulesViewModel, DarkroomModuleError> {
    let mut projected = project_edit_with_preferences(edit, preferences)?;
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

fn color_correction_grid_from_operation(
    operation: &Operation,
) -> Result<ColorCorrectionGridState, DarkroomModuleError> {
    ColorCorrectionGridState::new(
        color_correction_parameter(operation, "hia")?,
        color_correction_parameter(operation, "hib")?,
        color_correction_parameter(operation, "loa")?,
        color_correction_parameter(operation, "lob")?,
    )
    .map_err(|error| persistence_error(error.to_string()))
}

fn color_correction_parameter(
    operation: &Operation,
    name: &str,
) -> Result<f64, DarkroomModuleError> {
    let name = rusttable_core::ParameterName::new(name).map_err(|error| {
        persistence_error(format!(
            "invalid Color Correction parameter name {name}: {error:?}"
        ))
    })?;
    match operation.parameter(&name) {
        Some(ParameterValue::Scalar(value)) => Ok(value.get()),
        None => Ok(0.0),
        Some(_) => Err(persistence_error(format!(
            "Color Correction parameter {name} must be scalar"
        ))),
    }
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
    if action.is_instance_lifecycle() {
        return rewrite_instance_operations(edit, module, action);
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

fn rewrite_instance_operations(
    edit: &Edit,
    module: &DarkroomModuleViewModel,
    action: &DarkroomModuleAction,
) -> Result<Vec<Operation>, DarkroomModuleError> {
    let operation_id =
        action
            .operation_id()
            .ok_or_else(|| DarkroomModuleError::InstanceActionUnavailable {
                module_id: module.id().to_owned(),
                action: instance_action_name(action),
                reason: "an exact persisted operation target is required",
            })?;
    let definition = builtin_registry()
        .definitions()
        .iter()
        .find(|definition| definition.descriptor().id.compatibility_name == module.id())
        .ok_or_else(|| DarkroomModuleError::MissingOperation {
            module_id: module.id().to_owned(),
        })?;
    if !definition
        .descriptor()
        .flags
        .contains(OperationFlags::MULTI_INSTANCE)
    {
        return Err(DarkroomModuleError::InstanceActionUnavailable {
            module_id: module.id().to_owned(),
            action: instance_action_name(action),
            reason: "the operation is single-instance",
        });
    }

    let mut operations = edit.operations().cloned().collect::<Vec<_>>();
    let target_index = operations
        .iter()
        .position(|operation| operation.id() == operation_id)
        .ok_or_else(|| DarkroomModuleError::MissingOperation {
            module_id: format!("{} operation {operation_id}", module.id()),
        })?;
    if !operation_matches_module(&operations[target_index], module.id()) {
        return Err(DarkroomModuleError::WrongOperation {
            module_id: module.id().to_owned(),
            expected: Some(operation_id),
            actual: Some(operations[target_index].id()),
        });
    }
    let target_key = operations[target_index].key().clone();

    match action {
        DarkroomModuleAction::NewInstance { .. } => {
            let new_id =
                multi_instance_operation_id(edit, operation_id, target_key.as_str(), "new");
            let operation = builtin_registry()
                .materialize_operation(target_key.as_str(), new_id)
                .map_err(|error| materialization_error(module.id(), error.to_string()))?;
            operations.insert(target_index + 1, operation);
        }
        DarkroomModuleAction::DuplicateInstance { .. } => {
            let new_id =
                multi_instance_operation_id(edit, operation_id, target_key.as_str(), "duplicate");
            let completed = complete_operation_defaults(&operations[target_index])
                .map_err(|error| materialization_error(module.id(), error.to_string()))?;
            let duplicate = Operation::new_with_opacity(
                new_id,
                completed.key().clone(),
                true,
                completed.opacity(),
                completed
                    .parameters()
                    .map(|(name, value)| (name.clone(), value.clone())),
            )
            .map_err(|error| persistence_error(error.to_string()))?;
            operations.insert(target_index + 1, duplicate);
        }
        DarkroomModuleAction::MoveInstanceUp { .. } => {
            let previous = (0..target_index)
                .rev()
                .find(|index| operations[*index].key() == &target_key)
                .ok_or_else(|| DarkroomModuleError::InstanceActionUnavailable {
                    module_id: module.id().to_owned(),
                    action: "move up",
                    reason: "the instance is already first",
                })?;
            operations.swap(previous, target_index);
        }
        DarkroomModuleAction::MoveInstanceDown { .. } => {
            let next = ((target_index + 1)..operations.len())
                .find(|index| operations[*index].key() == &target_key)
                .ok_or_else(|| DarkroomModuleError::InstanceActionUnavailable {
                    module_id: module.id().to_owned(),
                    action: "move down",
                    reason: "the instance is already last",
                })?;
            operations.swap(target_index, next);
        }
        DarkroomModuleAction::DeleteInstance { .. } => {
            let instance_count = operations
                .iter()
                .filter(|operation| operation.key() == &target_key)
                .count();
            if instance_count <= 1 {
                return Err(DarkroomModuleError::InstanceActionUnavailable {
                    module_id: module.id().to_owned(),
                    action: "delete",
                    reason: "the final instance cannot be deleted",
                });
            }
            operations.remove(target_index);
        }
        DarkroomModuleAction::Disclosure { .. }
        | DarkroomModuleAction::Enable { .. }
        | DarkroomModuleAction::Reset { .. }
        | DarkroomModuleAction::Preset { .. }
        | DarkroomModuleAction::Control { .. }
        | DarkroomModuleAction::BloomSettled { .. }
        | DarkroomModuleAction::ColorCorrectionGrid { .. }
        | DarkroomModuleAction::ColorCorrectionResetParameters { .. }
        | DarkroomModuleAction::Recover { .. } => {
            return Err(persistence_error(
                "non-instance action reached the instance rewrite path",
            ));
        }
    }
    Ok(operations)
}

fn instance_action_name(action: &DarkroomModuleAction) -> &'static str {
    match action {
        DarkroomModuleAction::NewInstance { .. } => "new instance",
        DarkroomModuleAction::DuplicateInstance { .. } => "duplicate instance",
        DarkroomModuleAction::MoveInstanceUp { .. } => "move up",
        DarkroomModuleAction::MoveInstanceDown { .. } => "move down",
        DarkroomModuleAction::DeleteInstance { .. } => "delete",
        DarkroomModuleAction::Disclosure { .. }
        | DarkroomModuleAction::Enable { .. }
        | DarkroomModuleAction::Reset { .. }
        | DarkroomModuleAction::Preset { .. }
        | DarkroomModuleAction::Control { .. }
        | DarkroomModuleAction::BloomSettled { .. }
        | DarkroomModuleAction::ColorCorrectionGrid { .. }
        | DarkroomModuleAction::ColorCorrectionResetParameters { .. }
        | DarkroomModuleAction::Recover { .. } => "instance action",
    }
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
        DarkroomModuleAction::Preset { .. } | DarkroomModuleAction::BloomSettled { .. } => {
            module.enabled()
        }
        DarkroomModuleAction::ColorCorrectionGrid { .. }
        | DarkroomModuleAction::ColorCorrectionResetParameters { .. }
        | DarkroomModuleAction::Control { .. }
            if module.id() == COLORCORRECTION_MODULE_ID =>
        {
            module.enabled()
        }
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
    if let DarkroomModuleAction::BloomSettled {
        parameters: bloom, ..
    } = action
    {
        for (name, replacement) in [
            ("size", bloom.size),
            ("threshold", bloom.threshold),
            ("strength", bloom.strength),
        ] {
            let Some((_, value)) = parameters
                .iter_mut()
                .find(|(parameter, _)| parameter.as_str() == name)
            else {
                return Err(persistence_error(format!(
                    "Bloom operation is missing {name}"
                )));
            };
            *value =
                ParameterValue::Scalar(FiniteF64::new(f64::from(replacement)).map_err(|_| {
                    persistence_error(format!("Bloom parameter {name} must be finite"))
                })?);
        }
    }
    if let Some(grid) = module.color_correction_grid() {
        for (name, replacement) in [
            ("hia", grid.hia()),
            ("hib", grid.hib()),
            ("loa", grid.loa()),
            ("lob", grid.lob()),
        ] {
            let Some((_, value)) = parameters
                .iter_mut()
                .find(|(parameter, _)| parameter.as_str() == name)
            else {
                return Err(persistence_error(format!(
                    "Color Correction operation is missing {name}"
                )));
            };
            *value = ParameterValue::Scalar(FiniteF64::new(replacement).map_err(|_| {
                persistence_error(format!("Color Correction parameter {name} must be finite"))
            })?);
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

fn multi_instance_operation_id(
    edit: &Edit,
    base_id: OperationId,
    key: &str,
    action: &str,
) -> OperationId {
    for nonce in 0_u64.. {
        let mut digest = Sha256::new();
        digest.update(b"rusttable.darkroom.multi-instance-operation.v1\0");
        digest.update(edit.id().get().to_be_bytes());
        digest.update(edit.photo_id().get().to_be_bytes());
        digest.update(edit.revision().get().to_be_bytes());
        digest.update(base_id.get().to_be_bytes());
        digest.update(key.as_bytes());
        digest.update([0]);
        digest.update(action.as_bytes());
        digest.update(nonce.to_be_bytes());
        let bytes = digest.finalize();
        let mut id_bytes = [0_u8; 16];
        id_bytes.copy_from_slice(&bytes[..16]);
        let id = u128::from_be_bytes(id_bytes);
        let candidate = OperationId::new(if id == 0 { 1 } else { id })
            .expect("derived multi-instance operation ID is nonzero");
        if edit
            .operations()
            .all(|operation| operation.id() != candidate)
        {
            return candidate;
        }
    }
    unreachable!("the operation ID space cannot be exhausted by a finite edit")
}

// Source pipeline order. Color Zones is immediately after Vibrance and before Bloom.
pub(super) const DARKROOM_CANONICAL_ORDER: &[&str] = &[
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
    "velvia",
    "vibrance",
    "colorzones",
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

pub(super) fn canonical_rank(operation: &Operation) -> usize {
    let name = builtin_registry()
        .definition(operation.key().as_str())
        .map_or(operation.key().as_str(), |definition| {
            definition.descriptor().id.compatibility_name.as_str()
        });
    DARKROOM_CANONICAL_ORDER
        .iter()
        .position(|candidate| *candidate == name)
        .unwrap_or(DARKROOM_CANONICAL_ORDER.len())
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

fn colorzones_edit_error(error: ColorZonesEditError) -> DarkroomModuleError {
    match error {
        ColorZonesEditError::StaleRevision { expected, actual } => {
            DarkroomModuleError::StaleRevision { expected, actual }
        }
        ColorZonesEditError::MissingOperation(operation_id) => {
            DarkroomModuleError::MissingOperation {
                module_id: format!("colorzones operation {operation_id}"),
            }
        }
        ColorZonesEditError::WrongOperation(operation_id) => DarkroomModuleError::WrongOperation {
            module_id: "colorzones".to_owned(),
            expected: Some(operation_id),
            actual: None,
        },
        ColorZonesEditError::InvalidCanonicalOperation(message)
        | ColorZonesEditError::Revision(message) => persistence_error(message),
        error => DarkroomModuleError::Unsupported {
            module_id: "colorzones".to_owned(),
            reason: error.to_string(),
        },
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
    use rusttable_processing::{ColorZonesChannel, ColorZonesMode};

    use super::super::colorzones_edit::ColorZonesEditAction;
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

    #[test]
    fn colorzones_gui_preferences_rebuild_without_advancing_edit_revision() {
        let operation = builtin_registry()
            .materialize_operation(
                "rusttable.colorzones",
                OperationId::new(0xc700).expect("Color Zones ID"),
            )
            .expect("Color Zones defaults");
        let original = Edit::from_parts(
            EditId::new(0xc701).expect("edit ID"),
            PhotoId::new(0xc702).expect("photo ID"),
            Revision::ZERO,
            Revision::from_u64(4),
            [operation],
        )
        .expect("Color Zones edit");
        let catalog = TestCatalog::seed(&original);
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()));
        controller
            .select_photo(original.photo_id())
            .expect("select Color Zones edit");
        let preferences = ColorZonesGuiPreferences::new(
            ColorZonesChannel::Hue,
            rusttable_ui::iop::colorzones::ColorZonesGraphHeight::new(233).expect("graph height"),
        );

        let outcome = controller
            .update_colorzones_gui_preferences(preferences)
            .expect("rebuild GUI projection");
        let state = outcome
            .modules()
            .module("colorzones")
            .and_then(DarkroomModuleViewModel::colorzones_editor_state)
            .expect("Color Zones state");

        assert!(!outcome.processing_changed());
        assert_eq!(outcome.revision(), original.revision());
        assert_eq!(state.editor().output_channel(), ColorZonesChannel::Hue);
        assert_eq!(state.graph_height().logical_pixels(), 233);
        assert_eq!(controller.colorzones_gui_preferences(), preferences);
        assert_eq!(catalog.load(original.id()), original);
    }

    #[test]
    fn colorzones_action_persists_only_the_exact_mounted_instance() {
        let registry = builtin_registry();
        let first_id = OperationId::new(0xc701).expect("first Color Zones ID");
        let second_id = OperationId::new(0xc702).expect("second Color Zones ID");
        let first = registry
            .materialize_operation("rusttable.colorzones", first_id)
            .expect("first Color Zones defaults");
        let second_defaults = registry
            .materialize_operation("rusttable.colorzones", second_id)
            .expect("second Color Zones defaults");
        let second_opacity = OperationOpacity::new(0.375).expect("partial opacity");
        let second = Operation::new_with_opacity(
            second_defaults.id(),
            second_defaults.key().clone(),
            false,
            second_opacity,
            second_defaults
                .parameters()
                .map(|(name, value)| (name.clone(), value.clone())),
        )
        .expect("disabled second Color Zones instance");
        let original = Edit::from_parts(
            EditId::new(0xc703).expect("edit ID"),
            PhotoId::new(0xc704).expect("photo ID"),
            Revision::ZERO,
            Revision::from_u64(9),
            [first.clone(), second.clone()],
        )
        .expect("multi-instance Color Zones edit");
        let catalog = TestCatalog::seed(&original);
        let preferences = ColorZonesGuiPreferences::new(
            ColorZonesChannel::Hue,
            rusttable_ui::iop::colorzones::ColorZonesGraphHeight::new(233).expect("graph height"),
        );
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()))
            .with_colorzones_gui_preferences(preferences);
        controller
            .select_photo(original.photo_id())
            .expect("select Color Zones edit");
        let state = controller
            .colorzones_snapshots()
            .expect("Color Zones snapshots")
            .into_iter()
            .find(|state| state.operation_id() == second_id)
            .expect("second Color Zones snapshot");
        let mut parameters = state.editor().parameters_value();
        parameters.mode = ColorZonesMode::Strong.raw();
        let action = ColorZonesEditAction::new(
            state.operation_id(),
            state.revision(),
            parameters,
            !state.enabled(),
            state.materialization_required(),
        );

        let outcome = controller
            .apply_colorzones(&action)
            .expect("persist exact Color Zones action");
        let persisted = catalog.load(original.id());
        let operations = persisted.operations().collect::<Vec<_>>();

        assert_eq!(outcome.revision(), persisted.revision());
        assert!(outcome.processing_changed());
        let projected = outcome
            .modules()
            .instances("colorzones")
            .collect::<Vec<_>>();
        assert_eq!(projected.len(), 2);
        assert!(projected.iter().all(|module| {
            module.colorzones_editor_state().is_some_and(|state| {
                state.editor().output_channel() == ColorZonesChannel::Hue
                    && state.graph_height().logical_pixels() == 233
            })
        }));
        assert!(projected.iter().all(|module| !module.is_hidden()));
        assert!(
            projected
                .iter()
                .all(|module| module.availability().is_supported())
        );
        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0], &first);
        assert_eq!(operations[1].id(), second_id);
        assert!(operations[1].is_enabled());
        assert_eq!(operations[1].opacity(), second_opacity);
        assert_eq!(
            operations[1].parameter(&ParameterName::new("mode").expect("mode parameter")),
            Some(&ParameterValue::Integer(i64::from(
                ColorZonesMode::Strong.raw()
            )))
        );
        assert_eq!(
            operations
                .iter()
                .map(|operation| operation.id())
                .collect::<Vec<_>>(),
            [first_id, second_id]
        );
        assert_eq!(
            controller.apply_colorzones(&action),
            Err(DarkroomModuleError::StaleRevision {
                expected: original.revision(),
                actual: persisted.revision(),
            })
        );
        assert_eq!(catalog.load(original.id()), persisted);
    }

    #[test]
    fn bloom_settled_action_persists_only_the_exact_mounted_instance() {
        let registry = builtin_registry();
        let first_id = OperationId::new(0xb701).expect("first Bloom ID");
        let second_id = OperationId::new(0xb702).expect("second Bloom ID");
        let first = registry
            .materialize_operation("rusttable.bloom", first_id)
            .expect("first Bloom defaults");
        let second_defaults = registry
            .materialize_operation("rusttable.bloom", second_id)
            .expect("second Bloom defaults");
        let second_opacity = OperationOpacity::new(0.375).expect("partial opacity");
        let second = Operation::new_with_opacity(
            second_id,
            second_defaults.key().clone(),
            false,
            second_opacity,
            second_defaults
                .parameters()
                .map(|(name, value)| (name.clone(), value.clone())),
        )
        .expect("disabled second Bloom instance");
        let original = Edit::from_parts(
            EditId::new(0xb703).expect("edit ID"),
            PhotoId::new(0xb704).expect("photo ID"),
            Revision::ZERO,
            Revision::from_u64(9),
            [first.clone(), second],
        )
        .expect("multi-instance Bloom edit");
        let catalog = TestCatalog::seed(&original);
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()));
        let modules = controller
            .select_photo(original.photo_id())
            .expect("select Bloom edit");
        assert_eq!(modules.instances(BLOOM_MODULE_ID).count(), 2);
        let action = DarkroomModuleAction::BloomSettled {
            module_id: BLOOM_MODULE_ID.to_owned(),
            operation_id: Some(second_id),
            expected_revision: original.revision(),
            parameters: rusttable_processing::operations::bloom::BloomParametersV1::new(
                20.0, 90.0, 50.0,
            ),
            enable_required: true,
        };

        let outcome = controller
            .apply(&action)
            .expect("persist exact Bloom action");
        let persisted = catalog.load(original.id());
        let operations = persisted.operations().collect::<Vec<_>>();

        assert!(outcome.processing_changed());
        assert_eq!(outcome.revision(), persisted.revision());
        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0], &first);
        assert_eq!(operations[1].id(), second_id);
        assert!(operations[1].is_enabled());
        assert_eq!(operations[1].opacity(), second_opacity);
        assert_eq!(
            operations[1].parameter(&ParameterName::new("strength").expect("strength")),
            Some(&scalar(50.0)),
        );
        let projected = outcome
            .modules()
            .instances(BLOOM_MODULE_ID)
            .collect::<Vec<_>>();
        assert_eq!(projected.len(), 2);
        assert_eq!(
            projected[1]
                .bloom_editor_state()
                .expect("second Bloom state")
                .editor()
                .strength()
                .to_bits(),
            50.0_f32.to_bits(),
        );
        assert_eq!(
            controller.apply(&action),
            Err(DarkroomModuleError::StaleRevision {
                expected: original.revision(),
                actual: persisted.revision(),
            }),
        );
        assert_eq!(catalog.load(original.id()), persisted);
    }

    #[test]
    fn no_op_colorzones_replacement_does_not_persist_history() {
        let operation_id = OperationId::new(0xc711).expect("Color Zones ID");
        let operation = builtin_registry()
            .materialize_operation("rusttable.colorzones", operation_id)
            .expect("Color Zones defaults");
        let original = Edit::from_parts(
            EditId::new(0xc712).expect("edit ID"),
            PhotoId::new(0xc713).expect("photo ID"),
            Revision::ZERO,
            Revision::from_u64(9),
            [operation],
        )
        .expect("Color Zones edit");
        let catalog = TestCatalog::seed(&original);
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()));
        controller
            .select_photo(original.photo_id())
            .expect("select Color Zones edit");
        let state = controller
            .colorzones_snapshots()
            .expect("Color Zones snapshot")
            .remove(0);
        let action = ColorZonesEditAction::new(
            operation_id,
            state.revision(),
            state.editor().parameters_value(),
            !state.enabled(),
            state.materialization_required(),
        );

        let outcome = controller
            .apply_colorzones(&action)
            .expect("consume no-op replacement");

        assert_eq!(outcome.revision(), original.revision());
        assert!(!outcome.processing_changed());
        assert_eq!(catalog.load(original.id()), original);
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
    fn first_bloom_settle_on_imported_edit_materializes_complete_parameters() {
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
        let action = DarkroomModuleAction::BloomSettled {
            module_id: "bloom".to_owned(),
            operation_id: None,
            expected_revision: original.revision(),
            parameters: rusttable_processing::operations::bloom::BloomParametersV1::new(
                20.0, 90.0, 50.0,
            ),
            enable_required: true,
        };
        module.apply(action.clone()).expect("first settle");

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
    fn enabling_vibrance_materializes_native_default_between_velvia_and_bloom() {
        let registry = builtin_registry();
        let original = Edit::from_parts(
            EditId::new(306).expect("edit id"),
            PhotoId::new(307).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(7),
            [
                registry
                    .materialize_operation(
                        "rusttable.velvia",
                        OperationId::new(74).expect("Velvia id"),
                    )
                    .expect("Velvia defaults"),
                registry
                    .materialize_operation(
                        "rusttable.bloom",
                        OperationId::new(75).expect("bloom id"),
                    )
                    .expect("bloom defaults"),
            ],
        )
        .expect("source-relative edit");
        let mut modules = project_edit(&original).expect("projection");
        let vibrance = modules.module_mut("vibrance").expect("Vibrance module");
        let action = DarkroomModuleAction::Enable {
            module_id: "vibrance".to_owned(),
            operation_id: None,
            expected_revision: original.revision(),
            enabled: true,
        };
        vibrance.apply(action.clone()).expect("enable Vibrance");

        let operations =
            rewrite_operations(&original, vibrance, &action).expect("materialize Vibrance");
        assert_eq!(
            operations
                .iter()
                .map(|operation| operation.key().as_str())
                .collect::<Vec<_>>(),
            ["rusttable.velvia", "rusttable.vibrance", "rusttable.bloom"]
        );
        let vibrance = &operations[1];
        assert!(vibrance.is_enabled());
        assert_eq!(vibrance.parameters().count(), 1);
        assert_eq!(
            vibrance.parameter(&ParameterName::new("amount").expect("amount")),
            Some(&scalar(25.0))
        );
    }

    #[test]
    fn persisted_vibrance_outlier_survives_disable_and_reset_targets_exact_id() {
        let operation_id = OperationId::new(418).expect("Vibrance id");
        let original = Edit::from_parts(
            EditId::new(416).expect("edit id"),
            PhotoId::new(417).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(9),
            [Operation::new_with_opacity(
                operation_id,
                OperationKey::new("rusttable.vibrance").expect("Vibrance key"),
                true,
                OperationOpacity::new(0.4).expect("opacity"),
                [(ParameterName::new("amount").expect("amount"), scalar(125.0))],
            )
            .expect("Vibrance operation")],
        )
        .expect("persisted Vibrance edit");
        let mut modules = project_edit(&original).expect("finite native value projects");
        let vibrance = modules
            .module_target_mut("vibrance", Some(operation_id))
            .expect("exact Vibrance module");
        let disable = DarkroomModuleAction::Enable {
            module_id: "vibrance".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: original.revision(),
            enabled: false,
        };
        vibrance.apply(disable.clone()).expect("disable Vibrance");
        let disabled =
            rewrite_operations(&original, vibrance, &disable).expect("rewrite exact Vibrance");
        assert!(!disabled[0].is_enabled());
        assert_eq!(
            disabled[0].opacity(),
            OperationOpacity::new(0.4).expect("opacity")
        );
        assert_eq!(
            disabled[0].parameter(&ParameterName::new("amount").expect("amount")),
            Some(&scalar(125.0))
        );

        let disabled_edit = original.revised(disabled).expect("disabled edit");
        let mut projected = project_edit(&disabled_edit).expect("disabled projection");
        let vibrance = projected
            .module_target_mut("vibrance", Some(operation_id))
            .expect("exact disabled Vibrance");
        let reset = DarkroomModuleAction::Reset {
            module_id: "vibrance".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: disabled_edit.revision(),
        };
        vibrance.apply(reset.clone()).expect("reset Vibrance");
        let reset_operations =
            rewrite_operations(&disabled_edit, vibrance, &reset).expect("native reset rewrite");
        assert!(reset_operations[0].is_enabled());
        assert_eq!(reset_operations[0].opacity(), OperationOpacity::ONE);
        assert_eq!(
            reset_operations[0].parameter(&ParameterName::new("amount").expect("amount")),
            Some(&scalar(25.0))
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
    fn controller_persists_only_truthful_multi_instance_lifecycle_by_exact_id() {
        let registry = builtin_registry();
        let upstream_id = OperationId::new(801).expect("Velvia id");
        let base_id = OperationId::new(802).expect("base Vibrance id");
        let downstream_id = OperationId::new(803).expect("bloom id");
        let original = Edit::from_parts(
            EditId::new(800).expect("edit id"),
            PhotoId::new(800).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(50),
            [
                registry
                    .materialize_operation("rusttable.velvia", upstream_id)
                    .expect("Velvia defaults"),
                vibrance_operation(base_id, false, 80.0, 0.35),
                registry
                    .materialize_operation("rusttable.bloom", downstream_id)
                    .expect("bloom defaults"),
            ],
        )
        .expect("multi-instance lifecycle edit");
        let new_id = multi_instance_operation_id(&original, base_id, "rusttable.vibrance", "new");
        let catalog = TestCatalog::seed(&original);
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()));
        controller
            .select_photo(original.photo_id())
            .expect("select lifecycle edit");

        let new_outcome = controller
            .apply(&DarkroomModuleAction::NewInstance {
                module_id: "vibrance".to_owned(),
                operation_id: Some(base_id),
                expected_revision: original.revision(),
            })
            .expect("new instance persists defaults");
        assert!(new_outcome.processing_changed());
        assert_eq!(new_outcome.revision(), Revision::from_u64(51));
        let after_new = catalog.load(original.id());
        assert_eq!(
            after_new
                .operations()
                .map(Operation::id)
                .collect::<Vec<_>>(),
            [upstream_id, base_id, new_id, downstream_id]
        );
        let new_operation = after_new
            .operations()
            .find(|operation| operation.id() == new_id)
            .expect("new Vibrance operation");
        assert!(new_operation.is_enabled());
        assert_eq!(new_operation.opacity(), OperationOpacity::ONE);
        assert_eq!(
            new_operation.parameter(&ParameterName::new("amount").expect("amount")),
            Some(&scalar(25.0))
        );
        let projected = new_outcome
            .modules()
            .instances("vibrance")
            .collect::<Vec<_>>();
        assert_eq!(projected.len(), 2);
        assert_eq!(
            projected
                .iter()
                .map(|module| module.widget_id())
                .collect::<Vec<_>>(),
            [
                format!("vibrance-instance-{base_id}"),
                format!("vibrance-instance-{new_id}")
            ]
        );

        for (action, expected_action, expected_reason) in [
            (
                DarkroomModuleAction::DuplicateInstance {
                    module_id: "vibrance".to_owned(),
                    operation_id: Some(base_id),
                    expected_revision: new_outcome.revision(),
                },
                "duplicate instance",
                "the current edit model cannot copy native blend and mask state",
            ),
            (
                DarkroomModuleAction::MoveInstanceDown {
                    module_id: "vibrance".to_owned(),
                    operation_id: Some(base_id),
                    expected_revision: new_outcome.revision(),
                },
                "move down",
                "the current edit model cannot apply native adjacent-module ordering",
            ),
            (
                DarkroomModuleAction::MoveInstanceUp {
                    module_id: "vibrance".to_owned(),
                    operation_id: Some(new_id),
                    expected_revision: new_outcome.revision(),
                },
                "move up",
                "the current edit model cannot apply native adjacent-module ordering",
            ),
        ] {
            let error = controller
                .apply(&action)
                .expect_err("unfaithful structural action is rejected before persistence");
            assert!(matches!(
                error,
                DarkroomModuleError::InstanceActionUnavailable {
                    module_id,
                    action,
                    reason,
                } if module_id == "vibrance"
                    && action == expected_action
                    && reason == expected_reason
            ));
            assert_eq!(
                catalog.load(original.id()),
                after_new,
                "rejected {expected_action} must not change revision or operation state"
            );
        }

        let deleted_new = controller
            .apply(&DarkroomModuleAction::DeleteInstance {
                module_id: "vibrance".to_owned(),
                operation_id: Some(new_id),
                expected_revision: new_outcome.revision(),
            })
            .expect("delete down to one same-key instance");
        assert_eq!(deleted_new.revision(), Revision::from_u64(52));
        assert_eq!(
            catalog
                .load(original.id())
                .operations()
                .map(Operation::id)
                .collect::<Vec<_>>(),
            [upstream_id, base_id, downstream_id]
        );

        let error = controller
            .apply(&DarkroomModuleAction::DeleteInstance {
                module_id: "vibrance".to_owned(),
                operation_id: Some(base_id),
                expected_revision: deleted_new.revision(),
            })
            .expect_err("the final same-key instance cannot be deleted");
        assert!(matches!(
            error,
            DarkroomModuleError::InstanceActionUnavailable {
                module_id,
                action: "delete",
                reason: "the final instance cannot be deleted",
            } if module_id == "vibrance"
        ));
        assert_eq!(
            catalog.load(original.id()).revision(),
            Revision::from_u64(52),
            "rejected structural actions must not persist a replacement"
        );
    }

    #[test]
    fn targetless_colorcorrection_grid_materializes_exact_enabled_instance() {
        let registry = builtin_registry();
        let original = Edit::from_parts(
            EditId::new(840).expect("edit id"),
            PhotoId::new(840).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(20),
            [
                registry
                    .materialize_operation(
                        "rusttable.relight",
                        OperationId::new(841).expect("Relight id"),
                    )
                    .expect("Relight defaults"),
                registry
                    .materialize_operation(
                        "rusttable.velvia",
                        OperationId::new(842).expect("Velvia id"),
                    )
                    .expect("Velvia defaults"),
            ],
        )
        .expect("edit without Color Correction");
        let generated_id = materialized_operation_id(&original, "rusttable.colorcorrection");
        let catalog = TestCatalog::seed(&original);
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()));
        let initial = controller
            .select_photo(original.photo_id())
            .expect("select edit without Color Correction");
        let template = initial
            .module("colorcorrection")
            .expect("targetless Color Correction template");
        assert_eq!(template.operation_id(), None);
        assert!(!template.can_add_instance());

        let grid =
            ColorCorrectionGridState::new(-0.95, 4.5, 3.55, 0.0).expect("warming-filter grid");
        let outcome = controller
            .apply(&DarkroomModuleAction::ColorCorrectionGrid {
                module_id: "colorcorrection".to_owned(),
                operation_id: None,
                expected_revision: original.revision(),
                grid,
            })
            .expect("first targetless grid edit");

        assert_eq!(outcome.revision(), Revision::from_u64(21));
        assert!(outcome.processing_changed());
        let projected = outcome
            .modules()
            .module_target("colorcorrection", Some(generated_id))
            .expect("materialized exact Color Correction instance");
        assert_eq!(projected.operation_id(), Some(generated_id));
        assert_eq!(projected.color_correction_grid(), Some(grid));
        assert!(projected.enabled());
        assert!(projected.supports_multi_instance());
        assert!(projected.can_add_instance());
        assert_eq!(
            outcome.modules().instances("colorcorrection").count(),
            1,
            "the targetless template is replaced by one exact instance"
        );

        let persisted = catalog.load(original.id());
        let operations = persisted.operations().collect::<Vec<_>>();
        assert_eq!(
            operations
                .iter()
                .map(|operation| operation.key().as_str())
                .collect::<Vec<_>>(),
            [
                "rusttable.relight",
                "rusttable.colorcorrection",
                "rusttable.velvia"
            ],
            "first persistence inserts Color Correction at its canonical source position"
        );
        let colorcorrection = operations[1];
        assert_eq!(colorcorrection.id(), generated_id);
        assert!(colorcorrection.is_enabled());
        assert_eq!(colorcorrection.opacity(), OperationOpacity::ONE);
        for (name, expected) in [
            ("hia", -0.95),
            ("hib", 4.5),
            ("loa", 3.55),
            ("lob", 0.0),
            ("saturation", 1.0),
        ] {
            assert_eq!(
                colorcorrection.parameter(&ParameterName::new(name).expect("parameter name")),
                Some(&scalar(expected)),
                "{name} is persisted from the grid plus registry defaults"
            );
        }
    }

    #[test]
    fn targetless_colorcorrection_parameter_reset_materializes_enabled_defaults() {
        let registry = builtin_registry();
        let original = Edit::from_parts(
            EditId::new(843).expect("edit id"),
            PhotoId::new(843).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(24),
            [
                registry
                    .materialize_operation(
                        "rusttable.relight",
                        OperationId::new(844).expect("Relight id"),
                    )
                    .expect("Relight defaults"),
                registry
                    .materialize_operation(
                        "rusttable.velvia",
                        OperationId::new(845).expect("Velvia id"),
                    )
                    .expect("Velvia defaults"),
            ],
        )
        .expect("edit without Color Correction");
        let generated_id = materialized_operation_id(&original, "rusttable.colorcorrection");
        let catalog = TestCatalog::seed(&original);
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()));
        controller
            .select_photo(original.photo_id())
            .expect("select edit without Color Correction");

        let outcome = controller
            .apply(&DarkroomModuleAction::ColorCorrectionResetParameters {
                module_id: "colorcorrection".to_owned(),
                operation_id: None,
                expected_revision: original.revision(),
            })
            .expect("first targetless parameter reset");

        assert_eq!(outcome.revision(), Revision::from_u64(25));
        assert!(outcome.processing_changed());
        let projected = outcome
            .modules()
            .module_target("colorcorrection", Some(generated_id))
            .expect("materialized exact Color Correction instance");
        assert_eq!(projected.operation_id(), Some(generated_id));
        assert!(projected.enabled());
        assert!(projected.can_add_instance());
        assert_eq!(
            projected.color_correction_grid(),
            Some(ColorCorrectionGridState::DEFAULT)
        );
        assert_eq!(
            projected
                .controls()
                .control("colorcorrection-saturation")
                .expect("saturation")
                .value(),
            DarkroomControlValue::Slider(1.0)
        );

        let persisted = catalog.load(original.id());
        let operations = persisted.operations().collect::<Vec<_>>();
        assert_eq!(
            operations
                .iter()
                .map(|operation| operation.key().as_str())
                .collect::<Vec<_>>(),
            [
                "rusttable.relight",
                "rusttable.colorcorrection",
                "rusttable.velvia"
            ]
        );
        let colorcorrection = operations[1];
        assert_eq!(colorcorrection.id(), generated_id);
        assert!(colorcorrection.is_enabled());
        assert_eq!(colorcorrection.opacity(), OperationOpacity::ONE);
        for (name, expected) in [
            ("hia", 0.0),
            ("hib", 0.0),
            ("loa", 0.0),
            ("lob", 0.0),
            ("saturation", 1.0),
        ] {
            assert_eq!(
                colorcorrection.parameter(&ParameterName::new(name).expect("parameter name")),
                Some(&scalar(expected)),
                "{name} uses the source default"
            );
        }
    }

    #[test]
    fn colorcorrection_grid_persists_four_parameters_atomically_to_exact_instance() {
        let first_id = OperationId::new(851).expect("first Color Correction id");
        let second_id = OperationId::new(852).expect("second Color Correction id");
        let original = Edit::from_parts(
            EditId::new(850).expect("edit id"),
            PhotoId::new(850).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(30),
            [
                colorcorrection_operation(first_id, 1.0, 2.0, 3.0, 4.0, 0.75),
                colorcorrection_operation_with_enabled(
                    second_id, false, -1.0, -2.0, -3.0, -4.0, 4.25,
                ),
            ],
        )
        .expect("two-instance Color Correction edit");
        let catalog = TestCatalog::seed(&original);
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()));
        controller
            .select_photo(original.photo_id())
            .expect("select Color Correction edit");
        let next_grid =
            ColorCorrectionGridState::new(-0.95, 4.5, 3.55, 0.0).expect("warming-filter grid");
        let outcome = controller
            .apply(&DarkroomModuleAction::ColorCorrectionGrid {
                module_id: "colorcorrection".to_owned(),
                operation_id: Some(second_id),
                expected_revision: original.revision(),
                grid: next_grid,
            })
            .expect("persist one atomic endpoint state");
        assert_eq!(outcome.revision(), Revision::from_u64(31));
        assert!(outcome.processing_changed());
        let projected = outcome
            .modules()
            .module_target("colorcorrection", Some(second_id))
            .expect("exact second projection");
        assert_eq!(projected.color_correction_grid(), Some(next_grid));
        assert!(
            projected.enabled(),
            "native history insertion enables the exact disabled instance"
        );

        let persisted = catalog.load(original.id());
        let operations = persisted.operations().collect::<Vec<_>>();
        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0].id(), first_id);
        for (name, expected) in [
            ("hia", 1.0),
            ("hib", 2.0),
            ("loa", 3.0),
            ("lob", 4.0),
            ("saturation", 0.75),
        ] {
            assert_eq!(
                operations[0].parameter(&ParameterName::new(name).expect("parameter name")),
                Some(&scalar(expected)),
                "the untargeted first instance remains byte-for-byte semantic state"
            );
        }
        assert_eq!(operations[1].id(), second_id);
        assert!(operations[1].is_enabled());
        for (name, expected) in [
            ("hia", -0.95),
            ("hib", 4.5),
            ("loa", 3.55),
            ("lob", 0.0),
            ("saturation", 4.25),
        ] {
            assert_eq!(
                operations[1].parameter(&ParameterName::new(name).expect("parameter name")),
                Some(&scalar(expected))
            );
        }

        let preset_error = controller
            .apply(&DarkroomModuleAction::Preset {
                module_id: "colorcorrection".to_owned(),
                operation_id: Some(second_id),
                expected_revision: outcome.revision(),
                preset_id: "cooling filter".to_owned(),
            })
            .expect_err("incomplete Color Correction preset remains gated");
        assert!(matches!(
            preset_error,
            DarkroomModuleError::Unsupported { .. }
        ));
        assert_eq!(
            catalog.load(original.id()).revision(),
            Revision::from_u64(31)
        );

        let stale = controller
            .apply(&DarkroomModuleAction::ColorCorrectionGrid {
                module_id: "colorcorrection".to_owned(),
                operation_id: Some(second_id),
                expected_revision: original.revision(),
                grid: ColorCorrectionGridState::DEFAULT,
            })
            .expect_err("stale grid callback");
        assert!(matches!(
            stale,
            DarkroomModuleError::StaleRevision {
                expected,
                actual
            } if expected == Revision::from_u64(30) && actual == Revision::from_u64(31)
        ));
        assert_eq!(
            catalog.load(original.id()).revision(),
            Revision::from_u64(31),
            "stale endpoint actions cannot partially replace the edit"
        );
    }

    #[test]
    fn colorcorrection_parameter_reset_targets_second_instance_and_retains_opacity() {
        let first_id = OperationId::new(853).expect("first Color Correction id");
        let second_id = OperationId::new(854).expect("second Color Correction id");
        let original = Edit::from_parts(
            EditId::new(855).expect("edit id"),
            PhotoId::new(855).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(40),
            [
                colorcorrection_operation(first_id, 1.0, 2.0, 3.0, 4.0, 0.75),
                colorcorrection_operation_with_opacity(
                    second_id, false, -1.0, -2.0, -3.0, -4.0, 2.25, 0.37,
                ),
            ],
        )
        .expect("two-instance Color Correction edit");
        let before = original.operations().cloned().collect::<Vec<_>>();
        let catalog = TestCatalog::seed(&original);
        let mut controller = GtkDarkroomEditController::new(Some(catalog.path.clone()));
        controller
            .select_photo(original.photo_id())
            .expect("select Color Correction edit");

        let outcome = controller
            .apply(&DarkroomModuleAction::ColorCorrectionResetParameters {
                module_id: "colorcorrection".to_owned(),
                operation_id: Some(second_id),
                expected_revision: original.revision(),
            })
            .expect("source-specific parameter reset");

        assert_eq!(outcome.revision(), Revision::from_u64(41));
        let projected = outcome
            .modules()
            .module_target("colorcorrection", Some(second_id))
            .expect("exact reset projection");
        assert!(projected.enabled());
        assert_eq!(
            projected.color_correction_grid(),
            Some(ColorCorrectionGridState::DEFAULT)
        );
        assert_eq!(
            projected
                .controls()
                .control("colorcorrection-saturation")
                .expect("saturation")
                .value(),
            DarkroomControlValue::Slider(1.0)
        );

        let persisted = catalog.load(original.id());
        let operations = persisted.operations().collect::<Vec<_>>();
        assert_eq!(
            operations[0], &before[0],
            "untargeted instance is unchanged"
        );
        assert_eq!(operations[1].id(), second_id);
        assert!(operations[1].is_enabled());
        assert_eq!(
            operations[1].opacity(),
            OperationOpacity::new(0.37).expect("fixture opacity"),
            "empty-grid reset does not replace blend opacity"
        );
        for (name, expected) in [
            ("hia", 0.0),
            ("hib", 0.0),
            ("loa", 0.0),
            ("lob", 0.0),
            ("saturation", 1.0),
        ] {
            assert_eq!(
                operations[1].parameter(&ParameterName::new(name).expect("parameter name")),
                Some(&scalar(expected)),
                "{name} resets to the source default"
            );
        }
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

    fn colorcorrection_operation(
        id: OperationId,
        hia: f64,
        hib: f64,
        loa: f64,
        lob: f64,
        saturation: f64,
    ) -> Operation {
        colorcorrection_operation_with_enabled(id, true, hia, hib, loa, lob, saturation)
    }

    fn colorcorrection_operation_with_enabled(
        id: OperationId,
        enabled: bool,
        hia: f64,
        hib: f64,
        loa: f64,
        lob: f64,
        saturation: f64,
    ) -> Operation {
        colorcorrection_operation_with_opacity(id, enabled, hia, hib, loa, lob, saturation, 1.0)
    }

    #[allow(clippy::too_many_arguments)]
    fn colorcorrection_operation_with_opacity(
        id: OperationId,
        enabled: bool,
        hia: f64,
        hib: f64,
        loa: f64,
        lob: f64,
        saturation: f64,
        opacity: f64,
    ) -> Operation {
        Operation::new_with_opacity(
            id,
            OperationKey::new("rusttable.colorcorrection").expect("Color Correction key"),
            enabled,
            OperationOpacity::new(opacity).expect("Color Correction opacity"),
            [
                (ParameterName::new("hia").expect("hia"), scalar(hia)),
                (ParameterName::new("hib").expect("hib"), scalar(hib)),
                (ParameterName::new("loa").expect("loa"), scalar(loa)),
                (ParameterName::new("lob").expect("lob"), scalar(lob)),
                (
                    ParameterName::new("saturation").expect("saturation"),
                    scalar(saturation),
                ),
            ],
        )
        .expect("Color Correction operation")
    }

    fn vibrance_operation(id: OperationId, enabled: bool, amount: f64, opacity: f64) -> Operation {
        Operation::new_with_opacity(
            id,
            OperationKey::new("rusttable.vibrance").expect("Vibrance key"),
            enabled,
            OperationOpacity::new(opacity).expect("Vibrance opacity"),
            [(
                ParameterName::new("amount").expect("amount"),
                scalar(amount),
            )],
        )
        .expect("Vibrance operation")
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
            colorzones_gui_preferences: ColorZonesGuiPreferences::default(),
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
        let action = DarkroomModuleAction::BloomSettled {
            module_id: "bloom".to_owned(),
            operation_id: Some(OperationId::new(9).expect("bloom id")),
            expected_revision: Revision::from_u64(4),
            parameters: rusttable_processing::operations::bloom::BloomParametersV1::new(
                20.0, 90.0, 50.0,
            ),
            enable_required: false,
        };
        module.apply(action.clone()).expect("bloom action");
        let operations = rewrite_operations(&original, module, &action).expect("rewrite");
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
