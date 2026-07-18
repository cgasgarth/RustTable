use iced::widget::{button, column, text};
use iced::{Element, Task};
use rusttable_core::product_name;

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

pub(crate) fn view(shell: &Shell) -> Element<'_, Message> {
    let sidebar = if shell.sidebar_visible {
        text("Sidebar")
    } else {
        text("")
    };
    let toggle_label = if shell.sidebar_visible {
        "Hide sidebar"
    } else {
        "Show sidebar"
    };

    column![
        text(product_name()).size(32),
        button(text(toggle_label)).on_press(Message::ToggleSidebar),
        sidebar,
    ]
    .spacing(16)
    .into()
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
