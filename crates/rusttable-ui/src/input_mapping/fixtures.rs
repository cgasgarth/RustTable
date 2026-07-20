use super::types::{
    ActionContext, ActionDefinition, ActionId, Binding, BindingSource, ContinuousSettings,
    DeviceDescriptor, DeviceKind, KeyChord, KeyModifier, MappingSnapshot, MidiControl,
    PROFILE_SCHEMA_VERSION, ParameterSchema, RelativeMode,
};

/// Returns a deterministic registry/device fixture used by the standalone UI
/// until the #512 service supplies the live snapshot.
#[must_use]
pub fn default_snapshot() -> MappingSnapshot {
    MappingSnapshot {
        schema_version: PROFILE_SCHEMA_VERSION,
        generation: 1,
        actions: default_actions(),
        devices: default_devices(),
        bindings: default_bindings(),
    }
}

fn default_actions() -> Vec<ActionDefinition> {
    vec![
        action("view.toggle", "Toggle view", "View", None),
        action("image.import", "Import image", "File", None),
        action("image.export", "Export image", "File", None),
        action("edit.undo", "Undo", "Editing", None),
        action(
            "darkroom.exposure",
            "Adjust exposure",
            "Darkroom",
            Some(ParameterSchema {
                min: -18.0,
                max: 18.0,
                step: 0.01,
            }),
        ),
        action(
            "rating.set",
            "Set rating",
            "Metadata",
            Some(ParameterSchema {
                min: 0.0,
                max: 5.0,
                step: 1.0,
            }),
        ),
    ]
}

fn default_devices() -> Vec<DeviceDescriptor> {
    vec![
        device(
            "keyboard",
            "Keyboard",
            DeviceKind::Keyboard,
            true,
            vec!["chords", "sequences"],
        ),
        device(
            "pointer",
            "Pointer",
            DeviceKind::Pointer,
            true,
            vec!["buttons", "wheel"],
        ),
        device(
            "tablet",
            "Tablet",
            DeviceKind::Tablet,
            false,
            vec!["pressure", "tilt", "buttons"],
        ),
        device(
            "midi-1",
            "MIDI controller 1",
            DeviceKind::Midi,
            false,
            vec!["note", "cc", "pitch"],
        ),
        device(
            "gamepad-1",
            "Gamepad 1",
            DeviceKind::Gamepad,
            false,
            vec!["buttons", "axes", "hats"],
        ),
    ]
}

fn default_bindings() -> Vec<Binding> {
    vec![
        Binding {
            id: "default-view-toggle".to_owned(),
            action_id: ActionId::from("view.toggle"),
            device_alias: "keyboard".to_owned(),
            context: ActionContext::Global,
            source: BindingSource::Keyboard {
                sequence: vec![KeyChord::new("Tab", [])],
            },
            continuous: None,
            enabled: true,
            built_in: true,
        },
        Binding {
            id: "default-undo".to_owned(),
            action_id: ActionId::from("edit.undo"),
            device_alias: "keyboard".to_owned(),
            context: ActionContext::Global,
            source: BindingSource::Keyboard {
                sequence: vec![KeyChord::new("Z", [KeyModifier::Control])],
            },
            continuous: None,
            enabled: true,
            built_in: true,
        },
        Binding {
            id: "default-exposure".to_owned(),
            action_id: ActionId::from("darkroom.exposure"),
            device_alias: "midi-1".to_owned(),
            context: ActionContext::Darkroom,
            source: BindingSource::Midi {
                control: MidiControl::ControlChange(1),
                channel: Some(1),
                relative_mode: RelativeMode::Absolute,
            },
            continuous: Some(ContinuousSettings {
                target_min: -18.0,
                target_max: 18.0,
                ..ContinuousSettings::default()
            }),
            enabled: true,
            built_in: true,
        },
    ]
}

fn action(
    id: &str,
    label: &str,
    category: &str,
    parameter: Option<ParameterSchema>,
) -> ActionDefinition {
    ActionDefinition {
        id: ActionId::from(id),
        label: label.to_owned(),
        category: category.to_owned(),
        contexts: vec![
            ActionContext::Global,
            ActionContext::Lighttable,
            ActionContext::Darkroom,
        ],
        parameter,
        available: true,
        nonremovable: false,
    }
}

fn device(
    alias: &str,
    label: &str,
    kind: DeviceKind,
    available: bool,
    capabilities: Vec<&str>,
) -> DeviceDescriptor {
    DeviceDescriptor {
        alias: alias.to_owned(),
        label: label.to_owned(),
        kind,
        available,
        capabilities: capabilities.into_iter().map(str::to_owned).collect(),
    }
}
