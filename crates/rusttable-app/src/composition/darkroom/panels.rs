//! GTK composition bridge for controller-owned darkroom rail projections.

use std::cell::RefCell;
use std::rc::Rc;

use crate::composition::selected_preview::PreviewLifecycle;
use crate::diagnostics::AppDiagnostics;
use crate::gtk_controller::{
    DarkroomPanelProjections, GtkCatalogController, GtkDarkroomPanelController,
};
use rusttable_core::Revision;
use rusttable_ui::{DarkroomPanelAction, DarkroomPanelActionHandler, GtkShell};

#[derive(Clone)]
pub(crate) struct DarkroomPanelBridge {
    controller: Rc<RefCell<GtkDarkroomPanelController>>,
    handler: DarkroomPanelActionHandler,
}

pub(crate) fn install(
    shell: &GtkShell,
    catalog: &Rc<RefCell<GtkCatalogController>>,
    lifecycle: &Rc<RefCell<PreviewLifecycle>>,
    diagnostics: &AppDiagnostics,
) -> DarkroomPanelBridge {
    let controller = Rc::new(RefCell::new(GtkDarkroomPanelController::new(
        catalog
            .borrow()
            .catalog_path()
            .map(std::path::Path::to_path_buf),
    )));
    let handler_slot = Rc::new(RefCell::new(None::<DarkroomPanelActionHandler>));
    let action_controller = Rc::clone(&controller);
    let action_shell = shell.clone();
    let action_catalog = Rc::clone(catalog);
    let action_lifecycle = Rc::clone(lifecycle);
    let diagnostics = diagnostics.clone();
    let handler_slot_for_action = Rc::clone(&handler_slot);
    let handler: DarkroomPanelActionHandler = Rc::new(move |action| {
        let refresh_preview = refreshes_preview(&action);
        let result = action_controller.borrow_mut().apply(&action);
        match result {
            Ok(projections) => {
                if refresh_preview {
                    crate::composition::selected_preview::start_selected_preview(
                        &action_shell,
                        action_catalog.borrow().clone(),
                        Rc::clone(&action_lifecycle),
                        diagnostics.clone(),
                    );
                    if let Some(target) = action_shell.darkroom_panel_target() {
                        let rebound = action_controller.borrow_mut().rebind_target(target);
                        if let Ok(projections) = rebound {
                            project(&action_shell, &handler_slot_for_action, &projections);
                        }
                    }
                } else {
                    project(&action_shell, &handler_slot_for_action, &projections);
                }
                action_shell
                    .set_darkroom_status("darkroom rail action applied · preview refreshed");
            }
            Err(error) => {
                if let Some(projections) = action_controller.borrow().projections() {
                    project(&action_shell, &handler_slot_for_action, projections);
                }
                action_shell.set_darkroom_status(&format!("Darkroom rail error · {error}"));
            }
        }
    });
    handler_slot.replace(Some(handler.clone()));
    DarkroomPanelBridge {
        controller,
        handler,
    }
}

impl DarkroomPanelBridge {
    pub(crate) fn refresh(&self, shell: &GtkShell, catalog: &GtkCatalogController) {
        let mut controller = self.controller.borrow_mut();
        controller.set_catalog_path(catalog.catalog_path().map(std::path::Path::to_path_buf));
        let target = shell.darkroom_panel_target();
        let result = target.map_or(Ok(None), |target| {
            controller.rebind_target(target).map(Some)
        });
        match result.and_then(|_| controller.refresh().map(Some)) {
            Ok(Some(projections)) => {
                drop(controller);
                project(
                    shell,
                    &Rc::new(RefCell::new(Some(self.handler.clone()))),
                    &projections,
                );
            }
            Ok(None) => {}
            Err(error) => {
                shell.set_darkroom_status(&format!("Darkroom rail error · {error}"));
            }
        }
    }

    pub(crate) fn select(&self, shell: &GtkShell, catalog: &GtkCatalogController) {
        let Some(target) = shell.darkroom_panel_target() else {
            shell.clear_darkroom_selection("select a photo to view history and snapshots");
            return;
        };
        let loading = DarkroomPanelProjections::loading(target, Revision::ZERO);
        project(
            shell,
            &Rc::new(RefCell::new(Some(self.handler.clone()))),
            &loading,
        );
        let mut controller = self.controller.borrow_mut();
        controller.set_catalog_path(catalog.catalog_path().map(std::path::Path::to_path_buf));
        match controller.select_photo(target) {
            Ok(projections) => {
                drop(controller);
                project(
                    shell,
                    &Rc::new(RefCell::new(Some(self.handler.clone()))),
                    &projections,
                );
                shell.set_darkroom_status("selected photo · darkroom rails loaded");
            }
            Err(error) => {
                let projections =
                    DarkroomPanelProjections::error(target, Revision::ZERO, error.to_string());
                drop(controller);
                if let Ok(projections) = projections {
                    project(
                        shell,
                        &Rc::new(RefCell::new(Some(self.handler.clone()))),
                        &projections,
                    );
                }
                shell.set_darkroom_status(&format!("Darkroom rail error · {error}"));
            }
        }
    }
}

fn project(
    shell: &GtkShell,
    handler_slot: &Rc<RefCell<Option<DarkroomPanelActionHandler>>>,
    projections: &DarkroomPanelProjections,
) {
    let handler = handler_slot.borrow().clone();
    shell.set_history_projection(projections.history(), handler.clone());
    shell.set_snapshots_projection(projections.snapshots(), handler);
}

fn refreshes_preview(action: &DarkroomPanelAction) -> bool {
    matches!(
        action,
        DarkroomPanelAction::SelectHistory { .. }
            | DarkroomPanelAction::NavigateHistory { .. }
            | DarkroomPanelAction::SelectSnapshot { .. }
            | DarkroomPanelAction::RestoreSnapshot { .. }
    )
}
