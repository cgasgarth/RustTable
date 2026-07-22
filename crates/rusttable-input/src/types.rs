use std::fmt;

/// Stable identity of an action exposed by a `RustTable` subsystem.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActionId(String);

impl ActionId {
    /// Creates an action path such as `view/darkroom`.
    ///
    /// # Errors
    ///
    /// Returns `ActionIdError` when the path is empty or contains shortcut
    /// file delimiters.
    pub fn new(value: impl Into<String>) -> Result<Self, ActionIdError> {
        let value = value.into();
        if value.is_empty()
            || value.starts_with('/')
            || value.ends_with('/')
            || value.contains("//")
            || value
                .chars()
                .any(|character| matches!(character, '=' | ';' | '\n' | '\r'))
        {
            return Err(ActionIdError);
        }
        Ok(Self(value))
    }

    /// Returns the action's stable path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl TryFrom<&str> for ActionId {
    type Error = ActionIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<String> for ActionId {
    type Error = ActionIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// An invalid action path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionIdError;

impl fmt::Display for ActionIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("action id must be a non-empty slash-separated path")
    }
}

impl std::error::Error for ActionIdError {}

/// The physical input family that produced an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputSource {
    Keyboard,
    Midi,
    Gamepad,
    Tablet,
}

impl InputSource {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Keyboard => "keyboard",
            Self::Midi => "midi",
            Self::Gamepad => "gamepad",
            Self::Tablet => "tablet",
        }
    }
}

/// A generation-stamped device identity. Events from a removed device cannot
/// be delivered after the same physical id is reconnected.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceToken {
    source: InputSource,
    id: String,
    generation: u64,
}

impl DeviceToken {
    #[must_use]
    pub fn new(source: InputSource, id: impl Into<String>, generation: u64) -> Self {
        Self {
            source,
            id: id.into(),
            generation,
        }
    }

    #[must_use]
    pub fn source(&self) -> InputSource {
        self.source
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// Metadata used when a backend discovers an input device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceDescriptor {
    pub source: InputSource,
    pub id: String,
    pub name: String,
}

impl DeviceDescriptor {
    #[must_use]
    pub fn new(source: InputSource, id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            source,
            id: id.into(),
            name: name.into(),
        }
    }
}

/// Current backend availability, kept separate from mapping state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceState {
    Connected(DeviceToken),
    Disconnected,
    Unavailable(String),
}

/// GTK/GDK-independent key identity.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyCode {
    Named(String),
    Character(char),
    Hardware(u32),
}

impl KeyCode {
    #[must_use]
    pub fn named(value: impl Into<String>) -> Self {
        Self::Named(value.into())
    }

    #[must_use]
    pub const fn character(value: char) -> Self {
        Self::Character(value)
    }

    #[must_use]
    pub fn from_name(value: &str) -> Self {
        let mut characters = value.chars();
        match (characters.next(), characters.next()) {
            (Some(character), None) => Self::Character(character),
            _ => Self::Named(value.to_owned()),
        }
    }

    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Named(value) => value.clone(),
            Self::Character(value) => value.to_string(),
            Self::Hardware(value) => format!("hardware:{value}"),
        }
    }
}

/// Keyboard modifier mask shared by all frontends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const SHIFT: Self = Self(1 << 0);
    pub const CONTROL: Self = Self(1 << 1);
    pub const ALT: Self = Self(1 << 2);
    pub const SUPER: Self = Self(1 << 3);

    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }
}

/// Normalized MIDI channel selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MidiChannel {
    Any,
    Number(u8),
}

impl MidiChannel {
    #[must_use]
    pub fn number(value: u8) -> Option<Self> {
        (value < 16).then_some(Self::Number(value))
    }

    #[must_use]
    pub const fn matches(self, channel: u8) -> bool {
        matches!(self, Self::Any) || matches!(self, Self::Number(value) if value == channel)
    }
}

/// MIDI note or controller selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MidiControl {
    Note(u8),
    ControlChange(u8),
}

/// MIDI input event with 7-bit values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiEvent {
    pub device: DeviceToken,
    pub timestamp: u64,
    pub channel: u8,
    pub control: MidiControl,
    pub value: u8,
    pub pressed: bool,
}

/// SDL-independent gamepad controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GamepadButton {
    South,
    East,
    West,
    North,
    Back,
    Guide,
    Start,
    LeftStick,
    RightStick,
    LeftShoulder,
    RightShoulder,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum GamepadAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GamepadControl {
    Button(GamepadButton),
    Axis { axis: GamepadAxis, deadzone: f32 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum GamepadEvent {
    Button {
        device: DeviceToken,
        timestamp: u64,
        button: GamepadButton,
        pressed: bool,
    },
    Axis {
        device: DeviceToken,
        timestamp: u64,
        axis: GamepadAxis,
        value: f32,
    },
}

/// GTK-independent tablet controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TabletControl {
    Pressure,
    TiltX,
    TiltY,
    Eraser,
    Button(u8),
    X,
    Y,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabletEvent {
    pub device: DeviceToken,
    pub timestamp: u64,
    pub phase: ActionPhase,
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub eraser: bool,
    pub button: Option<u8>,
}

/// Input event supplied by a platform adapter or a hardware backend.
#[derive(Debug, Clone, PartialEq)]
pub enum InputEvent {
    Keyboard(KeyboardEvent),
    Midi(MidiEvent),
    Gamepad(GamepadEvent),
    Tablet(TabletEvent),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyboardEvent {
    pub device: DeviceToken,
    pub timestamp: u64,
    pub key: KeyCode,
    pub modifiers: Modifiers,
    pub pressed: bool,
    pub repeat: bool,
}

/// Action event delivered to application-owned action handlers.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionEvent {
    pub sequence: u64,
    pub timestamp: u64,
    pub action: ActionId,
    pub source: InputSource,
    pub device: DeviceToken,
    pub phase: ActionPhase,
    pub value: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionPhase {
    Pressed,
    Released,
    Changed,
}

/// How a physical binding is presented to an action handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionMode {
    Activate,
    Hold,
    Value,
    Relative,
}

/// Matching context for focus, view, and modal event capture.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InputContext {
    pub view: Option<String>,
    pub focus: Option<String>,
    pub modal: bool,
}

/// Mapping precedence. More specific scopes win before explicit priority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Global,
    View(String),
    Focus(String),
    Modal,
}

impl Scope {
    #[must_use]
    pub fn specificity(&self) -> u8 {
        match self {
            Self::Global => 0,
            Self::View(_) => 1,
            Self::Focus(_) => 2,
            Self::Modal => 3,
        }
    }

    #[must_use]
    pub fn matches(&self, context: &InputContext) -> bool {
        match self {
            Self::Global => !context.modal,
            Self::View(view) => !context.modal && context.view.as_deref() == Some(view),
            Self::Focus(focus) => !context.modal && context.focus.as_deref() == Some(focus),
            Self::Modal => context.modal,
        }
    }
}

/// One action binding. The service preserves insertion order to make ties
/// deterministic and conflict resolution reproducible.
#[derive(Debug, Clone, PartialEq)]
pub struct ActionMapping {
    pub action: ActionId,
    pub binding: Binding,
    pub scope: Scope,
    pub mode: ActionMode,
    pub priority: i16,
    pub repeat: bool,
    pub enabled: bool,
}

impl ActionMapping {
    #[must_use]
    pub fn new(action: ActionId, binding: Binding) -> Self {
        Self {
            action,
            binding,
            scope: Scope::Global,
            mode: ActionMode::Activate,
            priority: 0,
            repeat: false,
            enabled: true,
        }
    }

    #[must_use]
    pub fn with_scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }

    #[must_use]
    pub const fn with_mode(mut self, mode: ActionMode) -> Self {
        self.mode = mode;
        self
    }

    #[must_use]
    pub const fn with_priority(mut self, priority: i16) -> Self {
        self.priority = priority;
        self
    }

    #[must_use]
    pub const fn accepting_repeat(mut self, repeat: bool) -> Self {
        self.repeat = repeat;
        self
    }
}

/// A binding normalized from a device-specific event.
#[derive(Debug, Clone, PartialEq)]
pub enum Binding {
    Keyboard {
        key: KeyCode,
        modifiers: Modifiers,
    },
    Midi {
        channel: MidiChannel,
        control: MidiControl,
    },
    Gamepad(GamepadControl),
    Tablet(TabletControl),
}

/// Opaque handle for temporary modal/focus capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CaptureToken(u64);

impl CaptureToken {
    pub(crate) const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}
