use std::path::{Path, PathBuf};

use rusttable_catalog::ImportRegistration;
use rusttable_catalog_store::{AtomicCatalogStoreError, RedbCatalogRepository};
use rusttable_image::DecodeLimits;
use rusttable_image_io::FileImageInput;
use rusttable_import::{
    AtomicRasterCatalog, AtomicRasterCatalogError, FileSourceSnapshotReader, ImportSourceLimits,
    RasterCatalogEntry, RasterDuplicateIdentity, RasterImportBatch, RasterImportCancellation,
    RasterImportObserver, RasterImportRequest, RasterImportService, RasterPreviewError,
    RasterPreviewPort, RasterPreviewReceipt, SourceSnapshotReader, decode_reference_source,
    reference_source_identity,
};
use rusttable_metadata::{ExifMetadataInput, MetadataLimits};
use rusttable_render::PreviewBounds;
use sha2::{Digest, Sha256};

use crate::PreviewService;
use crate::diagnostics::AppDiagnostics;

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;
const MAX_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const THUMBNAIL_EDGE: u32 = 256;

/// Imports a validated nonempty raster batch into the supplied catalog.
///
/// # Panics
///
/// Panics when `paths` is empty or exceeds the import request's bounded input
/// contract. Callers must validate that batch before invoking this operation.
pub fn run_raster_import(
    catalog_path: &Path,
    paths: Vec<PathBuf>,
    cancellation: &RasterImportCancellation,
    observer: &dyn RasterImportObserver,
) -> RasterImportBatch {
    run_raster_import_with_diagnostics(
        catalog_path,
        paths,
        cancellation,
        observer,
        AppDiagnostics::default(),
    )
}

pub(crate) fn run_raster_import_with_diagnostics(
    catalog_path: &Path,
    paths: Vec<PathBuf>,
    cancellation: &RasterImportCancellation,
    observer: &dyn RasterImportObserver,
    diagnostics: AppDiagnostics,
) -> RasterImportBatch {
    let request =
        RasterImportRequest::new(paths).expect("application starts only nonempty bounded requests");
    let snapshot = FileSourceSnapshotReader;
    let image = FileImageInput::new(decode_limits());
    let metadata = ExifMetadataInput::new(metadata_limits());
    let service = RasterImportService::new(source_limits(), &snapshot, &image, &metadata);
    if let Ok(repository) = RedbCatalogRepository::open(catalog_path) {
        let mut catalog = AppCatalog(repository);
        service.import(
            &request,
            &mut catalog,
            &AppPreview {
                diagnostics: diagnostics.clone(),
            },
            cancellation,
            observer,
        )
    } else {
        let mut catalog = UnavailableCatalog;
        service.import(
            &request,
            &mut catalog,
            &AppPreview { diagnostics },
            cancellation,
            observer,
        )
    }
}

struct AppCatalog(RedbCatalogRepository);

impl AtomicRasterCatalog for AppCatalog {
    fn find_by_content(
        &self,
        identity: RasterDuplicateIdentity,
    ) -> Result<Option<RasterCatalogEntry>, AtomicRasterCatalogError> {
        self.0
            .find_by_content(identity.content_sha256, identity.byte_length)
            .map(|entry| {
                entry.and_then(|(record, edit)| {
                    if record.probe() != identity.probe
                        || reference_source_identity(record.source()).ok()?
                            != identity.source_identity
                    {
                        return None;
                    }
                    Some(RasterCatalogEntry { record, edit })
                })
            })
            .map_err(map_store_error)
    }

    fn commit_import(
        &mut self,
        entry: &RasterCatalogEntry,
        registration: &ImportRegistration,
    ) -> Result<(), AtomicRasterCatalogError> {
        self.0
            .commit_import_with_edit(&entry.record, &entry.edit, registration)
            .map_err(map_store_error)
    }
}

struct UnavailableCatalog;

impl AtomicRasterCatalog for UnavailableCatalog {
    fn find_by_content(
        &self,
        _identity: RasterDuplicateIdentity,
    ) -> Result<Option<RasterCatalogEntry>, AtomicRasterCatalogError> {
        Err(AtomicRasterCatalogError::Unavailable)
    }

    fn commit_import(
        &mut self,
        _entry: &RasterCatalogEntry,
        _registration: &ImportRegistration,
    ) -> Result<(), AtomicRasterCatalogError> {
        Err(AtomicRasterCatalogError::Unavailable)
    }
}

struct AppPreview {
    diagnostics: AppDiagnostics,
}

impl RasterPreviewPort for AppPreview {
    fn generate_thumbnail(
        &self,
        entry: &RasterCatalogEntry,
    ) -> Result<RasterPreviewReceipt, RasterPreviewError> {
        let path = decode_reference_source(entry.record.source())
            .map_err(|_| RasterPreviewError::Unavailable)?;
        let reader = FileSourceSnapshotReader;
        let snapshot = reader
            .read_snapshot(&path, source_limits())
            .map_err(|_| RasterPreviewError::Unavailable)?;
        let bytes = snapshot
            .materialize(source_limits())
            .map_err(|_| RasterPreviewError::Unavailable)?;
        let output = PreviewService::new(
            decode_limits(),
            PreviewBounds::new(THUMBNAIL_EDGE, THUMBNAIL_EDGE)
                .expect("constant preview bounds are valid"),
        )
        .render_bytes(&bytes, &entry.edit)
        .map_err(|error| {
            self.diagnostics.import_preview_failure(
                Some(entry.record.probe().format()),
                Some((
                    entry.record.probe().dimensions().width(),
                    entry.record.probe().dimensions().height(),
                )),
                preview_error_cause(&error),
            );
            RasterPreviewError::Render
        })?;
        reader.revalidate(&snapshot, source_limits()).map_err(|_| {
            self.diagnostics.import_preview_failure(
                Some(entry.record.probe().format()),
                Some((
                    entry.record.probe().dimensions().width(),
                    entry.record.probe().dimensions().height(),
                )),
                "source_changed",
            );
            RasterPreviewError::SourceChanged
        })?;
        let dimensions = output.image().dimensions();
        let mut hasher = Sha256::new();
        hasher.update(output.image().pixels());
        Ok(RasterPreviewReceipt {
            width: dimensions.width(),
            height: dimensions.height(),
            pixel_sha256: hasher.finalize().into(),
        })
    }
}

fn preview_error_cause(error: &crate::PreviewError) -> &'static str {
    match error {
        crate::PreviewError::Decode(_) => "decode",
        crate::PreviewError::DecodedFrame => "decoded_frame",
        crate::PreviewError::UnsupportedPixelpipeColor { .. } => "unsupported_color",
        crate::PreviewError::PixelpipeInput(_) => "pixelpipe_input",
        crate::PreviewError::PixelpipeSnapshot(_) => "pixelpipe_snapshot",
        crate::PreviewError::Graph(_) => "processing_graph",
        crate::PreviewError::Pixelpipe(_) => "processing_pixelpipe",
        crate::PreviewError::Prepared(_) => "processing_prepare",
        crate::PreviewError::Render(_) => "render",
    }
}

fn map_store_error(error: AtomicCatalogStoreError) -> AtomicRasterCatalogError {
    match error {
        AtomicCatalogStoreError::Unavailable => AtomicRasterCatalogError::Unavailable,
        AtomicCatalogStoreError::Conflict => AtomicRasterCatalogError::Conflict,
        AtomicCatalogStoreError::Corrupt => AtomicRasterCatalogError::Corrupt,
        AtomicCatalogStoreError::CommitFailed => AtomicRasterCatalogError::CommitFailed,
    }
}

fn source_limits() -> ImportSourceLimits {
    ImportSourceLimits::new(MAX_SOURCE_BYTES).expect("constant source limits are valid")
}

fn decode_limits() -> DecodeLimits {
    DecodeLimits::new(
        MAX_SOURCE_BYTES,
        MAX_DIMENSION,
        MAX_DIMENSION,
        MAX_PIXELS,
        MAX_DECODE_BYTES,
    )
    .expect("constant decode limits are valid")
}

const fn metadata_limits() -> MetadataLimits {
    match MetadataLimits::new(MAX_SOURCE_BYTES, 512 * 1024, 512, 512, 8, 2_048, 128 * 1024) {
        Ok(limits) => limits,
        Err(_) => panic!("constant metadata limits are valid"),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use rusttable_catalog::ImportRegistrationStatus;
    use rusttable_catalog_store::RedbCatalogRepository;
    use rusttable_image::InputFormat;
    use rusttable_import::{
        AtomicRasterCatalog, RASTER_DECODER_IDENTITY_VERSION, RasterDuplicateIdentity,
        RasterImportCancellation, RasterImportStatus,
    };
    use sha2::{Digest, Sha256};

    use super::{AppCatalog, preview_error_cause, run_raster_import};
    use crate::PreviewError;
    use crate::library::{LibraryLoadResult, load_catalog};
    use crate::workspace::load_selected_preview;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let number = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "rusttable-app-raster-import-{}-{number}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("temporary directory");
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn real_import_restarts_into_library_and_existing_current_edit_preview() {
        let directory = TempDirectory::new();
        let source = directory.0.join("selected.png");
        let catalog = directory.0.join("catalog.redb");
        let bytes = decode_base64(include_str!(
            "../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
        ));
        fs::write(&source, &bytes).expect("fixture write");

        let batch = run_raster_import(
            &catalog,
            vec![source.clone()],
            &RasterImportCancellation::default(),
            &|_| {},
        );
        let receipt = batch.receipts().next().expect("receipt");

        assert_eq!(receipt.status, RasterImportStatus::Imported);
        let photo_id = receipt.photo_id.expect("persisted photo ID");
        let repository = RedbCatalogRepository::open(&catalog).expect("reopen details");
        let details = repository
            .find_import_details_by_photo_id(photo_id)
            .expect("details lookup")
            .expect("durable registration details");
        assert_eq!(details.summary().format(), InputFormat::Png);
        assert_eq!(details.summary().dimensions().width(), 2);
        assert_eq!(details.summary().dimensions().height(), 1);
        assert_eq!(details.receipt().source_alias(), "selected.png");
        assert_eq!(details.receipt().photo_id(), photo_id);
        assert_eq!(details.receipt().asset_id(), receipt.asset_id.unwrap());
        assert_eq!(details.receipt().edit_id(), receipt.edit_id.unwrap());
        assert_eq!(
            details.receipt().status(),
            ImportRegistrationStatus::Registered
        );
        assert_eq!(details.receipt().replaces_photo_id(), None);
        drop(repository);
        let LibraryLoadResult::Ready(workspace) = load_catalog(&catalog) else {
            panic!("reopened catalog must be ready")
        };
        assert!(workspace.detail(photo_id).is_some());
        let preview = load_selected_preview(&catalog, Path::new("unused"), photo_id)
            .expect("persisted neutral edit renders through reference snapshot");
        let (_, dimensions, pixels) = preview.into_parts();
        assert_eq!((dimensions.width(), dimensions.height()), (2, 1));
        assert_eq!(pixels.len(), 8);
        assert_eq!(fs::read(source).unwrap(), bytes);
    }

    #[test]
    fn import_preview_error_causes_are_stable_and_path_free() {
        assert_eq!(
            preview_error_cause(&PreviewError::Decode(
                rusttable_image::ImageInputError::ArithmeticOverflow,
            )),
            "decode"
        );
        assert_eq!(
            preview_error_cause(&PreviewError::UnsupportedPixelpipeColor {
                actual: rusttable_image::ColorEncoding::DisplayP3D65,
            }),
            "unsupported_color"
        );
    }

    #[test]
    fn duplicate_reuse_and_changed_path_replacement_survive_restart() {
        let directory = TempDirectory::new();
        let source = directory.0.join("reused.png");
        let duplicate = directory.0.join("duplicate.png");
        let catalog = directory.0.join("catalog.redb");
        let first_bytes = decode_base64(include_str!(
            "../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
        ));
        let changed_bytes = decode_base64(include_str!(
            "../../../rusttable-image-io/tests/fixtures/rgba-1x2.png.b64"
        ));
        fs::write(&source, &first_bytes).unwrap();
        fs::write(&duplicate, &first_bytes).unwrap();

        let first = run_raster_import(
            &catalog,
            vec![source.clone()],
            &RasterImportCancellation::default(),
            &|_| {},
        );
        let first_photo = first.receipts().next().unwrap().photo_id.unwrap();
        let duplicate_batch = run_raster_import(
            &catalog,
            vec![duplicate],
            &RasterImportCancellation::default(),
            &|_| {},
        );
        let duplicate_receipt = duplicate_batch.receipts().next().unwrap();
        assert_eq!(
            duplicate_receipt.status,
            RasterImportStatus::AlreadyImported
        );
        assert_eq!(duplicate_receipt.photo_id, Some(first_photo));

        fs::write(&source, &changed_bytes).unwrap();
        let changed = run_raster_import(
            &catalog,
            vec![source.clone()],
            &RasterImportCancellation::default(),
            &|_| {},
        );
        let changed_receipt = changed.receipts().next().unwrap();
        let changed_photo = changed_receipt.photo_id.unwrap();
        assert_eq!(changed_receipt.status, RasterImportStatus::Imported);
        assert_ne!(changed_photo, first_photo);

        let repository = RedbCatalogRepository::open(&catalog).expect("restart catalog");
        let details = repository
            .find_import_details_by_photo_id(changed_photo)
            .expect("details lookup")
            .expect("changed registration details");
        assert_eq!(details.receipt().replaces_photo_id(), Some(first_photo));
        assert_eq!(details.receipt().source_alias(), "reused.png");
        assert!(!format!("{details:?}").contains(directory.0.to_str().unwrap()));
        assert_eq!(fs::read(source).unwrap(), changed_bytes);
    }

    #[test]
    fn duplicate_lookup_rejects_a_persisted_source_with_a_different_decoder_identity() {
        let directory = TempDirectory::new();
        let source = directory.0.join("identity.png");
        let catalog_path = directory.0.join("catalog.redb");
        let bytes = decode_base64(include_str!(
            "../../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64"
        ));
        fs::write(&source, &bytes).unwrap();

        let batch = run_raster_import(
            &catalog_path,
            vec![source],
            &RasterImportCancellation::default(),
            &|_| {},
        );
        assert_eq!(
            batch.receipts().next().unwrap().status,
            RasterImportStatus::Imported
        );

        let hash: [u8; 32] = Sha256::digest(&bytes).into();
        let repository = RedbCatalogRepository::open(&catalog_path).unwrap();
        let (record, _) = repository
            .find_by_content(hash, u64::try_from(bytes.len()).unwrap())
            .unwrap()
            .unwrap();
        let catalog = AppCatalog(repository);
        let found = catalog
            .find_by_content(RasterDuplicateIdentity {
                content_sha256: hash,
                byte_length: u64::try_from(bytes.len()).unwrap(),
                decoder_identity_version: RASTER_DECODER_IDENTITY_VERSION,
                probe: record.probe(),
                source_identity: [0; 32],
            })
            .unwrap();

        assert!(found.is_none());
    }

    fn decode_base64(encoded: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut quartet = [0_u8; 4];
        let mut count = 0;
        for byte in encoded.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
            quartet[count] = match byte {
                b'A'..=b'Z' => byte - b'A',
                b'a'..=b'z' => byte - b'a' + 26,
                b'0'..=b'9' => byte - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => 64,
                _ => panic!("invalid fixture"),
            };
            count += 1;
            if count == 4 {
                bytes.push((quartet[0] << 2) | (quartet[1] >> 4));
                if quartet[2] != 64 {
                    bytes.push((quartet[1] << 4) | (quartet[2] >> 2));
                }
                if quartet[3] != 64 {
                    bytes.push((quartet[2] << 6) | quartet[3]);
                }
                count = 0;
            }
        }
        bytes
    }
}
