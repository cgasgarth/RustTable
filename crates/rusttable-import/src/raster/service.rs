use std::path::Path;

use rusttable_catalog::{ImportCandidate, ImportRecord};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, FiniteF64, Operation,
    OperationId, OperationKey, ParameterName, ParameterValue, Photo, PhotoId, Revision,
};
use rusttable_image::{ImageInput, InputFormat};
use rusttable_metadata::MetadataInput;
use sha2::{Digest, Sha256};

use super::model::{
    AtomicRasterCatalog, AtomicRasterCatalogError, RasterCatalogEntry, RasterImportBatch,
    RasterImportCancellation, RasterImportFailure, RasterImportItemId, RasterImportObserver,
    RasterImportProgress, RasterImportReceipt, RasterImportRequest, RasterImportStage,
    RasterImportStatus, RasterPreviewPort,
};
use super::reference::{ReferenceSourceError, encode_reference_source};
use crate::{ImportSourceLimits, SourceSnapshot, SourceSnapshotError, SourceSnapshotReader};

pub struct RasterImportService<'a> {
    source_limits: ImportSourceLimits,
    snapshot_reader: &'a dyn SourceSnapshotReader,
    image_input: &'a dyn ImageInput,
    metadata_input: &'a dyn MetadataInput,
}

impl<'a> RasterImportService<'a> {
    #[must_use]
    pub const fn new(
        source_limits: ImportSourceLimits,
        snapshot_reader: &'a dyn SourceSnapshotReader,
        image_input: &'a dyn ImageInput,
        metadata_input: &'a dyn MetadataInput,
    ) -> Self {
        Self {
            source_limits,
            snapshot_reader,
            image_input,
            metadata_input,
        }
    }

    pub fn import(
        &self,
        request: &RasterImportRequest,
        catalog: &mut dyn AtomicRasterCatalog,
        preview: &dyn RasterPreviewPort,
        cancellation: &RasterImportCancellation,
        observer: &dyn RasterImportObserver,
    ) -> RasterImportBatch {
        for (item_id, _) in request.items() {
            report(observer, item_id, RasterImportStage::Queued);
        }
        let receipts = request
            .items()
            .map(|(item_id, path)| {
                self.import_one(item_id, path, catalog, preview, cancellation, observer)
            })
            .collect();
        RasterImportBatch::new(receipts)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the ordered item transaction keeps every cancellation and failure boundary visible"
    )]
    fn import_one(
        &self,
        item_id: RasterImportItemId,
        path: &Path,
        catalog: &mut dyn AtomicRasterCatalog,
        preview: &dyn RasterPreviewPort,
        cancellation: &RasterImportCancellation,
        observer: &dyn RasterImportObserver,
    ) -> RasterImportReceipt {
        let alias = safe_alias(path);
        if cancellation.is_cancelled() {
            report(observer, item_id, RasterImportStage::Cancelled);
            return receipt(item_id, alias, RasterImportStatus::Cancelled);
        }
        report(observer, item_id, RasterImportStage::Opening);
        let snapshot = match self.snapshot_reader.read_snapshot(path, self.source_limits) {
            Ok(snapshot) => snapshot,
            Err(error) => return failed(item_id, alias, map_snapshot_error(&error), observer),
        };
        report(observer, item_id, RasterImportStage::Hashing);
        let hash = sha256(snapshot.bytes());
        report(observer, item_id, RasterImportStage::Probing);
        let Ok(probe) = self.image_input.probe_bytes(snapshot.bytes()) else {
            return failed_with_evidence(
                item_id,
                alias,
                hash,
                None,
                RasterImportFailure::UnsupportedOrMalformedRaster,
                observer,
            );
        };
        report(observer, item_id, RasterImportStage::DecodingHeader);
        let Ok(metadata) = self
            .metadata_input
            .read_bytes(probe.format(), snapshot.bytes())
        else {
            return failed_with_evidence(
                item_id,
                alias,
                hash,
                Some(probe.format()),
                RasterImportFailure::MetadataInvalid,
                observer,
            );
        };
        if cancellation.is_cancelled() {
            report(observer, item_id, RasterImportStage::Cancelled);
            return evidence_receipt(
                item_id,
                alias,
                hash,
                probe.format(),
                RasterImportStatus::Cancelled,
            );
        }
        if let Err(error) = self
            .snapshot_reader
            .revalidate(&snapshot, self.source_limits)
        {
            return failed_with_evidence(
                item_id,
                alias,
                hash,
                Some(probe.format()),
                map_snapshot_error(&error),
                observer,
            );
        }
        report(observer, item_id, RasterImportStage::Registering);
        let byte_length = snapshot.byte_length().get();
        let existing = match catalog.find_by_content(hash, byte_length) {
            Ok(existing) => existing,
            Err(error) => {
                return failed_with_evidence(
                    item_id,
                    alias,
                    hash,
                    Some(probe.format()),
                    map_catalog_error(error),
                    observer,
                );
            }
        };
        let (entry, imported) = if let Some(entry) = existing {
            (entry, false)
        } else {
            let entry = match build_entry(path, hash, &snapshot, probe, metadata) {
                Ok(entry) => entry,
                Err(error) => {
                    return failed_with_evidence(
                        item_id,
                        alias,
                        hash,
                        Some(probe.format()),
                        error,
                        observer,
                    );
                }
            };
            if cancellation.is_cancelled() {
                report(observer, item_id, RasterImportStage::Cancelled);
                return evidence_receipt(
                    item_id,
                    alias,
                    hash,
                    probe.format(),
                    RasterImportStatus::Cancelled,
                );
            }
            if let Err(error) = catalog.commit_import(&entry) {
                return failed_with_evidence(
                    item_id,
                    alias,
                    hash,
                    Some(probe.format()),
                    map_catalog_error(error),
                    observer,
                );
            }
            (entry, true)
        };
        let mut result = evidence_receipt(
            item_id,
            alias,
            hash,
            probe.format(),
            if imported {
                RasterImportStatus::Imported
            } else {
                RasterImportStatus::AlreadyImported
            },
        );
        result.photo_id = Some(entry.record.photo().id());
        result.asset_id = Some(entry.record.photo().primary_asset_id());
        result.edit_id = Some(entry.edit.id());
        if cancellation.is_cancelled() {
            result.status = RasterImportStatus::ImportedPreviewPending;
            report(observer, item_id, RasterImportStage::Completed);
            return result;
        }
        report(observer, item_id, RasterImportStage::GeneratingPreview);
        match preview.generate_thumbnail(&entry) {
            Ok(preview) => result.preview = Some(preview),
            Err(_) => result.status = RasterImportStatus::ImportedPreviewFailed,
        }
        report(
            observer,
            item_id,
            if imported {
                RasterImportStage::Completed
            } else {
                RasterImportStage::AlreadyImported
            },
        );
        result
    }
}

fn build_entry(
    path: &Path,
    hash: [u8; 32],
    snapshot: &SourceSnapshot,
    probe: rusttable_image::ImageProbe,
    metadata: rusttable_core::ImageMetadata,
) -> Result<RasterCatalogEntry, RasterImportFailure> {
    let photo_id =
        PhotoId::new(derived_id(b"photo", hash)).ok_or(RasterImportFailure::InternalInvariant)?;
    let asset_id =
        AssetId::new(derived_id(b"asset", hash)).ok_or(RasterImportFailure::InternalInvariant)?;
    let edit_id =
        EditId::new(derived_id(b"edit", hash)).ok_or(RasterImportFailure::InternalInvariant)?;
    let source = encode_reference_source(path, hash).map_err(map_reference_error)?;
    let candidate = ImportCandidate::new(
        photo_id,
        asset_id,
        source,
        ContentHash::Sha256(hash),
        ByteLength::from_bytes(snapshot.byte_length().get()),
        probe,
        metadata,
    )
    .map_err(|_| RasterImportFailure::InternalInvariant)?;
    let asset = Asset::new(
        asset_id,
        AssetRole::Primary,
        ContentHash::Sha256(hash),
        candidate.byte_length(),
    );
    let photo =
        Photo::new(photo_id, [asset]).map_err(|_| RasterImportFailure::InternalInvariant)?;
    let record =
        ImportRecord::new(&candidate, photo).map_err(|_| RasterImportFailure::InternalInvariant)?;
    let edit = neutral_edit(edit_id, photo_id, hash)?;
    Ok(RasterCatalogEntry { record, edit })
}

fn neutral_edit(
    edit_id: EditId,
    photo_id: PhotoId,
    hash: [u8; 32],
) -> Result<Edit, RasterImportFailure> {
    let exposure = Operation::new(
        operation_id(b"exposure", hash)?,
        OperationKey::new("rusttable.exposure")
            .map_err(|_| RasterImportFailure::InternalInvariant)?,
        true,
        [(parameter("stops")?, scalar(0.0)?)],
    )
    .map_err(|_| RasterImportFailure::InternalInvariant)?;
    let rgb_gain = Operation::new(
        operation_id(b"rgb-gain", hash)?,
        OperationKey::new("rusttable.rgb_gain")
            .map_err(|_| RasterImportFailure::InternalInvariant)?,
        true,
        [
            (parameter("red")?, scalar(1.0)?),
            (parameter("green")?, scalar(1.0)?),
            (parameter("blue")?, scalar(1.0)?),
        ],
    )
    .map_err(|_| RasterImportFailure::InternalInvariant)?;
    Edit::new(edit_id, photo_id, Revision::ZERO, [exposure, rgb_gain])
        .map_err(|_| RasterImportFailure::InternalInvariant)
}

fn operation_id(domain: &[u8], hash: [u8; 32]) -> Result<OperationId, RasterImportFailure> {
    OperationId::new(derived_id(domain, hash)).ok_or(RasterImportFailure::InternalInvariant)
}

fn parameter(name: &str) -> Result<ParameterName, RasterImportFailure> {
    ParameterName::new(name).map_err(|_| RasterImportFailure::InternalInvariant)
}

fn scalar(value: f64) -> Result<ParameterValue, RasterImportFailure> {
    FiniteF64::new(value)
        .map(ParameterValue::Scalar)
        .map_err(|_| RasterImportFailure::InternalInvariant)
}

fn derived_id(domain: &[u8], hash: [u8; 32]) -> u128 {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable-raster-import-v1");
    hasher.update(domain);
    hasher.update(hash);
    let digest: [u8; 32] = hasher.finalize().into();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    u128::from_be_bytes(bytes).max(1)
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

fn safe_alias(path: &Path) -> String {
    let value = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("Image");
    let alias = value
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect::<String>();
    if alias.is_empty() {
        "Image".to_owned()
    } else {
        alias
    }
}

fn map_snapshot_error(error: &SourceSnapshotError) -> RasterImportFailure {
    match error {
        SourceSnapshotError::NotRegularFile { .. } => RasterImportFailure::NonRegularSource,
        SourceSnapshotError::SymlinkRejected { .. } => RasterImportFailure::SymlinkRejected,
        SourceSnapshotError::SourceChanged { .. } => RasterImportFailure::SourceChanged,
        SourceSnapshotError::SourceTooLarge { .. } => RasterImportFailure::SourceTooLarge,
        SourceSnapshotError::Io { .. }
        | SourceSnapshotError::EmptySource
        | SourceSnapshotError::AllocationFailure { .. } => RasterImportFailure::SourceUnavailable,
        SourceSnapshotError::LengthConversion | SourceSnapshotError::MaxPlusOneOverflow => {
            RasterImportFailure::InternalInvariant
        }
    }
}

fn map_reference_error(error: ReferenceSourceError) -> RasterImportFailure {
    match error {
        ReferenceSourceError::UnsupportedPathEncoding => {
            RasterImportFailure::UnsupportedPathEncoding
        }
        ReferenceSourceError::InvalidEncoding | ReferenceSourceError::InvalidSourcePath => {
            RasterImportFailure::InternalInvariant
        }
    }
}

fn map_catalog_error(error: AtomicRasterCatalogError) -> RasterImportFailure {
    match error {
        AtomicRasterCatalogError::Unavailable => RasterImportFailure::CatalogUnavailable,
        AtomicRasterCatalogError::Conflict => RasterImportFailure::CatalogConflict,
        AtomicRasterCatalogError::Corrupt => RasterImportFailure::CatalogCorrupt,
        AtomicRasterCatalogError::CommitFailed => RasterImportFailure::CatalogCommitFailed,
    }
}

fn report(
    observer: &dyn RasterImportObserver,
    item_id: RasterImportItemId,
    stage: RasterImportStage,
) {
    observer.progress(RasterImportProgress { item_id, stage });
}

fn receipt(
    item_id: RasterImportItemId,
    source_alias: String,
    status: RasterImportStatus,
) -> RasterImportReceipt {
    RasterImportReceipt {
        schema_version: 1,
        item_id,
        source_alias,
        content_sha256: None,
        format: None,
        photo_id: None,
        asset_id: None,
        edit_id: None,
        status,
        preview: None,
    }
}

fn evidence_receipt(
    item_id: RasterImportItemId,
    source_alias: String,
    hash: [u8; 32],
    format: InputFormat,
    status: RasterImportStatus,
) -> RasterImportReceipt {
    let mut receipt = receipt(item_id, source_alias, status);
    receipt.content_sha256 = Some(hash);
    receipt.format = Some(format);
    receipt
}

fn failed(
    item_id: RasterImportItemId,
    alias: String,
    failure: RasterImportFailure,
    observer: &dyn RasterImportObserver,
) -> RasterImportReceipt {
    report(observer, item_id, RasterImportStage::Failed);
    receipt(item_id, alias, RasterImportStatus::Failed(failure))
}

fn failed_with_evidence(
    item_id: RasterImportItemId,
    alias: String,
    hash: [u8; 32],
    format: Option<InputFormat>,
    failure: RasterImportFailure,
    observer: &dyn RasterImportObserver,
) -> RasterImportReceipt {
    report(observer, item_id, RasterImportStage::Failed);
    let mut receipt = receipt(item_id, alias, RasterImportStatus::Failed(failure));
    receipt.content_sha256 = Some(hash);
    receipt.format = format;
    receipt
}
