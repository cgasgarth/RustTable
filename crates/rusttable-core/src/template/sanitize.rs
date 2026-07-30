use std::fmt;

use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SanitizerPolicy {
    pub max_component_bytes: usize,
    pub max_path_bytes: usize,
    pub hash_suffix_on_truncate: bool,
}

impl Default for SanitizerPolicy {
    fn default() -> Self {
        Self {
            max_component_bytes: 255,
            max_path_bytes: 4_096,
            hash_suffix_on_truncate: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedComponent {
    pub value: String,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SanitizeError {
    EmptyLimit,
    ComponentTooLong,
    PathTooLong,
}

impl fmt::Display for SanitizeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyLimit => "sanitizer limit cannot be zero",
            Self::ComponentTooLong => "sanitized component exceeds its byte limit",
            Self::PathTooLong => "sanitized relative path exceeds its byte limit",
        })
    }
}

impl std::error::Error for SanitizeError {}

/// Sanitizes one expanded component using portable cross-platform rules.
///
/// # Errors
///
/// Returns an error when configured component or path bounds cannot be honored.
pub fn sanitize_component(
    input: &str,
    policy: SanitizerPolicy,
) -> Result<SanitizedComponent, SanitizeError> {
    if policy.max_component_bytes == 0 || policy.max_path_bytes == 0 {
        return Err(SanitizeError::EmptyLimit);
    }
    let normalized: String = input.nfc().map(|(character, _)| character).collect();
    let mut output = String::with_capacity(normalized.len());
    for character in normalized.chars() {
        if character.is_control() || is_illegal(character) {
            output.push('_');
        } else {
            output.push(character);
        }
    }
    while output.ends_with(['.', ' ']) {
        output.pop();
    }
    if is_reserved_name(&output) {
        output.insert(0, '_');
    }
    if output.is_empty() {
        output.push('_');
    }
    let original = output.clone();
    if output.len() > policy.max_component_bytes {
        if !policy.hash_suffix_on_truncate {
            return Err(SanitizeError::ComponentTooLong);
        }
        let suffix = format!("~{}", &super::hash_hex(input.as_bytes())[..8]);
        let budget = policy.max_component_bytes.saturating_sub(suffix.len());
        let prefix = truncate_utf8(&output, budget);
        output = format!("{prefix}{suffix}");
        if output.len() > policy.max_component_bytes {
            return Err(SanitizeError::ComponentTooLong);
        }
    }
    Ok(SanitizedComponent {
        changed: output != input || original != output,
        value: output,
    })
}

/// Validates the aggregate byte bound of already-sanitized components.
///
/// # Errors
///
/// Returns an error when the relative path exceeds its configured bound.
pub fn validate_relative_components(
    components: &[String],
    policy: SanitizerPolicy,
) -> Result<(), SanitizeError> {
    let path_bytes = components
        .iter()
        .map(String::len)
        .sum::<usize>()
        .saturating_add(components.len().saturating_sub(1));
    if path_bytes > policy.max_path_bytes {
        return Err(SanitizeError::PathTooLong);
    }
    Ok(())
}

fn is_illegal(character: char) -> bool {
    matches!(
        character,
        '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
    )
}

fn is_reserved_name(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or(value);
    let upper = stem.to_ascii_uppercase();
    matches!(
        upper.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    &value[..end]
}

pub(crate) fn slugify(value: &str) -> String {
    let mut output = String::new();
    let mut separator = false;
    for (character, _) in value.nfc() {
        if character.is_alphanumeric() {
            if separator && !output.is_empty() {
                output.push('-');
            }
            output.extend(character.to_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    output
}
