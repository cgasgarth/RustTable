#![forbid(unsafe_code)]
#![doc = "Catalog state and use cases for `RustTable`."]
#![doc = "Normal dependencies flow from this crate to `rusttable-core` and `rusttable-image`; codecs, UI, and processing stay outside it."]

/// Identifies the catalog crate's current dependency boundary.
#[must_use]
pub const fn dependency_direction() -> &'static str {
    "rusttable-catalog -> rusttable-core"
}
mod command;
mod develop;
mod durable_edit;
mod edit_repository;
mod error;
mod import;
mod repository;
mod restore;
mod snapshot;
mod source_path;
mod state;

pub use command::CatalogCommand;
pub use develop::{DevelopInput, DevelopInputError, DevelopSelection};
pub use durable_edit::{DurableEditError, DurableEditOutcome, DurableEditService};
pub use edit_repository::{EditRepository, EditRepositoryError};
pub use error::CatalogError;
pub use import::{
    ImportCandidate, ImportCandidateError, ImportError, ImportOutcome, ImportRecord,
    ImportRecordError, ImportService,
};
pub use repository::{ImportRepository, RepositoryError};
pub use restore::CatalogRestoreError;
pub use snapshot::{CatalogEntry, CatalogSnapshot, CatalogSnapshotError};
pub use source_path::{SourcePath, SourcePathError};
pub use state::CatalogState;
