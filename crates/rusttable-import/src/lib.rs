#![forbid(unsafe_code)]
#![doc = "Bounded one-read source inspection and registration for `RustTable`."]

mod request;
mod service;
mod snapshot;

pub use request::SourceImportRequest;
pub use service::{SourceImportError, SourceImportService};
pub use snapshot::{
    FileSourceSnapshotReader, ImportSourceLimits, ImportSourceLimitsError, SourceReadStage,
    SourceSnapshot, SourceSnapshotError, SourceSnapshotReader,
};
