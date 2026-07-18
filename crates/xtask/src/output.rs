use std::io::{self, Write};

use serde::Serialize;

use crate::cli::OutputFormat;

const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: u32,
    pub record: &'static str,
    pub command: String,
    pub status: &'static str,
    pub data: serde_json::Value,
}

impl Report {
    #[must_use]
    pub fn new(command: impl Into<String>, data: serde_json::Value) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            record: "xtask.command",
            command: command.into(),
            status: "ok",
            data,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Output {
    format: OutputFormat,
}

impl Output {
    #[must_use]
    pub const fn new(format: OutputFormat) -> Self {
        Self { format }
    }

    /// Emits exactly one selected-format record on stdout.
    pub fn emit(self, report: &Report) -> io::Result<()> {
        match self.format {
            OutputFormat::Human => {
                println!("{}: {}", report.command, report.data);
                Ok(())
            }
            OutputFormat::Json => {
                let mut stdout = io::stdout().lock();
                serde_json::to_writer(&mut stdout, &report).map_err(io::Error::other)?;
                stdout.write_all(b"\n")
            }
        }
    }

    pub fn emit_error(self, message: &str) {
        if matches!(self.format, OutputFormat::Json) {
            let value = serde_json::json!({
                "schema_version": SCHEMA_VERSION,
                "record": "xtask.error",
                "command": "xtask",
                "status": "error",
                "data": { "message": message },
            });
            if let Ok(serialized) = serde_json::to_string(&value) {
                println!("{serialized}");
            }
        }
    }
}
