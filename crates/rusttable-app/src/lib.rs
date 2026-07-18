#![forbid(unsafe_code)]
#![doc = "The `RustTable` iced application shell."]

mod action_button;
mod app;
mod bootstrap;
mod input;
mod library;
mod navigation;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "presentation contracts are staged before their rendering adapter"
    )
)]
mod presentation;
mod theme;
mod view;

#[cfg(test)]
mod ui_smoke;

use app::{Shell, update};
use bootstrap::run_with_bootstrap;

/// Starts the `RustTable` desktop application.
///
/// # Errors
///
/// Returns an error if iced cannot create or run the desktop window.
pub fn run() -> iced::Result {
    run_with_bootstrap(
        rusttable_diagnostics::install,
        || {
            iced::application(Shell::default, update, view::view)
                .title("RustTable")
                .theme(theme::theme)
                .centered()
                .run()
        },
        |warning| eprintln!("{warning}"),
    )
}
