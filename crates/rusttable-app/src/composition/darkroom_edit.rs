//! GTK composition bridge for controller-owned darkroom edit actions.

use std::cell::RefCell;
use std::rc::Rc;

use crate::composition::preview_lifecycle::PreviewLifecycle;
use crate::gtk_controller::{GtkCatalogController, GtkDarkroomEditController};
use rusttable_ui::{DarkroomModuleActionHandler, GtkShell};

pub(super) struct DarkroomEditBridge {
    pub(super) controller: Rc<RefCell<GtkDarkroomEditController>>,
    pub(super) handler: DarkroomModuleActionHandler,
}

pub(super) fn install(
    shell: &GtkShell,
    catalog: &Rc<RefCell<GtkCatalogController>>,
    lifecycle: &Rc<RefCell<PreviewLifecycle>>,
) -> DarkroomEditBridge {
    let catalog = Rc::clone(catalog);
    let lifecycle = Rc::clone(lifecycle);
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
    let slot_for_handler = Rc::clone(&slot);
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
                super::start_selected_preview(
                    &action_shell,
                    action_catalog.borrow().clone(),
                    Rc::clone(&action_lifecycle),
                );
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
    }
}
