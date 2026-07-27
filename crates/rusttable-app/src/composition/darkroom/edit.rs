//! GTK composition bridge for controller-owned darkroom edit actions.

use std::cell::RefCell;
use std::rc::Rc;

use crate::composition::selected_preview::PreviewLifecycle;
use crate::composition::thumbnails::ThumbnailLifecycle;
use crate::diagnostics::AppDiagnostics;
use crate::gtk_controller::colorzones_edit::{ColorZonesEditAction, ColorZonesGuiPreferences};
use crate::gtk_controller::{DarkroomEditOutcome, GtkCatalogController, GtkDarkroomEditController};
use rusttable_display_profile::DisplayProfileSnapshot;
use rusttable_ui::iop::colorzones::{
    ColorZonesGtkActionHandler, ColorZonesGtkHandlerOutcome, ColorZonesGtkPreferencesHandler,
};
use rusttable_ui::{
    DarkroomControlValue, DarkroomModuleAction, DarkroomModuleActionHandler, GtkShell,
};

pub(crate) type DarkroomEditCommitHandler = Rc<dyn Fn()>;

pub(crate) struct DarkroomEditBridge {
    pub(crate) controller: Rc<RefCell<GtkDarkroomEditController>>,
    pub(crate) handler: DarkroomModuleActionHandler,
    pub(crate) colorzones_handler: ColorZonesGtkActionHandler,
    pub(crate) colorzones_preferences_handler: ColorZonesGtkPreferencesHandler,
    after_commit: Rc<RefCell<Option<DarkroomEditCommitHandler>>>,
}

pub(crate) fn install(
    shell: &GtkShell,
    catalog: &Rc<RefCell<GtkCatalogController>>,
    lifecycle: &Rc<RefCell<PreviewLifecycle>>,
    thumbnail_lifecycle: &Rc<RefCell<ThumbnailLifecycle>>,
    display_profile: &Rc<RefCell<Option<DisplayProfileSnapshot>>>,
    diagnostics: &AppDiagnostics,
) -> DarkroomEditBridge {
    let catalog = Rc::clone(catalog);
    let lifecycle = Rc::clone(lifecycle);
    let thumbnail_lifecycle = Rc::clone(thumbnail_lifecycle);
    let display_profile = Rc::clone(display_profile);
    let diagnostics = diagnostics.clone();
    let controller = Rc::new(RefCell::new(
        GtkDarkroomEditController::new(
            catalog
                .borrow()
                .catalog_path()
                .map(std::path::Path::to_path_buf),
        )
        .with_colorzones_gui_preferences(crate::configuration::colorzones_gui_preferences()),
    ));
    let after_commit = Rc::new(RefCell::new(None::<DarkroomEditCommitHandler>));
    let publish_shell = shell.clone();
    let publish_catalog = Rc::clone(&catalog);
    let publish_lifecycle = Rc::clone(&lifecycle);
    let publish_thumbnail_lifecycle = Rc::clone(&thumbnail_lifecycle);
    let publish_display_profile = Rc::clone(&display_profile);
    let publish_diagnostics = diagnostics.clone();
    let publish_after_commit = Rc::clone(&after_commit);
    let publish_processing_change = Rc::new(move |outcome: &DarkroomEditOutcome| {
        if !outcome.processing_changed() {
            return;
        }
        publish_shell
            .set_darkroom_status(&format!("Edit persisted · revision {}", outcome.revision()));
        if let Some(after_commit) = publish_after_commit.borrow().as_ref() {
            // Invalidate history and the shared filmstrip before the new preview
            // starts so an older worker cannot publish across edit identity.
            after_commit();
        }
        crate::composition::selected_preview::start_selected_preview(
            &publish_shell,
            publish_catalog.borrow().clone(),
            Rc::clone(&publish_lifecycle),
            &publish_thumbnail_lifecycle,
            publish_diagnostics.clone(),
            publish_display_profile.borrow().as_ref(),
        );
    });

    let slot = Rc::new(RefCell::new(None::<DarkroomModuleActionHandler>));
    let action_controller = Rc::clone(&controller);
    let action_shell = shell.clone();
    let slot_for_handler = Rc::clone(&slot);
    let publish_generic_change = Rc::clone(&publish_processing_change);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        let preserve_mounted_control = preserves_mounted_control(&action);
        let result = action_controller.borrow_mut().apply(&action);
        match result {
            Ok(outcome) => {
                if preserve_mounted_control {
                    action_shell.update_darkroom_module_stack_snapshot(
                        outcome.modules(),
                        outcome.revision(),
                    );
                } else {
                    action_shell.set_darkroom_module_stack(
                        outcome.modules(),
                        slot_for_handler.borrow().clone(),
                    );
                }
                publish_generic_change(&outcome);
                Ok(outcome.revision())
            }
            Err(error) => {
                let selected_photo = action_controller.borrow().selected_photo();
                tracing::error!(
                    target: "rusttable.gtk.darkroom.edit",
                    photo_id = ?selected_photo,
                    module_id = action.module_id(),
                    expected_revision = %action.expected_revision(),
                    cause = ?error,
                    "darkroom edit action was not persisted; retaining the last published state"
                );
                if let Some(modules) = action_controller.borrow().modules().cloned() {
                    action_shell
                        .set_darkroom_module_stack(&modules, slot_for_handler.borrow().clone());
                }
                action_shell.set_darkroom_status(&error.to_string());
                Err(error)
            }
        }
    });
    slot.replace(Some(handler.clone()));

    let colorzones_controller = Rc::clone(&controller);
    let colorzones_shell = shell.clone();
    let publish_colorzones_change = Rc::clone(&publish_processing_change);
    let colorzones_handler: ColorZonesGtkActionHandler = Rc::new(move |settled| {
        let target = settled.target();
        let expected_revision = settled.expected_revision();
        let action = ColorZonesEditAction::from(settled);
        let result = colorzones_controller.borrow_mut().apply_colorzones(&action);
        match result {
            Ok(outcome) => {
                // The custom mount reconciles persisted truth in place while
                // retaining its UI-owned output tab and stable controllers.
                colorzones_shell
                    .update_darkroom_module_stack_snapshot(outcome.modules(), outcome.revision());
                publish_colorzones_change(&outcome);
                ColorZonesGtkHandlerOutcome::Commit {
                    revision: outcome.revision(),
                }
            }
            Err(error) => {
                let selected_photo = colorzones_controller.borrow().selected_photo();
                tracing::error!(
                    target: "rusttable.gtk.darkroom.colorzones.edit",
                    photo_id = ?selected_photo,
                    operation_id = %target,
                    expected_revision = %expected_revision,
                    cause = ?error,
                    "Color Zones settled action was not persisted"
                );
                colorzones_shell.set_darkroom_status(&error.to_string());
                if matches!(
                    error,
                    rusttable_ui::DarkroomModuleError::StaleRevision { .. }
                ) {
                    let reconciled = colorzones_controller
                        .borrow()
                        .reconcile_colorzones_snapshot(target);
                    if let Ok(state) = reconciled {
                        let modules = colorzones_controller.borrow().modules().cloned();
                        if let Some(modules) = modules {
                            colorzones_shell
                                .update_darkroom_module_stack_snapshot(&modules, state.revision());
                        }
                        return ColorZonesGtkHandlerOutcome::Reconcile(state);
                    }
                }
                ColorZonesGtkHandlerOutcome::Rollback
            }
        }
    });

    let preferences_controller = Rc::clone(&controller);
    let preferences_shell = shell.clone();
    let colorzones_preferences_handler: ColorZonesGtkPreferencesHandler =
        Rc::new(move |preferences| {
            let preferences = ColorZonesGuiPreferences::from(preferences);
            let previous = preferences_controller.borrow().colorzones_gui_preferences();
            let outcome = match preferences_controller
                .borrow_mut()
                .update_colorzones_gui_preferences(preferences)
            {
                Ok(outcome) => outcome,
                Err(error) => {
                    tracing::error!(
                        target: "rusttable.gtk.darkroom.colorzones.preferences",
                        cause = ?error,
                        "Color Zones GUI preferences could not be reconciled"
                    );
                    preferences_shell.set_darkroom_status(&error.to_string());
                    return false;
                }
            };
            if let Err(error) =
                crate::configuration::persist_colorzones_gui_preferences(preferences)
            {
                tracing::error!(
                    target: "rusttable.gtk.darkroom.colorzones.preferences",
                    cause = ?error,
                    "Color Zones GUI preferences were not persisted"
                );
                if let Ok(rollback) = preferences_controller
                    .borrow_mut()
                    .update_colorzones_gui_preferences(previous)
                {
                    preferences_shell.update_darkroom_module_stack_snapshot(
                        rollback.modules(),
                        rollback.revision(),
                    );
                }
                preferences_shell.set_darkroom_status(&error.to_string());
                return false;
            }
            preferences_shell
                .update_darkroom_module_stack_snapshot(outcome.modules(), outcome.revision());
            true
        });

    DarkroomEditBridge {
        controller,
        handler,
        colorzones_handler,
        colorzones_preferences_handler,
        after_commit,
    }
}

fn preserves_mounted_control(action: &DarkroomModuleAction) -> bool {
    action.operation_id().is_some()
        && matches!(
            action,
            DarkroomModuleAction::Control {
                value: DarkroomControlValue::Slider(_),
                ..
            } | DarkroomModuleAction::BloomSettled { .. }
                | DarkroomModuleAction::ColorCorrectionGrid { .. }
        )
}

impl DarkroomEditBridge {
    pub(crate) fn set_after_commit(&self, handler: DarkroomEditCommitHandler) {
        self.after_commit.replace(Some(handler));
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::{OperationId, Revision};
    use rusttable_ui::iop::colorcorrection::ColorCorrectionGridState;
    use rusttable_ui::{DarkroomControlValue, DarkroomModuleAction};

    use super::preserves_mounted_control;

    #[test]
    fn only_exact_continuous_controls_preserve_the_mounted_module_stack() {
        let operation_id = OperationId::new(1).expect("operation id");
        let exact_slider = DarkroomModuleAction::Control {
            module_id: "vibrance".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: Revision::ZERO,
            id: "vibrance-amount".to_owned(),
            value: DarkroomControlValue::Slider(25.0),
        };
        let targetless_slider = exact_slider.clone().with_operation_id(None);
        let toggle = DarkroomModuleAction::Control {
            module_id: "example".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: Revision::ZERO,
            id: "example-enabled".to_owned(),
            value: DarkroomControlValue::Toggle(true),
        };
        let reset = DarkroomModuleAction::Reset {
            module_id: "vibrance".to_owned(),
            operation_id: None,
            expected_revision: Revision::ZERO,
        };
        let exact_bloom = DarkroomModuleAction::BloomSettled {
            module_id: "bloom".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: Revision::ZERO,
            parameters: rusttable_processing::operations::bloom::BloomParametersV1::defaults(),
            enable_required: true,
        };
        let exact_grid = DarkroomModuleAction::ColorCorrectionGrid {
            module_id: "colorcorrection".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: Revision::ZERO,
            grid: ColorCorrectionGridState::DEFAULT,
        };
        let targetless_grid = exact_grid.clone().with_operation_id(None);
        let parameter_reset = DarkroomModuleAction::ColorCorrectionResetParameters {
            module_id: "colorcorrection".to_owned(),
            operation_id: Some(operation_id),
            expected_revision: Revision::ZERO,
        };

        assert!(preserves_mounted_control(&exact_slider));
        assert!(preserves_mounted_control(&exact_bloom));
        assert!(preserves_mounted_control(&exact_grid));
        assert!(!preserves_mounted_control(&targetless_slider));
        assert!(!preserves_mounted_control(&targetless_grid));
        assert!(!preserves_mounted_control(&toggle));
        assert!(!preserves_mounted_control(&reset));
        assert!(!preserves_mounted_control(&parameter_reset));
    }
}
