use std::fmt;

use super::canonical::{CanonicalEncodingError, CanonicalPayload};
use super::error::HistoryError;
use super::state::HistoryState;
use super::types::{
    HistoryBlobRefs, HistoryOperationKind, HistoryProvenance, HistoryRevision, HistoryRevisionId,
};
use rusttable_core::PhotoId;

const MAX_PAGE_SIZE: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryPageDirection {
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryPageRequest {
    cursor: Option<HistoryRevisionId>,
    limit: usize,
    direction: HistoryPageDirection,
}

impl HistoryPageRequest {
    #[must_use]
    pub const fn new(
        cursor: Option<HistoryRevisionId>,
        limit: usize,
        direction: HistoryPageDirection,
    ) -> Self {
        Self {
            cursor,
            limit,
            direction,
        }
    }

    #[must_use]
    pub const fn cursor(self) -> Option<HistoryRevisionId> {
        self.cursor
    }

    #[must_use]
    pub const fn limit(self) -> usize {
        self.limit
    }

    #[must_use]
    pub const fn direction(self) -> HistoryPageDirection {
        self.direction
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPage {
    revisions: Vec<HistoryRevision>,
    next_cursor: Option<HistoryRevisionId>,
    has_more: bool,
}

impl HistoryPage {
    #[must_use]
    pub fn revisions(&self) -> &[HistoryRevision] {
        &self.revisions
    }

    #[must_use]
    pub const fn next_cursor(&self) -> Option<HistoryRevisionId> {
        self.next_cursor
    }

    #[must_use]
    pub const fn has_more(&self) -> bool {
        self.has_more
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryPageError {
    InvalidLimit,
    UnknownCursor(HistoryRevisionId),
}

impl fmt::Display for HistoryPageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLimit => {
                write!(formatter, "history page size must be 1..={MAX_PAGE_SIZE}")
            }
            Self::UnknownCursor(id) => write!(formatter, "history page cursor {id} is unknown"),
        }
    }
}

impl std::error::Error for HistoryPageError {}

impl HistoryState {
    /// Returns a bounded deterministic page without exposing storage details.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid page size or unknown cursor.
    pub fn page(&self, request: HistoryPageRequest) -> Result<HistoryPage, HistoryPageError> {
        if request.limit == 0 || request.limit > MAX_PAGE_SIZE {
            return Err(HistoryPageError::InvalidLimit);
        }
        let mut revisions = self.revisions().cloned().collect::<Vec<_>>();
        if request.direction == HistoryPageDirection::Descending {
            revisions.reverse();
        }
        let start = match request.cursor {
            None => 0,
            Some(cursor) => revisions
                .iter()
                .position(|revision| revision.id() == cursor)
                .map(|index| index + 1)
                .ok_or(HistoryPageError::UnknownCursor(cursor))?,
        };
        let end = start.saturating_add(request.limit).min(revisions.len());
        let page = revisions[start..end].to_vec();
        Ok(HistoryPage {
            next_cursor: page.last().map(HistoryRevision::id),
            has_more: end < revisions.len(),
            revisions: page,
        })
    }

    /// Reconstructs one retained revision payload exactly from the immutable graph.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested revision is not retained.
    pub fn reconstruct(
        &self,
        revision: HistoryRevisionId,
    ) -> Result<HistoryRevision, HistoryError> {
        self.revision(revision)
            .cloned()
            .ok_or(HistoryError::UnknownRevision(revision))
    }

    /// Builds a privacy-safe receipt for one appended revision.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested revision is not retained or its canonical payload is
    /// invalid.
    pub fn receipt(
        &self,
        revision: HistoryRevisionId,
    ) -> Result<HistoryCommitReceipt, HistoryError> {
        let value = self
            .revision(revision)
            .ok_or(HistoryError::UnknownRevision(revision))?;
        let payload = CanonicalPayload::from_history(value.payload())?;
        let blobs = HistoryBlobRefs::new(
            payload.edit().id(),
            payload.mask_blend().id(),
            payload.pipeline().id(),
        );
        Ok(HistoryCommitReceipt::new(
            self.photo_id(),
            revision,
            value.parent(),
            self.commit_sequence(),
            value.payload().summary().kind(),
            blobs,
            self.revisions().count(),
            self.provenance(revision)
                .map_or(HistoryReceiptProvenance::Native, provenance_kind),
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryReceiptProvenance {
    Native,
    Darktable,
    RustTable,
}

fn provenance_kind(value: &HistoryProvenance) -> HistoryReceiptProvenance {
    match value {
        HistoryProvenance::Native => HistoryReceiptProvenance::Native,
        HistoryProvenance::Darktable { .. } => HistoryReceiptProvenance::Darktable,
        HistoryProvenance::RustTable { .. } => HistoryReceiptProvenance::RustTable,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryCommitReceipt {
    photo_id: PhotoId,
    revision: HistoryRevisionId,
    parent: Option<HistoryRevisionId>,
    commit_sequence: u64,
    command: HistoryOperationKind,
    blobs: HistoryBlobRefs,
    revision_count: usize,
    provenance: HistoryReceiptProvenance,
}

impl HistoryCommitReceipt {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        photo_id: PhotoId,
        revision: HistoryRevisionId,
        parent: Option<HistoryRevisionId>,
        commit_sequence: u64,
        command: HistoryOperationKind,
        blobs: HistoryBlobRefs,
        revision_count: usize,
        provenance: HistoryReceiptProvenance,
    ) -> Self {
        Self {
            photo_id,
            revision,
            parent,
            commit_sequence,
            command,
            blobs,
            revision_count,
            provenance,
        }
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }
    #[must_use]
    pub const fn revision(&self) -> HistoryRevisionId {
        self.revision
    }
    #[must_use]
    pub const fn parent(&self) -> Option<HistoryRevisionId> {
        self.parent
    }
    #[must_use]
    pub const fn commit_sequence(&self) -> u64 {
        self.commit_sequence
    }
    #[must_use]
    pub const fn command(&self) -> HistoryOperationKind {
        self.command
    }
    #[must_use]
    pub const fn blobs(&self) -> &HistoryBlobRefs {
        &self.blobs
    }
    #[must_use]
    pub const fn revision_count(&self) -> usize {
        self.revision_count
    }
    #[must_use]
    pub const fn provenance(&self) -> HistoryReceiptProvenance {
        self.provenance
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistoryInvariantReport {
    pub photo_id: PhotoId,
    pub revisions: usize,
    pub journal_entries: usize,
    pub current_revision: Option<HistoryRevisionId>,
    pub unique_blobs: usize,
}

impl From<CanonicalEncodingError> for HistoryError {
    fn from(_: CanonicalEncodingError) -> Self {
        Self::InvalidPersistedState
    }
}
