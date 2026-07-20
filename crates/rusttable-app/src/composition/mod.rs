mod catalog_preview;
mod preview_lifecycle;

pub use catalog_preview::{CatalogPreviewError, CatalogPreviewRequest, CatalogPreviewService};

use crate::gtk_controller::{CollectionController, CollectionSnapshot, GtkCatalogController};
use crate::gtk_export::{
    ExportCancellation, ExportCollisionSelection, ExportCompletion, ExportRequest, ExportRunError,
    ExportSettings, ExportSizeSelection, ExportStage, ExportStatus, run_with_progress,
};
use crate::gtk_preview_controller::{GtkPreviewController, GtkPreviewFailureKind, GtkPreviewState};
use crate::lifecycle::run_with_bootstrap;
use gtk4::gio::prelude::{ApplicationExt, ApplicationExtManual, FileExt};
use gtk4::glib::{self, ControlFlow};
use rusttable_ui::{
    CollectionControlAction, CollectionControlState, CollectionFilterState, CollectionProperty,
    ExportAction, ExportSize as UiExportSize,
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
                    let export_panel = shell.export_panel().clone();
                    let export_lifecycle = Rc::new(RefCell::new(ExportLifecycle::default()));
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
                    connect_export_actions(
                        &shell,
                        Rc::clone(&catalog_controller),
                        Rc::clone(&export_lifecycle),
                    );
                    let selection_controller = Rc::clone(&catalog_controller);
                    let preview = shell.darkroom_preview().clone();
                    let preview_lifecycle = Rc::new(RefCell::new(PreviewLifecycle::default()));
                    let export_selection = export_panel.clone();
                    let export_selection_lifecycle = Rc::clone(&export_lifecycle);
                    shell.set_photo_selected_handler(move |photo_id| {
                        if !selection_controller.borrow_mut().select_photo(photo_id) {
                            return;
                        }
                        export_selection_lifecycle.borrow_mut().invalidate();
                        export_selection.set_selected(true);
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

#[derive(Debug, Default)]
struct ExportLifecycle {
    generation: u64,
    active: Option<(u64, ExportCancellation)>,
    pending_collision: Option<(u64, ExportRequest)>,
}

impl ExportLifecycle {
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        if let Some((_, cancellation)) = self.active.take() {
            cancellation.cancel();
        }
        self.pending_collision = None;
    }

    fn begin(&mut self, cancellation: ExportCancellation) -> u64 {
        self.invalidate();
        self.active = Some((self.generation, cancellation));
        self.generation
    }

    fn is_current(&self, token: u64) -> bool {
        self.generation == token
    }
}

enum ExportWorkerMessage {
    Status {
        status: ExportStatus,
    },
    Finished {
        request: ExportRequest,
        result: Result<ExportCompletion, ExportRunError>,
    },
}

fn connect_export_actions(
    shell: &rusttable_ui::GtkShell,
    catalog: Rc<RefCell<GtkCatalogController>>,
    lifecycle: Rc<RefCell<ExportLifecycle>>,
) {
    let panel = shell.export_panel().clone();
    let window = shell.window().clone();
    shell
        .export_panel()
        .connect_action(move |action| match action {
            ExportAction::SelectSize(_) => {}
            ExportAction::Start => {
                let Some(photo_id) = catalog.borrow().selected_photo() else {
                    panel.set_finished("Select a photo to export.", false);
                    return;
                };
                let size = panel.size();
                let catalog_snapshot = catalog.borrow().clone();
                let (catalog_path, source_root, edit_id) =
                    match export_snapshot(&catalog_snapshot, photo_id) {
                        Ok(snapshot) => snapshot,
                        Err(message) => {
                            panel.set_finished(&message, false);
                            return;
                        }
                    };
                let token = lifecycle.borrow_mut().generation.wrapping_add(1);
                lifecycle.borrow_mut().generation = token;
                panel.set_idle("Choose a PNG destination…");
                let panel = panel.clone();
                let lifecycle = Rc::clone(&lifecycle);
                let window = window.clone();
                let dialog = gtk4::FileDialog::builder()
                    .title("Save selected edit as PNG")
                    .accept_label("Save")
                    .modal(true)
                    .build();
                let filter = gtk4::FileFilter::new();
                filter.set_name(Some("PNG image"));
                filter.add_suffix("png");
                dialog.set_default_filter(Some(&filter));
                dialog.set_initial_name(Some("RustTable export.png"));
                dialog.save(
                    Some(&window),
                    None::<&gtk4::gio::Cancellable>,
                    move |result| {
                        if !lifecycle.borrow().is_current(token) {
                            return;
                        }
                        let Ok(file) = result else {
                            panel.set_idle("PNG export cancelled.");
                            return;
                        };
                        let Some(destination) = file.path() else {
                            panel.set_finished("The destination is not a local file.", false);
                            return;
                        };
                        let request = match export_request(
                            photo_id,
                            edit_id,
                            catalog_path,
                            source_root,
                            destination,
                            size,
                            ExportCollisionSelection::CreateNew,
                        ) {
                            Ok(request) => request,
                            Err(message) => {
                                panel.set_finished(&message, false);
                                return;
                            }
                        };
                        start_export(panel, lifecycle, &request);
                    },
                );
            }
            ExportAction::Cancel => {
                if let Some((_, cancellation)) = lifecycle.borrow().active.as_ref() {
                    cancellation.cancel();
                    panel.set_running("Cancelling PNG export…");
                }
            }
            ExportAction::ReplaceExisting => {
                let request = lifecycle.borrow_mut().pending_collision.take();
                if let Some((token, request)) = request
                    && lifecycle.borrow().is_current(token)
                {
                    let replacement =
                        request.with_collision(ExportCollisionSelection::ReplaceExisting);
                    start_export(panel.clone(), Rc::clone(&lifecycle), &replacement);
                }
            }
        });
}

fn start_export(
    panel: rusttable_ui::ExportPanel,
    lifecycle: Rc<RefCell<ExportLifecycle>>,
    request: &ExportRequest,
) {
    let cancellation = ExportCancellation::default();
    let token = lifecycle.borrow_mut().begin(cancellation.clone());
    panel.set_running(ExportStage::Preparing.label());
    let (sender, receiver) = mpsc::channel();
    let worker_request = request.clone();
    let worker_cancellation = cancellation;
    let worker = thread::Builder::new()
        .name("rusttable-png-export".to_owned())
        .spawn(move || {
            let result = run_with_progress(&worker_request, &worker_cancellation, |status| {
                let _ = sender.send(ExportWorkerMessage::Status { status });
            });
            let _ = sender.send(ExportWorkerMessage::Finished {
                request: worker_request,
                result,
            });
        });
    if worker.is_err() {
        panel.set_finished("Could not start PNG export.", false);
        lifecycle.borrow_mut().active = None;
        return;
    }

    glib::source::timeout_add_local(Duration::from_millis(16), move || {
        loop {
            let message = match receiver.try_recv() {
                Ok(message) => message,
                Err(TryRecvError::Empty) => return ControlFlow::Continue,
                Err(TryRecvError::Disconnected) => {
                    if lifecycle.borrow().is_current(token) {
                        panel.set_finished("PNG export stopped unexpectedly.", false);
                    }
                    return ControlFlow::Break;
                }
            };
            if !lifecycle.borrow().is_current(token) {
                return ControlFlow::Break;
            }
            match message {
                ExportWorkerMessage::Status { status } => {
                    panel.set_running(status.text());
                }
                ExportWorkerMessage::Finished { request, result } => {
                    lifecycle.borrow_mut().active = None;
                    match result {
                        Ok(completion) => panel.set_finished(&completion.summary(), true),
                        Err(ExportRunError::DestinationExists(path)) => {
                            let alias = path.file_name().map_or_else(
                                || "the selected destination".to_owned(),
                                |name| name.to_string_lossy().into_owned(),
                            );
                            lifecycle.borrow_mut().pending_collision = Some((token, request));
                            panel.set_collision(&format!(
                                "{alias} already exists. Choose replace or save elsewhere."
                            ));
                        }
                        Err(ExportRunError::Cancelled) => {
                            panel.set_finished("PNG export cancelled.", true);
                        }
                        Err(error) => panel.set_finished(&error.to_string(), false),
                    }
                    return ControlFlow::Break;
                }
            }
        }
    });
}

fn export_snapshot(
    catalog: &GtkCatalogController,
    photo_id: rusttable_core::PhotoId,
) -> Result<
    (
        std::path::PathBuf,
        std::path::PathBuf,
        rusttable_core::EditId,
    ),
    String,
> {
    let crate::gtk_controller::GtkCatalogState::Ready(ready) = catalog.state() else {
        return Err("The library is unavailable.".to_owned());
    };
    let edit_id = crate::workspace::selected_edit_id(ready.location().catalog_path(), photo_id)
        .map_err(|error| format!("Could not snapshot the selected edit: {error}"))?;
    Ok((
        ready.location().catalog_path().to_owned(),
        ready.location().source_root().to_owned(),
        edit_id,
    ))
}

fn export_request(
    photo_id: rusttable_core::PhotoId,
    edit_id: rusttable_core::EditId,
    catalog_path: std::path::PathBuf,
    source_root: std::path::PathBuf,
    destination: std::path::PathBuf,
    size: rusttable_ui::ExportSize,
    collision: ExportCollisionSelection,
) -> Result<ExportRequest, String> {
    let size = match size {
        UiExportSize::Original => ExportSizeSelection::Original,
        UiExportSize::Fit2048 => ExportSizeSelection::Fit2048,
        UiExportSize::Fit4096 => ExportSizeSelection::Fit4096,
        UiExportSize::Custom(value) => {
            ExportSizeSelection::custom_maximum(value).map_err(|error| error.to_string())?
        }
    };
    Ok(ExportRequest::new(
        catalog_path,
        source_root,
        photo_id,
        edit_id,
        destination,
        ExportSettings::from_selection(size, collision),
    ))
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
