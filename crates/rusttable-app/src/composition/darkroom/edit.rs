//! GTK composition bridge for controller-owned darkroom edit actions.

use std::cell::RefCell;
use std::rc::Rc;

use crate::composition::selected_preview::PreviewLifecycle;
use crate::diagnostics::AppDiagnostics;
use crate::gtk_controller::{GtkCatalogController, GtkDarkroomEditController};
use rusttable_display_profile::DisplayProfileSnapshot;
use rusttable_ui::{DarkroomModuleActionHandler, GtkShell};

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
    display_profile: &Rc<RefCell<Option<DisplayProfileSnapshot>>>,
    diagnostics: &AppDiagnostics,
) -> DarkroomEditBridge {
    let catalog = Rc::clone(catalog);
    let lifecycle = Rc::clone(lifecycle);
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
        let result = action_controller.borrow_mut().apply(&action);
        match result {
            Ok(outcome) => {
                action_shell.set_darkroom_module_stack(
                    outcome.modules(),
                    slot_for_handler.borrow().clone(),
                );
                action_shell.set_darkroom_status(&format!(
                    "Edit persisted · revision {}",
                    outcome.revision()
                ));
                crate::composition::selected_preview::start_selected_preview(
                    &action_shell,
                    action_catalog.borrow().clone(),
                    Rc::clone(&action_lifecycle),
                    diagnostics.clone(),
                    action_display_profile.borrow().as_ref(),
                );
                if let Some(after_commit) = after_commit_for_handler.borrow().as_ref() {
                    after_commit();
                }
                Ok(outcome.revision())
            }
            Err(error) => {
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

impl DarkroomEditBridge {
    pub(crate) fn set_after_commit(&self, handler: DarkroomEditCommitHandler) {
        self.after_commit.replace(Some(handler));
    }
}
