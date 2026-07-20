use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rusttable_catalog::{ImportRecord, ImportRegistration};
use rusttable_core::{AssetId, Edit, EditId, PhotoId};
use rusttable_image::{ImageProbe, InputFormat};

pub const MAX_RASTER_IMPORT_ITEMS: usize = 256;
pub const RASTER_DECODER_IDENTITY_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterDuplicateIdentity {
    pub content_sha256: [u8; 32],
    pub byte_length: u64,
    pub decoder_identity_version: u8,
    pub probe: ImageProbe,
    pub source_identity: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RasterImportItemId(NonZeroU64);

impl RasterImportItemId {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterImportRequest {
    items: Vec<(RasterImportItemId, PathBuf)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterImportRequestError {
    Empty,
    TooMany { maximum: usize, actual: usize },
    IndexOverflow,
}

impl RasterImportRequest {
    /// Creates one bounded ordered multi-file request.
    ///
    /// # Errors
    ///
    /// Returns a typed error for an empty or oversized selection.
    pub fn new<I, P>(paths: I) -> Result<Self, RasterImportRequestError>
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        let paths = paths.into_iter().map(Into::into).collect::<Vec<_>>();
        if paths.is_empty() {
            return Err(RasterImportRequestError::Empty);
        }
        if paths.len() > MAX_RASTER_IMPORT_ITEMS {
            return Err(RasterImportRequestError::TooMany {
                maximum: MAX_RASTER_IMPORT_ITEMS,
                actual: paths.len(),
            });
        }
        let items = paths
            .into_iter()
            .enumerate()
            .map(|(index, path)| {
                let value = u64::try_from(index)
                    .map_err(|_| RasterImportRequestError::IndexOverflow)?
                    .checked_add(1)
                    .and_then(NonZeroU64::new)
                    .ok_or(RasterImportRequestError::IndexOverflow)?;
                Ok((RasterImportItemId(value), path))
            })
            .collect::<Result<Vec<_>, RasterImportRequestError>>()?;
        Ok(Self { items })
    }

    #[must_use]
    pub fn items(&self) -> impl ExactSizeIterator<Item = (RasterImportItemId, &Path)> {
        self.items.iter().map(|(id, path)| (*id, path.as_path()))
    }
}

#[derive(Debug, Clone, Default)]
pub struct RasterImportCancellation(Arc<AtomicBool>);

impl PartialEq for RasterImportCancellation {
    fn eq(&self, other: &Self) -> bool {
        self.is_cancelled() == other.is_cancelled()
    }
}

impl Eq for RasterImportCancellation {}

impl RasterImportCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterImportStage {
    Queued,
    Opening,
    Hashing,
    Probing,
    DecodingHeader,
    Registering,
    GeneratingPreview,
    Completed,
    AlreadyImported,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterImportProgress {
    pub item_id: RasterImportItemId,
    pub stage: RasterImportStage,
}

pub trait RasterImportObserver: Send + Sync {
    fn progress(&self, progress: RasterImportProgress);
}

impl<F> RasterImportObserver for F
where
    F: Fn(RasterImportProgress) + Send + Sync,
{
    fn progress(&self, progress: RasterImportProgress) {
        self(progress);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterImportFailure {
    SourceUnavailable,
    NonRegularSource,
    SymlinkRejected,
    SourceChanged,
    SourceTooLarge,
    UnsupportedOrMalformedRaster,
    MetadataInvalid,
    UnsupportedPathEncoding,
    CatalogUnavailable,
    CatalogConflict,
    CatalogCorrupt,
    CatalogCommitFailed,
    PreviewFailed,
    InternalInvariant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterImportStatus {
    Imported,
    AlreadyImported,
    ImportedPreviewPending,
    ImportedPreviewFailed,
    Failed(RasterImportFailure),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterPreviewReceipt {
    pub width: u32,
    pub height: u32,
    pub pixel_sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterImportReceipt {
    pub schema_version: u8,
    pub item_id: RasterImportItemId,
    pub source_alias: String,
    pub content_sha256: Option<[u8; 32]>,
    pub format: Option<InputFormat>,
    pub photo_id: Option<PhotoId>,
    pub asset_id: Option<AssetId>,
    pub edit_id: Option<EditId>,
    pub status: RasterImportStatus,
    pub preview: Option<RasterPreviewReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterImportBatch {
    receipts: Vec<RasterImportReceipt>,
}

impl RasterImportBatch {
    #[must_use]
    pub fn new(receipts: Vec<RasterImportReceipt>) -> Self {
        Self { receipts }
    }

    #[must_use]
    pub fn receipts(&self) -> impl ExactSizeIterator<Item = &RasterImportReceipt> {
        self.receipts.iter()
    }

    #[must_use]
    pub fn first_selected_photo(&self) -> Option<PhotoId> {
        self.receipts
            .iter()
            .find_map(|receipt| match receipt.status {
                RasterImportStatus::Imported
                | RasterImportStatus::AlreadyImported
                | RasterImportStatus::ImportedPreviewPending
                | RasterImportStatus::ImportedPreviewFailed => receipt.photo_id,
                RasterImportStatus::Failed(_) | RasterImportStatus::Cancelled => None,
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterCatalogEntry {
    pub record: ImportRecord,
    pub edit: Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicRasterCatalogError {
    Unavailable,
    Conflict,
    Corrupt,
    CommitFailed,
}

pub trait AtomicRasterCatalog {
    /// Finds one exact-content import and its current edit.
    ///
    /// # Errors
    ///
    /// Returns a typed storage failure.
    fn find_by_content(
        &self,
        identity: RasterDuplicateIdentity,
    ) -> Result<Option<RasterCatalogEntry>, AtomicRasterCatalogError>;

    /// Atomically persists the source, photo, and default edit.
    ///
    /// # Errors
    ///
    /// Returns a typed failure without publishing a partial entry.
    fn commit_import(
        &mut self,
        entry: &RasterCatalogEntry,
        registration: &ImportRegistration,
    ) -> Result<(), AtomicRasterCatalogError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterPreviewError {
    Unavailable,
    SourceChanged,
    Decode,
    Render,
}

pub trait RasterPreviewPort {
    /// Generates one bounded thumbnail from the persisted current edit.
    ///
    /// # Errors
    ///
    /// Returns a typed source, decode, render, or availability failure.
    fn generate_thumbnail(
        &self,
        entry: &RasterCatalogEntry,
    ) -> Result<RasterPreviewReceipt, RasterPreviewError>;
}
