use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Barrier, Mutex};

use rusttable_catalog::{ImportMetadataStatus, ImportRegistration};
use rusttable_core::{
    ContentHash, ImageMetadata, MetadataEntry, MetadataText, ParameterName, ParameterValue,
};
use rusttable_image::{
    DecodeLimits, DecodedImage, ImageInput, ImageInputError, ImageProbe, InputFormat,
};
use rusttable_image_io::FileImageInput;
use rusttable_import::{
    AtomicRasterCatalog, AtomicRasterCatalogError, FileSourceSnapshotReader, ImportSourceLimits,
    RasterCatalogEntry, RasterDuplicateIdentity, RasterImportCancellation, RasterImportProgress,
    RasterImportRequest, RasterImportService, RasterImportStage, RasterImportStatus,
    RasterPreviewError, RasterPreviewPort, RasterPreviewReceipt, SourceSnapshot,
    SourceSnapshotError, SourceSnapshotReader, decode_reference_source,
};
use rusttable_metadata::{MetadataInput, MetadataInputError, MetadataReadResult};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        let number = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusttable-raster-import-{}-{number}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temporary directory");
        Self(path)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, bytes).expect("fixture write");
        path
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct MemoryCatalog {
    entries: Vec<RasterCatalogEntry>,
    fail_commit: bool,
}

impl AtomicRasterCatalog for MemoryCatalog {
    fn find_by_content(
        &self,
        identity: RasterDuplicateIdentity,
    ) -> Result<Option<RasterCatalogEntry>, AtomicRasterCatalogError> {
        Ok(self
            .entries
            .iter()
            .find(|entry| {
                entry.record.photo().primary_asset().content_hash()
                    == ContentHash::Sha256(identity.content_sha256)
                    && entry.record.photo().primary_asset().byte_length().get()
                        == identity.byte_length
                    && entry.record.probe() == identity.probe
            })
            .cloned())
    }

    fn commit_import(
        &mut self,
        entry: &RasterCatalogEntry,
        _registration: &ImportRegistration,
    ) -> Result<(), AtomicRasterCatalogError> {
        if self.fail_commit {
            return Err(AtomicRasterCatalogError::CommitFailed);
        }
        self.entries.push(entry.clone());
        Ok(())
    }

    fn refresh_metadata(
        &mut self,
        entry: &RasterCatalogEntry,
        metadata: ImageMetadata,
    ) -> Result<RasterCatalogEntry, AtomicRasterCatalogError> {
        let refreshed = RasterCatalogEntry {
            record: entry.record.with_metadata(metadata),
            edit: entry.edit.clone(),
            metadata_status: ImportMetadataStatus::Available,
        };
        let slot = self
            .entries
            .iter_mut()
            .find(|candidate| candidate.record.photo().id() == entry.record.photo().id())
            .ok_or(AtomicRasterCatalogError::Corrupt)?;
        *slot = refreshed.clone();
        Ok(refreshed)
    }
}

struct CheckedPreview;

impl RasterPreviewPort for CheckedPreview {
    fn generate_thumbnail(
        &self,
        entry: &RasterCatalogEntry,
    ) -> Result<RasterPreviewReceipt, RasterPreviewError> {
        let operations = entry.edit.operations().collect::<Vec<_>>();
        assert_eq!(operations.len(), 3);
        assert_eq!(operations[0].key().as_str(), "rusttable.flip");
        assert_eq!(
            operations[0].parameter(&ParameterName::new("mode").unwrap()),
            Some(&ParameterValue::Integer(0))
        );
        assert_eq!(
            operations[0].parameter(&ParameterName::new("orientation").unwrap()),
            Some(&ParameterValue::Integer(0))
        );
        assert_eq!(operations[1].key().as_str(), "rusttable.exposure");
        assert_eq!(
            operations[1].parameter(&ParameterName::new("stops").unwrap()),
            Some(&ParameterValue::Scalar(
                rusttable_core::FiniteF64::new(0.0).unwrap()
            ))
        );
        assert_eq!(operations[2].key().as_str(), "rusttable.rgb_gain");
        for name in ["red", "green", "blue"] {
            assert_eq!(
                operations[2].parameter(&ParameterName::new(name).unwrap()),
                Some(&ParameterValue::Scalar(
                    rusttable_core::FiniteF64::new(1.0).unwrap()
                ))
            );
        }
        Ok(RasterPreviewReceipt {
            width: 2,
            height: 1,
            pixel_sha256: [9; 32],
        })
    }
}

fn service() -> RasterImportService<'static> {
    static SNAPSHOT: FileSourceSnapshotReader = FileSourceSnapshotReader;
    static IMAGE: std::sync::LazyLock<FileImageInput> = std::sync::LazyLock::new(|| {
        FileImageInput::new(
            DecodeLimits::new(4 * 1024 * 1024, 8_192, 8_192, 32_000_000, 128_000_000)
                .expect("decode limits"),
        )
    });
    static METADATA: EmptyMetadata = EmptyMetadata;
    RasterImportService::new(
        ImportSourceLimits::new(4 * 1024 * 1024).expect("source limits"),
        &SNAPSHOT,
        &*IMAGE,
        &METADATA,
    )
}

struct EmptyMetadata;

impl MetadataInput for EmptyMetadata {
    fn read_bytes(
        &self,
        _format: InputFormat,
        _source: &[u8],
    ) -> Result<ImageMetadata, MetadataInputError> {
        Ok(ImageMetadata::empty())
    }
}

struct UnavailableMetadata;

impl MetadataInput for UnavailableMetadata {
    fn read_bytes(
        &self,
        _format: InputFormat,
        _source: &[u8],
    ) -> Result<ImageMetadata, MetadataInputError> {
        Err(MetadataInputError::MalformedExif)
    }
}

struct PartialMetadata;

impl MetadataInput for PartialMetadata {
    fn read_bytes(
        &self,
        _format: InputFormat,
        _source: &[u8],
    ) -> Result<ImageMetadata, MetadataInputError> {
        Err(MetadataInputError::MalformedExif)
    }

    fn read_bytes_tolerant(&self, _format: InputFormat, _source: &[u8]) -> MetadataReadResult {
        MetadataReadResult::unavailable(
            ImageMetadata::from_entries([MetadataEntry::CameraMake(
                MetadataText::from_bytes(b"validated camera".to_vec()).unwrap(),
            )])
            .unwrap(),
            MetadataInputError::MalformedExif,
        )
    }
}

struct RefreshableMetadata {
    unavailable: std::sync::atomic::AtomicBool,
}

impl RefreshableMetadata {
    fn new() -> Self {
        Self {
            unavailable: std::sync::atomic::AtomicBool::new(true),
        }
    }

    fn make_available(&self) {
        self.unavailable.store(false, Ordering::Release);
    }
}

impl MetadataInput for RefreshableMetadata {
    fn read_bytes(
        &self,
        _format: InputFormat,
        _source: &[u8],
    ) -> Result<ImageMetadata, MetadataInputError> {
        if self.unavailable.load(Ordering::Acquire) {
            Err(MetadataInputError::MalformedExif)
        } else {
            Ok(ImageMetadata::from_entries([MetadataEntry::CameraModel(
                MetadataText::from_bytes(b"reparsed camera".to_vec()).unwrap(),
            )])
            .unwrap())
        }
    }
}

struct DecodeFails {
    delegate: FileImageInput,
}

impl ImageInput for DecodeFails {
    fn probe_bytes(&self, bytes: &[u8]) -> Result<ImageProbe, ImageInputError> {
        self.delegate.probe_bytes(bytes)
    }

    fn decode_bytes(&self, _bytes: &[u8]) -> Result<DecodedImage, ImageInputError> {
        Err(ImageInputError::MalformedInput {
            format: InputFormat::Png,
            message: "test decode failure".to_owned(),
        })
    }

    fn probe_path(&self, path: &Path) -> Result<ImageProbe, ImageInputError> {
        self.delegate.probe_path(path)
    }

    fn decode_path(&self, path: &Path) -> Result<DecodedImage, ImageInputError> {
        self.delegate.decode_path(path)
    }
}

struct ConcurrentReader {
    delegate: FileSourceSnapshotReader,
    first_wave: Barrier,
    preparation_calls: AtomicUsize,
    active: AtomicUsize,
    maximum_active: AtomicUsize,
}

impl ConcurrentReader {
    fn new() -> Self {
        Self {
            delegate: FileSourceSnapshotReader,
            first_wave: Barrier::new(4),
            preparation_calls: AtomicUsize::new(0),
            active: AtomicUsize::new(0),
            maximum_active: AtomicUsize::new(0),
        }
    }
}

impl SourceSnapshotReader for ConcurrentReader {
    fn read_snapshot(
        &self,
        path: &Path,
        limits: ImportSourceLimits,
    ) -> Result<SourceSnapshot, SourceSnapshotError> {
        let call = self.preparation_calls.fetch_add(1, Ordering::AcqRel);
        if call >= 6 {
            return self.delegate.read_snapshot(path, limits);
        }
        let active = self.active.fetch_add(1, Ordering::AcqRel) + 1;
        self.maximum_active.fetch_max(active, Ordering::AcqRel);
        if call < 4 {
            self.first_wave.wait();
        }
        let result = self.delegate.read_snapshot(path, limits);
        self.active.fetch_sub(1, Ordering::AcqRel);
        result
    }
}

#[test]
fn six_items_use_at_most_four_preparations_and_keep_request_order() {
    let directory = TempDirectory::new();
    let bytes = fixture("rgba-2x1.png.b64");
    let paths = (1..=6)
        .map(|index| directory.write(&format!("ordered-{index}.png"), &bytes))
        .collect::<Vec<_>>();
    let request = RasterImportRequest::new(paths).unwrap();
    let reader = ConcurrentReader::new();
    let image = FileImageInput::new(
        DecodeLimits::new(4 * 1024 * 1024, 8_192, 8_192, 32_000_000, 128_000_000)
            .expect("decode limits"),
    );
    let service = RasterImportService::new(
        ImportSourceLimits::new(4 * 1024 * 1024).expect("source limits"),
        &reader,
        &image,
        &EmptyMetadata,
    );
    let mut catalog = MemoryCatalog::default();

    let batch = service.import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &RasterImportCancellation::default(),
        &|_| {},
    );
    let receipts = batch.receipts().collect::<Vec<_>>();

    assert_eq!(reader.maximum_active.load(Ordering::Acquire), 4);
    assert_eq!(receipts.len(), 6);
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.item_id.get())
            .collect::<Vec<_>>(),
        [1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.source_alias.as_str())
            .collect::<Vec<_>>(),
        [
            "ordered-1.png",
            "ordered-2.png",
            "ordered-3.png",
            "ordered-4.png",
            "ordered-5.png",
            "ordered-6.png",
        ]
    );
    assert_eq!(batch.first_selected_photo(), receipts[0].photo_id);
}

#[test]
fn raster_import_real_png_jpeg_tiff_is_ordered_signature_first_and_neutral() {
    let directory = TempDirectory::new();
    let png = directory.write("png-with-jpeg-extension.jpg", &fixture("rgba-2x1.png.b64"));
    let jpeg = directory.write("jpeg-with-tiff-extension.tiff", &fixture("rgb-2x1.jpg.b64"));
    let tiff = directory.write("tiff-with-png-extension.png", &fixture("rgb-2x1.tiff.b64"));
    let original = [
        fs::read(&png).unwrap(),
        fs::read(&jpeg).unwrap(),
        fs::read(&tiff).unwrap(),
    ];
    let request = RasterImportRequest::new([png.clone(), jpeg.clone(), tiff.clone()]).unwrap();
    let progress = Mutex::new(Vec::new());
    let observer = |event: RasterImportProgress| progress.lock().unwrap().push(event);
    let mut catalog = MemoryCatalog::default();

    let batch = service().import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &RasterImportCancellation::default(),
        &observer,
    );
    let receipts = batch.receipts().collect::<Vec<_>>();

    assert_eq!(receipts.len(), 3);
    assert_eq!(
        receipts
            .iter()
            .map(|receipt| receipt.format)
            .collect::<Vec<_>>(),
        [
            Some(InputFormat::Png),
            Some(InputFormat::Jpeg),
            Some(InputFormat::Tiff)
        ]
    );
    assert!(
        receipts
            .iter()
            .all(|receipt| receipt.status == RasterImportStatus::Imported),
        "{receipts:?}"
    );
    assert!(receipts.iter().all(|receipt| receipt.preview.is_some()));
    assert_eq!(catalog.entries.len(), 3);
    for (entry, expected_path) in catalog.entries.iter().zip([&png, &jpeg, &tiff]) {
        assert_eq!(
            decode_reference_source(entry.record.source()).unwrap(),
            expected_path.as_path()
        );
        assert!(!entry.record.source().as_str().contains("extension"));
    }
    assert_eq!(fs::read(&png).unwrap(), original[0]);
    assert_eq!(fs::read(&jpeg).unwrap(), original[1]);
    assert_eq!(fs::read(&tiff).unwrap(), original[2]);
    let events = progress.into_inner().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.stage == RasterImportStage::Queued)
            .count(),
        3
    );
    assert_eq!(
        events.last().map(|event| event.stage),
        Some(RasterImportStage::Completed)
    );
}

#[test]
fn duplicate_content_reuses_one_photo_and_preserves_selection_order() {
    let directory = TempDirectory::new();
    let bytes = fixture("rgba-2x1.png.b64");
    let first = directory.write("first.png", &bytes);
    let second = directory.write("duplicate.dat", &bytes);
    let request = RasterImportRequest::new([first, second]).unwrap();
    let mut catalog = MemoryCatalog::default();

    let batch = service().import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &RasterImportCancellation::default(),
        &|_| {},
    );
    let receipts = batch.receipts().collect::<Vec<_>>();

    assert_eq!(receipts[0].status, RasterImportStatus::Imported);
    assert_eq!(receipts[1].status, RasterImportStatus::AlreadyImported);
    assert_eq!(receipts[0].photo_id, receipts[1].photo_id);
    assert_eq!(batch.first_selected_photo(), receipts[0].photo_id);
    assert_eq!(catalog.entries.len(), 1);
}

#[test]
fn catalog_commit_failure_leaves_no_partial_photo_or_edit() {
    let directory = TempDirectory::new();
    let path = directory.write("photo.png", &fixture("rgba-2x1.png.b64"));
    let request = RasterImportRequest::new([path]).unwrap();
    let mut catalog = MemoryCatalog {
        fail_commit: true,
        ..MemoryCatalog::default()
    };

    let batch = service().import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &RasterImportCancellation::default(),
        &|_| {},
    );

    assert!(matches!(
        batch.receipts().next().unwrap().status,
        RasterImportStatus::Failed(rusttable_import::RasterImportFailure::CatalogCommitFailed)
    ));
    assert!(catalog.entries.is_empty());
}

#[test]
fn cancellation_before_commit_creates_no_catalog_record() {
    let directory = TempDirectory::new();
    let path = directory.write("photo.png", &fixture("rgba-2x1.png.b64"));
    let request = RasterImportRequest::new([path]).unwrap();
    let cancellation = RasterImportCancellation::default();
    let observer_cancellation = cancellation.clone();
    let observer = move |progress: RasterImportProgress| {
        if progress.stage == RasterImportStage::Registering {
            observer_cancellation.cancel();
        }
    };
    let mut catalog = MemoryCatalog::default();

    let batch = service().import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &cancellation,
        &observer,
    );

    assert_eq!(
        batch.receipts().next().unwrap().status,
        RasterImportStatus::Cancelled
    );
    assert!(catalog.entries.is_empty());
}

#[test]
fn valid_decode_imports_when_metadata_is_unavailable_and_persists_typed_status() {
    let directory = TempDirectory::new();
    let path = directory.write("metadata-broken.png", &fixture("rgba-2x1.png.b64"));
    let request = RasterImportRequest::new([path]).unwrap();
    let image = FileImageInput::new(
        DecodeLimits::new(4 * 1024 * 1024, 8_192, 8_192, 32_000_000, 128_000_000)
            .expect("decode limits"),
    );
    let metadata = UnavailableMetadata;
    let service = RasterImportService::new(
        ImportSourceLimits::new(4 * 1024 * 1024).unwrap(),
        &FileSourceSnapshotReader,
        &image,
        &metadata,
    );
    let mut catalog = MemoryCatalog::default();

    let batch = service.import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &RasterImportCancellation::default(),
        &|_| {},
    );
    let receipt = batch.receipts().next().unwrap();

    assert_eq!(receipt.status, RasterImportStatus::Imported);
    assert_eq!(
        receipt.metadata_status,
        Some(MetadataInputError::MalformedExif)
    );
    assert_eq!(catalog.entries.len(), 1);
    assert_eq!(
        catalog.entries[0].metadata_status,
        ImportMetadataStatus::Unavailable
    );
    assert!(catalog.entries[0].record.metadata().is_empty());
}

#[test]
fn unavailable_metadata_keeps_independently_validated_fields() {
    let directory = TempDirectory::new();
    let path = directory.write("metadata-partial.png", &fixture("rgba-2x1.png.b64"));
    let request = RasterImportRequest::new([path]).unwrap();
    let image = FileImageInput::new(
        DecodeLimits::new(4 * 1024 * 1024, 8_192, 8_192, 32_000_000, 128_000_000)
            .expect("decode limits"),
    );
    let metadata = PartialMetadata;
    let service = RasterImportService::new(
        ImportSourceLimits::new(4 * 1024 * 1024).unwrap(),
        &FileSourceSnapshotReader,
        &image,
        &metadata,
    );
    let mut catalog = MemoryCatalog::default();

    let batch = service.import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &RasterImportCancellation::default(),
        &|_| {},
    );
    let entry = &catalog.entries[0];

    assert_eq!(
        batch.receipts().next().unwrap().status,
        RasterImportStatus::Imported
    );
    assert_eq!(
        entry
            .record
            .metadata()
            .get(rusttable_core::MetadataField::CameraMake),
        Some(&MetadataEntry::CameraMake(
            MetadataText::from_bytes(b"validated camera".to_vec()).unwrap()
        ))
    );
}

#[test]
fn reimport_refreshes_unavailable_metadata_without_duplicate_photo() {
    let directory = TempDirectory::new();
    let path = directory.write("metadata-refresh.png", &fixture("rgba-2x1.png.b64"));
    let request = RasterImportRequest::new([path]).unwrap();
    let image = FileImageInput::new(
        DecodeLimits::new(4 * 1024 * 1024, 8_192, 8_192, 32_000_000, 128_000_000)
            .expect("decode limits"),
    );
    let metadata = RefreshableMetadata::new();
    let service = RasterImportService::new(
        ImportSourceLimits::new(4 * 1024 * 1024).unwrap(),
        &FileSourceSnapshotReader,
        &image,
        &metadata,
    );
    let mut catalog = MemoryCatalog::default();

    let first = service.import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &RasterImportCancellation::default(),
        &|_| {},
    );
    let first_photo = first.receipts().next().unwrap().photo_id;
    metadata.make_available();
    let second = service.import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &RasterImportCancellation::default(),
        &|_| {},
    );
    let second_receipt = second.receipts().next().unwrap();

    assert_eq!(second_receipt.status, RasterImportStatus::AlreadyImported);
    assert_eq!(second_receipt.photo_id, first_photo);
    assert_eq!(catalog.entries.len(), 1);
    assert_eq!(
        catalog.entries[0].metadata_status,
        ImportMetadataStatus::Available
    );
    assert!(
        catalog.entries[0]
            .record
            .metadata()
            .get(rusttable_core::MetadataField::CameraModel)
            .is_some()
    );
}

#[test]
fn full_decode_failure_remains_fatal_before_metadata_registration() {
    let directory = TempDirectory::new();
    let path = directory.write("decode-fails.png", &fixture("rgba-2x1.png.b64"));
    let request = RasterImportRequest::new([path]).unwrap();
    let image = DecodeFails {
        delegate: FileImageInput::new(
            DecodeLimits::new(4 * 1024 * 1024, 8_192, 8_192, 32_000_000, 128_000_000)
                .expect("decode limits"),
        ),
    };
    let metadata = UnavailableMetadata;
    let service = RasterImportService::new(
        ImportSourceLimits::new(4 * 1024 * 1024).unwrap(),
        &FileSourceSnapshotReader,
        &image,
        &metadata,
    );
    let mut catalog = MemoryCatalog::default();

    let batch = service.import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &RasterImportCancellation::default(),
        &|_| {},
    );

    assert_eq!(
        batch.receipts().next().unwrap().status,
        RasterImportStatus::Failed(
            rusttable_import::RasterImportFailure::UnsupportedOrMalformedRaster
        )
    );
    assert!(catalog.entries.is_empty());
}

fn fixture(name: &str) -> Vec<u8> {
    let encoded = match name {
        "rgba-2x1.png.b64" => {
            include_str!("../../rusttable-image-io/tests/fixtures/rgba-2x1.png.b64")
        }
        "rgb-2x1.jpg.b64" => {
            include_str!("../../rusttable-image-io/tests/fixtures/rgb-2x1.jpg.b64")
        }
        "rgb-2x1.tiff.b64" => {
            include_str!("../../rusttable-image-io/tests/fixtures/rgb-2x1.tiff.b64")
        }
        _ => panic!("unknown fixture"),
    };
    decode_base64(encoded)
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
            _ => panic!("fixture base64 is invalid"),
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

#[test]
fn request_rejects_directories_and_receipts_never_contain_parent_paths() {
    let directory = TempDirectory::new();
    let request = RasterImportRequest::new([directory.0.clone()]).unwrap();
    let mut catalog = MemoryCatalog::default();

    let batch = service().import(
        &request,
        &mut catalog,
        &CheckedPreview,
        &RasterImportCancellation::default(),
        &|_| {},
    );
    let receipt = batch.receipts().next().unwrap();

    assert!(matches!(receipt.status, RasterImportStatus::Failed(_)));
    assert!(!receipt.source_alias.contains(std::path::MAIN_SEPARATOR));
    assert!(!format!("{receipt:?}").contains(directory.0.parent().unwrap().to_str().unwrap()));
}
