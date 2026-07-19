use crate::lifecycle::run_with_bootstrap;
use crate::ui_shell::{DaemonState, boot, subscription, update, view as daemon_view};

#[cfg(test)]
mod view {
    use iced::Element;

    use crate::application::{Message, Shell};

    pub(super) fn view(shell: &Shell) -> Element<'_, Message> {
        rusttable_ui::view::view(shell.ui_state()).map(Message::from)
    }
}

#[cfg(test)]
mod ui_smoke;

/// Starts the `RustTable` desktop application.
///
/// # Errors
///
/// Returns an error if Iced cannot create or run the desktop window.
pub fn run() -> iced::Result {
    let preflight = crate::platform::startup_preflight();
    run_with_bootstrap(
        rusttable_diagnostics::install,
        || {
            if !preflight.is_supported() {
                return Ok(());
            }
            iced::daemon(boot, update, daemon_view)
                .title("RustTable")
                .theme(|state: &DaemonState, _window| rusttable_ui::tokens::theme(state.ui_theme()))
                .subscription(subscription)
                .run()
        },
        |warning| eprintln!("{warning}"),
    )
}
