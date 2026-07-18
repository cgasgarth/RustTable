use iced::Task;

use crate::navigation::{NavigationIntent, NavigationState};
use crate::presentation::PhotoWorkspaceViewModel;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Shell {
    sidebar_visible: bool,
    navigation: NavigationState,
    photo_workspace: PhotoWorkspaceViewModel,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
            navigation: NavigationState::default(),
            photo_workspace: PhotoWorkspaceViewModel::default(),
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
        }
    }

    pub(crate) fn photo_workspace(&self) -> &PhotoWorkspaceViewModel {
        &self.photo_workspace
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Message {
    ToggleSidebar,
    Navigate(NavigationIntent),
}

pub(crate) fn update(shell: &mut Shell, message: Message) -> Task<Message> {
    match message {
        Message::ToggleSidebar => shell.sidebar_visible = !shell.sidebar_visible,
        Message::Navigate(intent) => {
            let _ = shell.navigation.apply(intent);
        }
    }
    Task::none()
}

#[cfg(test)]
mod tests {
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
