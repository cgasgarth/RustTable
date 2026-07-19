use iced::{Event, Subscription};

use super::model::{AppUiState, UiMessage, WindowKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionIdentity {
    source: SubscriptionSource,
    window: Option<WindowKey>,
    generation: u64,
}

impl SubscriptionIdentity {
    #[must_use]
    pub const fn new(
        source: SubscriptionSource,
        window: Option<WindowKey>,
        generation: u64,
    ) -> Self {
        Self {
            source,
            window,
            generation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubscriptionSource {
    Window,
    Keyboard,
    Pointer,
    Timer,
    Service,
    Filesystem,
    Device,
}

pub fn subscriptions(state: &AppUiState) -> Subscription<UiMessage> {
    let window_identity = SubscriptionIdentity::new(
        SubscriptionSource::Window,
        Some(WindowKey::MAIN),
        state
            .window(WindowKey::MAIN)
            .map_or(0, |window| window.generation().value()),
    );
    let window_events = Subscription::batch([
        iced::window::close_requests()
            .map(map_close_request)
            .with(window_identity)
            .map(|(_, message)| message),
        iced::window::resize_events()
            .map(map_resize)
            .with(window_identity)
            .map(|(_, message)| message),
    ]);
    let keyboard = iced::keyboard::listen()
        .map(UiMessage::Keyboard)
        .with(SubscriptionIdentity::new(
            SubscriptionSource::Keyboard,
            None,
            0,
        ))
        .map(|(_, message)| message);
    let pointer = iced::event::listen_with(map_pointer)
        .with(SubscriptionIdentity::new(
            SubscriptionSource::Pointer,
            None,
            0,
        ))
        .map(|(_, message)| message);

    Subscription::batch([window_events, keyboard, pointer])
}

fn map_close_request(window: iced::window::Id) -> UiMessage {
    UiMessage::RuntimeCloseRequest(window)
}

fn map_resize((window, size): (iced::window::Id, iced::Size)) -> UiMessage {
    UiMessage::RuntimeResize { window, size }
}

fn map_pointer(
    event: Event,
    _status: iced::event::Status,
    window: iced::window::Id,
) -> Option<UiMessage> {
    matches!(event, Event::Mouse(_) | Event::Touch(_))
        .then_some(UiMessage::Pointer { window, event })
}

#[cfg(test)]
mod tests {
    use super::{SubscriptionIdentity, SubscriptionSource};
    use crate::shell::WindowKey;

    #[test]
    fn subscription_identity_includes_owner_and_generation() {
        let first = SubscriptionIdentity::new(SubscriptionSource::Window, Some(WindowKey::MAIN), 1);
        let second =
            SubscriptionIdentity::new(SubscriptionSource::Window, Some(WindowKey::MAIN), 2);
        assert_ne!(first, second);
    }
}
