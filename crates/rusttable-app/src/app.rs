use iced::Task;

use crate::input::{FocusTarget, InputEffect, InputIntent, InputState};
use crate::navigation::{NavigationIntent, NavigationState};
use crate::presentation::PhotoWorkspaceViewModel;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Shell {
    sidebar_visible: bool,
    navigation: NavigationState,
    photo_workspace: PhotoWorkspaceViewModel,
    input: InputState,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            navigation: NavigationState::default(),
            photo_workspace: PhotoWorkspaceViewModel::default(),
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
        Self {
            sidebar_visible: true,
            navigation: NavigationState::default(),
            photo_workspace,
            input: InputState::default(),
        }
    }

    pub(crate) fn photo_workspace(&self) -> &PhotoWorkspaceViewModel {
        &self.photo_workspace
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
                .reconcile(shell.sidebar_visible, shell.route(), &shell.photo_workspace);
        }
        Message::Navigate(intent) => {
            let _ = shell.navigation.apply(intent);
            shell.input.note_navigation(intent, &shell.photo_workspace);
            shell
                .input
                .reconcile(shell.sidebar_visible, shell.route(), &shell.photo_workspace);
        }
        Message::Input(intent) => {
            let effect = shell.input.apply(
                intent,
                shell.sidebar_visible,
                shell.route(),
                &shell.photo_workspace,
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
                .reconcile(shell.sidebar_visible, shell.route(), &shell.photo_workspace);
        }
    }
    Task::none()
}

#[cfg(test)]
mod tests {
    use crate::input::InputState;
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
                photo_workspace: PhotoWorkspaceViewModel::default(),
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

        assert_eq!(shell.photo_workspace(), &workspace);
    }
}
