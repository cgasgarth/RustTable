use std::collections::BTreeMap;

use iced::{Point, Size};

use super::layout::{MonitorBounds, SavedWindowPlacement, restore_placement};
use super::{ServiceEvent, TaskGeneration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowKey(u64);

impl WindowKey {
    pub const MAIN: Self = Self(1);

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRole {
    MainLibrary,
    Darkroom,
    SecondaryPreview,
    Preferences,
    Progress,
    Tool,
}

impl WindowRole {
    #[must_use]
    pub const fn default_workspace(self) -> &'static str {
        match self {
            Self::MainLibrary => "library",
            Self::Darkroom | Self::SecondaryPreview => "photo",
            Self::Preferences => "preferences",
            Self::Progress => "progress",
            Self::Tool => "tool",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMode {
    Normal,
    Maximized,
    Minimized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusOwner {
    Shell,
    Workspace,
    Modal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modal {
    ShutdownConfirmation,
    Recovery,
    TaskProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStatus {
    Starting,
    Ready,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeSelection {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitPolicy {
    ExitWhenIdle,
    KeepDaemonAlive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitState {
    Running,
    Requested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkState {
    Idle,
    Durable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MotionPreference {
    Full,
    Reduced,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccessibilityState {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WindowState {
    role: WindowRole,
    workspace_identity: String,
    monitor_identity: String,
    scale_factor: f32,
    bounds: WindowBounds,
    mode: WindowMode,
    focus_owner: FocusOwner,
    modal_stack: Vec<Modal>,
    generation: TaskGeneration,
}

impl WindowState {
    #[must_use]
    pub fn new(key: WindowKey, role: WindowRole) -> Self {
        let size = if key == WindowKey::MAIN {
            Size::new(1_280.0, 800.0)
        } else {
            Size::new(960.0, 640.0)
        };
        Self {
            role,
            workspace_identity: role.default_workspace().to_owned(),
            monitor_identity: String::from("primary"),
            scale_factor: 1.0,
            bounds: WindowBounds::new(Point::ORIGIN, size),
            mode: WindowMode::Normal,
            focus_owner: FocusOwner::Workspace,
            modal_stack: Vec::new(),
            generation: TaskGeneration::zero(),
        }
    }

    #[must_use]
    pub const fn role(&self) -> WindowRole {
        self.role
    }

    #[must_use]
    pub fn workspace_identity(&self) -> &str {
        &self.workspace_identity
    }

    #[must_use]
    pub fn monitor_identity(&self) -> &str {
        &self.monitor_identity
    }

    #[must_use]
    pub const fn scale_factor(&self) -> f32 {
        self.scale_factor
    }

    #[must_use]
    pub const fn bounds(&self) -> WindowBounds {
        self.bounds
    }

    #[must_use]
    pub const fn mode(&self) -> WindowMode {
        self.mode
    }

    #[must_use]
    pub const fn focus_owner(&self) -> FocusOwner {
        self.focus_owner
    }

    #[must_use]
    pub fn modal_stack(&self) -> &[Modal] {
        &self.modal_stack
    }

    #[must_use]
    pub const fn generation(&self) -> TaskGeneration {
        self.generation
    }

    fn move_to(&mut self, point: Point) {
        self.bounds = self.bounds.with_position(point);
    }

    fn resize(&mut self, size: Size) {
        self.bounds = self.bounds.with_size(size);
    }

    fn set_scale_factor(&mut self, scale_factor: f32) {
        self.scale_factor = scale_factor.max(0.5);
    }

    fn placement(&self) -> SavedWindowPlacement {
        SavedWindowPlacement::new(self.monitor_identity.clone(), self.bounds)
    }

    fn restore_placement(&mut self, placement: &SavedWindowPlacement) {
        placement
            .monitor_identity()
            .clone_into(&mut self.monitor_identity);
        self.bounds = placement.bounds();
    }

    fn bump_generation(&mut self) {
        self.generation = self.generation.next();
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WindowBounds {
    position: Point,
    size: Size,
}

impl WindowBounds {
    #[must_use]
    pub const fn new(position: Point, size: Size) -> Self {
        Self { position, size }
    }

    #[must_use]
    pub const fn position(&self) -> Point {
        self.position
    }

    #[must_use]
    pub const fn size(&self) -> Size {
        self.size
    }

    #[must_use]
    pub const fn with_position(&self, position: Point) -> Self {
        Self { position, ..*self }
    }

    #[must_use]
    pub const fn with_size(&self, size: Size) -> Self {
        Self { size, ..*self }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppUiState {
    windows: BTreeMap<WindowKey, WindowState>,
    saved_placements: BTreeMap<WindowKey, SavedWindowPlacement>,
    next_window_key: u64,
    service_status: ServiceStatus,
    exit_policy: ExitPolicy,
    exit_state: ExitState,
    work_state: WorkState,
    theme: ThemeSelection,
    motion_preference: MotionPreference,
    accessibility: AccessibilityState,
}

impl AppUiState {
    #[must_use]
    pub fn boot_preset() -> Self {
        let mut windows = BTreeMap::new();
        windows.insert(
            WindowKey::MAIN,
            WindowState::new(WindowKey::MAIN, WindowRole::MainLibrary),
        );
        Self {
            windows,
            saved_placements: BTreeMap::new(),
            next_window_key: 2,
            service_status: ServiceStatus::Starting,
            exit_policy: ExitPolicy::KeepDaemonAlive,
            exit_state: ExitState::Running,
            work_state: WorkState::Idle,
            theme: ThemeSelection::System,
            motion_preference: MotionPreference::Full,
            accessibility: AccessibilityState::Enabled,
        }
    }

    pub fn windows(&self) -> impl Iterator<Item = (&WindowKey, &WindowState)> {
        self.windows.iter()
    }

    #[must_use]
    pub fn window(&self, key: WindowKey) -> Option<&WindowState> {
        self.windows.get(&key)
    }

    #[must_use]
    pub fn saved_placement(&self, key: WindowKey) -> Option<&SavedWindowPlacement> {
        self.saved_placements.get(&key)
    }

    #[must_use]
    pub const fn exit_policy(&self) -> ExitPolicy {
        self.exit_policy
    }

    #[must_use]
    pub const fn exit_requested(&self) -> bool {
        matches!(self.exit_state, ExitState::Requested)
    }

    #[must_use]
    pub const fn durable_work(&self) -> bool {
        matches!(self.work_state, WorkState::Durable)
    }

    #[must_use]
    pub const fn service_status(&self) -> ServiceStatus {
        self.service_status
    }

    #[must_use]
    pub const fn theme(&self) -> ThemeSelection {
        self.theme
    }

    #[must_use]
    pub const fn reduced_motion(&self) -> bool {
        matches!(self.motion_preference, MotionPreference::Reduced)
    }

    #[must_use]
    pub const fn accessibility_enabled(&self) -> bool {
        matches!(self.accessibility, AccessibilityState::Enabled)
    }

    pub fn update(&mut self, message: UiMessage) -> UpdateEffect {
        match message {
            UiMessage::OpenWindow(role) => {
                let key = WindowKey(self.next_window_key);
                self.next_window_key = self.next_window_key.saturating_add(1);
                self.windows.insert(key, WindowState::new(key, role));
                UpdateEffect::OpenWindow(key)
            }
            UiMessage::CloseWindow(key) => {
                if self.windows.remove(&key).is_some() {
                    UpdateEffect::CloseWindow(key)
                } else {
                    UpdateEffect::None
                }
            }
            UiMessage::FocusWindow(key) => {
                if let Some(window) = self.windows.get_mut(&key) {
                    window.focus_owner = FocusOwner::Workspace;
                    UpdateEffect::FocusWindow(key)
                } else {
                    UpdateEffect::None
                }
            }
            UiMessage::MoveWindow { key, position } => {
                if let Some(window) = self.windows.get_mut(&key) {
                    window.move_to(position);
                }
                UpdateEffect::None
            }
            UiMessage::ResizeWindow { key, size } => {
                if let Some(window) = self.windows.get_mut(&key) {
                    window.resize(size);
                }
                UpdateEffect::None
            }
            UiMessage::ScaleFactorChanged { key, scale_factor } => {
                if let Some(window) = self.windows.get_mut(&key) {
                    window.set_scale_factor(scale_factor);
                }
                UpdateEffect::None
            }
            message => self.update_secondary(message),
        }
    }

    fn update_secondary(&mut self, message: UiMessage) -> UpdateEffect {
        match message {
            UiMessage::SavePlacement { key } => {
                if let Some(window) = self.windows.get(&key) {
                    self.saved_placements.insert(key, window.placement());
                }
                UpdateEffect::None
            }
            UiMessage::RestorePlacement { key, monitors } => {
                if let (Some(window), Some(saved)) =
                    (self.windows.get_mut(&key), self.saved_placements.get(&key))
                {
                    window.restore_placement(&restore_placement(saved, &monitors));
                }
                UpdateEffect::None
            }
            UiMessage::WindowTaskCancelled(key) => {
                if let Some(window) = self.windows.get_mut(&key) {
                    window.bump_generation();
                }
                UpdateEffect::None
            }
            UiMessage::TaskCompleted {
                window: key,
                generation,
                result: _result,
            } => self
                .windows
                .get(&key)
                .filter(|window| window.generation() == generation)
                .map_or(UpdateEffect::None, |_| UpdateEffect::TaskCompleted(key)),
            UiMessage::Service(event) => {
                self.service_status = match event {
                    ServiceEvent::Ready => ServiceStatus::Ready,
                    ServiceEvent::Degraded => ServiceStatus::Degraded,
                    ServiceEvent::Failed => ServiceStatus::Failed,
                    ServiceEvent::Progress { .. } | ServiceEvent::Finished { .. } => {
                        self.service_status
                    }
                };
                UpdateEffect::None
            }
            UiMessage::SetDurableWork(value) => {
                self.work_state = if value {
                    WorkState::Durable
                } else {
                    WorkState::Idle
                };
                UpdateEffect::None
            }
            UiMessage::SetTheme(theme) => {
                self.theme = theme;
                UpdateEffect::None
            }
            UiMessage::SetReducedMotion(value) => {
                self.motion_preference = if value {
                    MotionPreference::Reduced
                } else {
                    MotionPreference::Full
                };
                UpdateEffect::None
            }
            UiMessage::SetAccessibility(value) => {
                self.accessibility = if value {
                    AccessibilityState::Enabled
                } else {
                    AccessibilityState::Disabled
                };
                UpdateEffect::None
            }
            UiMessage::RequestExit => {
                self.exit_state = ExitState::Requested;
                UpdateEffect::Exit
            }
            UiMessage::OpenWindow(_)
            | UiMessage::CloseWindow(_)
            | UiMessage::FocusWindow(_)
            | UiMessage::MoveWindow { .. }
            | UiMessage::ResizeWindow { .. }
            | UiMessage::ScaleFactorChanged { .. } => {
                unreachable!("primary window messages are reduced before secondary state")
            }
            UiMessage::RuntimeCloseRequest(_)
            | UiMessage::RuntimeResize { .. }
            | UiMessage::Keyboard(_)
            | UiMessage::Pointer { .. } => UpdateEffect::None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UiMessage {
    OpenWindow(WindowRole),
    CloseWindow(WindowKey),
    FocusWindow(WindowKey),
    MoveWindow {
        key: WindowKey,
        position: Point,
    },
    ResizeWindow {
        key: WindowKey,
        size: Size,
    },
    ScaleFactorChanged {
        key: WindowKey,
        scale_factor: f32,
    },
    SavePlacement {
        key: WindowKey,
    },
    RestorePlacement {
        key: WindowKey,
        monitors: Vec<MonitorBounds>,
    },
    WindowTaskCancelled(WindowKey),
    TaskCompleted {
        window: WindowKey,
        generation: TaskGeneration,
        result: String,
    },
    Service(ServiceEvent),
    SetDurableWork(bool),
    SetTheme(ThemeSelection),
    SetReducedMotion(bool),
    SetAccessibility(bool),
    RequestExit,
    RuntimeCloseRequest(iced::window::Id),
    RuntimeResize {
        window: iced::window::Id,
        size: Size,
    },
    Keyboard(iced::keyboard::Event),
    Pointer {
        window: iced::window::Id,
        event: iced::Event,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateEffect {
    None,
    OpenWindow(WindowKey),
    CloseWindow(WindowKey),
    FocusWindow(WindowKey),
    TaskCompleted(WindowKey),
    Exit,
}

impl UpdateEffect {
    #[must_use]
    pub const fn is_empty(self) -> bool {
        matches!(self, Self::None)
    }
}
