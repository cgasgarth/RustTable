use std::fmt;

use super::{HistoryApplyOutcome, HistoryCommand, HistoryError, HistoryState, HistoryVersion};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryRepositoryError {
    Unavailable,
    CorruptPersistedData,
    VersionConflict {
        expected: HistoryVersion,
        actual: HistoryVersion,
    },
    CommitFailure,
}

impl fmt::Display for HistoryRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("history repository is unavailable"),
            Self::CorruptPersistedData => formatter.write_str("history data is corrupt"),
            Self::VersionConflict { expected, actual } => write!(
                formatter,
                "persisted history version conflict: expected {expected}, actual {actual}"
            ),
            Self::CommitFailure => formatter.write_str("history repository commit failed"),
        }
    }
}

impl std::error::Error for HistoryRepositoryError {}

pub trait HistoryRepository {
    /// Loads the complete immutable graph and pointer metadata.
    ///
    /// # Errors
    ///
    /// Returns a typed availability, corruption, or persistence error.
    fn load(&self) -> Result<Option<HistoryState>, HistoryRepositoryError>;

    /// Commits one complete state after atomically checking its prior version.
    ///
    /// # Errors
    ///
    /// Returns a typed availability, corruption, version-conflict, or commit error.
    fn commit(
        &mut self,
        expected: HistoryVersion,
        state: &HistoryState,
    ) -> Result<(), HistoryRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableHistoryError {
    Domain(HistoryError),
    Repository(HistoryRepositoryError),
}

impl fmt::Display for DurableHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Domain(source) => write!(formatter, "history domain failure: {source}"),
            Self::Repository(source) => write!(formatter, "history persistence failure: {source}"),
        }
    }
}

impl std::error::Error for DurableHistoryError {}

/// Applies a command in memory, then publishes the resulting graph in one
/// repository transaction. Callers can reload after a version conflict.
pub struct DurableHistoryService;

impl DurableHistoryService {
    /// Applies one command and commits the resulting state transactionally.
    ///
    /// # Errors
    ///
    /// Returns a domain validation or repository concurrency/persistence error;
    /// the caller's state is unchanged when the commit fails.
    pub fn apply(
        state: &mut HistoryState,
        expected: HistoryVersion,
        command: HistoryCommand,
        repository: &mut dyn HistoryRepository,
    ) -> Result<HistoryApplyOutcome, DurableHistoryError> {
        let mut candidate = state.clone();
        let outcome = candidate
            .apply(expected, command)
            .map_err(DurableHistoryError::Domain)?;
        repository
            .commit(expected, &candidate)
            .map_err(DurableHistoryError::Repository)?;
        *state = candidate;
        Ok(outcome)
    }
}
