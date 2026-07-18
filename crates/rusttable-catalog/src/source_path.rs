use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourcePath(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePathError {
    Empty,
    LeadingSeparator,
    TrailingSeparator,
    EmptyComponent,
    DotComponent,
    Nul,
    Backslash,
    ComponentTooLong,
    PathTooLong,
}

impl SourcePath {
    /// Parses a relative UTF-8 logical catalog key with `/` separators.
    ///
    /// # Errors
    ///
    /// Returns a typed error for empty, ambiguous, NUL-containing, or
    /// overlong keys.
    pub fn new(value: &str) -> Result<Self, SourcePathError> {
        if value.is_empty() {
            return Err(SourcePathError::Empty);
        }
        if value.starts_with('/') {
            return Err(SourcePathError::LeadingSeparator);
        }
        if value.ends_with('/') {
            return Err(SourcePathError::TrailingSeparator);
        }
        if value.contains('\0') {
            return Err(SourcePathError::Nul);
        }
        if value.contains('\\') {
            return Err(SourcePathError::Backslash);
        }
        if value.len() > 4_096 {
            return Err(SourcePathError::PathTooLong);
        }
        for component in value.split('/') {
            if component.is_empty() {
                return Err(SourcePathError::EmptyComponent);
            }
            if component == "." || component == ".." {
                return Err(SourcePathError::DotComponent);
            }
            if component.len() > 255 {
                return Err(SourcePathError::ComponentTooLong);
            }
        }
        Ok(Self(value.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }
}

impl fmt::Display for SourcePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl fmt::Display for SourcePathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "source path is empty",
            Self::LeadingSeparator => "source path cannot start with `/`",
            Self::TrailingSeparator => "source path cannot end with `/`",
            Self::EmptyComponent => "source path contains an empty component",
            Self::DotComponent => "source path contains `.` or `..`",
            Self::Nul => "source path contains NUL",
            Self::Backslash => "source path contains a backslash",
            Self::ComponentTooLong => "source path component exceeds 255 UTF-8 bytes",
            Self::PathTooLong => "source path exceeds 4096 UTF-8 bytes",
        })
    }
}

impl std::error::Error for SourcePathError {}
