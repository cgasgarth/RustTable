mod bridge;
mod layout;
mod model;
mod subscriptions;
mod tasks;
mod viewport;

pub use bridge::{BoundedServiceBridge, ServiceEvent, ServiceEventKind, service_subscription};
pub use layout::{MonitorBounds, SavedWindowPlacement, restore_placement};
pub use model::{
    AppUiState, ExitPolicy, FocusOwner, Modal, ServiceStatus, ThemeSelection, UiMessage,
    UpdateEffect, WindowBounds, WindowKey, WindowMode, WindowRole, WindowState,
};
pub use subscriptions::{SubscriptionIdentity, SubscriptionSource, subscriptions};
pub use tasks::{
    GenerationTask, TaskGeneration, TaskResult, abortable_generation_task, progress_generation_task,
};
pub use viewport::{PresentationReceipt, TextureHandle, ViewportFailure, ViewportState, viewport};

#[cfg(test)]
mod tests {
    use iced::{Point, Size};

    use super::{
        AppUiState, BoundedServiceBridge, ExitPolicy, MonitorBounds, ServiceEvent,
        ServiceEventKind, TaskGeneration, UiMessage, WindowKey, WindowRole, WindowState,
        restore_placement,
    };

    #[test]
    fn iced_shell_every_window_role_has_explicit_state_and_stable_identity() {
        let state = AppUiState::boot_preset();

        assert_eq!(state.windows().count(), 1);
        assert_eq!(
            state.window(WindowKey::MAIN).map(WindowState::role),
            Some(WindowRole::MainLibrary)
        );
        assert_eq!(state.exit_policy(), ExitPolicy::KeepDaemonAlive);
        assert!(
            state
                .window(WindowKey::MAIN)
                .is_some_and(|window| window.generation() == super::TaskGeneration::zero())
        );
    }

    #[test]
    fn close_last_window_does_not_exit_durable_daemon() {
        let mut state = AppUiState::boot_preset();

        let effect = state.update(UiMessage::CloseWindow(WindowKey::MAIN));

        assert!(state.windows().next().is_none());
        assert!(!state.exit_requested());
        assert_eq!(effect, super::UpdateEffect::CloseWindow(WindowKey::MAIN));
    }

    #[test]
    fn explicit_exit_is_the_only_exit_request() {
        let mut state = AppUiState::boot_preset();

        let _ = state.update(UiMessage::CloseWindow(WindowKey::MAIN));
        let _ = state.update(UiMessage::RequestExit);

        assert!(state.exit_requested());
    }

    #[test]
    fn stale_task_result_is_ignored_after_generation_change() {
        let mut state = AppUiState::boot_preset();
        let generation = TaskGeneration::new(0);

        let _ = state.update(UiMessage::OpenWindow(WindowRole::Progress));
        let _ = state.update(UiMessage::WindowTaskCancelled(WindowKey::MAIN));
        let effect = state.update(UiMessage::TaskCompleted {
            window: WindowKey::MAIN,
            generation,
            result: String::from("stale"),
        });

        assert!(effect.is_empty());
        assert!(
            state
                .window(WindowKey::MAIN)
                .is_some_and(|window| window.generation() > generation)
        );
    }

    #[test]
    fn removed_monitor_and_invalid_bounds_restore_to_safe_clamped_placement() {
        let saved = super::SavedWindowPlacement::new(
            String::from("missing-monitor"),
            super::WindowBounds::new(Point::new(-500.0, -400.0), Size::new(12.0, 12.0)),
        );
        let restored = restore_placement(
            &saved,
            &[MonitorBounds::new(
                String::from("primary"),
                Point::ORIGIN,
                Size::new(800.0, 600.0),
            )],
        );

        assert_eq!(restored.monitor_identity(), "primary");
        assert_eq!(restored.bounds().size(), Size::new(320.0, 240.0));
        assert_eq!(restored.bounds().position(), Point::ORIGIN);
    }

    #[test]
    fn window_placement_is_saved_and_restored_through_state_messages() {
        let mut state = AppUiState::boot_preset();
        let _ = state.update(UiMessage::MoveWindow {
            key: WindowKey::MAIN,
            position: Point::new(700.0, 500.0),
        });
        let _ = state.update(UiMessage::SavePlacement {
            key: WindowKey::MAIN,
        });
        let _ = state.update(UiMessage::MoveWindow {
            key: WindowKey::MAIN,
            position: Point::ORIGIN,
        });
        let _ = state.update(UiMessage::RestorePlacement {
            key: WindowKey::MAIN,
            monitors: vec![MonitorBounds::new(
                String::from("primary"),
                Point::ORIGIN,
                Size::new(1_280.0, 800.0),
            )],
        });

        assert_eq!(
            state
                .window(WindowKey::MAIN)
                .expect("main window")
                .bounds()
                .position(),
            Point::new(0.0, 0.0)
        );
        assert!(state.saved_placement(WindowKey::MAIN).is_some());
    }

    #[test]
    fn bounded_service_bridge_coalesces_progress_and_preserves_terminal_events() {
        let bridge = BoundedServiceBridge::new(2);

        bridge.push(ServiceEvent::progress(7, 0.1));
        bridge.push(ServiceEvent::progress(7, 0.2));
        bridge.push(ServiceEvent::finished(7));

        let events = bridge.drain();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0].kind(),
            ServiceEventKind::Progress {
                job: 7,
                fraction: 0.2
            }
        );
        assert_eq!(events[1].kind(), ServiceEventKind::Finished { job: 7 });
    }
}
