//! GTK composition bridge for controller-owned darkroom edit actions.

use std::cell::RefCell;
use std::rc::Rc;

use crate::composition::selected_preview::PreviewLifecycle;
use crate::composition::thumbnails::ThumbnailLifecycle;
use crate::diagnostics::AppDiagnostics;
use crate::gtk_controller::{GtkCatalogController, GtkDarkroomEditController};
use rusttable_display_profile::DisplayProfileSnapshot;
use rusttable_ui::{
    DarkroomControlValue, DarkroomModuleAction, DarkroomModuleActionHandler, GtkShell,
};

pub(crate) type DarkroomEditCommitHandler = Rc<dyn Fn()>;

pub(crate) struct DarkroomEditBridge {
    pub(crate) controller: Rc<RefCell<GtkDarkroomEditController>>,
    pub(crate) handler: DarkroomModuleActionHandler,
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
    let controller = Rc::new(RefCell::new(GtkDarkroomEditController::new(
        catalog
            .borrow()
            .catalog_path()
            .map(std::path::Path::to_path_buf),
    )));
    let slot = Rc::new(RefCell::new(None::<DarkroomModuleActionHandler>));
    let action_controller = Rc::clone(&controller);
    let action_shell = shell.clone();
    let action_catalog = Rc::clone(&catalog);
    let action_lifecycle = Rc::clone(&lifecycle);
    let action_display_profile = Rc::clone(&display_profile);
    let slot_for_handler = Rc::clone(&slot);
    let after_commit = Rc::new(RefCell::new(None::<DarkroomEditCommitHandler>));
    let after_commit_for_handler = Rc::clone(&after_commit);
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
                if outcome.processing_changed() {
                    action_shell.set_darkroom_status(&format!(
                        "Edit persisted · revision {}",
                        outcome.revision()
                    ));
                    if let Some(after_commit) = after_commit_for_handler.borrow().as_ref() {
                        // Invalidate the shared filmstrip before the new preview request starts.
                        // This prevents an old thumbnail worker from briefly publishing after
                        // persistence has advanced the edit identity but before the new preview
                        // is ready.
                        after_commit();
                    }
                    crate::composition::selected_preview::start_selected_preview(
                        &action_shell,
                        action_catalog.borrow().clone(),
                        Rc::clone(&action_lifecycle),
                        &thumbnail_lifecycle,
                        diagnostics.clone(),
                        action_display_profile.borrow().as_ref(),
                    );
                }
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
    DarkroomEditBridge {
        controller,
        handler,
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
            } | DarkroomModuleAction::ColorCorrectionGrid { .. }
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
        assert!(preserves_mounted_control(&exact_grid));
        assert!(!preserves_mounted_control(&targetless_slider));
        assert!(!preserves_mounted_control(&targetless_grid));
        assert!(!preserves_mounted_control(&toggle));
        assert!(!preserves_mounted_control(&reset));
        assert!(!preserves_mounted_control(&parameter_reset));
    }
}
