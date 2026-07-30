//! Profile persistence port used by the GTK editor.

use super::types::MappingProfile;
use std::fmt;
use std::path::Path;

/// Object-safe profile persistence boundary for application-owned storage.
pub trait ProfileIo {
    /// Loads and validates one user-selected profile.
    ///
    /// # Errors
    ///
    /// Returns a bounded, display-safe persistence error.
    fn load(&self, path: &Path) -> Result<MappingProfile, ProfileIoError>;

    /// Writes one canonical profile to a user-selected destination.
    ///
    /// # Errors
    ///
    /// Returns a bounded, display-safe persistence error.
    fn save(&self, path: &Path, profile: &MappingProfile) -> Result<(), ProfileIoError>;
}

/// Safe profile persistence failure returned by a [`ProfileIo`] implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileIoError(String);

impl ProfileIoError {
    /// Creates a persistence error that is safe to show in the editor.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ProfileIoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ProfileIoError {}

/// Default local profile port used by the standalone preferences window.
#[derive(Debug, Default, Clone, Copy)]
pub struct LocalProfileIo;

impl ProfileIo for LocalProfileIo {
    fn load(&self, path: &Path) -> Result<MappingProfile, ProfileIoError> {
        let contents = std::fs::read_to_string(path)
            .map_err(|error| ProfileIoError::new(format!("profile read failed: {error}")))?;
        MappingProfile::parse_json(&contents)
            .map_err(|error| ProfileIoError::new(error.to_string()))
    }

    fn save(&self, path: &Path, profile: &MappingProfile) -> Result<(), ProfileIoError> {
        let contents = profile
            .canonical_json()
            .map_err(|error| ProfileIoError::new(format!("profile encode failed: {error}")))?;
        std::fs::write(path, contents)
            .map_err(|error| ProfileIoError::new(format!("profile write failed: {error}")))
    }
}
