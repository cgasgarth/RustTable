use super::types::{
    BranchTransferPolicy, HistoryCursor, HistoryEvidence, HistoryEvidenceKind, HistoryPayload,
    HistoryRevisionId,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryCommand {
    Append {
        payload: HistoryPayload,
    },
    Undo,
    Redo,
    Restore {
        source: HistoryRevisionId,
    },
    CreateBranch {
        name: String,
        from: Option<HistoryCursor>,
    },
    SwitchBranch {
        branch: super::types::HistoryBranchId,
    },
    Transfer {
        source: HistoryCursor,
        policy: BranchTransferPolicy,
    },
    /// Validates and records an explicit clipboard selection without changing the edit.
    Copy {
        source: HistoryCursor,
    },
    /// Applies an explicit clipboard selection as one immutable revision.
    Paste {
        source: HistoryCursor,
    },
    /// Merges two branch tips after checking operation, mask, and pipeline conflicts.
    Merge {
        source: HistoryCursor,
        target: HistoryCursor,
    },
    CreateSnapshot {
        name: String,
    },
    DeleteSnapshot {
        snapshot: super::types::HistorySnapshotId,
    },
    DeleteBranch {
        branch: super::types::HistoryBranchId,
    },
    RetainEvidence {
        revision: HistoryRevisionId,
        kind: HistoryEvidenceKind,
    },
    ReleaseEvidence {
        revision: HistoryRevisionId,
        kind: HistoryEvidenceKind,
    },
    PruneOrphans,
}

impl HistoryCommand {
    #[must_use]
    pub const fn kind(&self) -> super::types::HistoryOperationKind {
        match self {
            Self::Append { payload } => payload.summary().kind(),
            Self::Undo | Self::Redo | Self::Restore { .. } => {
                super::types::HistoryOperationKind::Reset
            }
            Self::CreateBranch { .. }
            | Self::SwitchBranch { .. }
            | Self::CreateSnapshot { .. }
            | Self::DeleteSnapshot { .. }
            | Self::DeleteBranch { .. }
            | Self::RetainEvidence { .. }
            | Self::ReleaseEvidence { .. }
            | Self::PruneOrphans
            | Self::Merge { .. } => super::types::HistoryOperationKind::Merge,
            Self::Transfer { policy, .. } => match policy {
                BranchTransferPolicy::Copy => super::types::HistoryOperationKind::Copy,
                BranchTransferPolicy::Merge => super::types::HistoryOperationKind::Merge,
            },
            Self::Copy { .. } | Self::Paste { .. } => super::types::HistoryOperationKind::Copy,
        }
    }

    #[must_use]
    pub fn evidence(revision: HistoryRevisionId, kind: HistoryEvidenceKind) -> HistoryEvidence {
        HistoryEvidence::new(revision, kind)
    }
}
