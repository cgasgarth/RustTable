#![forbid(unsafe_code)]
#![doc = "Catalog state and use cases for `RustTable`."]
#![doc = "Normal dependencies flow from this crate to `rusttable-core` and `rusttable-image`; codecs, UI, and processing stay outside it."]

/// Identifies the catalog crate's current dependency boundary.
#[must_use]
pub const fn dependency_direction() -> &'static str {
    "rusttable-catalog -> rusttable-core"
}
mod collections;
mod command;
mod develop;
mod durable_edit;
mod edit_repository;
mod error;
mod history;
mod import;
mod import_details;
mod organization;
mod repository;
mod restore;
mod snapshot;
mod source_path;
mod state;

pub use collections::{
    ActiveLibraryView, CollectionCommand, CollectionError, CollectionField, CollectionId,
    CollectionProvenance, CollectionQuery, CollectionRepository, CollectionRepositoryError,
    CollectionSort, CollectionState, CollectionValidationError, CollectionViewDefinition,
    GroupCollapsePolicy, MAX_RECENT_QUERIES, RecentQuery, SavedCollection,
};
pub use command::{CatalogChangeEvent, CatalogCommand};
pub use develop::{DevelopInput, DevelopInputError, DevelopSelection};
pub use durable_edit::{DurableEditError, DurableEditOutcome, DurableEditService};
pub use edit_repository::{EditRepository, EditRepositoryError};
pub use error::CatalogError;
pub use history::{
    BranchTransferPolicy, CanonicalBlob, CanonicalEncodingError, CanonicalHistoryCommand,
    CanonicalPayload, ContentBlobId, ContentBlobKind, DurableHistoryError, DurableHistoryService,
    HistoryApplyOutcome, HistoryBlobRefs, HistoryBranch, HistoryBranchId, HistoryCommand,
    HistoryCommitReceipt, HistoryComparisonPair, HistoryCursor, HistoryError, HistoryEvidence,
    HistoryEvidenceKind, HistoryExecutionStatus, HistoryImport, HistoryImportEntry,
    HistoryImportError, HistoryImportSource, HistoryInvariantReport, HistoryJournalEntry,
    HistoryOperationKind, HistoryOperationSummary, HistoryPage, HistoryPageDirection,
    HistoryPageError, HistoryPageRequest, HistoryPayload, HistoryProvenance,
    HistoryReceiptProvenance, HistoryRepository, HistoryRepositoryError, HistoryRevision,
    HistoryRevisionId, HistorySnapshot, HistorySnapshotId, HistoryState, HistoryStateSnapshot,
    HistorySummaryError, HistoryVersion, canonical_edit_bytes, canonical_mask_blend_bytes,
    canonical_pipeline_bytes,
};
pub use import::{
    ImportCandidate, ImportCandidateError, ImportError, ImportOutcome, ImportRecord,
    ImportRecordError, ImportService,
};
pub use import_details::{
    IMPORT_DETAILS_VERSION, ImportDetails, ImportDetailsValidationError, ImportMetadataSummary,
    ImportRegistration, ImportRegistrationReceipt, ImportRegistrationReceiptError,
    ImportRegistrationStatus, ReferencePathIdentity,
};
pub use organization::{
    CatalogQuery, ColorLabel, OrganizationProjection, PhotoOrganizationState, Rating,
};
pub use repository::{ImportRepository, RepositoryError};
pub use restore::CatalogRestoreError;
pub use snapshot::{CatalogEntry, CatalogSnapshot, CatalogSnapshotError};
pub use source_path::{SourcePath, SourcePathError};
pub use state::CatalogState;
