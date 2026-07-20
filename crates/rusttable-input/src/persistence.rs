use std::fmt;

use crate::types::{
    ActionId, ActionMapping, Binding, KeyCode, MidiChannel, MidiControl, Modifiers, Scope,
};

/// A malformed Darktable shortcutsrc line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShortcutParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ShortcutParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "shortcutsrc line {}: {}",
            self.line, self.message
        )
    }
}

impl std::error::Error for ShortcutParseError {}

/// Imports the representable subset of Darktable's line-oriented shortcutsrc.
/// Unsupported action effect tokens are intentionally ignored after the action
/// path, preserving the useful device/action binding rather than failing the
/// whole file.
///
/// # Errors
///
/// Returns the first line that cannot be represented as a stable action
/// mapping. Unsupported right-hand-side effect tokens are returned as
/// warnings instead of errors.
pub fn parse_darktable_shortcuts(
    source: &str,
) -> Result<(Vec<ActionMapping>, Vec<ShortcutParseError>), ShortcutParseError> {
    let mut mappings = Vec::new();
    let mut warnings = Vec::new();
    for (line_index, line) in source.lines().enumerate() {
        let line_number = line_index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((left, right)) = line.split_once('=') else {
            return Err(ShortcutParseError {
                line: line_number,
                message: "expected key=action assignment".to_owned(),
            });
        };
        let mut right_tokens = right.split(';').map(str::trim);
        let Some(first_action_token) = right_tokens.next().filter(|value| !value.is_empty()) else {
            return Err(ShortcutParseError {
                line: line_number,
                message: "missing action path".to_owned(),
            });
        };
        if first_action_token == "disabled" {
            continue;
        }
        let action_name = first_action_token;
        let action = ActionId::new(action_name).map_err(|_| ShortcutParseError {
            line: line_number,
            message: format!("invalid action path {action_name:?}"),
        })?;
        let (binding, scope, mode) = parse_binding(left.trim(), line_number)?;
        let mut mapping = ActionMapping::new(action, binding)
            .with_scope(scope)
            .with_mode(mode);
        for token in right_tokens {
            let token = token.trim();
            if token.is_empty() || token == "button" || token == "first" || token == "last" {
                continue;
            }
            if token.starts_with('*') || token.starts_with('+') || token.starts_with('-') {
                continue;
            }
            warnings.push(ShortcutParseError {
                line: line_number,
                message: format!("ignored unsupported action token {token:?}"),
            });
        }
        mapping.enabled = true;
        mappings.push(mapping);
    }
    Ok((mappings, warnings))
}

fn parse_binding(
    left: &str,
    line: usize,
) -> Result<(Binding, Scope, crate::types::ActionMode), ShortcutParseError> {
    let mut tokens = left.split(';').map(str::trim);
    let first = tokens.next().unwrap_or_default();
    let mut modifiers = Modifiers::empty();
    let mut mode = crate::types::ActionMode::Activate;
    let mut move_token = None;
    for token in tokens {
        match token.to_ascii_lowercase().as_str() {
            "shift" => modifiers = modifiers.union(Modifiers::SHIFT),
            "control" | "ctrl" => modifiers = modifiers.union(Modifiers::CONTROL),
            "alt" | "mod1" => modifiers = modifiers.union(Modifiers::ALT),
            "super" | "meta" | "logo" => modifiers = modifiers.union(Modifiers::SUPER),
            "scroll" | "horizontal" | "vertical" | "pan" | "diagonal" | "skew" => {
                move_token = Some(token.to_owned());
                mode = crate::types::ActionMode::Relative;
            }
            "double" | "triple" | "long" | "left" | "middle" | "right" | "up" | "down" => {}
            unknown => {
                return Err(ShortcutParseError {
                    line,
                    message: format!("unsupported binding token {unknown:?}"),
                });
            }
        }
    }
    let scope = Scope::Global;
    let binding = if first.eq_ignore_ascii_case("none") {
        return Err(ShortcutParseError {
            line,
            message: "unbound action is not a mapping".to_owned(),
        });
    } else if let Some(value) = first.strip_prefix("midi:") {
        Binding::Midi {
            channel: MidiChannel::Any,
            control: parse_midi_control(value).ok_or_else(|| ShortcutParseError {
                line,
                message: format!("invalid MIDI control {value:?}"),
            })?,
        }
    } else if let Some(value) = first.strip_prefix("gamepad:") {
        return Err(ShortcutParseError {
            line,
            message: format!("gamepad binding {value:?} requires a native control name"),
        });
    } else if let Some(value) = first.strip_prefix("tablet button ") {
        let button = value.parse::<u8>().map_err(|_| ShortcutParseError {
            line,
            message: format!("invalid tablet button {value:?}"),
        })?;
        Binding::Tablet(crate::types::TabletControl::Button(button))
    } else {
        Binding::Keyboard {
            key: KeyCode::from_name(first),
            modifiers,
        }
    };
    if move_token.is_some() {
        mode = crate::types::ActionMode::Relative;
    }
    Ok((binding, scope, mode))
}

fn parse_midi_control(value: &str) -> Option<MidiControl> {
    if let Some(value) = value.strip_prefix("CC") {
        return value
            .parse::<u8>()
            .ok()
            .filter(|value| *value < 128)
            .map(MidiControl::ControlChange);
    }
    value
        .parse::<u8>()
        .ok()
        .filter(|value| *value < 128)
        .map(MidiControl::Note)
}

/// Writes mappings in a stable Darktable-compatible subset of shortcutsrc.
#[must_use]
pub fn write_darktable_shortcuts(mappings: &[ActionMapping]) -> String {
    mappings
        .iter()
        .filter(|mapping| mapping.enabled)
        .map(|mapping| {
            let mut line = format!("{}={}", binding_text(&mapping.binding), mapping.action);
            if mapping.mode == crate::types::ActionMode::Relative {
                line.insert_str(line.find('=').unwrap_or(line.len()), ";scroll");
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
        + if mappings.iter().any(|mapping| mapping.enabled) {
            "\n"
        } else {
            ""
        }
}

fn binding_text(binding: &Binding) -> String {
    match binding {
        Binding::Keyboard { key, modifiers } => {
            let mut tokens = vec![key.name()];
            if modifiers.contains(Modifiers::SHIFT) {
                tokens.push("shift".to_owned());
            }
            if modifiers.contains(Modifiers::CONTROL) {
                tokens.push("control".to_owned());
            }
            if modifiers.contains(Modifiers::ALT) {
                tokens.push("alt".to_owned());
            }
            if modifiers.contains(Modifiers::SUPER) {
                tokens.push("super".to_owned());
            }
            tokens.join(";")
        }
        Binding::Midi { channel, control } => {
            let control = match control {
                MidiControl::Note(value) => value.to_string(),
                MidiControl::ControlChange(value) => format!("CC{value}"),
            };
            match channel {
                MidiChannel::Any => format!("midi:{control}"),
                MidiChannel::Number(value) => format!("midi{value}:{control}"),
            }
        }
        Binding::Gamepad(_) => "gamepad:unsupported".to_owned(),
        Binding::Tablet(crate::types::TabletControl::Button(button)) => {
            format!("tablet button {button}")
        }
        Binding::Tablet(control) => format!("tablet:{control:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ActionMode, Binding, MidiControl, Scope};

    #[test]
    fn imports_darktable_keyboard_midi_and_relative_lines() {
        let source = "d=view/darkroom\nF11;shift;scroll=iop/toneequal/preserve details\nd;scroll=disabled;action\nmidi:CC7=iop/exposure/exposure\n";
        let (mappings, warnings) = parse_darktable_shortcuts(source).expect("shortcutsrc");
        assert_eq!(mappings.len(), 3);
        assert!(warnings.is_empty());
        assert_eq!(mappings[1].mode, ActionMode::Relative);
        assert!(matches!(
            mappings[2].binding,
            Binding::Midi {
                control: MidiControl::ControlChange(7),
                ..
            }
        ));
    }

    #[test]
    fn round_trip_uses_stable_text_for_representable_bindings() {
        let source = "d=view/darkroom\nmidi:CC7=iop/exposure/exposure\n";
        let (mappings, _) = parse_darktable_shortcuts(source).expect("shortcutsrc");
        assert_eq!(write_darktable_shortcuts(&mappings), source);
        assert_eq!(mappings[0].scope, Scope::Global);
    }

    #[test]
    fn malformed_lines_are_actionable() {
        let error = parse_darktable_shortcuts("not an assignment").expect_err("invalid line");
        assert_eq!(error.line, 1);
        assert!(error.message.contains("assignment"));
    }

    #[test]
    fn bundled_darktable_shortcuts_are_importable() {
        let (mappings, _) = parse_darktable_shortcuts(include_str!("../../../data/shortcutsrc"))
            .expect("bundled shortcutsrc remains representable");
        assert!(mappings.len() > 50);
    }
}
