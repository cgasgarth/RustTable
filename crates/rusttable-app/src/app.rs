use iced::Task;

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Shell {
    sidebar_visible: bool,
}

impl Default for Shell {
    fn default() -> Self {
        Self {
            sidebar_visible: true,
        }
    }
}

impl Shell {
    pub(crate) fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Message {
    ToggleSidebar,
}

pub(crate) fn update(shell: &mut Shell, message: Message) -> Task<Message> {
    match message {
        Message::ToggleSidebar => shell.sidebar_visible = !shell.sidebar_visible,
    }
    Task::none()
}

#[cfg(test)]
mod tests {
    use super::{Message, Shell, update};

    #[test]
    fn default_shell_shows_the_sidebar() {
        assert_eq!(
            Shell::default(),
            Shell {
                sidebar_visible: true
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
}
