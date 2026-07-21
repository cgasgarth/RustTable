use std::fmt;

use rusttable_core::PhotoId;

use super::error::HistoryError;
use super::state::HistoryState;
use super::types::HistoryImportEntry;

/// A bounded source history sequence from darktable or an older `RustTable` store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryImport {
    photo_id: PhotoId,
    entries: Vec<HistoryImportEntry>,
    current_index: Option<usize>,
}

impl HistoryImport {
    /// Creates an import preserving source order and an explicit current/redo split.
    ///
    /// # Errors
    ///
    /// Rejects an empty source ID, a photo mismatch, or an out-of-range current index.
    pub fn new(
        photo_id: PhotoId,
        entries: Vec<HistoryImportEntry>,
        current_index: Option<usize>,
    ) -> Result<Self, HistoryImportError> {
        if current_index.is_some_and(|index| index >= entries.len()) {
            return Err(HistoryImportError::InvalidCurrentIndex);
        }
        if entries
            .iter()
            .any(|entry| entry.payload().edit().photo_id() != photo_id)
        {
            return Err(HistoryImportError::PhotoMismatch);
        }
        Ok(Self {
            photo_id,
            entries,
            current_index,
        })
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub fn entries(&self) -> &[HistoryImportEntry] {
        &self.entries
    }

    #[must_use]
    pub const fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    /// Reconstructs one immutable history graph and retains import/redo/restore evidence.
    ///
    /// # Errors
    ///
    /// Returns the same checked history errors used by native commands.
    pub fn reconstruct(self) -> Result<HistoryState, HistoryImportError> {
        let mut state = HistoryState::new(self.photo_id);
        let mut imported = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let revision = state
                .append_for_import(entry.payload().clone())
                .map_err(HistoryImportError::History)?;
            state.set_import_provenance(revision, entry.provenance().clone());
            state.retain_import_evidence(revision, entry.is_redo());
            imported.push(revision);
        }
        state.finish_import(&self.entries, &imported, self.current_index)?;
        Ok(state)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryImportError {
    InvalidCurrentIndex,
    PhotoMismatch,
    History(HistoryError),
}

impl fmt::Display for HistoryImportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCurrentIndex => {
                formatter.write_str("history import current index is invalid")
            }
            Self::PhotoMismatch => formatter.write_str("history import contains another photo"),
            Self::History(source) => write!(formatter, "history import failed: {source}"),
        }
    }
}

impl std::error::Error for HistoryImportError {}

impl From<HistoryError> for HistoryImportError {
    fn from(value: HistoryError) -> Self {
        Self::History(value)
    }
}
