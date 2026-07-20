#![forbid(unsafe_code)]
#![doc = "Display-independent keyboard, MIDI, gamepad, and tablet action input for `RustTable`."]

mod persistence;
mod service;
mod types;

pub use persistence::{ShortcutParseError, parse_darktable_shortcuts, write_darktable_shortcuts};
pub use service::{ActionInputService, DispatchReport, InputIgnoreReason};
pub use types::{
    ActionEvent, ActionId, ActionMapping, ActionMode, ActionPhase, Binding, CaptureToken,
    DeviceDescriptor, DeviceState, DeviceToken, GamepadAxis, GamepadButton, GamepadControl,
    GamepadEvent, InputContext, InputEvent, InputSource, KeyCode, KeyboardEvent, MidiChannel,
    MidiControl, MidiEvent, Modifiers, Scope, TabletControl, TabletEvent,
};
