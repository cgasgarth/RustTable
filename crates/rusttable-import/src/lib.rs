#![forbid(unsafe_code)]
#![doc = "Bounded one-read source inspection and registration for `RustTable`."]

mod raster;
mod request;
mod service;
mod snapshot;
mod stable_copy;

pub use raster::{
    AtomicRasterCatalog, AtomicRasterCatalogError, MAX_RASTER_IMPORT_ITEMS,
    RASTER_DECODER_IDENTITY_VERSION, RasterCatalogEntry, RasterDuplicateIdentity,
    RasterImportBatch, RasterImportCancellation, RasterImportFailure, RasterImportItemId,
    RasterImportObserver, RasterImportProgress, RasterImportReceipt, RasterImportRequest,
    RasterImportRequestError, RasterImportService, RasterImportStage, RasterImportStatus,
    RasterPreviewError, RasterPreviewPort, RasterPreviewReceipt, decode_reference_source,
    encode_reference_source, normalize_reference_path, reference_path_identity,
    reference_source_identity,
};
pub use request::SourceImportRequest;
pub use service::{SourceImportError, SourceImportService};
pub use snapshot::{
    FileSourceSnapshotReader, ImportSourceLimits, ImportSourceLimitsError, SourceHashStatus,
    SourceIdentityClass, SourceReadStage, SourceSnapshot, SourceSnapshotError,
    SourceSnapshotReadError, SourceSnapshotReader, SourceSnapshotSequentialReader,
};
pub use stable_copy::{
    StableCopyError, StableCopyOptions, StableCopyOptionsError, StableCopyReceipt,
    StableCopyResult, StableCopyStage,
};
