#![forbid(unsafe_code)]
#![doc = "The `RustTable` iced application shell."]

use iced::widget::text;
use iced::{Element, Task};
use rusttable_core::product_name;

#[derive(Debug, Default)]
struct State;

#[derive(Debug, Clone, Copy)]
enum Message {}

/// Starts the `RustTable` desktop application.
///
/// # Errors
///
/// Returns an error if iced cannot create or run the desktop window.
pub fn run() -> iced::Result {
    iced::application(State::default, update, view)
        .title("RustTable")
        .centered()
        .run()
}

fn update(_state: &mut State, _message: Message) -> Task<Message> {
    Task::none()
}

fn view(_state: &State) -> Element<'_, Message> {
    text(product_name()).size(32).into()
}

#[cfg(test)]
mod tests {
    use super::product_name;

    #[test]
    fn shell_uses_the_core_product_name() {
        assert_eq!(product_name(), "RustTable");
    }
}
