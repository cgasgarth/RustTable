use std::fmt;

use rusttable_core::{AssetId, PhotoId};

use crate::{ImportRecord, SourcePath};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoryError {
    Unavailable,
    CorruptPersistedData,
    SourceConflict { source: SourcePath },
    PhotoIdConflict { photo_id: PhotoId },
    AssetIdConflict { asset_id: AssetId },
    CommitFailure,
}

pub trait ImportRepository {
    /// Finds the authoritative record for a canonical source key.
    ///
    /// # Errors
    ///
    /// Returns a typed storage or corruption error.
    fn find_by_source(&self, source: &SourcePath) -> Result<Option<ImportRecord>, RepositoryError>;

    /// Finds the authoritative record owning a photo ID.
    ///
    /// # Errors
    ///
    /// Returns a typed storage or corruption error.
    fn find_by_photo_id(&self, photo_id: PhotoId) -> Result<Option<ImportRecord>, RepositoryError>;

    /// Finds the authoritative record owning an asset ID.
    ///
    /// # Errors
    ///
    /// Returns a typed storage or corruption error.
    fn find_by_asset_id(&self, asset_id: AssetId) -> Result<Option<ImportRecord>, RepositoryError>;

    /// Commits one record while enforcing repository uniqueness.
    ///
    /// # Errors
    ///
    /// Returns a typed availability, corruption, conflict, or commit error.
    fn commit(&mut self, record: &ImportRecord) -> Result<(), RepositoryError>;

    /// Lists records in canonical source-key order.
    ///
    /// # Errors
    ///
    /// Returns a typed storage or corruption error.
    fn list(&self) -> Result<Vec<ImportRecord>, RepositoryError>;
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("import repository is unavailable"),
            Self::CorruptPersistedData => formatter.write_str("import repository data is corrupt"),
            Self::SourceConflict { source } => write!(formatter, "source {source} already exists"),
            Self::PhotoIdConflict { photo_id } => {
                write!(formatter, "photo ID {photo_id} conflicts")
            }
            Self::AssetIdConflict { asset_id } => {
                write!(formatter, "asset ID {asset_id} conflicts")
            }
            Self::CommitFailure => formatter.write_str("import repository commit failed"),
        }
    }
}

impl std::error::Error for RepositoryError {}
