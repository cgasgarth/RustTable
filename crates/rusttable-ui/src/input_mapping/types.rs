//! Display-independent models for the shortcut and device-mapping editor.
//!
//! The model is the boundary between the GTK editor and the #512 input
//! service.  It contains no hardware access and never dispatches actions.

use serde::{Deserialize, Serialize};
use std::fmt;

/// The maximum number of key chords accepted by the editor's recorder.
pub const MAX_SEQUENCE_LENGTH: usize = 4;
/// The number of one-second inactivity ticks before learn mode expires.
pub const LEARN_TIMEOUT_TICKS: u8 = 15;
/// The profile schema written by this editor.
pub const PROFILE_SCHEMA_VERSION: u16 = 1;

/// A stable action identity supplied by the runtime action registry.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ActionId(String);

impl ActionId {
    /// Creates an action identity from a stable, non-localized identifier.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the stable identifier used in profiles and searches.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ActionId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Input context precedence follows the runtime resolver's shadowing order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionContext {
    Global,
    Lighttable,
    Darkroom,
    Map,
    Tethering,
    Print,
    Slideshow,
    ModalDialog,
    TextEntry,
    FocusedControl,
}

impl ActionContext {
    /// Returns the resolver priority used to explain shadowed bindings.
    #[must_use]
    pub const fn priority(self) -> u8 {
        match self {
            Self::Global => 0,
            Self::Lighttable
            | Self::Darkroom
            | Self::Map
            | Self::Tethering
            | Self::Print
            | Self::Slideshow => 1,
            Self::ModalDialog => 2,
            Self::TextEntry => 3,
            Self::FocusedControl => 4,
        }
    }

    /// Returns the localized-ready label used by the current GTK surface.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::Lighttable => "Lighttable",
            Self::Darkroom => "Darkroom",
            Self::Map => "Map",
            Self::Tethering => "Tethering",
            Self::Print => "Print",
            Self::Slideshow => "Slideshow",
            Self::ModalDialog => "Modal dialog",
            Self::TextEntry => "Text entry",
            Self::FocusedControl => "Focused control",
        }
    }
}

/// A normalized keyboard modifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyModifier {
    Shift,
    Control,
    Alt,
    Super,
}

/// One layout-independent keyboard chord.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyChord {
    pub key: String,
    pub modifiers: Vec<KeyModifier>,
}

impl KeyChord {
    /// Creates a canonical chord with sorted, duplicate-free modifiers.
    #[must_use]
    pub fn new(key: impl Into<String>, modifiers: impl IntoIterator<Item = KeyModifier>) -> Self {
        let mut modifiers: Vec<_> = modifiers.into_iter().collect();
        modifiers.sort_unstable();
        modifiers.dedup();
        Self {
            key: key.into(),
            modifiers,
        }
    }

    /// Formats a chord for an accessible GTK label.
    #[must_use]
    pub fn display(&self) -> String {
        let mut parts: Vec<&str> = self
            .modifiers
            .iter()
            .map(|modifier| match modifier {
                KeyModifier::Shift => "Shift",
                KeyModifier::Control => "Ctrl",
                KeyModifier::Alt => "Alt",
                KeyModifier::Super => "Super",
            })
            .collect();
        parts.push(&self.key);
        parts.join("+")
    }
}

/// A supported source family shown by the device-centric editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceKind {
    Keyboard,
    Pointer,
    Tablet,
    Midi,
    Gamepad,
}

impl DeviceKind {
    /// Returns the user-facing source label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Keyboard => "Keyboard",
            Self::Pointer => "Pointer",
            Self::Tablet => "Tablet",
            Self::Midi => "MIDI",
            Self::Gamepad => "Gamepad",
        }
    }
}

/// Normalized pointer/tablet controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointerControl {
    Button(u8),
    WheelVertical,
    WheelHorizontal,
}

/// Normalized tablet controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TabletControl {
    Button(u8),
    Pressure,
    TiltX,
    TiltY,
    Eraser,
}

/// Normalized MIDI controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MidiControl {
    Note(u8),
    ControlChange(u8),
    PitchBend,
    FourteenBitControlChange(u8),
}

/// Explicit relative encoder interpretation; there is no hidden auto-detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelativeMode {
    Absolute,
    TwoComplement,
    BinaryOffset,
    SignedBit,
    IncrementDecrement,
}

/// Normalized gamepad controls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GamepadControl {
    Button(String),
    Axis(String),
    Hat(String),
}

/// The source-specific identity used for conflict analysis.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum BindingSource {
    Keyboard {
        sequence: Vec<KeyChord>,
    },
    Pointer {
        control: PointerControl,
    },
    Tablet {
        control: TabletControl,
    },
    Midi {
        control: MidiControl,
        channel: Option<u8>,
        relative_mode: RelativeMode,
    },
    Gamepad {
        control: GamepadControl,
    },
}

impl BindingSource {
    /// Returns its device family without touching a backend.
    #[must_use]
    pub const fn device_kind(&self) -> DeviceKind {
        match self {
            Self::Keyboard { .. } => DeviceKind::Keyboard,
            Self::Pointer { .. } => DeviceKind::Pointer,
            Self::Tablet { .. } => DeviceKind::Tablet,
            Self::Midi { .. } => DeviceKind::Midi,
            Self::Gamepad { .. } => DeviceKind::Gamepad,
        }
    }

    /// Returns a stable source identity for deterministic conflict analysis.
    #[must_use]
    pub fn identity(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "invalid-source".to_owned())
    }

    /// Formats the source for the binding column.
    #[must_use]
    pub fn display(&self) -> String {
        match self {
            Self::Keyboard { sequence } => sequence
                .iter()
                .map(KeyChord::display)
                .collect::<Vec<_>>()
                .join(" then "),
            Self::Pointer { control } => format!("Pointer: {control:?}"),
            Self::Tablet { control } => format!("Tablet: {control:?}"),
            Self::Midi {
                control,
                channel,
                relative_mode,
            } => format!(
                "MIDI {control:?}, ch {}, {relative_mode:?}",
                channel.map_or_else(|| "all".to_owned(), |value| value.to_string())
            ),
            Self::Gamepad { control } => format!("Gamepad: {control:?}"),
        }
    }
}

/// A curve applied to continuous input after deadzone and before inversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Curve {
    #[default]
    Linear,
    FineCenter,
    Exponential {
        exponent_hundredths: u16,
    },
}

/// Continuous range and takeover settings shared by MIDI, tablet, and axes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContinuousSettings {
    pub input_min: f64,
    pub input_max: f64,
    pub deadzone: f64,
    pub curve: Curve,
    pub invert: bool,
    pub relative: bool,
    pub target_min: f64,
    pub target_max: f64,
    pub step: f64,
    pub soft_takeover: SoftTakeover,
}

impl Default for ContinuousSettings {
    fn default() -> Self {
        Self {
            input_min: 0.0,
            input_max: 1.0,
            deadzone: 0.0,
            curve: Curve::Linear,
            invert: false,
            relative: false,
            target_min: 0.0,
            target_max: 1.0,
            step: 0.01,
            soft_takeover: SoftTakeover::Pickup,
        }
    }
}

/// Policy for the first absolute value after a device is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoftTakeover {
    #[default]
    Pickup,
    Jump,
}

/// One user or immutable built-in mapping record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub id: String,
    pub action_id: ActionId,
    pub device_alias: String,
    pub context: ActionContext,
    pub source: BindingSource,
    pub continuous: Option<ContinuousSettings>,
    pub enabled: bool,
    pub built_in: bool,
}

impl Binding {
    /// Creates a user binding with a deterministic caller-supplied identity.
    #[must_use]
    pub fn user(
        id: impl Into<String>,
        action_id: ActionId,
        device_alias: impl Into<String>,
        context: ActionContext,
        source: BindingSource,
    ) -> Self {
        Self {
            id: id.into(),
            action_id,
            device_alias: device_alias.into(),
            context,
            source,
            continuous: None,
            enabled: true,
            built_in: false,
        }
    }
}

/// A runtime-registered action shown in the action-centric view.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionDefinition {
    pub id: ActionId,
    pub label: String,
    pub category: String,
    pub contexts: Vec<ActionContext>,
    pub parameter: Option<ParameterSchema>,
    pub available: bool,
    pub nonremovable: bool,
}

/// Parameter contract used to constrain continuous binding editors.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterSchema {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

/// A privacy-safe device descriptor supplied by the input service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceDescriptor {
    pub alias: String,
    pub label: String,
    pub kind: DeviceKind,
    pub available: bool,
    pub capabilities: Vec<String>,
}

/// A complete versioned mapping snapshot from the input service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MappingSnapshot {
    pub schema_version: u16,
    pub generation: u64,
    pub actions: Vec<ActionDefinition>,
    pub devices: Vec<DeviceDescriptor>,
    pub bindings: Vec<Binding>,
}

/// Optional match criteria retained in profiles without private identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceMatch {
    pub kind: DeviceKind,
    pub vendor: Option<String>,
    pub product: Option<String>,
}

/// Portable profile format written by export and consumed by import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MappingProfile {
    pub schema_version: u16,
    pub name: String,
    pub device_match: Vec<DeviceMatch>,
    pub mappings: Vec<Binding>,
}

impl MappingProfile {
    /// Creates a canonical profile from a snapshot, sorting records by ID.
    #[must_use]
    pub fn from_snapshot(snapshot: &MappingSnapshot, name: impl Into<String>) -> Self {
        let mut mappings = snapshot.bindings.clone();
        mappings.sort_by(|left, right| left.id.cmp(&right.id));
        Self {
            schema_version: PROFILE_SCHEMA_VERSION,
            name: name.into(),
            device_match: Vec::new(),
            mappings,
        }
    }

    /// Serializes stable bytes for reproducible export and round-trip tests.
    ///
    /// # Errors
    ///
    /// Returns a serialization error if a future profile field cannot be
    /// represented by `serde_json`.
    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Parses and validates the profile envelope before the editor sees records.
    ///
    /// # Errors
    ///
    /// Returns a [`ProfileError`] for malformed JSON, an unsupported schema,
    /// an empty name, or an oversized mapping list.
    pub fn parse_json(value: &str) -> Result<Self, ProfileError> {
        let profile: Self = serde_json::from_str(value).map_err(ProfileError::Malformed)?;
        if profile.schema_version != PROFILE_SCHEMA_VERSION {
            return Err(ProfileError::UnsupportedSchema(profile.schema_version));
        }
        if profile.name.trim().is_empty() {
            return Err(ProfileError::EmptyName);
        }
        if profile.mappings.len() > 10_000 {
            return Err(ProfileError::TooManyMappings);
        }
        Ok(profile)
    }
}

/// Import failures that can be displayed as recoverable editor status.
#[derive(Debug)]
pub enum ProfileError {
    Malformed(serde_json::Error),
    UnsupportedSchema(u16),
    EmptyName,
    TooManyMappings,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(error) => write!(formatter, "malformed profile: {error}"),
            Self::UnsupportedSchema(version) => {
                write!(formatter, "unsupported profile schema {version}")
            }
            Self::EmptyName => formatter.write_str("profile name cannot be empty"),
            Self::TooManyMappings => formatter.write_str("profile contains too many mappings"),
        }
    }
}

impl std::error::Error for ProfileError {}

/// Whether a pair of bindings blocks save or merely shadows another context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConflictKind {
    Exact,
    Shadowed { winner: ActionContext },
}

/// A conflict projection shown beside an action and binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MappingConflict {
    pub left_binding_id: String,
    pub right_binding_id: String,
    pub kind: ConflictKind,
    pub explanation: String,
}

impl MappingConflict {
    /// Whether this conflict prevents an apply.
    #[must_use]
    pub const fn blocks_apply(&self) -> bool {
        matches!(self.kind, ConflictKind::Exact)
    }
}

/// Action or device list selected in the GTK editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EditorView {
    #[default]
    Actions,
    Devices,
}

/// Current learn target; no captured observation is ever dispatched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LearnTarget {
    Keyboard,
    Pointer,
    Tablet,
    Midi,
    Gamepad,
}

impl From<DeviceKind> for LearnTarget {
    fn from(value: DeviceKind) -> Self {
        match value {
            DeviceKind::Keyboard => Self::Keyboard,
            DeviceKind::Pointer => Self::Pointer,
            DeviceKind::Tablet => Self::Tablet,
            DeviceKind::Midi => Self::Midi,
            DeviceKind::Gamepad => Self::Gamepad,
        }
    }
}

/// Editor status suitable for a live status label and screen reader.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum EditorStatus {
    #[default]
    Clean,
    Dirty,
    Learning(LearnTarget),
    LearnCaptured,
    LearnTimedOut,
    Testing,
    TestPreview(String),
    Applied(u64),
    StaleGeneration,
    ValidationError(String),
    Imported {
        changed: usize,
        unknown: usize,
    },
}

/// Typed operations sent from GTK controls to the editor model.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorMessage {
    SetView(EditorView),
    SetSearch(String),
    SelectAction(ActionId),
    SelectDevice(String),
    BeginLearn(LearnTarget),
    CaptureKeyboard(KeyChord),
    LearnTick,
    CancelLearn,
    TestBinding(String),
    StopTest,
    RemoveBinding(String),
    ToggleBinding { binding_id: String, enabled: bool },
    Reset(ResetScope),
    Apply { live_generation: u64 },
    Revert,
}

/// Reset scope mirrors Darktable's default/user shortcut reset affordances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetScope {
    Action,
    Device(DeviceKind),
    All,
}

/// Result of a successful generation-safe apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyResult {
    pub generation: u64,
    pub changed_bindings: usize,
}

/// Recoverable editor operation errors.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorError {
    NoSelectedAction,
    LearnNotActive,
    SequenceTooLong,
    ExactConflict,
    InvalidContinuous(String),
    NonRemovableFallback,
    StaleGeneration,
}

impl fmt::Display for EditorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSelectedAction => formatter.write_str("select an action first"),
            Self::LearnNotActive => formatter.write_str("learn mode is not active"),
            Self::SequenceTooLong => write!(
                formatter,
                "keyboard sequences are limited to {MAX_SEQUENCE_LENGTH} chords"
            ),
            Self::ExactConflict => {
                formatter.write_str("an exact same-context conflict must be resolved first")
            }
            Self::InvalidContinuous(message) => formatter.write_str(message),
            Self::NonRemovableFallback => {
                formatter.write_str("the action requires an accessibility fallback")
            }
            Self::StaleGeneration => {
                formatter.write_str("mapping generation changed; reload before applying")
            }
        }
    }
}

impl std::error::Error for EditorError {}
