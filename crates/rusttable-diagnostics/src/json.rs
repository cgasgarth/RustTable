use crate::event::{DiagnosticRecord, SCHEMA_VERSION};
use crate::redaction::Redactor;

pub(crate) fn escape(value: &str) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"[encoding-error]\"".to_owned())
        .trim_matches('"')
        .to_owned()
}

pub(crate) fn record_line(
    record: &DiagnosticRecord,
    sequence: u64,
    timestamp: u128,
    build: &str,
    redactor: &Redactor,
) -> String {
    let mut output = format!(
        "{{\"schema_version\":{SCHEMA_VERSION},\"sequence\":{sequence},\"timestamp_unix_ms\":{timestamp},\"build_identity\":\"{}\",\"code\":\"{}\",\"severity\":\"{}\",\"subsystem\":\"{}\",\"operation\":\"{}\",\"context\":{{",
        escape(build),
        escape(record.code.as_str()),
        record.severity.as_str(),
        record.subsystem.as_str(),
        escape(&record.operation)
    );
    let mut first = true;
    for (name, value) in record.context.values() {
        if let Some(value) = value {
            comma(&mut output, &mut first);
            let field = crate::event::DiagnosticField::text(
                name,
                value,
                crate::event::PrivacyClass::Private,
            )
            .expect("context field");
            let rendered = redactor.render(&field).expect("private context");
            let _ = write!(output, "\"{name}\":\"{}\"", escape(&rendered.value));
        }
    }
    output.push_str("},\"fields\":{");
    first = true;
    for field in &record.fields {
        if let Some(field) = redactor.render(field) {
            comma(&mut output, &mut first);
            let _ = write!(
                output,
                "\"{}\":{{\"value\":\"{}\",\"private\":{}}}",
                escape(&field.name),
                escape(&field.value),
                field.private
            );
        }
    }
    output.push_str("}}\n");
    output
}

pub(crate) fn human_line(
    record: &DiagnosticRecord,
    sequence: u64,
    timestamp: u128,
    redactor: &Redactor,
) -> String {
    let mut output = format!(
        "{timestamp} #{sequence} {} {} {}",
        record.severity.as_str(),
        record.subsystem.as_str(),
        record.code.as_str()
    );
    if !record.operation.is_empty() {
        output.push(' ');
        output.push_str(&record.operation);
    }
    for field in &record.fields {
        if let Some(field) = redactor.render(field) {
            output.push(' ');
            output.push_str(&field.name);
            output.push('=');
            output.push_str(&field.value);
        }
    }
    output.push('\n');
    output
}

fn comma(output: &mut String, first: &mut bool) {
    if !*first {
        output.push(',');
    }
    *first = false;
}

pub(crate) struct PanicFields<'a> {
    pub(crate) file: Option<&'a str>,
    pub(crate) line: Option<u32>,
    pub(crate) column: Option<u32>,
    pub(crate) payload_kind: &'a str,
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
        "{{\"schema_version\":{SCHEMA_VERSION},\"package_version\":\"{}\",\"timestamp_unix_ms\":{timestamp},\"pid\":{pid},\"target_os\":\"{}\",\"target_arch\":\"{}\",\"panic_file\":{},\"panic_line\":{},\"panic_column\":{},\"payload_kind\":\"{}\",\"payload_text\":\"[redacted]\",\"backtrace_status\":\"{}\",\"backtrace_text\":\"{}\"}}\n",
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
        escape(backtrace_status),
        escape(backtrace_text)
    )
}

fn optional_string(value: Option<&str>) -> String {
    value.map_or_else(
        || "null".to_owned(),
        |value| format!("\"{}\"", escape(value)),
    )
}
use std::fmt::Write;
