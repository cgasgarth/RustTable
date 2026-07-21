use std::fmt;

use super::types::{
    HistoryBranchId, HistoryRevisionId, HistorySnapshotId, HistorySummaryError, HistoryVersion,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryError {
    VersionConflict {
        expected: HistoryVersion,
        actual: HistoryVersion,
    },
    RevisionOverflow,
    UnknownRevision(HistoryRevisionId),
    UnknownBranch(HistoryBranchId),
    UnknownSnapshot(HistorySnapshotId),
    PhotoMismatch,
    InvalidBranchName,
    InvalidSnapshotName,
    EmptyHistory,
    NoUndo,
    NoRedo,
    InvalidCursor,
    ActiveBranchDeletion,
    BranchHasSnapshot(HistoryBranchId),
    BranchHasEvidence(HistoryBranchId),
    SourceBranchMismatch,
    DuplicateEvidence,
    MissingEvidence,
    CannotPruneReferencedRevision(HistoryRevisionId),
    InvalidPersistedState,
    Summary(HistorySummaryError),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::VersionConflict { expected, actual } => {
                write!(
                    formatter,
                    "history version conflict: expected {expected}, actual {actual}"
                )
            }
            Self::RevisionOverflow => formatter.write_str("history revision ID overflow"),
            Self::UnknownRevision(id) => write!(formatter, "history revision {id} is unknown"),
            Self::UnknownBranch(id) => write!(formatter, "history branch {id} is unknown"),
            Self::UnknownSnapshot(id) => write!(formatter, "history snapshot {id} is unknown"),
            Self::PhotoMismatch => formatter.write_str("history payload belongs to another photo"),
            Self::InvalidBranchName => formatter.write_str("history branch name is invalid"),
            Self::InvalidSnapshotName => formatter.write_str("history snapshot name is invalid"),
            Self::EmptyHistory => formatter.write_str("history has no current revision"),
            Self::NoUndo => formatter.write_str("history cannot undo from the current cursor"),
            Self::NoRedo => formatter.write_str("history cannot redo from the current cursor"),
            Self::InvalidCursor => formatter.write_str("history cursor is invalid"),
            Self::ActiveBranchDeletion => {
                formatter.write_str("the active history branch cannot be deleted")
            }
            Self::BranchHasSnapshot(id) => {
                write!(formatter, "history branch {id} has a named snapshot")
            }
            Self::BranchHasEvidence(id) => {
                write!(formatter, "history branch {id} has retained evidence")
            }
            Self::SourceBranchMismatch => {
                formatter.write_str("source cursor does not identify a valid branch revision")
            }
            Self::DuplicateEvidence => formatter.write_str("history evidence is already retained"),
            Self::MissingEvidence => formatter.write_str("history evidence is not retained"),
            Self::CannotPruneReferencedRevision(id) => {
                write!(formatter, "history revision {id} is still referenced")
            }
            Self::InvalidPersistedState => {
                formatter.write_str("persisted history state is invalid")
            }
            Self::Summary(source) => source.fmt(formatter),
        }
    }
}

impl std::error::Error for HistoryError {}

impl From<HistorySummaryError> for HistoryError {
    fn from(value: HistorySummaryError) -> Self {
        Self::Summary(value)
    }
}
