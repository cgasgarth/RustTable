use iced::Task;

use crate::input::{FocusTarget, InputEffect, InputIntent, InputState};
use crate::library::LibraryState;
use crate::navigation::{NavigationIntent, NavigationState};
use crate::presentation::PhotoWorkspaceViewModel;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Shell {
    sidebar_visible: bool,
    navigation: NavigationState,
    library_state: LibraryState,
    input: InputState,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            navigation: NavigationState::default(),
            library_state: LibraryState::default(),
            input: InputState::default(),
        }
    }
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
        }
    }

    pub(crate) fn library_state(&self) -> &LibraryState {
        &self.library_state
    }

    pub(crate) fn is_focused(&self, target: FocusTarget) -> bool {
        self.input.is_focused(target)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Message {
    ToggleSidebar,
    Navigate(NavigationIntent),
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
            }
            shell
                .input
                .reconcile(shell.sidebar_visible, shell.route(), &shell.library_state);
        }
    }
    Task::none()
}

#[cfg(test)]
mod tests {
    use crate::input::InputState;
    use crate::library::LibraryState;
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
}
