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
    pub fn evidence(revision: HistoryRevisionId, kind: HistoryEvidenceKind) -> HistoryEvidence {
        HistoryEvidence::new(revision, kind)
    }
}
