mod edit;
mod export;
mod preview;

use iced::Task;
use std::path::PathBuf;

use crate::library::{self, LibraryLoadRequestId, LibraryLoadResult};
use crate::workspace::{BasicEditSession, SelectedPreview, pick_raster_files, run_raster_import};
use rusttable_core::PhotoId;
use rusttable_import::{
    RasterImportBatch, RasterImportCancellation, RasterImportProgress, RasterImportStage,
    RasterImportStatus,
};
use rusttable_ui::{
    ImportPanelViewModel, ImportRowState, ImportRowViewModel, InputIntent, LibraryFailureKind,
    LibraryState, NavigationIntent, PhotoWorkspaceViewModel, PresentationText, UiEffect, UiMessage,
    UiState, WorkspaceRoute,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveExport {
    photo_id: PhotoId,
    request: export::ExportRequest,
    cancellation: export::ExportCancellation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingExport {
    photo_id: PhotoId,
    settings: export::ExportSettings,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingExportCollision {
    photo_id: PhotoId,
    request: export::ExportRequest,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Shell {
    ui: UiState,
    active_load_request_id: LibraryLoadRequestId,
    load_in_flight: bool,
    catalog_path: Result<PathBuf, LibraryFailureKind>,
    source_root: Result<PathBuf, LibraryFailureKind>,
    preview_generation: u64,
    active_preview: Option<(u64, PhotoId)>,
    import_paths: Vec<PathBuf>,
    import_cancellation: Option<RasterImportCancellation>,
    active_export: Option<ActiveExport>,
    pending_export: Option<PendingExport>,
    pending_export_collision: Option<PendingExportCollision>,
    pending_import_selection: Option<PhotoId>,
    basic_edit: Option<BasicEditSession>,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            ui: UiState::default(),
            active_load_request_id: LibraryLoadRequestId::first(),
            load_in_flight: false,
            catalog_path: Err(LibraryFailureKind::CatalogLocationUnavailable),
            source_root: Err(LibraryFailureKind::CatalogLocationUnavailable),
            preview_generation: 0,
            active_preview: None,
            import_paths: Vec::new(),
            import_cancellation: None,
            active_export: None,
            pending_export: None,
            pending_export_collision: None,
            pending_import_selection: None,
            basic_edit: None,
        }
    }
}

pub(crate) fn boot() -> (Shell, Task<Message>) {
    let request_id = LibraryLoadRequestId::first();
    let catalog_path = library::catalog_path();
    let source_root = catalog_path
        .as_ref()
        .map_err(|kind| *kind)
        .and_then(|path| library::source_root(path));
    let shell = Shell::loading(request_id, catalog_path.clone(), source_root);
    let task = start_load(request_id, catalog_path);
    (shell, task)
}

impl Shell {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "injection is the boundary for the future presentation adapter"
        )
    )]
    pub(crate) fn with_photo_workspace(photo_workspace: PhotoWorkspaceViewModel) -> Self {
        Self::with_library_state(LibraryState::Ready(photo_workspace))
    }

    pub(crate) fn with_library_state(library_state: LibraryState) -> Self {
        Self {
            ui: UiState::with_library_state(library_state),
            active_load_request_id: LibraryLoadRequestId::first(),
            load_in_flight: false,
            catalog_path: Err(LibraryFailureKind::CatalogLocationUnavailable),
            source_root: Err(LibraryFailureKind::CatalogLocationUnavailable),
            preview_generation: 0,
            active_preview: None,
            import_paths: Vec::new(),
            import_cancellation: None,
            active_export: None,
            pending_export: None,
            pending_export_collision: None,
            pending_import_selection: None,
            basic_edit: None,
        }
    }

    fn loading(
        request_id: LibraryLoadRequestId,
        catalog_path: Result<PathBuf, LibraryFailureKind>,
        source_root: Result<PathBuf, LibraryFailureKind>,
    ) -> Self {
        Self {
            ui: UiState::with_library_state(LibraryState::Loading),
            active_load_request_id: request_id,
            load_in_flight: true,
            catalog_path,
            source_root,
            preview_generation: 0,
            active_preview: None,
            import_paths: Vec::new(),
            import_cancellation: None,
            active_export: None,
            pending_export: None,
            pending_export_collision: None,
            pending_import_selection: None,
            basic_edit: None,
        }
    }

    pub(crate) fn library_state(&self) -> &LibraryState {
        self.ui.library_state()
    }

    pub(crate) fn ui_state(&self) -> &UiState {
        &self.ui
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "request identity is asserted by reducer tests")
    )]
    pub(crate) fn active_load_request_id(&self) -> LibraryLoadRequestId {
        self.active_load_request_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Message {
    ToggleSidebar,
    Navigate(NavigationIntent),
    LibraryLoaded {
        request_id: LibraryLoadRequestId,
        result: LibraryLoadResult,
    },
    RetryLibrary,
    Input(InputIntent),
    PreviewLoaded {
        generation: u64,
        photo_id: PhotoId,
        result: PreviewLoadResult,
    },
    EditLoaded {
        photo_id: PhotoId,
        result: edit::EditLoadResult,
    },
    EditCommitted {
        photo_id: PhotoId,
        result: edit::EditCommitResult,
    },
    ImportFiles,
    ImportPickerCompleted(Vec<PathBuf>),
    FilesDropped(Vec<PathBuf>),
    CancelImport,
    RetryImport(u64),
    RemoveImportResult(u64),
    CloseImportPanel,
    SaveRenderedCopy(PhotoId),
    ExportDestinationSelected {
        photo_id: PhotoId,
        destination: Option<PathBuf>,
    },
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Iced collision dialog will send an explicit replacement choice"
        )
    )]
    ExportCollisionSelected {
        photo_id: PhotoId,
        selection: export::ExportCollisionSelection,
    },
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the Iced export progress control sends cancellation events"
        )
    )]
    CancelExport(PhotoId),
    ExportStatus {
        photo_id: PhotoId,
        status: export::ExportStatus,
    },
    ExportFinished {
        photo_id: PhotoId,
        result: export::ExportTaskResult,
    },
    ImportProgress(RasterImportProgress),
    ImportFinished(Option<RasterImportBatch>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreviewLoadResult {
    Ready(SelectedPreview),
    Draft(SelectedPreview),
    Failed,
}

impl From<UiMessage> for Message {
    fn from(message: UiMessage) -> Self {
        match message {
            UiMessage::ToggleSidebar => Self::ToggleSidebar,
            UiMessage::Navigate(intent) => Self::Navigate(intent),
            UiMessage::RetryLibrary => Self::RetryLibrary,
            UiMessage::Input(intent) => Self::Input(intent),
            UiMessage::ImportFiles => Self::ImportFiles,
            UiMessage::CancelImport => Self::CancelImport,
            UiMessage::RetryImport(item_id) => Self::RetryImport(item_id),
            UiMessage::RemoveImportResult(item_id) => Self::RemoveImportResult(item_id),
            UiMessage::CloseImportPanel => Self::CloseImportPanel,
            UiMessage::SaveRenderedCopy(photo_id) => Self::SaveRenderedCopy(photo_id),
        }
    }
}

pub(crate) fn update(shell: &mut Shell, message: Message) -> Task<Message> {
    match message {
        Message::ToggleSidebar => return handle_ui_message(shell, UiMessage::ToggleSidebar),
        Message::Navigate(intent) => return handle_ui_message(shell, UiMessage::Navigate(intent)),
        Message::Input(intent) => return handle_ui_message(shell, UiMessage::Input(intent)),
        Message::LibraryLoaded { request_id, result } => {
            return handle_library_loaded(shell, request_id, result);
        }
        Message::RetryLibrary => return retry_library(shell),
        Message::PreviewLoaded {
            generation,
            photo_id,
            result,
        } => preview::handle_loaded(shell, generation, photo_id, result),
        Message::EditLoaded {
            photo_id,
            ref result,
        } => edit::apply_loaded(shell, photo_id, result),
        Message::EditCommitted {
            photo_id,
            ref result,
        } => return edit::apply_committed(shell, photo_id, result),
        Message::ImportFiles => return import_picker_task(),
        Message::ImportPickerCompleted(paths) | Message::FilesDropped(paths) => {
            return begin_import(shell, paths);
        }
        Message::CancelImport => cancel_import(shell),
        Message::RetryImport(item_id) => return retry_import(shell, item_id),
        Message::RemoveImportResult(item_id) => {
            let _ = shell.ui.handle(UiMessage::RemoveImportResult(item_id));
        }
        Message::CloseImportPanel => {
            let _ = shell.ui.handle(UiMessage::CloseImportPanel);
        }
        message @ (Message::SaveRenderedCopy(_)
        | Message::ExportDestinationSelected { .. }
        | Message::ExportCollisionSelected { .. }
        | Message::CancelExport(_)
        | Message::ExportStatus { .. }
        | Message::ExportFinished { .. }) => return handle_export_message(shell, message),
        Message::ImportProgress(progress) => {
            shell.ui.update_import_row(
                progress.item_id.get(),
                import_progress_row_state(progress.stage),
            );
        }
        Message::ImportFinished(Some(batch)) => return finish_import(shell, &batch),
        Message::ImportFinished(None) => {
            shell.import_cancellation = None;
            shell
                .ui
                .set_import_panel(failed_import_panel(&shell.import_paths));
        }
    }
    Task::none()
}

fn handle_export_message(shell: &mut Shell, message: Message) -> Task<Message> {
    match message {
        Message::SaveRenderedCopy(photo_id) => begin_destination_pick(shell, photo_id),
        Message::ExportDestinationSelected {
            photo_id,
            destination: Some(destination),
        } => begin_export(shell, photo_id, destination),
        Message::ExportDestinationSelected {
            photo_id,
            destination: None,
        } => {
            clear_pending_destination(shell, photo_id);
            Task::none()
        }
        Message::ExportCollisionSelected {
            photo_id,
            selection,
        } => resolve_export_collision(shell, photo_id, selection),
        Message::CancelExport(photo_id) => {
            cancel_export(shell, photo_id);
            Task::none()
        }
        Message::ExportStatus { photo_id, status } => {
            if active_export_matches(shell, photo_id) {
                shell
                    .ui
                    .set_export_status(photo_id, status.text().to_owned());
            }
            Task::none()
        }
        Message::ExportFinished { photo_id, result } => {
            if !active_export_matches(shell, photo_id) {
                return Task::none();
            }
            let status = match result {
                export::ExportTaskResult::Completed { summary, collision } => {
                    shell.active_export = None;
                    format!("{summary} {}", collision.text())
                }
                export::ExportTaskResult::Collision(collision) => {
                    let active = shell
                        .active_export
                        .take()
                        .expect("active export was checked above");
                    shell.pending_export_collision = Some(PendingExportCollision {
                        photo_id,
                        request: active.request,
                    });
                    collision.text().to_owned()
                }
                export::ExportTaskResult::Failed(status) => {
                    shell.active_export = None;
                    status
                }
            };
            shell.ui.set_export_status(photo_id, status);
            Task::none()
        }
        _ => unreachable!("only export messages reach the export handler"),
    }
}

fn begin_destination_pick(shell: &mut Shell, photo_id: PhotoId) -> Task<Message> {
    if shell.active_export.is_some() {
        shell.ui.set_export_status(
            photo_id,
            "Another export is already in progress.".to_owned(),
        );
        return Task::none();
    }
    let size = export::ExportSizeSelection::from_ui(shell.ui.export_size());
    shell.pending_export = Some(PendingExport {
        photo_id,
        settings: export::ExportSettings::from_selection(
            size,
            export::ExportCollisionSelection::CreateNew,
        ),
    });
    export::pick_destination(photo_id)
}

fn begin_export(shell: &mut Shell, photo_id: PhotoId, destination: PathBuf) -> Task<Message> {
    if shell.active_export.is_some() {
        shell.ui.set_export_status(
            photo_id,
            "Another export is already in progress.".to_owned(),
        );
        return Task::none();
    }
    let (Ok(catalog_path), Ok(source_root)) =
        (shell.catalog_path.clone(), shell.source_root.clone())
    else {
        shell.ui.set_export_status(
            photo_id,
            "The catalog is unavailable for export.".to_owned(),
        );
        return Task::none();
    };
    let Some(pending) = shell
        .pending_export
        .take()
        .filter(|pending| pending.photo_id == photo_id)
    else {
        return Task::none();
    };
    let request = export::ExportRequest::new(
        catalog_path,
        source_root,
        photo_id,
        destination,
        pending.settings,
    );
    start_export(shell, request)
}

fn resolve_export_collision(
    shell: &mut Shell,
    photo_id: PhotoId,
    selection: export::ExportCollisionSelection,
) -> Task<Message> {
    let Some(pending) = shell
        .pending_export_collision
        .take()
        .filter(|pending| pending.photo_id == photo_id)
    else {
        return Task::none();
    };
    match selection {
        export::ExportCollisionSelection::CreateNew => {
            shell.pending_export = Some(PendingExport {
                photo_id,
                settings: pending.request.settings(),
            });
            export::pick_destination(photo_id)
        }
        export::ExportCollisionSelection::ReplaceExisting => {
            start_export(shell, pending.request.with_collision(selection))
        }
    }
}

fn start_export(shell: &mut Shell, request: export::ExportRequest) -> Task<Message> {
    let photo_id = request.photo_id();
    let task = export::start_request(request.clone());
    shell.active_export = Some(ActiveExport {
        photo_id,
        request,
        cancellation: task.cancellation(),
    });
    task.into_task()
}

fn clear_pending_destination(shell: &mut Shell, photo_id: PhotoId) {
    if shell
        .pending_export
        .as_ref()
        .is_some_and(|pending| pending.photo_id == photo_id)
    {
        shell.pending_export = None;
    }
}

fn handle_ui_message(shell: &mut Shell, message: UiMessage) -> Task<Message> {
    if let UiMessage::Input(InputIntent::BasicEdit(intent)) = message {
        return edit::handle_intent(shell, intent);
    }
    let previous_route = shell.ui.route();
    match shell.ui.handle(message) {
        UiEffect::RetryLibrary => return retry_library(shell),
        UiEffect::ImportFiles => return import_picker_task(),
        UiEffect::CancelImport => cancel_import(shell),
        UiEffect::RetryImport(item_id) => return retry_import(shell, item_id),
        UiEffect::SaveRenderedCopy(photo_id) => return begin_destination_pick(shell, photo_id),
        UiEffect::None => {}
    }
    preview::reconcile_route(shell, previous_route);
    if let WorkspaceRoute::PhotoDetail(photo_id) = shell.ui.route()
        && previous_route != shell.ui.route()
    {
        shell.basic_edit = None;
        return Task::batch([
            preview::start_persisted(shell, photo_id),
            edit::start_load(shell.catalog_path.as_ref().ok().cloned(), photo_id),
        ]);
    }
    Task::none()
}

fn handle_library_loaded(
    shell: &mut Shell,
    request_id: LibraryLoadRequestId,
    result: LibraryLoadResult,
) -> Task<Message> {
    if !shell.load_in_flight || request_id != shell.active_load_request_id {
        return Task::none();
    }
    shell.load_in_flight = false;
    shell.ui.set_library_state(result.into_library_state());
    shell.active_preview = None;
    if let WorkspaceRoute::PhotoDetail(photo_id) = shell.ui.route() {
        if shell
            .ui
            .library_state()
            .ready_workspace()
            .is_some_and(|workspace| workspace.detail(photo_id).is_some())
        {
            return preview::start_persisted(shell, photo_id);
        }
        let _ = shell
            .ui
            .handle(UiMessage::Navigate(NavigationIntent::ShowLibrary));
    }
    if let Some(photo_id) = shell.pending_import_selection.take()
        && shell
            .ui
            .library_state()
            .ready_workspace()
            .is_some_and(|workspace| workspace.detail(photo_id).is_some())
    {
        let _ = shell
            .ui
            .handle(UiMessage::Navigate(NavigationIntent::ShowPhoto(photo_id)));
        return preview::start_persisted(shell, photo_id);
    }
    Task::none()
}

fn import_picker_task() -> Task<Message> {
    Task::perform(pick_raster_files(), Message::ImportPickerCompleted)
}

fn begin_import(shell: &mut Shell, mut paths: Vec<PathBuf>) -> Task<Message> {
    if shell.import_cancellation.is_some() || paths.is_empty() {
        return Task::none();
    }
    paths.truncate(rusttable_import::MAX_RASTER_IMPORT_ITEMS);
    let rows = paths
        .iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let item_id = u64::try_from(index).ok()?.checked_add(1)?;
            let alias = safe_import_alias(path);
            Some(ImportRowViewModel::new(
                item_id,
                alias,
                ImportRowState::Queued,
            ))
        })
        .collect();
    shell
        .ui
        .set_import_panel(ImportPanelViewModel::new(rows, true));
    shell.import_paths.clone_from(&paths);
    let cancellation = RasterImportCancellation::default();
    shell.import_cancellation = Some(cancellation.clone());
    let Ok(catalog_path) = shell.catalog_path.clone() else {
        shell.import_cancellation = None;
        shell.ui.set_import_panel(failed_import_panel(&paths));
        return Task::none();
    };
    import_task(catalog_path, paths, cancellation)
}

fn import_task(
    catalog_path: PathBuf,
    paths: Vec<PathBuf>,
    cancellation: RasterImportCancellation,
) -> Task<Message> {
    let sipper = iced::task::sipper(move |sender| async move {
        let (finished, receiver) = iced::futures::channel::oneshot::channel();
        std::thread::spawn(move || {
            let sender = std::sync::Mutex::new(sender);
            let observer = |progress| {
                let mut sender = sender
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                iced::futures::executor::block_on(sender.send(progress));
            };
            let batch = run_raster_import(&catalog_path, paths, &cancellation, &observer);
            let _ = finished.send(batch);
        });
        receiver.await.ok()
    });
    Task::sip(sipper, Message::ImportProgress, Message::ImportFinished)
}

fn finish_import(shell: &mut Shell, batch: &RasterImportBatch) -> Task<Message> {
    shell.import_cancellation = None;
    shell.pending_import_selection = batch.first_selected_photo();
    let rows = batch
        .receipts()
        .map(|receipt| {
            ImportRowViewModel::new(
                receipt.item_id.get(),
                PresentationText::new(&receipt.source_alias)
                    .unwrap_or_else(|_| PresentationText::new("Image").expect("constant text")),
                import_row_state(receipt.status),
            )
        })
        .collect();
    shell
        .ui
        .set_import_panel(ImportPanelViewModel::new(rows, false));
    refresh_library(shell)
}

fn refresh_library(shell: &mut Shell) -> Task<Message> {
    let Some(request_id) = shell.active_load_request_id.next() else {
        return Task::none();
    };
    shell.active_load_request_id = request_id;
    shell.load_in_flight = true;
    start_load(request_id, shell.catalog_path.clone())
}

fn retry_import(shell: &mut Shell, item_id: u64) -> Task<Message> {
    let path = usize::try_from(item_id)
        .ok()
        .and_then(|item_id| item_id.checked_sub(1))
        .and_then(|index| shell.import_paths.get(index))
        .cloned();
    path.map_or_else(Task::none, |path| begin_import(shell, vec![path]))
}

fn cancel_import(shell: &mut Shell) {
    if let Some(cancellation) = &shell.import_cancellation {
        cancellation.cancel();
    }
}

fn active_export_matches(shell: &Shell, photo_id: PhotoId) -> bool {
    shell
        .active_export
        .as_ref()
        .is_some_and(|export| export.photo_id == photo_id)
}

fn cancel_export(shell: &mut Shell, photo_id: PhotoId) {
    let Some(export) = shell
        .active_export
        .as_ref()
        .filter(|export| export.photo_id == photo_id)
    else {
        return;
    };
    export.cancellation.cancel();
    shell
        .ui
        .set_export_status(photo_id, "Cancelling export…".to_owned());
}

fn safe_import_alias(path: &std::path::Path) -> PresentationText {
    let alias = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Image")
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect::<String>();
    PresentationText::new(if alias.is_empty() { "Image" } else { &alias })
        .unwrap_or_else(|_| PresentationText::new("Image").expect("constant text"))
}

fn failed_import_panel(paths: &[PathBuf]) -> ImportPanelViewModel {
    ImportPanelViewModel::new(
        paths
            .iter()
            .enumerate()
            .filter_map(|(index, path)| {
                Some(ImportRowViewModel::new(
                    u64::try_from(index).ok()?.checked_add(1)?,
                    safe_import_alias(path),
                    ImportRowState::Failed,
                ))
            })
            .collect(),
        false,
    )
}

const fn import_row_state(status: RasterImportStatus) -> ImportRowState {
    match status {
        RasterImportStatus::Imported => ImportRowState::Completed,
        RasterImportStatus::AlreadyImported => ImportRowState::AlreadyImported,
        RasterImportStatus::ImportedPreviewPending => ImportRowState::ImportedPreviewPending,
        RasterImportStatus::ImportedPreviewFailed => ImportRowState::ImportedPreviewFailed,
        RasterImportStatus::Failed(_) => ImportRowState::Failed,
        RasterImportStatus::Cancelled => ImportRowState::Cancelled,
    }
}

const fn import_progress_row_state(stage: RasterImportStage) -> ImportRowState {
    match stage {
        RasterImportStage::Queued => ImportRowState::Queued,
        RasterImportStage::Opening => ImportRowState::Opening,
        RasterImportStage::Hashing => ImportRowState::Hashing,
        RasterImportStage::Probing => ImportRowState::Probing,
        RasterImportStage::DecodingHeader => ImportRowState::DecodingHeader,
        RasterImportStage::Registering => ImportRowState::Registering,
        RasterImportStage::GeneratingPreview => ImportRowState::GeneratingPreview,
        RasterImportStage::Completed => ImportRowState::Completed,
        RasterImportStage::AlreadyImported => ImportRowState::AlreadyImported,
        RasterImportStage::Failed => ImportRowState::Failed,
        RasterImportStage::Cancelled => ImportRowState::Cancelled,
    }
}

fn start_load(
    request_id: LibraryLoadRequestId,
    catalog_path: Result<PathBuf, LibraryFailureKind>,
) -> Task<Message> {
    match catalog_path {
        Ok(path) => load_task(request_id, path),
        Err(kind) => Task::done(Message::LibraryLoaded {
            request_id,
            result: LibraryLoadResult::Failed(kind),
        }),
    }
}

fn load_task(request_id: LibraryLoadRequestId, path: std::path::PathBuf) -> Task<Message> {
    Task::perform(async move { library::load_catalog(&path) }, move |result| {
        Message::LibraryLoaded { request_id, result }
    })
}

fn retry_library(shell: &mut Shell) -> Task<Message> {
    if !matches!(shell.library_state(), LibraryState::Failed(_)) || shell.load_in_flight {
        return Task::none();
    }
    let Some(request_id) = shell.active_load_request_id.next() else {
        return Task::none();
    };
    shell.active_load_request_id = request_id;
    shell.load_in_flight = true;
    shell.active_preview = None;
    shell.ui.begin_library_load();
    start_load(request_id, shell.catalog_path.clone())
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
