use std::collections::BTreeMap;

use crate::types::{
    ActionEvent, ActionMapping, ActionMode, ActionPhase, Binding, CaptureToken, DeviceDescriptor,
    DeviceState, DeviceToken, GamepadControl, GamepadEvent, InputContext, InputEvent, InputSource,
    KeyboardEvent, MidiEvent, TabletControl,
};

/// Why an otherwise well-formed input event produced no action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputIgnoreReason {
    UnknownDevice,
    StaleDeviceGeneration,
    DeviceUnavailable,
    NoMapping,
    CapturedByHigherPriorityMapping,
    InvalidValue,
}

/// Result of routing one physical event.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchReport {
    pub events: Vec<ActionEvent>,
    pub ignored: Option<InputIgnoreReason>,
}

impl DispatchReport {
    fn ignored(reason: InputIgnoreReason) -> Self {
        Self {
            events: Vec::new(),
            ignored: Some(reason),
        }
    }

    fn delivered(event: ActionEvent) -> Self {
        Self {
            events: vec![event],
            ignored: None,
        }
    }
}

#[derive(Debug, Clone)]
struct DeviceRecord {
    descriptor: DeviceDescriptor,
    state: DeviceState,
    next_generation: u64,
}

/// Unified, deterministic action-input router.
///
/// Hardware backends only need to translate their native events into
/// [`InputEvent`]. Mapping, focus/modal capture, action ordering, repeat
/// suppression, deadzones, and device generations stay in this service so
/// they behave identically in tests and in the GTK desktop.
#[derive(Debug, Clone)]
pub struct ActionInputService {
    mappings: Vec<ActionMapping>,
    devices: BTreeMap<(InputSource, String), DeviceRecord>,
    context: InputContext,
    capture: Option<CaptureToken>,
    next_capture: u64,
    next_sequence: u64,
    pressed: Vec<(DeviceToken, Binding)>,
    last_axes: BTreeMap<(DeviceToken, crate::types::GamepadAxis), f32>,
}

impl Default for ActionInputService {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionInputService {
    /// Creates a service with the built-in keyboard device connected.
    #[must_use]
    pub fn new() -> Self {
        let keyboard = DeviceDescriptor::new(InputSource::Keyboard, "keyboard", "System keyboard");
        let token = DeviceToken::new(InputSource::Keyboard, "keyboard", 1);
        let mut devices = BTreeMap::new();
        devices.insert(
            (keyboard.source, keyboard.id.clone()),
            DeviceRecord {
                descriptor: keyboard,
                state: DeviceState::Connected(token),
                next_generation: 2,
            },
        );
        Self {
            mappings: Vec::new(),
            devices,
            context: InputContext::default(),
            capture: None,
            next_capture: 1,
            next_sequence: 1,
            pressed: Vec::new(),
            last_axes: BTreeMap::new(),
        }
    }

    /// Adds a mapping. Existing mappings remain stable so ties resolve by
    /// registration order, matching the predictable behavior of shortcutsrc.
    pub fn add_mapping(&mut self, mapping: ActionMapping) {
        self.mappings.push(mapping);
    }

    /// Replaces all mappings, useful when importing a persisted shortcutsrc.
    pub fn replace_mappings(&mut self, mappings: impl IntoIterator<Item = ActionMapping>) {
        self.mappings = mappings.into_iter().collect();
    }

    /// Returns the registered mapping order.
    #[must_use]
    pub fn mappings(&self) -> &[ActionMapping] {
        &self.mappings
    }

    /// Reports whether a binding is currently held for a device.
    #[must_use]
    pub fn is_pressed(&self, device: &DeviceToken, binding: &Binding) -> bool {
        self.pressed
            .iter()
            .any(|(held_device, held_binding)| held_device == device && held_binding == binding)
    }

    /// Changes the view/focus/modal context used for scope matching.
    pub fn set_context(&mut self, context: InputContext) {
        self.context = context;
    }

    /// Returns the current dispatch context.
    #[must_use]
    pub const fn context(&self) -> &InputContext {
        &self.context
    }

    /// Claims a temporary capture slot. While held, non-modal mappings are
    /// blocked by setting the context's modal flag through [`Self::set_modal`].
    #[must_use]
    pub fn begin_capture(&mut self) -> CaptureToken {
        let token = CaptureToken::new(self.next_capture);
        self.next_capture = self.next_capture.wrapping_add(1).max(1);
        self.capture = Some(token);
        token
    }

    /// Releases a capture slot only if it is still the active owner.
    pub fn end_capture(&mut self, token: CaptureToken) -> bool {
        if self.capture == Some(token) {
            self.capture = None;
            true
        } else {
            false
        }
    }

    /// Marks the current window as modal. Modal mappings are then the only
    /// mappings eligible for dispatch.
    pub fn set_modal(&mut self, modal: bool) {
        self.context.modal = modal;
    }

    /// Registers or reconnects a backend device with a fresh generation.
    pub fn connect_device(&mut self, descriptor: DeviceDescriptor) -> DeviceToken {
        let key = (descriptor.source, descriptor.id.clone());
        let generation = self
            .devices
            .get(&key)
            .map_or(1, |record| record.next_generation);
        let token = DeviceToken::new(descriptor.source, descriptor.id.clone(), generation);
        self.devices.insert(
            key,
            DeviceRecord {
                descriptor,
                state: DeviceState::Connected(token.clone()),
                next_generation: generation.saturating_add(1),
            },
        );
        token
    }

    /// Marks a device disconnected and invalidates all later events carrying
    /// the old generation.
    pub fn disconnect_device(&mut self, token: &DeviceToken) -> bool {
        let Some(record) = self
            .devices
            .get_mut(&(token.source(), token.id().to_owned()))
        else {
            return false;
        };
        if record.state != DeviceState::Connected(token.clone()) {
            return false;
        }
        record.state = DeviceState::Disconnected;
        self.pressed.retain(|(device, _)| device != token);
        self.last_axes.retain(|(device, _), _| device != token);
        true
    }

    /// Publishes a backend failure without losing persisted mappings.
    pub fn set_device_unavailable(
        &mut self,
        source: InputSource,
        id: impl Into<String>,
        reason: impl Into<String>,
    ) {
        let id = id.into();
        let descriptor = DeviceDescriptor::new(source, id.clone(), id.clone());
        let entry = self.devices.entry((source, id)).or_insert(DeviceRecord {
            descriptor,
            state: DeviceState::Unavailable(String::new()),
            next_generation: 1,
        });
        entry.state = DeviceState::Unavailable(reason.into());
    }

    /// Returns the currently published state for one logical device.
    #[must_use]
    pub fn device_state(&self, source: InputSource, id: &str) -> Option<&DeviceState> {
        self.devices
            .get(&(source, id.to_owned()))
            .map(|record| &record.state)
    }

    /// Returns the human-readable backend name if known.
    #[must_use]
    pub fn device_name(&self, source: InputSource, id: &str) -> Option<&str> {
        self.devices
            .get(&(source, id.to_owned()))
            .map(|record| record.descriptor.name.as_str())
    }

    /// Routes one event into at most one action, preserving event ordering.
    pub fn ingest(&mut self, input: &InputEvent) -> DispatchReport {
        let (device, timestamp, source) = input_identity(input);
        match self.device_status(device) {
            DeviceStatus::Unknown => {
                return DispatchReport::ignored(InputIgnoreReason::UnknownDevice);
            }
            DeviceStatus::Stale => {
                return DispatchReport::ignored(InputIgnoreReason::StaleDeviceGeneration);
            }
            DeviceStatus::Unavailable => {
                return DispatchReport::ignored(InputIgnoreReason::DeviceUnavailable);
            }
            DeviceStatus::Connected => {}
        }

        let candidates = self
            .mappings
            .iter()
            .enumerate()
            .filter_map(|(index, mapping)| {
                if !mapping.enabled || !mapping.scope.matches(&self.context) {
                    return None;
                }
                let matched = event_binding(input, mapping)?;
                if !mapping.repeat && matched.repeat && mapping.mode != ActionMode::Value {
                    return None;
                }
                Some((
                    mapping.scope.specificity(),
                    mapping.priority,
                    index,
                    matched,
                ))
            })
            .collect::<Vec<_>>();
        let Some((_, _, index, matched)) = candidates.into_iter().max_by(|left, right| {
            (left.0, left.1, std::cmp::Reverse(left.2)).cmp(&(
                right.0,
                right.1,
                std::cmp::Reverse(right.2),
            ))
        }) else {
            return DispatchReport::ignored(InputIgnoreReason::NoMapping);
        };
        let mapping = &self.mappings[index];
        let action = mapping.action.clone();
        let mode = mapping.mode;
        if let Some(binding) = matched.binding {
            update_pressed(&mut self.pressed, device, binding, matched.phase);
        }
        let Some(phase) = action_phase(mode, matched.phase) else {
            return DispatchReport::ignored(InputIgnoreReason::NoMapping);
        };
        let value = matched.value.map(|value| match mode {
            ActionMode::Relative => value * 0.01,
            ActionMode::Value | ActionMode::Activate | ActionMode::Hold => value,
        });
        if value.is_some_and(|value| !value.is_finite()) {
            return DispatchReport::ignored(InputIgnoreReason::InvalidValue);
        }
        let event = ActionEvent {
            sequence: self.take_sequence(),
            timestamp,
            action,
            source,
            device: device.clone(),
            phase,
            value,
        };
        DispatchReport::delivered(event)
    }

    fn device_status(&self, token: &DeviceToken) -> DeviceStatus {
        if token.source() == InputSource::Keyboard && token.id() == "keyboard" {
            return if token.generation() == 1 {
                DeviceStatus::Connected
            } else {
                DeviceStatus::Stale
            };
        }
        let Some(record) = self.devices.get(&(token.source(), token.id().to_owned())) else {
            return DeviceStatus::Unknown;
        };
        match &record.state {
            DeviceState::Connected(active) if active == token => DeviceStatus::Connected,
            DeviceState::Connected(_) | DeviceState::Disconnected => DeviceStatus::Stale,
            DeviceState::Unavailable(_) => DeviceStatus::Unavailable,
        }
    }

    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1).max(1);
        sequence
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceStatus {
    Connected,
    Unknown,
    Stale,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq)]
struct MatchedInput {
    binding: Option<Binding>,
    phase: ActionPhase,
    value: Option<f32>,
    repeat: bool,
}

fn input_identity(input: &InputEvent) -> (&DeviceToken, u64, InputSource) {
    match input {
        InputEvent::Keyboard(event) => (&event.device, event.timestamp, InputSource::Keyboard),
        InputEvent::Midi(event) => (&event.device, event.timestamp, InputSource::Midi),
        InputEvent::Gamepad(
            GamepadEvent::Button {
                device, timestamp, ..
            }
            | GamepadEvent::Axis {
                device, timestamp, ..
            },
        ) => (device, *timestamp, InputSource::Gamepad),
        InputEvent::Tablet(event) => (&event.device, event.timestamp, InputSource::Tablet),
    }
}

fn event_binding(input: &InputEvent, mapping: &ActionMapping) -> Option<MatchedInput> {
    match (&mapping.binding, input) {
        (
            Binding::Keyboard { key, modifiers },
            InputEvent::Keyboard(KeyboardEvent {
                key: event_key,
                modifiers: event_modifiers,
                pressed,
                repeat,
                device: _,
                timestamp: _,
            }),
        ) if key == event_key && modifiers == event_modifiers => Some(MatchedInput {
            binding: Some(mapping.binding.clone()),
            phase: if *pressed {
                ActionPhase::Pressed
            } else {
                ActionPhase::Released
            },
            value: None,
            repeat: *repeat,
        }),
        (
            Binding::Midi { channel, control },
            InputEvent::Midi(MidiEvent {
                channel: event_channel,
                control: event_control,
                value,
                pressed,
                ..
            }),
        ) if channel.matches(*event_channel) && control == event_control => Some(MatchedInput {
            binding: Some(mapping.binding.clone()),
            phase: if *pressed {
                ActionPhase::Pressed
            } else {
                ActionPhase::Released
            },
            value: Some(f32::from(*value) / 127.0),
            repeat: false,
        }),
        (
            Binding::Gamepad(GamepadControl::Button(button)),
            InputEvent::Gamepad(GamepadEvent::Button {
                button: event_button,
                pressed,
                ..
            }),
        ) if button == event_button => Some(MatchedInput {
            binding: Some(mapping.binding.clone()),
            phase: if *pressed {
                ActionPhase::Pressed
            } else {
                ActionPhase::Released
            },
            value: None,
            repeat: false,
        }),
        (
            Binding::Gamepad(GamepadControl::Axis { axis, deadzone }),
            InputEvent::Gamepad(GamepadEvent::Axis {
                axis: event_axis,
                value,
                ..
            }),
        ) if axis == event_axis => {
            let value = normalize_axis(*value, *deadzone)?;
            Some(MatchedInput {
                binding: Some(mapping.binding.clone()),
                phase: ActionPhase::Changed,
                value: Some(value),
                repeat: false,
            })
        }
        (Binding::Tablet(control), InputEvent::Tablet(event)) => {
            let value = match control {
                TabletControl::Pressure => event.pressure,
                TabletControl::TiltX => event.tilt_x,
                TabletControl::TiltY => event.tilt_y,
                TabletControl::Eraser => f32::from(event.eraser),
                TabletControl::Button(button) => {
                    if event.button == Some(*button) {
                        f32::from(event.phase == ActionPhase::Pressed)
                    } else {
                        return None;
                    }
                }
                TabletControl::X => event.x,
                TabletControl::Y => event.y,
            };
            Some(MatchedInput {
                binding: Some(mapping.binding.clone()),
                phase: event.phase,
                value: Some(value),
                repeat: false,
            })
        }
        _ => None,
    }
}

fn normalize_axis(value: f32, deadzone: f32) -> Option<f32> {
    if !value.is_finite() || !deadzone.is_finite() || !(0.0..1.0).contains(&deadzone) {
        return None;
    }
    let magnitude = value.abs();
    if magnitude <= deadzone {
        return None;
    }
    Some(value.signum() * ((magnitude - deadzone) / (1.0 - deadzone)))
}

fn action_phase(mode: ActionMode, phase: ActionPhase) -> Option<ActionPhase> {
    match (mode, phase) {
        (ActionMode::Activate, ActionPhase::Pressed | ActionPhase::Changed)
        | (ActionMode::Value | ActionMode::Relative, ActionPhase::Pressed) => {
            Some(ActionPhase::Pressed)
        }
        (ActionMode::Activate, ActionPhase::Released) => None,
        (ActionMode::Hold, phase) => Some(phase),
        (ActionMode::Value | ActionMode::Relative, ActionPhase::Changed) => {
            Some(ActionPhase::Changed)
        }
        (ActionMode::Value | ActionMode::Relative, ActionPhase::Released) => {
            Some(ActionPhase::Released)
        }
    }
}

fn update_pressed(
    pressed: &mut Vec<(DeviceToken, Binding)>,
    device: &DeviceToken,
    binding: Binding,
    phase: ActionPhase,
) {
    let entry = (device.clone(), binding);
    match phase {
        ActionPhase::Pressed => {
            if !pressed.contains(&entry) {
                pressed.push(entry);
            }
        }
        ActionPhase::Released => pressed.retain(|held| held != &entry),
        ActionPhase::Changed => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ActionId, ActionMapping, Binding, GamepadAxis, InputEvent, KeyCode, KeyboardEvent,
        Modifiers, Scope,
    };

    fn action(value: &str) -> ActionId {
        ActionId::new(value).expect("test action")
    }

    fn keyboard_event(key: KeyCode, pressed: bool) -> InputEvent {
        InputEvent::Keyboard(KeyboardEvent {
            device: DeviceToken::new(InputSource::Keyboard, "keyboard", 1),
            timestamp: 42,
            key,
            modifiers: Modifiers::empty(),
            pressed,
            repeat: false,
        })
    }

    #[test]
    fn keyboard_activate_suppresses_release_and_assigns_sequence() {
        let mut service = ActionInputService::new();
        service.add_mapping(ActionMapping::new(
            action("view/darkroom"),
            Binding::Keyboard {
                key: KeyCode::character('d'),
                modifiers: Modifiers::empty(),
            },
        ));
        let pressed_input = keyboard_event(KeyCode::character('d'), true);
        let released_input = keyboard_event(KeyCode::character('d'), false);
        let pressed = service.ingest(&pressed_input);
        let released = service.ingest(&released_input);
        assert_eq!(pressed.events[0].sequence, 1);
        assert_eq!(pressed.events[0].phase, ActionPhase::Pressed);
        assert!(released.events.is_empty());
    }

    #[test]
    fn scope_precedence_and_modal_capture_are_deterministic() {
        let mut service = ActionInputService::new();
        let binding = Binding::Keyboard {
            key: KeyCode::character('x'),
            modifiers: Modifiers::empty(),
        };
        service.add_mapping(ActionMapping::new(action("global"), binding.clone()));
        service.add_mapping(
            ActionMapping::new(action("darkroom"), binding.clone())
                .with_scope(Scope::View("darkroom".to_owned())),
        );
        service.set_context(InputContext {
            view: Some("darkroom".to_owned()),
            ..InputContext::default()
        });
        let input = keyboard_event(KeyCode::character('x'), true);
        let event = service.ingest(&input);
        assert_eq!(event.events[0].action.as_str(), "darkroom");
        service.set_modal(true);
        let ignored = service.ingest(&input);
        assert_eq!(ignored.ignored, Some(InputIgnoreReason::NoMapping));
    }

    #[test]
    fn midi_and_gamepad_values_are_normalized_without_native_backends() {
        let mut service = ActionInputService::new();
        let midi = service.connect_device(DeviceDescriptor::new(
            InputSource::Midi,
            "midi-1",
            "Controller",
        ));
        service.add_mapping(
            ActionMapping::new(
                action("exposure/value"),
                Binding::Midi {
                    channel: crate::types::MidiChannel::Number(0),
                    control: crate::types::MidiControl::ControlChange(7),
                },
            )
            .with_mode(ActionMode::Value),
        );
        let input = InputEvent::Midi(crate::types::MidiEvent {
            device: midi,
            timestamp: 1,
            channel: 0,
            control: crate::types::MidiControl::ControlChange(7),
            value: 64,
            pressed: true,
        });
        let report = service.ingest(&input);
        assert!((report.events[0].value.expect("MIDI value") - 64.0 / 127.0).abs() < 0.001);

        service.add_mapping(
            ActionMapping::new(
                action("navigation/axis"),
                Binding::Gamepad(GamepadControl::Axis {
                    axis: GamepadAxis::LeftX,
                    deadzone: 0.2,
                }),
            )
            .with_mode(ActionMode::Value),
        );
        let gamepad =
            service.connect_device(DeviceDescriptor::new(InputSource::Gamepad, "pad-1", "Pad"));
        let input = InputEvent::Gamepad(GamepadEvent::Axis {
            device: gamepad,
            timestamp: 2,
            axis: GamepadAxis::LeftX,
            value: 0.6,
        });
        let report = service.ingest(&input);
        assert!((report.events[0].value.expect("axis value") - 0.5).abs() < 0.001);
    }

    #[test]
    fn stale_device_generation_is_rejected_after_reconnect() {
        let mut service = ActionInputService::new();
        let first = service.connect_device(DeviceDescriptor::new(
            InputSource::Midi,
            "controller",
            "Controller",
        ));
        service.disconnect_device(&first);
        let second = service.connect_device(DeviceDescriptor::new(
            InputSource::Midi,
            "controller",
            "Controller",
        ));
        assert!(second.generation() > first.generation());
        let input = InputEvent::Midi(crate::types::MidiEvent {
            device: first,
            timestamp: 0,
            channel: 0,
            control: crate::types::MidiControl::Note(1),
            value: 1,
            pressed: true,
        });
        let report = service.ingest(&input);
        assert_eq!(
            report.ignored,
            Some(InputIgnoreReason::StaleDeviceGeneration)
        );
    }

    #[test]
    fn hold_and_tablet_bindings_preserve_release_and_pen_state() {
        let mut service = ActionInputService::new();
        service.add_mapping(
            ActionMapping::new(
                action("mask/brush"),
                Binding::Keyboard {
                    key: KeyCode::character('b'),
                    modifiers: Modifiers::empty(),
                },
            )
            .with_mode(ActionMode::Hold),
        );
        let pressed_input = keyboard_event(KeyCode::character('b'), true);
        let released_input = keyboard_event(KeyCode::character('b'), false);
        let pressed = service.ingest(&pressed_input);
        let released = service.ingest(&released_input);
        assert_eq!(pressed.events[0].phase, ActionPhase::Pressed);
        assert_eq!(released.events[0].phase, ActionPhase::Released);

        let tablet =
            service.connect_device(DeviceDescriptor::new(InputSource::Tablet, "pen-1", "Pen"));
        service.add_mapping(
            ActionMapping::new(
                action("mask/pressure"),
                Binding::Tablet(crate::types::TabletControl::Pressure),
            )
            .with_mode(ActionMode::Value),
        );
        let input = InputEvent::Tablet(crate::types::TabletEvent {
            device: tablet,
            timestamp: 4,
            phase: ActionPhase::Changed,
            x: 10.0,
            y: 20.0,
            pressure: 0.75,
            tilt_x: 0.1,
            tilt_y: -0.2,
            eraser: true,
            button: None,
        });
        let report = service.ingest(&input);
        assert_eq!(report.events[0].action.as_str(), "mask/pressure");
        assert_eq!(report.events[0].value, Some(0.75));
    }

    #[test]
    fn unavailable_devices_keep_mappings_but_reject_events() {
        let mut service = ActionInputService::new();
        service.set_device_unavailable(InputSource::Gamepad, "missing", "backend unavailable");
        let token = service.connect_device(DeviceDescriptor::new(
            InputSource::Gamepad,
            "present",
            "Pad",
        ));
        service.add_mapping(ActionMapping::new(
            action("pad/button"),
            Binding::Gamepad(GamepadControl::Button(crate::types::GamepadButton::South)),
        ));
        let input = InputEvent::Gamepad(GamepadEvent::Button {
            device: token,
            timestamp: 1,
            button: crate::types::GamepadButton::South,
            pressed: true,
        });
        let report = service.ingest(&input);
        assert_eq!(report.events[0].action.as_str(), "pad/button");
        assert_eq!(
            service.device_state(InputSource::Gamepad, "missing"),
            Some(&DeviceState::Unavailable("backend unavailable".to_owned()))
        );
        let input = InputEvent::Gamepad(GamepadEvent::Button {
            device: DeviceToken::new(InputSource::Gamepad, "missing", 1),
            timestamp: 2,
            button: crate::types::GamepadButton::South,
            pressed: true,
        });
        let unavailable = service.ingest(&input);
        assert_eq!(
            unavailable.ignored,
            Some(InputIgnoreReason::DeviceUnavailable)
        );
    }
}
