#![forbid(unsafe_code)]
#![doc = "The `RustTable` iced application shell."]

mod app;

use app::{Shell, update, view};

/// Starts the `RustTable` desktop application.
///
/// # Errors
///
/// Returns an error if iced cannot create or run the desktop window.
pub fn run() -> iced::Result {
    iced::application(Shell::default, update, view)
        .title("RustTable")
        .centered()
        .run()
}
