use crate::context::CorrelationContext;
use crate::event::{DiagnosticEvent, SCHEMA_VERSION};
use crate::privacy::{Redactor, VisibleValue, visible_value};

pub(crate) fn escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
            character if character.is_control() => {
                use std::fmt::Write;
                write!(&mut escaped, "\\u{:04x}", character as u32).expect("String cannot fail");
            }
            character => escaped.push(character),
        }
    }
    escaped
}

pub(crate) fn event_line(
    package_version: &str,
    sequence: u64,
    timestamp: u128,
    event: &DiagnosticEvent,
    redactor: &Redactor,
) -> String {
    let mut line = format!(
        "{{\"schema_version\":{SCHEMA_VERSION},\"sequence\":{sequence},\"timestamp_unix_ms\":{timestamp},\"code\":\"{}\",\"severity\":\"{}\",\"subsystem\":\"{}\",\"operation\":\"{}\",\"context\":",
        escape(&event.code.as_str()),
        event.severity.as_str(),
        escape(event.code.subsystem().as_str()),
        escape(&event.operation),
    );
    context_json(&mut line, &event.context);
    line.push_str(",\"fields\":[");
    let mut first = true;
    for field in &event.fields {
        let Some(value) = visible_value(field, redactor) else {
            continue;
        };
        if !first {
            line.push(',');
        }
        first = false;
        line.push_str("{\"key\":\"");
        line.push_str(&escape(field.key()));
        line.push_str("\",\"privacy\":\"");
        line.push_str(field.privacy.as_str());
        line.push_str("\",\"value\":");
        json_value(&mut line, value);
        line.push('}');
    }
    line.push_str("],\"source_build\":{\"package_version\":\"");
    line.push_str(&escape(package_version));
    line.push_str("\",\"target_os\":\"");
    line.push_str(std::env::consts::OS);
    line.push_str("\",\"target_arch\":\"");
    line.push_str(std::env::consts::ARCH);
    line.push_str("\"}}\n");
    line
}

pub(crate) fn human_line(
    sequence: u64,
    timestamp: u128,
    event: &DiagnosticEvent,
    redactor: &Redactor,
) -> String {
    let mut line = format!(
        "{timestamp} seq={sequence} {} {} op={} ",
        event.severity.as_str(),
        event.code.as_str(),
        escape_human(&event.operation),
    );
    context_human(&mut line, &event.context);
    for field in &event.fields {
        let Some(value) = visible_value(field, redactor) else {
            continue;
        };
        line.push(' ');
        line.push_str(field.key());
        line.push('=');
        human_value(&mut line, value);
    }
    line.push('\n');
    line
}

fn context_json(line: &mut String, context: &CorrelationContext) {
    line.push('{');
    let values = [
        ("request", context.request.as_ref()),
        ("photo", context.photo.as_ref()),
        ("edit", context.edit.as_ref()),
        ("task", context.task.as_ref()),
        ("device", context.device.as_ref()),
    ];
    let mut first = true;
    for (name, value) in values
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name, value)))
    {
        if !first {
            line.push(',');
        }
        first = false;
        line.push('"');
        line.push_str(name);
        line.push_str("\":\"");
        line.push_str(&escape(value.as_str()));
        line.push('"');
    }
    line.push('}');
}

fn context_human(line: &mut String, context: &CorrelationContext) {
    for (name, value) in [
        ("request", context.request.as_ref()),
        ("photo", context.photo.as_ref()),
        ("edit", context.edit.as_ref()),
        ("task", context.task.as_ref()),
        ("device", context.device.as_ref()),
    ] {
        if let Some(value) = value {
            line.push_str(name);
            line.push('=');
            line.push_str(value.as_str());
            line.push(' ');
        }
    }
}

fn json_value(line: &mut String, value: VisibleValue<'_>) {
    match value {
        VisibleValue::Text(value) | VisibleValue::Float(value) => {
            line.push('"');
            line.push_str(&escape(value));
            line.push('"');
        }
        VisibleValue::Integer(value) => line.push_str(&value.to_string()),
        VisibleValue::Unsigned(value) => line.push_str(&value.to_string()),
        VisibleValue::Boolean(value) => line.push_str(if value { "true" } else { "false" }),
        VisibleValue::PrivateAlias(value) => {
            line.push('"');
            line.push_str(value.as_str());
            line.push('"');
        }
    }
}

fn human_value(line: &mut String, value: VisibleValue<'_>) {
    match value {
        VisibleValue::Text(value) | VisibleValue::Float(value) => {
            line.push_str(&escape_human(value));
        }
        VisibleValue::Integer(value) => line.push_str(&value.to_string()),
        VisibleValue::Unsigned(value) => line.push_str(&value.to_string()),
        VisibleValue::Boolean(value) => line.push_str(if value { "true" } else { "false" }),
        VisibleValue::PrivateAlias(value) => line.push_str(value.as_str()),
    }
}

fn escape_human(value: &str) -> String {
    value.chars().fold(String::new(), |mut escaped, character| {
        if character.is_control() {
            use std::fmt::Write;
            write!(&mut escaped, "\\u{:04x}", character as u32).expect("String cannot fail");
        } else {
            escaped.push(character);
        }
        escaped
    })
}

#[cfg(test)]
mod tests {
    use super::escape;

    #[test]
    fn escapes_json_controls_and_preserves_unicode() {
        assert_eq!(
            escape("\"\\\n\r\t\u{08}\u{0c}\u{01}é"),
            "\\\"\\\\\\n\\r\\t\\b\\f\\u0001é"
        );
    }
}
