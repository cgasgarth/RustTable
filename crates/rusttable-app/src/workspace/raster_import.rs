use std::path::{Path, PathBuf};

use rusttable_catalog_store::{AtomicCatalogStoreError, RedbCatalogRepository};
use rusttable_image::DecodeLimits;
use rusttable_image_io::FileImageInput;
use rusttable_import::{
    AtomicRasterCatalog, AtomicRasterCatalogError, FileSourceSnapshotReader, ImportSourceLimits,
    RasterCatalogEntry, RasterImportBatch, RasterImportCancellation, RasterImportRequest,
    RasterImportService, RasterPreviewError, RasterPreviewPort, RasterPreviewReceipt,
    SourceSnapshotReader, decode_reference_source,
};
use rusttable_metadata::{ExifMetadataInput, MetadataLimits};
use rusttable_render::PreviewBounds;
use sha2::{Digest, Sha256};

use crate::PreviewService;

const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;
const MAX_PIXELS: u64 = 64 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 256 * 1024 * 1024;
const THUMBNAIL_EDGE: u32 = 256;

pub(crate) async fn pick_raster_files() -> Vec<PathBuf> {
    rfd::AsyncFileDialog::new()
        .add_filter(
            "Supported raster images",
            &["png", "jpg", "jpeg", "tif", "tiff"],
        )
        .set_title("Import raster files")
        .pick_files()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|handle| handle.path().to_path_buf())
        .collect()
}

pub(crate) fn run_raster_import(
    catalog_path: &Path,
    paths: Vec<PathBuf>,
    cancellation: &RasterImportCancellation,
) -> RasterImportBatch {
    let request =
        RasterImportRequest::new(paths).expect("application starts only nonempty bounded requests");
    let snapshot = FileSourceSnapshotReader;
    let image = FileImageInput::new(decode_limits());
    let metadata = ExifMetadataInput::new(metadata_limits());
    let service = RasterImportService::new(source_limits(), &snapshot, &image, &metadata);
    if let Ok(repository) = RedbCatalogRepository::open(catalog_path) {
        let mut catalog = AppCatalog(repository);
        service.import(&request, &mut catalog, &AppPreview, cancellation, &|_| {})
    } else {
        let mut catalog = UnavailableCatalog;
        service.import(&request, &mut catalog, &AppPreview, cancellation, &|_| {})
    }
}

struct AppCatalog(RedbCatalogRepository);

impl AtomicRasterCatalog for AppCatalog {
    fn find_by_content(
        &self,
        sha256: [u8; 32],
        byte_length: u64,
    ) -> Result<Option<RasterCatalogEntry>, AtomicRasterCatalogError> {
        self.0
            .find_by_content(sha256, byte_length)
            .map(|entry| entry.map(|(record, edit)| RasterCatalogEntry { record, edit }))
            .map_err(map_store_error)
    }

    fn commit_import(
        &mut self,
        entry: &RasterCatalogEntry,
    ) -> Result<(), AtomicRasterCatalogError> {
        self.0
            .commit_import_with_edit(&entry.record, &entry.edit)
            .map_err(map_store_error)
    }
}

struct UnavailableCatalog;

impl AtomicRasterCatalog for UnavailableCatalog {
    fn find_by_content(
        &self,
        _sha256: [u8; 32],
        _byte_length: u64,
    ) -> Result<Option<RasterCatalogEntry>, AtomicRasterCatalogError> {
        Err(AtomicRasterCatalogError::Unavailable)
    }

    fn commit_import(
        &mut self,
        _entry: &RasterCatalogEntry,
    ) -> Result<(), AtomicRasterCatalogError> {
        Err(AtomicRasterCatalogError::Unavailable)
    }
}

struct AppPreview;

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
        let output = PreviewService::new(
            decode_limits(),
            PreviewBounds::new(THUMBNAIL_EDGE, THUMBNAIL_EDGE)
                .expect("constant preview bounds are valid"),
        )
        .render_bytes(snapshot.bytes(), &entry.edit)
        .map_err(|_| RasterPreviewError::Render)?;
        reader
            .revalidate(&snapshot, source_limits())
            .map_err(|_| RasterPreviewError::SourceChanged)?;
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

    use rusttable_import::{RasterImportCancellation, RasterImportStatus};

    use super::run_raster_import;
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
        );
        let receipt = batch.receipts().next().expect("receipt");

        assert_eq!(receipt.status, RasterImportStatus::Imported);
        let photo_id = receipt.photo_id.expect("persisted photo ID");
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
