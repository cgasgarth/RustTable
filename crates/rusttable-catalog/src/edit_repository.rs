use std::fmt;

use rusttable_core::{Edit, EditId, PhotoId, Revision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditRepositoryError {
    Unavailable,
    CorruptPersistedData,
    NewEditIdConflict {
        edit_id: EditId,
    },
    UnknownEdit {
        edit_id: EditId,
    },
    EditRevisionConflict {
        edit_id: EditId,
        expected: Revision,
        actual: Revision,
    },
    PhotoIdentityMismatch {
        edit_id: EditId,
        expected: PhotoId,
        actual: PhotoId,
    },
    BasePhotoRevisionMismatch {
        edit_id: EditId,
        expected: Revision,
        actual: Revision,
    },
    CommitFailure,
}

impl fmt::Display for EditRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("edit repository is unavailable"),
            Self::CorruptPersistedData => formatter.write_str("edit repository data is corrupt"),
            Self::NewEditIdConflict { edit_id } => {
                write!(formatter, "edit ID {edit_id} already exists")
            }
            Self::UnknownEdit { edit_id } => write!(formatter, "edit ID {edit_id} is unknown"),
            Self::EditRevisionConflict {
                edit_id,
                expected,
                actual,
            } => write!(
                formatter,
                "edit {edit_id} revision conflict: expected {expected}, actual {actual}"
            ),
            Self::PhotoIdentityMismatch {
                edit_id,
                expected,
                actual,
            } => write!(
                formatter,
                "edit {edit_id} photo mismatch: expected {expected}, actual {actual}"
            ),
            Self::BasePhotoRevisionMismatch {
                edit_id,
                expected,
                actual,
            } => write!(
                formatter,
                "edit {edit_id} base photo revision mismatch: expected {expected}, actual {actual}"
            ),
            Self::CommitFailure => formatter.write_str("edit repository commit failed"),
        }
    }
}

impl std::error::Error for EditRepositoryError {}

pub trait EditRepository {
    /// Finds the current immutable edit value by identity.
    ///
    /// # Errors
    ///
    /// Returns a typed availability or corruption error.
    fn find_by_edit_id(&self, edit_id: EditId) -> Result<Option<Edit>, EditRepositoryError>;

    /// Lists current edits in ascending ID order.
    ///
    /// # Errors
    ///
    /// Returns a typed availability or corruption error.
    fn list(&self) -> Result<Vec<Edit>, EditRepositoryError>;

    /// Commits a new edit while rechecking ID absence in the durable transaction.
    ///
    /// # Errors
    ///
    /// Returns a typed conflict, availability, corruption, or commit error.
    fn commit_new(&mut self, edit: &Edit) -> Result<(), EditRepositoryError>;

    /// Commits a replacement while rechecking the expected current revision.
    ///
    /// # Errors
    ///
    /// Returns a typed conflict, availability, corruption, or commit error.
    fn commit_replacement(
        &mut self,
        expected_edit_revision: Revision,
        edit: &Edit,
    ) -> Result<(), EditRepositoryError>;
}
