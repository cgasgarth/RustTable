#![forbid(unsafe_code)]
#![doc = "The `RustTable` iced application shell."]

mod app;
mod bootstrap;

use app::{Shell, update, view};
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
            iced::application(Shell::default, update, view)
                .title("RustTable")
                .centered()
                .run()
        },
        |warning| eprintln!("{warning}"),
    )
}
