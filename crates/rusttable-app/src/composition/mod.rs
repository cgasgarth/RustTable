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
use crate::macos::{
    MacApplicationBridge, MacApplicationCommand, MacOpenRequest, MacTerminationDecision,
};
use gtk4::gio::prelude::{ActionMapExt, ApplicationExt, ApplicationExtManual, FileExt};
use gtk4::glib::{self, ControlFlow};
use gtk4::prelude::GtkWindowExt;
use gtk4::prelude::{GtkApplicationExt, RecentManagerExt, WidgetExt};
use rusttable_i18n::{I18n, LocaleSelection};
use rusttable_import::{RasterImportBatch, RasterImportStatus};
use rusttable_input::{
    ActionId, ActionInputService, ActionMapping, ActionMode, ActionPhase, Binding, InputSource,
    KeyCode, Modifiers,
};
use rusttable_ui::{
    CollectionControlAction, CollectionControlState, CollectionFilterState, CollectionProperty,
    ExportAction, ExportSize as UiExportSize,
};
use std::cell::RefCell;
use std::fmt;
use std::path::Path;
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
                .application_id(crate::macos::BUNDLE_IDENTIFIER)
                .build();
            let active_shell = Rc::new(RefCell::new(None::<rusttable_ui::GtkShell>));
            let active_catalog = Rc::new(RefCell::new(None::<Rc<RefCell<GtkCatalogController>>>));
            let active_collection = Rc::new(RefCell::new(None::<CollectionController>));
            let native_bridge = Rc::new(RefCell::new(MacApplicationBridge::default()));
            connect_application_signals(
                &application,
                Rc::clone(&active_shell),
                Rc::clone(&active_catalog),
                Rc::clone(&active_collection),
                Rc::clone(&native_bridge),
            );
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

fn connect_application_signals(
    application: &gtk4::Application,
    active_shell: Rc<RefCell<Option<rusttable_ui::GtkShell>>>,
    active_catalog: Rc<RefCell<Option<Rc<RefCell<GtkCatalogController>>>>>,
    active_collection: Rc<RefCell<Option<CollectionController>>>,
    native_bridge: Rc<RefCell<MacApplicationBridge>>,
) {
    application.connect_startup({
        let native_bridge = Rc::clone(&native_bridge);
        let active_shell = Rc::clone(&active_shell);
        move |application| install_application_menus(application, &native_bridge, &active_shell)
    });
    application.connect_open({
        let active_shell = Rc::clone(&active_shell);
        let active_catalog = Rc::clone(&active_catalog);
        let active_collection = Rc::clone(&active_collection);
        let native_bridge = Rc::clone(&native_bridge);
        move |_, files, _hint| {
            let delivery = native_bridge
                .borrow_mut()
                .receive_optional_paths(files.iter().map(FileExt::path));
            if let Some(request) = delivery.request().cloned() {
                dispatch_open_request(&request, &active_shell, &active_catalog, &active_collection);
            }
        }
    });
    application.connect_shutdown({
        let native_bridge = Rc::clone(&native_bridge);
        move |_| native_bridge.borrow_mut().mark_stopped()
    });
    application.connect_activate(move |application| {
        activate_application(
            application,
            &active_shell,
            &active_catalog,
            &active_collection,
            &native_bridge,
        );
    });
}

fn activate_application(
    application: &gtk4::Application,
    active_shell: &Rc<RefCell<Option<rusttable_ui::GtkShell>>>,
    active_catalog: &Rc<RefCell<Option<Rc<RefCell<GtkCatalogController>>>>>,
    active_collection: &Rc<RefCell<Option<CollectionController>>>,
    native_bridge: &Rc<RefCell<MacApplicationBridge>>,
) {
    if let Some(shell) = active_shell.borrow().as_ref() {
        shell.present();
        return;
    }

    if let Some(display) = gtk4::gdk::Display::default() {
        rusttable_ui::install_darktable_theme(&display);
    }
    let catalog_controller = Rc::new(RefCell::new(GtkCatalogController::load_persisted()));
    active_catalog.replace(Some(Rc::clone(&catalog_controller)));
    let resolved_locale = LocaleSelection::from_environment().resolve();
    active_collection.replace(
        catalog_controller
            .borrow()
            .collection_controller_with_locale(resolved_locale.locale().clone()),
    );
    let shell = rusttable_ui::GtkShell::with_i18n(
        application,
        I18n::new(resolved_locale.locale().clone()).unwrap_or_default(),
    );
    install_action_input(&shell);
    let export_panel = shell.export_panel().clone();
    let export_lifecycle = Rc::new(RefCell::new(ExportLifecycle::default()));
    let workspace = catalog_controller.borrow().state().workspace().cloned();
    if let Some(workspace) = workspace.as_ref() {
        shell.set_photo_workspace(workspace);
    }
    if let Some(controller) = active_collection.borrow().as_ref() {
        shell.set_collection_filter_state(&collection_filter_state(&controller.snapshot()));
    }
    let collection_for_actions = Rc::clone(active_collection);
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
    if let Some(request) = native_bridge.borrow_mut().mark_ready() {
        dispatch_open_request(&request, active_shell, active_catalog, active_collection);
    }
}

fn install_action_input(shell: &rusttable_ui::GtkShell) {
    let mut service = ActionInputService::new();
    service.add_mapping(ActionMapping::new(
        ActionId::new("view/lighttable").expect("static action id"),
        Binding::Keyboard {
            key: KeyCode::character('l'),
            modifiers: Modifiers::empty(),
        },
    ));
    service.add_mapping(ActionMapping::new(
        ActionId::new("view/darkroom").expect("static action id"),
        Binding::Keyboard {
            key: KeyCode::character('d'),
            modifiers: Modifiers::empty(),
        },
    ));
    service.add_mapping(
        ActionMapping::new(
            ActionId::new("window/fullscreen").expect("static action id"),
            Binding::Keyboard {
                key: KeyCode::named("F11"),
                modifiers: Modifiers::empty(),
            },
        )
        .with_mode(ActionMode::Activate),
    );
    let service = Rc::new(RefCell::new(service));
    let shell = shell.clone();
    let window = shell.window().clone();
    let callback_window = window.clone();
    let _input_adapter = rusttable_ui::GtkInputAdapter::attach(&window, &service, move |event| {
        if event.phase != ActionPhase::Pressed || event.source != InputSource::Keyboard {
            return;
        }
        match event.action.as_str() {
            "view/lighttable" => shell.show_workspace(rusttable_ui::WorkspaceRole::Lighttable),
            "view/darkroom" => shell.show_workspace(rusttable_ui::WorkspaceRole::Darkroom),
            "window/fullscreen" => {
                if callback_window.is_fullscreen() {
                    callback_window.unfullscreen();
                } else {
                    callback_window.fullscreen();
                }
            }
            _ => {}
        }
    });
}

fn install_application_menus(
    application: &gtk4::Application,
    native_bridge: &Rc<RefCell<MacApplicationBridge>>,
    active_shell: &Rc<RefCell<Option<rusttable_ui::GtkShell>>>,
) {
    let actions = [
        ("about", MacApplicationCommand::About),
        ("preferences", MacApplicationCommand::Preferences),
        ("services", MacApplicationCommand::Services),
        ("hide", MacApplicationCommand::Hide),
        ("hide-others", MacApplicationCommand::HideOthers),
        ("show-all", MacApplicationCommand::ShowAll),
        ("window", MacApplicationCommand::Window),
        ("quit", MacApplicationCommand::Quit),
    ];
    for (name, command) in actions {
        let action = gtk4::gio::SimpleAction::new(name, None);
        let action_application = application.clone();
        let native_bridge = Rc::clone(native_bridge);
        let active_shell = Rc::clone(active_shell);
        action.connect_activate(move |_, _| match command {
            MacApplicationCommand::Quit => {
                if native_bridge.borrow_mut().request_termination(false, true)
                    == MacTerminationDecision::Proceed
                {
                    action_application.quit();
                }
            }
            MacApplicationCommand::Hide => {
                if let Some(shell) = active_shell.borrow().as_ref() {
                    shell.window().set_visible(false);
                }
            }
            MacApplicationCommand::ShowAll
            | MacApplicationCommand::About
            | MacApplicationCommand::Preferences
            | MacApplicationCommand::Window => {
                if let Some(shell) = active_shell.borrow().as_ref() {
                    shell.present();
                }
            }
            MacApplicationCommand::HideOthers | MacApplicationCommand::Services => {}
        });
        application.add_action(&action);
    }
    application.set_accels_for_action("app.preferences", &["<Primary>comma"]);
    application.set_accels_for_action("app.hide", &["<Primary>h"]);
    application.set_accels_for_action("app.quit", &["<Primary>q"]);

    let application_menu = gtk4::gio::Menu::new();
    application_menu.append(Some("About RustTable"), Some("app.about"));
    application_menu.append(Some("Preferences…"), Some("app.preferences"));
    application_menu.append(Some("Services"), Some("app.services"));
    application_menu.append(Some("Hide RustTable"), Some("app.hide"));
    application_menu.append(Some("Hide Others"), Some("app.hide-others"));
    application_menu.append(Some("Show All"), Some("app.show-all"));
    application_menu.append(Some("Quit RustTable"), Some("app.quit"));

    let window_menu = gtk4::gio::Menu::new();
    window_menu.append(Some("Window"), Some("app.window"));

    let menubar = gtk4::gio::Menu::new();
    menubar.append_submenu(Some("RustTable"), &application_menu);
    menubar.append_submenu(Some("Window"), &window_menu);
    application.set_menubar(Some(&menubar));
}

fn dispatch_open_request(
    request: &MacOpenRequest,
    active_shell: &Rc<RefCell<Option<rusttable_ui::GtkShell>>>,
    active_catalog: &Rc<RefCell<Option<Rc<RefCell<GtkCatalogController>>>>>,
    active_collection: &Rc<RefCell<Option<CollectionController>>>,
) {
    let Some(catalog) = active_catalog.borrow().as_ref().cloned() else {
        return;
    };
    let Some(shell) = active_shell.borrow().as_ref().cloned() else {
        return;
    };

    if let Some(path) = request.catalog_path() {
        *catalog.borrow_mut() = GtkCatalogController::load_catalog_at(path.to_path_buf());
        refresh_catalog_shell(&shell, &catalog, active_collection);
        if catalog.borrow().opened_successfully() {
            record_recent_path(path);
        }
    }

    let image_paths = request
        .image_paths()
        .map(Path::to_path_buf)
        .collect::<Vec<_>>();
    if image_paths.is_empty() {
        return;
    }
    let Some(catalog_path) = catalog.borrow().catalog_path().map(Path::to_path_buf) else {
        return;
    };
    let recent_paths = image_paths.clone();
    let (sender, receiver) = mpsc::channel::<RasterImportBatch>();
    let worker_catalog_path = catalog_path.clone();
    thread::spawn(move || {
        let batch = crate::workspace::run_raster_import(
            &worker_catalog_path,
            image_paths,
            &rusttable_import::RasterImportCancellation::default(),
            &|_| {},
        );
        let _ = sender.send(batch);
    });

    let active_collection = Rc::clone(active_collection);
    glib::timeout_add_local(Duration::from_millis(16), move || {
        match receiver.try_recv() {
            Ok(batch) => {
                record_successful_recent_paths(&recent_paths, &batch);
                *catalog.borrow_mut() = GtkCatalogController::load_catalog_at(catalog_path.clone());
                refresh_catalog_shell(&shell, &catalog, &active_collection);
                ControlFlow::Break
            }
            Err(TryRecvError::Empty) => ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => ControlFlow::Break,
        }
    });
}

fn record_successful_recent_paths(paths: &[std::path::PathBuf], batch: &RasterImportBatch) {
    for (path, receipt) in paths.iter().zip(batch.receipts()) {
        if matches!(
            receipt.status,
            RasterImportStatus::Imported
                | RasterImportStatus::AlreadyImported
                | RasterImportStatus::ImportedPreviewPending
                | RasterImportStatus::ImportedPreviewFailed
        ) {
            record_recent_path(path);
        }
    }
}

fn record_recent_path(path: &Path) {
    let file = gtk4::gio::File::for_path(path);
    let _ = gtk4::RecentManager::default().add_item(file.uri().as_str());
}

fn refresh_catalog_shell(
    shell: &rusttable_ui::GtkShell,
    catalog: &Rc<RefCell<GtkCatalogController>>,
    active_collection: &Rc<RefCell<Option<CollectionController>>>,
) {
    let controller = catalog.borrow();
    active_collection.replace(controller.collection_controller());
    if let Some(workspace) = controller.state().workspace() {
        shell.set_photo_workspace(workspace);
    }
    if let Some(collection) = active_collection.borrow().as_ref() {
        shell.set_collection_filter_state(&collection_filter_state(&collection.snapshot()));
    }
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
