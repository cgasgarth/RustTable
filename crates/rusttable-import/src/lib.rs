#![forbid(unsafe_code)]
#![doc = "Bounded one-read source inspection and registration for `RustTable`."]

mod raster;
mod request;
mod service;
mod snapshot;

pub use raster::{
    AtomicRasterCatalog, AtomicRasterCatalogError, MAX_RASTER_IMPORT_ITEMS, RasterCatalogEntry,
    RasterImportBatch, RasterImportCancellation, RasterImportFailure, RasterImportItemId,
    RasterImportObserver, RasterImportProgress, RasterImportReceipt, RasterImportRequest,
    RasterImportRequestError, RasterImportService, RasterImportStage, RasterImportStatus,
    RasterPreviewError, RasterPreviewPort, RasterPreviewReceipt, decode_reference_source,
    encode_reference_source,
};
pub use request::SourceImportRequest;
pub use service::{SourceImportError, SourceImportService};
pub use snapshot::{
    FileSourceSnapshotReader, ImportSourceLimits, ImportSourceLimitsError, SourceReadStage,
    SourceSnapshot, SourceSnapshotError, SourceSnapshotReader,
};
