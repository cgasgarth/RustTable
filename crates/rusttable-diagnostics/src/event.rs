use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticEvent {
    Startup,
    Shutdown,
    ApplicationFailure(ApplicationFailureCode),
}

impl DiagnosticEvent {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Shutdown => "shutdown",
            Self::ApplicationFailure(_) => "application_failure",
        }
    }

    pub(crate) const fn failure_code(self) -> Option<&'static str> {
        match self {
            Self::ApplicationFailure(code) => Some(code.as_str()),
            Self::Startup | Self::Shutdown => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplicationFailureCode {
    IcedRun,
}

impl ApplicationFailureCode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::IcedRun => "iced_run",
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
}

impl fmt::Display for DiagnosticsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DirectoryUnavailable => formatter.write_str("diagnostics directory unavailable"),
            Self::InvalidPath => formatter.write_str("diagnostics path is invalid"),
            Self::SymlinkRefused(path) => write!(formatter, "diagnostics symlink refused: {path}"),
            Self::Storage(error) => write!(formatter, "diagnostics storage failure: {error}"),
            Self::Poisoned => formatter.write_str("diagnostics storage lock poisoned"),
        }
    }
}

impl std::error::Error for DiagnosticsError {}

impl From<std::io::Error> for DiagnosticsError {
    fn from(error: std::io::Error) -> Self {
        Self::Storage(error)
    }
}
