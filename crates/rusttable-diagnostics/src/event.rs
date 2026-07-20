use std::fmt;

use crate::context::CorrelationContext;
use crate::privacy::DiagnosticField;

pub const SCHEMA_VERSION: u16 = 1;
pub const MAX_EVENT_FIELDS: usize = 64;
pub const MAX_EVENT_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticCode {
    subsystem: Subsystem,
    identity: String,
}

impl DiagnosticCode {
    /// Creates a stable subsystem-scoped ASCII diagnostic identity.
    /// # Errors
    ///
    /// Returns an error when the identity is not bounded lowercase ASCII.
    pub fn new(subsystem: Subsystem, identity: &str) -> Result<Self, DiagnosticsError> {
        validate_identifier(identity, "diagnostic code")?;
        Ok(Self {
            subsystem,
            identity: identity.to_owned(),
        })
    }

    #[must_use]
    pub fn as_str(&self) -> String {
        format!("{}.{}", self.subsystem.as_str(), self.identity)
    }

    #[must_use]
    pub const fn subsystem(&self) -> &Subsystem {
        &self.subsystem
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Subsystem(String);

impl Subsystem {
    /// # Errors
    ///
    /// Returns an error when the name is not bounded lowercase ASCII.
    pub fn new(name: &str) -> Result<Self, DiagnosticsError> {
        validate_identifier(name, "subsystem")?;
        Ok(Self(name.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<Subsystem> for String {
    fn from(value: Subsystem) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    Trace,
    Debug,
    Info,
    Warning,
    Error,
    Critical,
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Critical => "critical",
        }
    }

    #[must_use]
    pub const fn is_warning_or_higher(self) -> bool {
        matches!(self, Self::Warning | Self::Error | Self::Critical)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticEvent {
    pub(crate) code: DiagnosticCode,
    pub(crate) severity: Severity,
    pub(crate) operation: String,
    pub(crate) context: CorrelationContext,
    pub(crate) fields: Vec<DiagnosticField>,
}

impl DiagnosticEvent {
    /// # Errors
    ///
    /// Returns an error when the operation is not a bounded lowercase identity.
    pub fn new(
        code: DiagnosticCode,
        severity: Severity,
        operation: &str,
    ) -> Result<Self, DiagnosticsError> {
        validate_identifier(operation, "operation")?;
        Ok(Self {
            code,
            severity,
            operation: operation.to_owned(),
            context: CorrelationContext::default(),
            fields: Vec::new(),
        })
    }

    #[must_use]
    pub fn startup() -> Self {
        Self::well_known("lifecycle", "startup", Severity::Info, "startup")
    }

    #[must_use]
    pub fn shutdown() -> Self {
        Self::well_known("lifecycle", "shutdown", Severity::Info, "shutdown")
    }

    #[must_use]
    pub fn application_failure(code: ApplicationFailureCode) -> Self {
        Self::well_known(
            "lifecycle",
            code.as_str(),
            Severity::Error,
            "application_failure",
        )
    }

    #[must_use]
    pub fn with_context(mut self, context: CorrelationContext) -> Self {
        self.context = context;
        self
    }

    /// # Errors
    ///
    /// Returns an error when the event reaches its field bound.
    pub fn with_field(mut self, field: DiagnosticField) -> Result<Self, DiagnosticsError> {
        if self.fields.len() == MAX_EVENT_FIELDS {
            return Err(DiagnosticsError::EventTooLarge);
        }
        self.fields.push(field);
        Ok(self)
    }

    #[must_use]
    pub fn code(&self) -> &DiagnosticCode {
        &self.code
    }

    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.severity
    }

    #[must_use]
    pub fn operation(&self) -> &str {
        &self.operation
    }

    fn well_known(subsystem: &str, identity: &str, severity: Severity, operation: &str) -> Self {
        Self {
            code: DiagnosticCode {
                subsystem: Subsystem(subsystem.to_owned()),
                identity: identity.to_owned(),
            },
            severity,
            operation: operation.to_owned(),
            context: CorrelationContext::default(),
            fields: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationFailureCode {
    DesktopUiRun,
    ConfigurationRejected,
}

impl ApplicationFailureCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::DesktopUiRun => "desktop_ui_run",
            Self::ConfigurationRejected => "configuration_rejected",
        }
    }
}

#[derive(Debug)]
pub enum DiagnosticsError {
    DirectoryUnavailable,
    InvalidPath,
    SymlinkRefused(&'static str),
    Storage(std::io::Error),
    Poisoned,
    InvalidIdentifier(&'static str),
    FieldTooLarge,
    EventTooLarge,
    NoAvailableSink,
}

impl fmt::Display for DiagnosticsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectoryUnavailable => formatter.write_str("diagnostics directory unavailable"),
            Self::InvalidPath => formatter.write_str("diagnostics path is invalid"),
            Self::SymlinkRefused(path) => write!(formatter, "diagnostics symlink refused: {path}"),
            Self::Storage(error) => write!(formatter, "diagnostics storage failure: {error}"),
            Self::Poisoned => formatter.write_str("diagnostics storage lock poisoned"),
            Self::InvalidIdentifier(kind) => write!(formatter, "invalid {kind} identity"),
            Self::FieldTooLarge => formatter.write_str("diagnostic field exceeds its bound"),
            Self::EventTooLarge => formatter.write_str("diagnostic event exceeds its bound"),
            Self::NoAvailableSink => formatter.write_str("no diagnostics sink is available"),
        }
    }
}

impl std::error::Error for DiagnosticsError {}

impl From<std::io::Error> for DiagnosticsError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error)
    }
}

fn validate_identifier(value: &str, kind: &'static str) -> Result<(), DiagnosticsError> {
    if value.is_empty()
        || value.len() > 64
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
        })
    {
        return Err(DiagnosticsError::InvalidIdentifier(kind));
    }
    Ok(())
}
