use iced::Task;
use std::path::PathBuf;

use crate::library::{self, LibraryLoadRequestId, LibraryLoadResult};
use rusttable_ui::{
    InputIntent, LibraryFailureKind, LibraryState, NavigationIntent, PhotoWorkspaceViewModel,
    UiEffect, UiMessage, UiState,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Shell {
    ui: UiState,
    active_load_request_id: LibraryLoadRequestId,
    load_in_flight: bool,
    catalog_path: Result<PathBuf, LibraryFailureKind>,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            ui: UiState::default(),
            active_load_request_id: LibraryLoadRequestId::first(),
            load_in_flight: false,
            catalog_path: Err(LibraryFailureKind::CatalogLocationUnavailable),
        }
    }
}

pub(crate) fn boot() -> (Shell, Task<Message>) {
    let request_id = LibraryLoadRequestId::first();
    let catalog_path = library::catalog_path();
    let shell = Shell::loading(request_id, catalog_path.clone());
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
        }
    }

    fn loading(
        request_id: LibraryLoadRequestId,
        catalog_path: Result<PathBuf, LibraryFailureKind>,
    ) -> Self {
        Self {
            ui: UiState::with_library_state(LibraryState::Loading),
            active_load_request_id: request_id,
            load_in_flight: true,
            catalog_path,
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
            let ui_message = match message {
                Message::ToggleSidebar => UiMessage::ToggleSidebar,
                Message::Navigate(intent) => UiMessage::Navigate(intent),
                Message::Input(intent) => UiMessage::Input(intent),
                Message::LibraryLoaded { .. } | Message::RetryLibrary => unreachable!(),
            };
            if shell.ui.handle(ui_message) == UiEffect::RetryLibrary {
                return retry_library(shell);
            }
        }
        Message::LibraryLoaded { request_id, result } => {
            if shell.load_in_flight && request_id == shell.active_load_request_id {
                shell.load_in_flight = false;
                shell.ui.set_library_state(result.into_library_state());
            }
        }
        Message::RetryLibrary => return retry_library(shell),
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
    shell.ui.begin_library_load();
    start_load(request_id, shell.catalog_path.clone())
}

#[cfg(test)]
mod tests {
    use crate::library::{LibraryFailureKind, LibraryState};
    use crate::library::{LibraryLoadRequestId, LibraryLoadResult};
    use rusttable_ui::{PhotoWorkspaceViewModel, UiState};

    use super::{Message, Shell, update};

    #[test]
    fn default_shell_shows_the_sidebar() {
        assert_eq!(
            Shell::default(),
            Shell {
                ui: UiState::default(),
                active_load_request_id: LibraryLoadRequestId::first(),
                load_in_flight: false,
                catalog_path: Err(LibraryFailureKind::CatalogLocationUnavailable),
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
