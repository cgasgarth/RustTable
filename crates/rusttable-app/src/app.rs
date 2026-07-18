use iced::Task;
use std::path::PathBuf;

use crate::input::{FocusTarget, InputEffect, InputIntent, InputState};
use crate::library::{LibraryFailureKind, LibraryState};
use crate::library_loader::{self, LibraryLoadRequestId, LibraryLoadResult};
use crate::navigation::{NavigationIntent, NavigationState};
use crate::presentation::PhotoWorkspaceViewModel;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Shell {
    sidebar_visible: bool,
    navigation: NavigationState,
    library_state: LibraryState,
    input: InputState,
    active_load_request_id: LibraryLoadRequestId,
    load_in_flight: bool,
    catalog_path: Result<PathBuf, LibraryFailureKind>,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            navigation: NavigationState::default(),
            library_state: LibraryState::default(),
            input: InputState::default(),
            active_load_request_id: LibraryLoadRequestId::first(),
            load_in_flight: false,
            catalog_path: Err(LibraryFailureKind::CatalogLocationUnavailable),
        }
    }
}

pub(crate) fn boot() -> (Shell, Task<Message>) {
    let request_id = LibraryLoadRequestId::first();
    let catalog_path = library_loader::catalog_path();
    let shell = Shell::loading(request_id, catalog_path.clone());
    let task = start_load(request_id, catalog_path);
    (shell, task)
}

impl Shell {
    pub(crate) fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    pub(crate) fn route(&self) -> crate::navigation::WorkspaceRoute {
        self.navigation.route()
    }

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
            sidebar_visible: true,
            navigation: NavigationState::default(),
            library_state,
            input: InputState::default(),
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
            sidebar_visible: true,
            navigation: NavigationState::default(),
            library_state: LibraryState::Loading,
            input: InputState::default(),
            active_load_request_id: request_id,
            load_in_flight: true,
            catalog_path,
        }
    }

    pub(crate) fn library_state(&self) -> &LibraryState {
        &self.library_state
    }

    pub(crate) fn is_focused(&self, target: FocusTarget) -> bool {
        self.input.is_focused(target)
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

pub(crate) fn update(shell: &mut Shell, message: Message) -> Task<Message> {
    match message {
        Message::ToggleSidebar => {
            shell.sidebar_visible = !shell.sidebar_visible;
            shell
                .input
                .reconcile(shell.sidebar_visible, shell.route(), &shell.library_state);
        }
        Message::Navigate(intent) => {
            let _ = shell.navigation.apply(intent);
            shell.input.note_navigation(intent, &shell.library_state);
            shell
                .input
                .reconcile(shell.sidebar_visible, shell.route(), &shell.library_state);
        }
        Message::LibraryLoaded { request_id, result } => {
            if shell.load_in_flight && request_id == shell.active_load_request_id {
                shell.load_in_flight = false;
                shell.library_state = result.into_library_state();
                shell
                    .input
                    .reconcile(shell.sidebar_visible, shell.route(), &shell.library_state);
            }
        }
        Message::RetryLibrary => return retry_library(shell),
        Message::Input(intent) => {
            let effect = shell.input.apply(
                intent,
                shell.sidebar_visible,
                shell.route(),
                &shell.library_state,
            );
            match effect {
                InputEffect::None => {}
                InputEffect::ToggleSidebar => shell.sidebar_visible = !shell.sidebar_visible,
                InputEffect::Navigate(navigation) => {
                    let _ = shell.navigation.apply(navigation);
                }
                InputEffect::RetryLibrary => return retry_library(shell),
            }
            shell
                .input
                .reconcile(shell.sidebar_visible, shell.route(), &shell.library_state);
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
    Task::perform(
        async move { library_loader::load_catalog(&path) },
        move |result| Message::LibraryLoaded { request_id, result },
    )
}

fn retry_library(shell: &mut Shell) -> Task<Message> {
    if !matches!(shell.library_state, LibraryState::Failed(_)) || shell.load_in_flight {
        return Task::none();
    }
    let Some(request_id) = shell.active_load_request_id.next() else {
        return Task::none();
    };
    shell.active_load_request_id = request_id;
    shell.load_in_flight = true;
    shell.library_state = LibraryState::Loading;
    shell
        .input
        .reconcile(shell.sidebar_visible, shell.route(), &shell.library_state);
    start_load(request_id, shell.catalog_path.clone())
}

#[cfg(test)]
mod tests {
    use crate::input::InputState;
    use crate::library::{LibraryFailureKind, LibraryState};
    use crate::library_loader::{LibraryLoadRequestId, LibraryLoadResult};
    use crate::navigation::NavigationState;
    use crate::presentation::PhotoWorkspaceViewModel;

    use super::{Message, Shell, update};

    #[test]
    fn default_shell_shows_the_sidebar() {
        assert_eq!(
            Shell::default(),
            Shell {
                sidebar_visible: true,
                navigation: NavigationState::default(),
                library_state: LibraryState::default(),
                input: InputState::default(),
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

        assert!(!shell.sidebar_visible);
    }

    #[test]
    fn toggling_sidebar_twice_restores_it() {
        let mut shell = Shell::default();

        let _ = update(&mut shell, Message::ToggleSidebar);
        let _ = update(&mut shell, Message::ToggleSidebar);

        assert!(shell.sidebar_visible);
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
