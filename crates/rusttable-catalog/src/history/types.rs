use std::fmt;
use std::num::NonZeroU64;

use rusttable_core::{Edit, OperationId, OperationKey, PhotoId};

use super::canonical::ContentBlobId;

macro_rules! define_history_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU64);

        impl $name {
            #[must_use]
            pub const fn new(value: u64) -> Option<Self> {
                match NonZeroU64::new(value) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            #[must_use]
            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.get().fmt(formatter)
            }
        }
    };
}

define_history_id!(HistoryRevisionId);
define_history_id!(HistoryBranchId);
define_history_id!(HistorySnapshotId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryVersion(u64);

impl HistoryVersion {
    pub const ZERO: Self = Self(0);

    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

impl fmt::Display for HistoryVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryCursor {
    branch: HistoryBranchId,
    revision: Option<HistoryRevisionId>,
}

/// A stable before/after selection over immutable history cursors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryComparisonPair {
    before: HistoryCursor,
    after: HistoryCursor,
}

impl HistoryComparisonPair {
    #[must_use]
    pub const fn new(before: HistoryCursor, after: HistoryCursor) -> Self {
        Self { before, after }
    }

    #[must_use]
    pub const fn before(self) -> HistoryCursor {
        self.before
    }

    #[must_use]
    pub const fn after(self) -> HistoryCursor {
        self.after
    }
}

impl HistoryCursor {
    #[must_use]
    pub const fn new(branch: HistoryBranchId, revision: Option<HistoryRevisionId>) -> Self {
        Self { branch, revision }
    }

    #[must_use]
    pub const fn branch(self) -> HistoryBranchId {
        self.branch
    }

    #[must_use]
    pub const fn revision(self) -> Option<HistoryRevisionId> {
        self.revision
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HistoryOperationKind {
    Parameter,
    Order,
    Enable,
    Mask,
    Blend,
    Style,
    Copy,
    Paste,
    Reset,
    Merge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryOperationSummary {
    kind: HistoryOperationKind,
    operation_id: Option<OperationId>,
    operation_key: Option<OperationKey>,
    label: String,
}

impl HistoryOperationSummary {
    /// Creates a bounded, deterministic operation-level description.
    ///
    /// The summary is metadata only; the immutable edit and pipeline payloads
    /// remain authoritative for reconstruction.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid label or incomplete operation identity.
    pub fn new(
        kind: HistoryOperationKind,
        operation_id: Option<OperationId>,
        operation_key: Option<OperationKey>,
        label: impl Into<String>,
    ) -> Result<Self, HistorySummaryError> {
        let label = label.into();
        if label.is_empty() || label.len() > 256 || label.chars().any(char::is_control) {
            return Err(HistorySummaryError::InvalidLabel);
        }
        if operation_id.is_none() != operation_key.is_none() {
            return Err(HistorySummaryError::IncompleteOperationIdentity);
        }
        Ok(Self {
            kind,
            operation_id,
            operation_key,
            label,
        })
    }

    #[must_use]
    pub const fn kind(&self) -> HistoryOperationKind {
        self.kind
    }

    #[must_use]
    pub const fn operation_id(&self) -> Option<OperationId> {
        self.operation_id
    }

    #[must_use]
    pub fn operation_key(&self) -> Option<&OperationKey> {
        self.operation_key.as_ref()
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistorySummaryError {
    InvalidLabel,
    IncompleteOperationIdentity,
}

impl fmt::Display for HistorySummaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLabel => formatter.write_str("history summary label is invalid"),
            Self::IncompleteOperationIdentity => {
                formatter.write_str("operation ID and key must be supplied together")
            }
        }
    }
}

impl std::error::Error for HistorySummaryError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPayload {
    edit: Edit,
    mask_bytes: Vec<u8>,
    pipeline_bytes: Vec<u8>,
    summary: HistoryOperationSummary,
}

impl HistoryPayload {
    #[must_use]
    pub fn new(
        edit: Edit,
        mask_bytes: impl Into<Vec<u8>>,
        pipeline_bytes: impl Into<Vec<u8>>,
        summary: HistoryOperationSummary,
    ) -> Self {
        Self {
            edit,
            mask_bytes: mask_bytes.into(),
            pipeline_bytes: pipeline_bytes.into(),
            summary,
        }
    }

    #[must_use]
    pub const fn edit(&self) -> &Edit {
        &self.edit
    }

    #[must_use]
    pub fn mask_bytes(&self) -> &[u8] {
        &self.mask_bytes
    }

    #[must_use]
    pub fn pipeline_bytes(&self) -> &[u8] {
        &self.pipeline_bytes
    }

    #[must_use]
    pub const fn summary(&self) -> &HistoryOperationSummary {
        &self.summary
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRevision {
    id: HistoryRevisionId,
    parent: Option<HistoryRevisionId>,
    payload: HistoryPayload,
}

impl HistoryRevision {
    #[must_use]
    pub const fn new(
        id: HistoryRevisionId,
        parent: Option<HistoryRevisionId>,
        payload: HistoryPayload,
    ) -> Self {
        Self {
            id,
            parent,
            payload,
        }
    }

    #[must_use]
    pub const fn id(&self) -> HistoryRevisionId {
        self.id
    }

    #[must_use]
    pub const fn parent(&self) -> Option<HistoryRevisionId> {
        self.parent
    }

    #[must_use]
    pub const fn payload(&self) -> &HistoryPayload {
        &self.payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryBranch {
    pub(crate) id: HistoryBranchId,
    pub(crate) name: String,
    pub(crate) origin: Option<HistoryRevisionId>,
    pub(crate) lineage: Vec<HistoryRevisionId>,
    pub(crate) cursor: Option<HistoryRevisionId>,
    pub(crate) redo: Vec<HistoryRevisionId>,
}

impl HistoryBranch {
    #[must_use]
    pub const fn new(
        id: HistoryBranchId,
        name: String,
        origin: Option<HistoryRevisionId>,
        lineage: Vec<HistoryRevisionId>,
        cursor: Option<HistoryRevisionId>,
    ) -> Self {
        Self {
            id,
            name,
            origin,
            lineage,
            cursor,
            redo: Vec::new(),
        }
    }

    #[must_use]
    pub const fn from_parts(
        id: HistoryBranchId,
        name: String,
        origin: Option<HistoryRevisionId>,
        lineage: Vec<HistoryRevisionId>,
        cursor: Option<HistoryRevisionId>,
        redo: Vec<HistoryRevisionId>,
    ) -> Self {
        Self {
            id,
            name,
            origin,
            lineage,
            cursor,
            redo,
        }
    }

    #[must_use]
    pub const fn id(&self) -> HistoryBranchId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn origin(&self) -> Option<HistoryRevisionId> {
        self.origin
    }

    #[must_use]
    pub fn lineage(&self) -> &[HistoryRevisionId] {
        &self.lineage
    }

    #[must_use]
    pub const fn cursor(&self) -> Option<HistoryRevisionId> {
        self.cursor
    }

    #[must_use]
    pub fn redo(&self) -> &[HistoryRevisionId] {
        &self.redo
    }

    #[must_use]
    pub fn head(&self) -> Option<HistoryRevisionId> {
        self.lineage.last().copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySnapshot {
    id: HistorySnapshotId,
    name: String,
    cursor: HistoryCursor,
}

impl HistorySnapshot {
    #[must_use]
    pub const fn new(id: HistorySnapshotId, name: String, cursor: HistoryCursor) -> Self {
        Self { id, name, cursor }
    }

    #[must_use]
    pub const fn from_parts(id: HistorySnapshotId, name: String, cursor: HistoryCursor) -> Self {
        Self { id, name, cursor }
    }

    #[must_use]
    pub const fn id(&self) -> HistorySnapshotId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn cursor(&self) -> HistoryCursor {
        self.cursor
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HistoryEvidenceKind {
    Export,
    Migration,
    Import,
    Redo,
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryEvidence {
    revision: HistoryRevisionId,
    kind: HistoryEvidenceKind,
}

impl HistoryEvidence {
    #[must_use]
    pub const fn new(revision: HistoryRevisionId, kind: HistoryEvidenceKind) -> Self {
        Self { revision, kind }
    }

    #[must_use]
    pub const fn revision(self) -> HistoryRevisionId {
        self.revision
    }

    #[must_use]
    pub const fn kind(self) -> HistoryEvidenceKind {
        self.kind
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HistoryProvenance {
    Native,
    Darktable { schema: u32, source_id: String },
    RustTable { schema: u32, source_id: String },
}

impl HistoryProvenance {
    #[must_use]
    pub const fn native() -> Self {
        Self::Native
    }

    #[must_use]
    pub fn darktable(schema: u32, source_id: impl Into<String>) -> Self {
        Self::Darktable {
            schema,
            source_id: source_id.into(),
        }
    }

    #[must_use]
    pub fn rusttable(schema: u32, source_id: impl Into<String>) -> Self {
        Self::RustTable {
            schema,
            source_id: source_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryImportSource {
    Darktable { schema: u32, source_id: String },
    RustTable { schema: u32, source_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryImportEntry {
    payload: HistoryPayload,
    source: HistoryImportSource,
    is_redo: bool,
    restore_from: Option<usize>,
}

impl HistoryImportEntry {
    #[must_use]
    pub fn new(
        payload: HistoryPayload,
        source: HistoryImportSource,
        is_redo: bool,
        restore_from: Option<usize>,
    ) -> Self {
        Self {
            payload,
            source,
            is_redo,
            restore_from,
        }
    }

    #[must_use]
    pub const fn payload(&self) -> &HistoryPayload {
        &self.payload
    }

    #[must_use]
    pub const fn is_redo(&self) -> bool {
        self.is_redo
    }

    #[must_use]
    pub const fn restore_from(&self) -> Option<usize> {
        self.restore_from
    }

    #[must_use]
    pub fn source(&self) -> &HistoryImportSource {
        &self.source
    }

    #[must_use]
    pub fn provenance(&self) -> HistoryProvenance {
        match &self.source {
            HistoryImportSource::Darktable { schema, source_id } => {
                HistoryProvenance::darktable(*schema, source_id.clone())
            }
            HistoryImportSource::RustTable { schema, source_id } => {
                HistoryProvenance::rusttable(*schema, source_id.clone())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryExecutionStatus {
    Executable,
    Opaque { reason: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryBlobRefs {
    edit: ContentBlobId,
    mask_blend: ContentBlobId,
    pipeline: ContentBlobId,
}

impl HistoryBlobRefs {
    #[must_use]
    pub const fn new(
        edit: ContentBlobId,
        mask_blend: ContentBlobId,
        pipeline: ContentBlobId,
    ) -> Self {
        Self {
            edit,
            mask_blend,
            pipeline,
        }
    }

    #[must_use]
    pub const fn edit(&self) -> ContentBlobId {
        self.edit
    }

    #[must_use]
    pub const fn mask_blend(&self) -> ContentBlobId {
        self.mask_blend
    }

    #[must_use]
    pub const fn pipeline(&self) -> ContentBlobId {
        self.pipeline
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryJournalEntry {
    sequence: u64,
    kind: HistoryOperationKind,
    revision: Option<HistoryRevisionId>,
    before: HistoryCursor,
    after: HistoryCursor,
    restore_from: Option<HistoryRevisionId>,
    provenance: HistoryProvenance,
}

impl HistoryJournalEntry {
    #[must_use]
    pub const fn new(
        sequence: u64,
        kind: HistoryOperationKind,
        revision: Option<HistoryRevisionId>,
        before: HistoryCursor,
        after: HistoryCursor,
        restore_from: Option<HistoryRevisionId>,
        provenance: HistoryProvenance,
    ) -> Self {
        Self {
            sequence,
            kind,
            revision,
            before,
            after,
            restore_from,
            provenance,
        }
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub const fn kind(&self) -> HistoryOperationKind {
        self.kind
    }

    #[must_use]
    pub const fn revision(&self) -> Option<HistoryRevisionId> {
        self.revision
    }

    #[must_use]
    pub const fn before(&self) -> HistoryCursor {
        self.before
    }

    #[must_use]
    pub const fn after(&self) -> HistoryCursor {
        self.after
    }

    #[must_use]
    pub const fn restore_from(&self) -> Option<HistoryRevisionId> {
        self.restore_from
    }

    #[must_use]
    pub fn provenance(&self) -> &HistoryProvenance {
        &self.provenance
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BranchTransferPolicy {
    Copy,
    Merge,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryStateSnapshot {
    pub(crate) photo_id: PhotoId,
    pub(crate) version: HistoryVersion,
    pub(crate) commit_sequence: u64,
    pub(crate) next_revision_id: u64,
    pub(crate) next_branch_id: u64,
    pub(crate) next_snapshot_id: u64,
    pub(crate) active_branch: HistoryBranchId,
    pub(crate) revisions: Vec<HistoryRevision>,
    pub(crate) branches: Vec<HistoryBranch>,
    pub(crate) snapshots: Vec<HistorySnapshot>,
    pub(crate) evidence: Vec<HistoryEvidence>,
    pub(crate) journal: Vec<HistoryJournalEntry>,
    pub(crate) provenance: std::collections::BTreeMap<HistoryRevisionId, HistoryProvenance>,
}

impl HistoryStateSnapshot {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn from_parts(
        photo_id: PhotoId,
        version: HistoryVersion,
        next_revision_id: u64,
        next_branch_id: u64,
        next_snapshot_id: u64,
        active_branch: HistoryBranchId,
        revisions: Vec<HistoryRevision>,
        branches: Vec<HistoryBranch>,
        snapshots: Vec<HistorySnapshot>,
        evidence: Vec<HistoryEvidence>,
    ) -> Self {
        Self::from_parts_with_journal(
            photo_id,
            version,
            version.get(),
            next_revision_id,
            next_branch_id,
            next_snapshot_id,
            active_branch,
            revisions,
            branches,
            snapshots,
            evidence,
            Vec::new(),
            std::collections::BTreeMap::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub const fn from_parts_with_journal(
        photo_id: PhotoId,
        version: HistoryVersion,
        commit_sequence: u64,
        next_revision_id: u64,
        next_branch_id: u64,
        next_snapshot_id: u64,
        active_branch: HistoryBranchId,
        revisions: Vec<HistoryRevision>,
        branches: Vec<HistoryBranch>,
        snapshots: Vec<HistorySnapshot>,
        evidence: Vec<HistoryEvidence>,
        journal: Vec<HistoryJournalEntry>,
        provenance: std::collections::BTreeMap<HistoryRevisionId, HistoryProvenance>,
    ) -> Self {
        Self {
            photo_id,
            version,
            commit_sequence,
            next_revision_id,
            next_branch_id,
            next_snapshot_id,
            active_branch,
            revisions,
            branches,
            snapshots,
            evidence,
            journal,
            provenance,
        }
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn version(&self) -> HistoryVersion {
        self.version
    }

    #[must_use]
    pub const fn commit_sequence(&self) -> u64 {
        self.commit_sequence
    }

    #[must_use]
    pub const fn next_revision_id(&self) -> u64 {
        self.next_revision_id
    }

    #[must_use]
    pub const fn next_branch_id(&self) -> u64 {
        self.next_branch_id
    }

    #[must_use]
    pub const fn next_snapshot_id(&self) -> u64 {
        self.next_snapshot_id
    }

    #[must_use]
    pub const fn active_branch(&self) -> HistoryBranchId {
        self.active_branch
    }

    #[must_use]
    pub fn revisions(&self) -> &[HistoryRevision] {
        &self.revisions
    }

    #[must_use]
    pub fn branches(&self) -> &[HistoryBranch] {
        &self.branches
    }

    #[must_use]
    pub fn snapshots(&self) -> &[HistorySnapshot] {
        &self.snapshots
    }

    #[must_use]
    pub fn evidence(&self) -> &[HistoryEvidence] {
        &self.evidence
    }

    #[must_use]
    pub fn journal(&self) -> &[HistoryJournalEntry] {
        &self.journal
    }

    #[must_use]
    pub fn provenance(&self) -> &std::collections::BTreeMap<HistoryRevisionId, HistoryProvenance> {
        &self.provenance
    }
}

pub(crate) fn validate_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && !name.chars().any(char::is_control)
}
