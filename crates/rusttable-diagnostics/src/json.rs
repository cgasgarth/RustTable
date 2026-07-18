use crate::event::DiagnosticEvent;

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

pub(crate) fn event_line(package_version: &str, timestamp: u128, event: DiagnosticEvent) -> String {
    let failure = event
        .failure_code()
        .map_or_else(|| "null".to_owned(), |code| format!("\"{}\"", escape(code)));
    format!(
        "{{\"schema_version\":1,\"timestamp_unix_ms\":{timestamp},\"package_version\":\"{}\",\"event\":\"{}\",\"failure_code\":{failure}}}\n",
        escape(package_version),
        escape(event.name()),
    )
}

pub(crate) fn crash_line(
    package_version: &str,
    timestamp: u128,
    pid: u32,
    panic: &PanicFields<'_>,
    backtrace_status: &str,
    backtrace_text: &str,
) -> String {
    format!(
        "{{\"schema_version\":1,\"package_version\":\"{}\",\"timestamp_unix_ms\":{timestamp},\"pid\":{pid},\"target_os\":\"{}\",\"target_arch\":\"{}\",\"panic_file\":{},\"panic_line\":{},\"panic_column\":{},\"payload_kind\":\"{}\",\"payload_text\":\"{}\",\"backtrace_status\":\"{}\",\"backtrace_text\":\"{}\"}}\n",
        escape(package_version),
        std::env::consts::OS,
        std::env::consts::ARCH,
        optional_string(panic.file),
        panic
            .line
            .map_or_else(|| "null".to_owned(), |value| value.to_string()),
        panic
            .column
            .map_or_else(|| "null".to_owned(), |value| value.to_string()),
        escape(panic.payload_kind),
        escape(panic.payload_text),
        escape(backtrace_status),
        escape(backtrace_text),
    )
}

fn optional_string(value: Option<&str>) -> String {
    value.map_or_else(
        || "null".to_owned(),
        |value| format!("\"{}\"", escape(value)),
    )
}

pub(crate) struct PanicFields<'a> {
    pub(crate) file: Option<&'a str>,
    pub(crate) line: Option<u32>,
    pub(crate) column: Option<u32>,
    pub(crate) payload_kind: &'a str,
    pub(crate) payload_text: &'a str,
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
