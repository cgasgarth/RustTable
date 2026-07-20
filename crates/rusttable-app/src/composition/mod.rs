mod catalog_preview;
mod preview_lifecycle;

pub use catalog_preview::{CatalogPreviewError, CatalogPreviewRequest, CatalogPreviewService};

use crate::gtk_controller::{CollectionController, CollectionSnapshot, GtkCatalogController};
use crate::gtk_preview_controller::{GtkPreviewController, GtkPreviewFailureKind, GtkPreviewState};
use crate::lifecycle::run_with_bootstrap;
use gtk4::gio::prelude::{ApplicationExt, ApplicationExtManual};
use gtk4::glib::{self, ControlFlow};
use rusttable_ui::{
    CollectionControlAction, CollectionControlState, CollectionFilterState, CollectionProperty,
};
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use preview_lifecycle::{PreviewLifecycle, PreviewSelectionToken};

/// Error returned when GTK terminates `RustTable` unsuccessfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopRunError {
    exit_code: u8,
}

impl fmt::Display for DesktopRunError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "RustTable GTK4 exited with status {}",
            self.exit_code
        )
    }
}

impl std::error::Error for DesktopRunError {}

/// Starts the GTK4 `RustTable` desktop application.
///
/// # Errors
///
/// Returns an error when GTK terminates the application with a failure exit
/// code.
pub fn run() -> Result<(), DesktopRunError> {
    let preflight = crate::platform::startup_preflight();
    run_with_bootstrap(
        rusttable_diagnostics::install,
        || {
            if !preflight.is_supported() {
                return Ok(());
            }

            let application = gtk4::Application::builder()
                .application_id("com.cgasgarth.rusttable")
                .build();
            let active_shell = Rc::new(RefCell::new(None::<rusttable_ui::GtkShell>));
            application.connect_activate({
                let active_shell = Rc::clone(&active_shell);
                move |application| {
                    if let Some(shell) = active_shell.borrow().as_ref() {
                        shell.present();
                        return;
                    }

                    if let Some(display) = gtk4::gdk::Display::default() {
                        rusttable_ui::install_darktable_theme(&display);
                    }
                    let catalog_controller =
                        Rc::new(RefCell::new(GtkCatalogController::load_persisted()));
                    let collection_controller = Rc::new(RefCell::new(
                        catalog_controller.borrow().collection_controller(),
                    ));
                    let shell = rusttable_ui::GtkShell::new(application);
                    let workspace = catalog_controller.borrow().state().workspace().cloned();
                    if let Some(workspace) = workspace.as_ref() {
                        shell.set_photo_workspace(workspace);
                    }
                    if let Some(controller) = collection_controller.borrow().as_ref() {
                        shell.set_collection_filter_state(&collection_filter_state(
                            &controller.snapshot(),
                        ));
                    }
                    let collection_for_actions = Rc::clone(&collection_controller);
                    shell.connect_collection_action(move |action| {
                        let mut controller = collection_for_actions.borrow_mut();
                        let Some(controller) = controller.as_mut() else {
                            return empty_collection_filter_state();
                        };
                        apply_collection_action(controller, action);
                        collection_filter_state(&controller.snapshot())
                    });
                    let selection_controller = Rc::clone(&catalog_controller);
                    let preview = shell.darkroom_preview().clone();
                    let preview_lifecycle = Rc::new(RefCell::new(PreviewLifecycle::default()));
                    shell.set_photo_selected_handler(move |photo_id| {
                        if !selection_controller.borrow_mut().select_photo(photo_id) {
                            return;
                        }
                        let catalog = selection_controller.borrow().clone();
                        start_selected_preview(&preview, catalog, Rc::clone(&preview_lifecycle));
                    });
                    shell.present();
                    active_shell.replace(Some(shell));
                }
            });
            let exit_code = application.run();
            if exit_code == gtk4::glib::ExitCode::SUCCESS {
                Ok(())
            } else {
                Err(DesktopRunError {
                    exit_code: exit_code.get(),
                })
            }
        },
        |warning| eprintln!("{warning}"),
    )
}

fn apply_collection_action(controller: &mut CollectionController, action: CollectionControlAction) {
    match action {
        CollectionControlAction::SetProperty(property) => controller.set_property(property),
        CollectionControlAction::SetSearchText(search_text) => {
            controller.set_search_text(search_text);
        }
        CollectionControlAction::Clear => controller.clear(),
    }
}

fn collection_filter_state(snapshot: &CollectionSnapshot) -> CollectionFilterState {
    let controls = CollectionControlState::new(snapshot.property(), snapshot.total_count())
        .with_results(snapshot.search_text(), snapshot.result_count());
    CollectionFilterState::new(controls, snapshot.matching_photo_ids().collect())
}

fn empty_collection_filter_state() -> CollectionFilterState {
    CollectionFilterState::new(
        CollectionControlState::new(CollectionProperty::Filename, 0),
        Vec::new(),
    )
}

struct PreviewResult {
    token: PreviewSelectionToken,
    state: GtkPreviewState,
}

fn start_selected_preview(
    preview: &rusttable_ui::gtk_shell::PhotoPreview,
    catalog: GtkCatalogController,
    lifecycle: Rc<RefCell<PreviewLifecycle>>,
) {
    let Some(photo_id) = catalog.selected_photo() else {
        preview.set_failure(GtkPreviewFailureKind::NoSelection.message());
        return;
    };
    let token = lifecycle.borrow_mut().begin(photo_id);
    preview.set_loading();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("rusttable-preview".to_owned())
        .spawn(move || {
            let state = GtkPreviewController::new().render_selected(&catalog);
            let _ = sender.send(PreviewResult { token, state });
        });
    if worker.is_err() {
        preview.set_failure(GtkPreviewFailureKind::RenderUnavailable.message());
        return;
    }

    let preview = preview.clone();
    glib::source::timeout_add_local(Duration::from_millis(16), move || {
        match receiver.try_recv() {
            Ok(result) => {
                if lifecycle.borrow().is_current(result.token) {
                    install_preview_state(&preview, result.state);
                }
                ControlFlow::Break
            }
            Err(TryRecvError::Empty) => ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                if lifecycle.borrow().is_current(token) {
                    preview.set_failure(GtkPreviewFailureKind::RenderUnavailable.message());
                }
                ControlFlow::Break
            }
        }
    });
}

fn install_preview_state(preview: &rusttable_ui::gtk_shell::PhotoPreview, state: GtkPreviewState) {
    let GtkPreviewState::Ready(rendered) = state else {
        if let GtkPreviewState::Failed(failure) = state {
            preview.set_failure(failure.message());
        }
        return;
    };

    let Ok(dimensions) = rusttable_ui::PreviewDimensions::new(
        rendered.dimensions().width(),
        rendered.dimensions().height(),
    ) else {
        preview.set_failure(GtkPreviewFailureKind::InvalidRgba8.message());
        return;
    };
    let Ok(status) = rusttable_ui::PresentationText::new("rendered") else {
        preview.set_failure(GtkPreviewFailureKind::RenderUnavailable.message());
        return;
    };
    let Ok(metadata) =
        rusttable_ui::Rgba8PreviewMetadata::new(dimensions, status, rendered.pixels().to_vec())
    else {
        preview.set_failure(GtkPreviewFailureKind::InvalidRgba8.message());
        return;
    };
    if preview.set_rgba8(&metadata).is_err() {
        preview.set_failure(GtkPreviewFailureKind::InvalidRgba8.message());
    }
}
