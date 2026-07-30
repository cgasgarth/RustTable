use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorCode {
    InvalidManifest,
    IncompatibleApi,
    PermissionDenied,
    LimitExceeded,
    LibraryDenied,
    ModuleDenied,
    ScriptFailed,
    ScriptQuarantined,
    Cancelled,
    StaleGeneration,
    StorageConflict,
    EventBackpressure,
    SerializationBound,
    PanicContained,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptError {
    pub code: ErrorCode,
    pub message: String,
}

impl ScriptError {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for ScriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ScriptError {}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::InvalidManifest => "invalid-manifest",
            Self::IncompatibleApi => "incompatible-api",
            Self::PermissionDenied => "permission-denied",
            Self::LimitExceeded => "limit-exceeded",
            Self::LibraryDenied => "library-denied",
            Self::ModuleDenied => "module-denied",
            Self::ScriptFailed => "script-failed",
            Self::ScriptQuarantined => "script-quarantined",
            Self::Cancelled => "cancelled",
            Self::StaleGeneration => "stale-generation",
            Self::StorageConflict => "storage-conflict",
            Self::EventBackpressure => "event-backpressure",
            Self::SerializationBound => "serialization-bound",
            Self::PanicContained => "panic-contained",
        };
        formatter.write_str(name)
    }
}

impl From<mlua::Error> for ScriptError {
    fn from(error: mlua::Error) -> Self {
        let code = match &error {
            mlua::Error::MemoryError(_) => ErrorCode::LimitExceeded,
            mlua::Error::RuntimeError(message) if message.contains("quota exceeded") => {
                ErrorCode::LimitExceeded
            }
            mlua::Error::RuntimeError(message) if message.contains("cancelled") => {
                ErrorCode::Cancelled
            }
            _ => ErrorCode::ScriptFailed,
        };
        Self::new(code, redact_lua_error(&error.to_string()))
    }
}

fn redact_lua_error(message: &str) -> String {
    message
        .split_whitespace()
        .map(|part| {
            if part.contains('/') || part.contains('\\') {
                "<redacted>"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_are_stable_and_paths_are_redacted() {
        assert_eq!(ErrorCode::PermissionDenied.to_string(), "permission-denied");
        let error = redact_lua_error("bad /Users/private/script.lua");
        assert_eq!(error, "bad <redacted>");
    }
}
