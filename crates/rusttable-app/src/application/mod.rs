use iced::Task;
use std::path::PathBuf;

use crate::library::{self, LibraryLoadRequestId, LibraryLoadResult};
use crate::workspace::{SelectedPreview, load_selected_preview};
use rusttable_core::PhotoId;
use rusttable_ui::{
    InputIntent, LibraryFailureKind, LibraryState, NavigationIntent, PhotoWorkspaceViewModel,
    PresentationText, PreviewDimensions, Rgba8PreviewMetadata, SelectedPreviewFailure,
    SelectedPreviewState, UiEffect, UiMessage, UiState, WorkspaceRoute,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Shell {
    ui: UiState,
    active_load_request_id: LibraryLoadRequestId,
    load_in_flight: bool,
    catalog_path: Result<PathBuf, LibraryFailureKind>,
    source_root: Result<PathBuf, LibraryFailureKind>,
    preview_generation: u64,
    active_preview: Option<(u64, PhotoId)>,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreviewLoadResult {
    Ready(SelectedPreview),
    Failed,
}

impl From<UiMessage> for Message {
    fn from(message: UiMessage) -> Self {
        match message {
            UiMessage::ToggleSidebar => Self::ToggleSidebar,
            UiMessage::Navigate(intent) => Self::Navigate(intent),
            UiMessage::RetryLibrary => Self::RetryLibrary,
            UiMessage::Input(intent) => Self::Input(intent),
        }
    }
}

pub(crate) fn update(shell: &mut Shell, message: Message) -> Task<Message> {
    match message {
        Message::ToggleSidebar | Message::Navigate(_) | Message::Input(_) => {
            let previous_route = shell.ui.route();
            let ui_message = match message {
                Message::ToggleSidebar => UiMessage::ToggleSidebar,
                Message::Navigate(intent) => UiMessage::Navigate(intent),
                Message::Input(intent) => UiMessage::Input(intent),
                Message::LibraryLoaded { .. }
                | Message::RetryLibrary
                | Message::PreviewLoaded { .. } => unreachable!(),
            };
            if shell.ui.handle(ui_message) == UiEffect::RetryLibrary {
                return retry_library(shell);
            }
            reconcile_preview_route(shell, previous_route);
            if let WorkspaceRoute::PhotoDetail(photo_id) = shell.ui.route()
                && previous_route != shell.ui.route()
            {
                return start_preview(shell, photo_id);
            }
        }
        Message::LibraryLoaded { request_id, result } => {
            if shell.load_in_flight && request_id == shell.active_load_request_id {
                shell.load_in_flight = false;
                shell.ui.set_library_state(result.into_library_state());
                shell.active_preview = None;
            }
        }
        Message::RetryLibrary => return retry_library(shell),
        Message::PreviewLoaded {
            generation,
            photo_id,
            result,
        } => {
            if shell.active_preview == Some((generation, photo_id))
                && shell.ui.route() == WorkspaceRoute::PhotoDetail(photo_id)
            {
                shell.active_preview = None;
                match result {
                    PreviewLoadResult::Ready(preview) => publish_preview(shell, photo_id, preview),
                    PreviewLoadResult::Failed => {
                        replace_preview(shell, photo_id, preview_failed_state());
                    }
                }
            }
        }
    }
    Task::none()
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

fn reconcile_preview_route(shell: &mut Shell, previous_route: WorkspaceRoute) {
    if previous_route != shell.ui.route() && matches!(shell.ui.route(), WorkspaceRoute::Library) {
        shell.active_preview = None;
    }
}

fn start_preview(shell: &mut Shell, photo_id: PhotoId) -> Task<Message> {
    let Some(generation) = shell.preview_generation.checked_add(1) else {
        replace_preview(shell, photo_id, preview_failed_state());
        return Task::none();
    };
    shell.preview_generation = generation;
    shell.active_preview = Some((generation, photo_id));
    replace_preview(shell, photo_id, SelectedPreviewState::Loading);
    let (Ok(catalog_path), Ok(source_root)) = (&shell.catalog_path, &shell.source_root) else {
        shell.active_preview = None;
        replace_preview(shell, photo_id, preview_failed_state());
        return Task::none();
    };
    let catalog_path = catalog_path.clone();
    let source_root = source_root.clone();
    Task::perform(
        async move {
            load_selected_preview(&catalog_path, &source_root, photo_id)
                .map_or(PreviewLoadResult::Failed, PreviewLoadResult::Ready)
        },
        move |result| Message::PreviewLoaded {
            generation,
            photo_id,
            result,
        },
    )
}

fn replace_preview(shell: &mut Shell, photo_id: PhotoId, preview: SelectedPreviewState) {
    let Some(workspace) = shell.ui.library_state().ready_workspace().cloned() else {
        return;
    };
    let Some(workspace) = workspace.with_selected_preview(photo_id, preview) else {
        return;
    };
    shell.ui.set_library_state(LibraryState::Ready(workspace));
}

fn publish_preview(shell: &mut Shell, photo_id: PhotoId, preview: SelectedPreview) {
    let (_, dimensions, pixels) = preview.into_parts();
    let ready = PreviewDimensions::new(dimensions.width(), dimensions.height())
        .ok()
        .and_then(|dimensions| {
            Rgba8PreviewMetadata::new(
                dimensions,
                PresentationText::new("Current persisted edit").expect("constant status is valid"),
                pixels,
            )
            .ok()
        })
        .map_or_else(preview_failed_state, SelectedPreviewState::Ready);
    replace_preview(shell, photo_id, ready);
}

fn preview_failed_state() -> SelectedPreviewState {
    SelectedPreviewState::Failed(SelectedPreviewFailure::new(
        PresentationText::new("The selected preview could not be rendered.")
            .expect("constant failure text is valid"),
    ))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::library::{LibraryFailureKind, LibraryState};
    use crate::library::{LibraryLoadRequestId, LibraryLoadResult};
    use rusttable_core::PhotoId;
    use rusttable_ui::{
        NavigationIntent, PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel,
        PresentationText, SelectedPreviewState, UiState, WorkspaceRoute,
    };

    use super::{Message, PreviewLoadResult, Shell, update};

    fn photo_id() -> PhotoId {
        PhotoId::new(1).expect("test photo ID is non-zero")
    }

    fn workspace() -> PhotoWorkspaceViewModel {
        let photo_id = photo_id();
        PhotoWorkspaceViewModel::new(
            vec![PhotoCardViewModel::new(
                photo_id,
                PresentationText::new("Test photo").expect("test title is valid"),
                None,
            )],
            vec![PhotoDetailViewModel::new(
                photo_id,
                PresentationText::new("Test photo").expect("test title is valid"),
                Vec::new(),
            )],
        )
        .expect("test workspace is valid")
    }

    #[test]
    fn default_shell_shows_the_sidebar() {
        assert_eq!(
            Shell::default(),
            Shell {
                ui: UiState::default(),
                active_load_request_id: LibraryLoadRequestId::first(),
                load_in_flight: false,
                catalog_path: Err(LibraryFailureKind::CatalogLocationUnavailable),
                source_root: Err(LibraryFailureKind::CatalogLocationUnavailable),
                preview_generation: 0,
                active_preview: None,
            }
        );
    }

    #[test]
    fn toggle_sidebar_hides_it() {
        let mut shell = Shell::default();

        let _ = update(&mut shell, Message::ToggleSidebar);

        assert!(!shell.ui_state().sidebar_visible());
    }

    #[test]
    fn toggling_sidebar_twice_restores_it() {
        let mut shell = Shell::default();

        let _ = update(&mut shell, Message::ToggleSidebar);
        let _ = update(&mut shell, Message::ToggleSidebar);

        assert!(shell.ui_state().sidebar_visible());
    }

    #[test]
    fn injected_photo_workspace_is_retained_read_only() {
        let workspace = PhotoWorkspaceViewModel::default();
        let shell = Shell::with_photo_workspace(workspace.clone());

        assert_eq!(shell.library_state(), &LibraryState::Ready(workspace));
    }

    #[test]
    fn opening_a_photo_starts_a_loading_preview_request() {
        let photo_id = photo_id();
        let mut shell = Shell {
            ui: UiState::with_photo_workspace(workspace()),
            active_load_request_id: LibraryLoadRequestId::first(),
            load_in_flight: false,
            catalog_path: Ok(PathBuf::from("/tmp/catalog.redb")),
            source_root: Ok(PathBuf::from("/tmp")),
            preview_generation: 0,
            active_preview: None,
        };

        let task = update(
            &mut shell,
            Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
        );

        assert_eq!(shell.ui.route(), WorkspaceRoute::PhotoDetail(photo_id));
        assert_eq!(shell.active_preview, Some((1, photo_id)));
        assert_eq!(
            shell
                .library_state()
                .ready_workspace()
                .and_then(|workspace| workspace.detail(photo_id))
                .map(PhotoDetailViewModel::selected_preview),
            Some(&SelectedPreviewState::Loading)
        );
        let _ = task;
    }

    #[test]
    fn stale_preview_completion_cannot_replace_the_active_photo_preview() {
        let photo_id = photo_id();
        let mut shell = Shell {
            ui: UiState::with_photo_workspace(workspace()),
            active_load_request_id: LibraryLoadRequestId::first(),
            load_in_flight: false,
            catalog_path: Ok(PathBuf::from("/tmp/catalog.redb")),
            source_root: Ok(PathBuf::from("/tmp")),
            preview_generation: 2,
            active_preview: None,
        };
        let _ = update(
            &mut shell,
            Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
        );

        let _ = update(
            &mut shell,
            Message::PreviewLoaded {
                generation: 1,
                photo_id,
                result: PreviewLoadResult::Failed,
            },
        );

        assert_eq!(shell.active_preview, Some((3, photo_id)));
        assert_eq!(
            shell
                .library_state()
                .ready_workspace()
                .and_then(|workspace| workspace.detail(photo_id))
                .map(PhotoDetailViewModel::selected_preview),
            Some(&SelectedPreviewState::Loading)
        );
    }

    #[test]
    fn boot_starts_loading_with_one_active_request() {
        let (shell, task) = super::boot();

        assert_eq!(shell.library_state(), &LibraryState::Loading);
        assert_eq!(shell.active_load_request_id().get(), 1);
        let _ = task;
    }

    #[test]
    fn active_completion_replaces_loading_state_and_duplicate_is_ignored() {
        let (mut shell, _) = super::boot();
        let request_id = shell.active_load_request_id();

        let _ = update(
            &mut shell,
            Message::LibraryLoaded {
                request_id,
                result: LibraryLoadResult::Empty,
            },
        );
        assert_eq!(shell.library_state(), &LibraryState::Empty);

        let _ = update(&mut shell, Message::RetryLibrary);
        assert_eq!(shell.active_load_request_id(), request_id);

        let _ = update(
            &mut shell,
            Message::LibraryLoaded {
                request_id,
                result: LibraryLoadResult::Failed(LibraryFailureKind::RepositoryUnavailable),
            },
        );
        assert_eq!(shell.library_state(), &LibraryState::Empty);
    }

    #[test]
    fn stale_completion_and_retry_while_loading_are_no_ops() {
        let (mut shell, _) = super::boot();
        let active = shell.active_load_request_id();
        let stale = active.next().expect("next request");

        let _ = update(
            &mut shell,
            Message::LibraryLoaded {
                request_id: stale,
                result: LibraryLoadResult::Empty,
            },
        );
        assert_eq!(shell.library_state(), &LibraryState::Loading);

        let _ = update(&mut shell, Message::RetryLibrary);
        assert_eq!(shell.active_load_request_id(), active);
        assert_eq!(shell.library_state(), &LibraryState::Loading);
    }

    #[test]
    fn failed_retry_advances_request_and_returns_to_loading() {
        let (mut shell, _) = super::boot();
        let first = shell.active_load_request_id();
        let _ = update(
            &mut shell,
            Message::LibraryLoaded {
                request_id: first,
                result: LibraryLoadResult::Failed(LibraryFailureKind::RepositoryUnavailable),
            },
        );

        let _ = update(&mut shell, Message::RetryLibrary);
        let retried = shell.active_load_request_id();
        let _ = update(&mut shell, Message::RetryLibrary);

        assert_eq!(retried.get(), first.get() + 1);
        assert_eq!(shell.active_load_request_id(), retried);
        assert_eq!(shell.library_state(), &LibraryState::Loading);
    }
}
