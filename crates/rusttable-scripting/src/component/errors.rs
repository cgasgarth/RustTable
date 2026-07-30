use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ErrorCode {
    MalformedComponent,
    IncompatibleWorld,
    InvalidManifest,
    UnknownPermission,
    PermissionDenied,
    LimitExceeded,
    DeadlineExpired,
    Cancelled,
    Trap,
    CacheRejected,
    InvalidHandle,
    NotFound,
    Quarantined,
    Reloaded,
    HostCallFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
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

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::MalformedComponent => "malformed-component",
            Self::IncompatibleWorld => "incompatible-world",
            Self::InvalidManifest => "invalid-manifest",
            Self::UnknownPermission => "unknown-permission",
            Self::PermissionDenied => "permission-denied",
            Self::LimitExceeded => "limit-exceeded",
            Self::DeadlineExpired => "deadline-expired",
            Self::Cancelled => "cancelled",
            Self::Trap => "trap",
            Self::CacheRejected => "cache-rejected",
            Self::InvalidHandle => "invalid-handle",
            Self::NotFound => "not-found",
            Self::Quarantined => "quarantined",
            Self::Reloaded => "reloaded",
            Self::HostCallFailed => "host-call-failed",
        };
        formatter.write_str(name)
    }
}

impl std::error::Error for ScriptError {}
