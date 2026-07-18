use crate::application::{Message, Shell, boot, update};
use crate::lifecycle::run_with_bootstrap;

#[cfg(test)]
mod ui_smoke;

mod view {
    use iced::Element;

    use super::{Message, Shell};

    pub(super) fn view(shell: &Shell) -> Element<'_, Message> {
        rusttable_ui::view::view(shell.ui_state()).map(Message::from)
    }
}

/// Starts the `RustTable` desktop application.
///
/// # Errors
///
/// Returns an error if Iced cannot create or run the desktop window.
pub fn run() -> iced::Result {
    run_with_bootstrap(
        rusttable_diagnostics::install,
        || {
            iced::application(boot, update, view::view)
                .title("RustTable")
                .theme(rusttable_ui::theme::theme)
                .centered()
                .run()
        },
        |warning| eprintln!("{warning}"),
    )
}
