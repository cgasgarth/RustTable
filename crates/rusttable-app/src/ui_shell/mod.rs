use std::collections::BTreeMap;

use iced::{Element, Size, Subscription, Task};

use crate::application::{self, Message as LibraryMessage, Shell};
use rusttable_ui::shell::{
    AppUiState, ServiceEvent, SubscriptionIdentity, SubscriptionSource, ThemeSelection, UiMessage,
    UpdateEffect, WindowKey, service_subscription, subscriptions,
};

#[derive(Debug)]
pub(crate) struct DaemonState {
    library: Shell,
    ui: AppUiState,
    runtime_windows: BTreeMap<WindowKey, iced::window::Id>,
}

#[derive(Debug, Clone)]
pub(crate) enum Message {
    Library(LibraryMessage),
    Ui(UiMessage),
    WindowOpened {
        key: WindowKey,
        id: iced::window::Id,
    },
}

impl DaemonState {
    pub(crate) const fn ui_theme(&self) -> ThemeSelection {
        self.ui.theme()
    }
}

pub(crate) fn boot() -> (DaemonState, Task<Message>) {
    let (library, library_task) = application::boot();
    let (id, open_task) = iced::window::open(window_settings());
    let state = DaemonState {
        library,
        ui: AppUiState::boot_preset(),
        runtime_windows: BTreeMap::from([(WindowKey::MAIN, id)]),
    };
    let open_task = open_task.map(move |id| Message::WindowOpened {
        key: WindowKey::MAIN,
        id,
    });
    (
        state,
        Task::batch([library_task.map(Message::Library), open_task]),
    )
}

pub(crate) fn update(state: &mut DaemonState, message: Message) -> Task<Message> {
    match message {
        Message::Library(message) => {
            application::update(&mut state.library, message).map(Message::Library)
        }
        Message::WindowOpened { key, id } => {
            state.runtime_windows.insert(key, id);
            Task::none()
        }
        Message::Ui(message) => update_ui(state, message),
    }
}

fn update_ui(state: &mut DaemonState, message: UiMessage) -> Task<Message> {
    let message = match message {
        UiMessage::RuntimeCloseRequest(id) => {
            let key = state
                .runtime_windows
                .iter()
                .find_map(|(key, runtime_id)| (*runtime_id == id).then_some(*key));
            key.map(|key| {
                let _ = state.ui.update(UiMessage::SavePlacement { key });
                UiMessage::CloseWindow(key)
            })
        }
        UiMessage::RuntimeResize { window, size } => {
            state.runtime_windows.iter().find_map(|(key, runtime_id)| {
                (*runtime_id == window).then_some(UiMessage::ResizeWindow { key: *key, size })
            })
        }
        other => Some(other),
    };
    let Some(message) = message else {
        return Task::none();
    };
    let effect = state.ui.update(message);
    match effect {
        UpdateEffect::OpenWindow(key) => {
            let (id, task) = iced::window::open(window_settings());
            state.runtime_windows.insert(key, id);
            task.map(move |id| Message::WindowOpened { key, id })
        }
        UpdateEffect::CloseWindow(key) => state
            .runtime_windows
            .remove(&key)
            .map_or_else(Task::none, iced::window::close),
        UpdateEffect::Exit => iced::exit(),
        UpdateEffect::None | UpdateEffect::FocusWindow(_) | UpdateEffect::TaskCompleted(_) => {
            Task::none()
        }
    }
}

pub(crate) fn view(state: &DaemonState, _window: iced::window::Id) -> Element<'_, Message> {
    rusttable_ui::view::view(state.library.ui_state()).map(map_library_ui_message)
}

pub(crate) fn subscription(state: &DaemonState) -> Subscription<Message> {
    let shell = subscriptions(&state.ui).map(map_ui_message);
    let service = service_subscription(
        SubscriptionIdentity::new(SubscriptionSource::Service, None, 0),
        empty_service_stream,
    )
    .map(map_ui_message);
    Subscription::batch([shell, service])
}

fn map_ui_message(message: UiMessage) -> Message {
    Message::Ui(message)
}

fn map_library_ui_message(message: rusttable_ui::UiMessage) -> Message {
    Message::Library(LibraryMessage::from(message))
}

fn empty_service_stream() -> iced::futures::stream::BoxStream<'static, ServiceEvent> {
    Box::pin(iced::futures::stream::empty())
}

fn window_settings() -> iced::window::Settings {
    iced::window::Settings {
        size: Size::new(1_280.0, 800.0),
        exit_on_close_request: false,
        ..iced::window::Settings::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{DaemonState, Message, boot, update};
    use rusttable_ui::shell::{UiMessage, WindowKey, WindowRole};

    #[test]
    fn iced_shell_daemon_boot_has_explicit_main_window_and_background_library_task() {
        let (state, task) = boot();
        assert!(state.ui.window(WindowKey::MAIN).is_some());
        assert_eq!(task.units(), 2);
    }

    #[test]
    fn opening_and_closing_a_secondary_window_is_explicit() {
        let (mut state, _) = boot();
        let task = update(
            &mut state,
            Message::Ui(UiMessage::OpenWindow(WindowRole::Progress)),
        );
        let key = state
            .ui
            .windows()
            .find_map(|(key, window)| (window.role() == WindowRole::Progress).then_some(*key))
            .expect("progress window key");
        assert_eq!(task.units(), 1);
        let _ = update(&mut state, Message::Ui(UiMessage::CloseWindow(key)));
        assert!(state.ui.window(key).is_none());
    }

    #[test]
    fn close_request_is_routed_to_stable_window_state() {
        let (mut state, _) = boot();
        let runtime_id = *state
            .runtime_windows
            .get(&WindowKey::MAIN)
            .expect("main runtime id");
        let _ = update(
            &mut state,
            Message::Ui(UiMessage::RuntimeCloseRequest(runtime_id)),
        );
        assert!(state.ui.window(WindowKey::MAIN).is_none());
    }

    #[test]
    fn state_type_is_not_accidentally_replaced_by_test_presets() {
        let (state, _) = boot();
        let _: &DaemonState = &state;
    }
}
