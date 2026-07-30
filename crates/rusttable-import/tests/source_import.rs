use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusttable_catalog::{
    CatalogState, ImportRecord, ImportRepository, RepositoryError, SourcePath,
};
use rusttable_core::{AssetId, ImageMetadata, PhotoId, Revision};
use rusttable_image::{
    DecodedImage, ImageDimensions, ImageInput, ImageInputError, ImageProbe, InputFormat,
};
use rusttable_import::{
    FileSourceSnapshotReader, ImportSourceLimits, SourceImportError, SourceImportRequest,
    SourceImportService, SourceSnapshotReader,
};
use rusttable_metadata::{MetadataInput, MetadataInputError};

fn physical(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusttable-source-import-{name}-{}",
        std::process::id()
    ))
}

#[derive(Clone, Default)]
struct RecordingBytes(Arc<Mutex<Vec<u8>>>);

impl RecordingBytes {
    fn replace(&self, bytes: &[u8]) {
        *self.0.lock().expect("recording lock") = bytes.to_vec();
    }
    fn bytes(&self) -> Vec<u8> {
        self.0.lock().expect("recording lock").clone()
    }
}

struct RecordingImage {
    seen: RecordingBytes,
    fail: bool,
}

impl ImageInput for RecordingImage {
    fn probe_bytes(&self, bytes: &[u8]) -> Result<ImageProbe, ImageInputError> {
        self.seen.replace(bytes);
        if self.fail {
            return Err(ImageInputError::UnsupportedSignature { signature: vec![] });
        }
        Ok(ImageProbe::new(
            InputFormat::Png,
            ImageDimensions::new(1, 1).expect("dimensions"),
        ))
    }
    fn decode_bytes(&self, _bytes: &[u8]) -> Result<DecodedImage, ImageInputError> {
        panic!("decode must not be called")
    }
    fn probe_path(&self, _path: &Path) -> Result<ImageProbe, ImageInputError> {
        panic!("path probe must not be called")
    }
    fn decode_path(&self, _path: &Path) -> Result<DecodedImage, ImageInputError> {
        panic!("path decode must not be called")
    }
}

struct RecordingMetadata {
    seen: RecordingBytes,
    fail: bool,
}

impl MetadataInput for RecordingMetadata {
    fn read_bytes(
        &self,
        _format: InputFormat,
        bytes: &[u8],
    ) -> Result<ImageMetadata, MetadataInputError> {
        self.seen.replace(bytes);
        if self.fail {
            Err(MetadataInputError::MalformedExif)
        } else {
            Ok(ImageMetadata::empty())
        }
    }
}

struct MutatingReader {
    calls: Arc<Mutex<u32>>,
}

impl SourceSnapshotReader for MutatingReader {
    fn read_snapshot(
        &self,
        path: &Path,
        limits: ImportSourceLimits,
    ) -> Result<rusttable_import::SourceSnapshot, rusttable_import::SourceSnapshotError> {
        *self.calls.lock().expect("reader lock") += 1;
        let snapshot = FileSourceSnapshotReader.read_snapshot(path, limits)?;
        fs::remove_file(path).expect("reader removes the backing path");
        Ok(snapshot)
    }
}

#[derive(Default)]
struct RecordingRepository {
    records: BTreeMap<SourcePath, ImportRecord>,
    commits: u32,
}

impl ImportRepository for RecordingRepository {
    fn find_by_source(&self, source: &SourcePath) -> Result<Option<ImportRecord>, RepositoryError> {
        Ok(self.records.get(source).cloned())
    }
    fn find_by_photo_id(&self, id: PhotoId) -> Result<Option<ImportRecord>, RepositoryError> {
        Ok(self
            .records
            .values()
            .find(|record| record.photo().id() == id)
            .cloned())
    }
    fn find_by_asset_id(&self, id: AssetId) -> Result<Option<ImportRecord>, RepositoryError> {
        Ok(self
            .records
            .values()
            .find(|record| record.photo().primary_asset_id() == id)
            .cloned())
    }
    fn commit(&mut self, record: &ImportRecord) -> Result<(), RepositoryError> {
        self.commits += 1;
        self.records.insert(record.source().clone(), record.clone());
        Ok(())
    }
    fn list(&self) -> Result<Vec<ImportRecord>, RepositoryError> {
        Ok(self.records.values().cloned().collect())
    }
}

fn request(path: &Path) -> SourceImportRequest {
    SourceImportRequest::new(
        PhotoId::new(1).expect("photo ID"),
        AssetId::new(2).expect("asset ID"),
        SourcePath::new("Camera/photo.png").expect("logical source"),
        path.to_owned(),
    )
}

#[test]
fn registration_uses_the_same_owned_bytes_once_for_every_inspection_stage() {
    let path = physical("success");
    let source = b"bytes-that-outlive-the-path";
    fs::write(&path, source).expect("fixture writes");
    let image_seen = RecordingBytes::default();
    let metadata_seen = RecordingBytes::default();
    let reader_calls = Arc::new(Mutex::new(0));
    let mut repository = RecordingRepository::default();
    let mut state = CatalogState::new();
    let outcome = SourceImportService::inspect_and_register(
        &mut state,
        Revision::ZERO,
        &request(&path),
        ImportSourceLimits::new(4096).unwrap(),
        &mut repository,
        &MutatingReader {
            calls: Arc::clone(&reader_calls),
        },
        &RecordingImage {
            seen: image_seen.clone(),
            fail: false,
        },
        &RecordingMetadata {
            seen: metadata_seen.clone(),
            fail: false,
        },
    )
    .expect("source import");
    assert!(matches!(
        outcome,
        rusttable_catalog::ImportOutcome::Imported { .. }
    ));
    assert_eq!(*reader_calls.lock().expect("reader lock"), 1);
    assert_eq!(image_seen.bytes(), source);
    assert_eq!(metadata_seen.bytes(), source);
    assert_eq!(repository.commits, 1);
    assert_eq!(state.revision(), Revision::from_u64(1));
    assert!(!path.exists());
}

#[test]
fn stale_revision_fails_before_any_source_or_repository_call() {
    let path = physical("stale");
    fs::write(&path, b"source").expect("fixture writes");
    let reader_calls = Arc::new(Mutex::new(0));
    let mut repository = RecordingRepository::default();
    let mut state = CatalogState::new();
    let result = SourceImportService::inspect_and_register(
        &mut state,
        Revision::from_u64(1),
        &request(&path),
        ImportSourceLimits::new(4096).unwrap(),
        &mut repository,
        &MutatingReader {
            calls: Arc::clone(&reader_calls),
        },
        &RecordingImage {
            seen: RecordingBytes::default(),
            fail: false,
        },
        &RecordingMetadata {
            seen: RecordingBytes::default(),
            fail: false,
        },
    );
    assert!(matches!(
        result,
        Err(SourceImportError::StaleRevision { .. })
    ));
    assert_eq!(*reader_calls.lock().expect("reader lock"), 0);
    assert_eq!(repository.commits, 0);
    fs::remove_file(path).expect("fixture removes");
}

#[test]
fn inspection_failure_makes_zero_repository_calls_and_preserves_state() {
    let path = physical("image-failure");
    fs::write(&path, b"source").expect("fixture writes");
    let reader_calls = Arc::new(Mutex::new(0));
    let mut repository = RecordingRepository::default();
    let mut state = CatalogState::new();
    let result = SourceImportService::inspect_and_register(
        &mut state,
        Revision::ZERO,
        &request(&path),
        ImportSourceLimits::new(4096).unwrap(),
        &mut repository,
        &MutatingReader {
            calls: Arc::clone(&reader_calls),
        },
        &RecordingImage {
            seen: RecordingBytes::default(),
            fail: true,
        },
        &RecordingMetadata {
            seen: RecordingBytes::default(),
            fail: false,
        },
    );
    assert!(matches!(result, Err(SourceImportError::Image(_))));
    assert_eq!(*reader_calls.lock().expect("reader lock"), 1);
    assert_eq!(repository.commits, 0);
    assert_eq!(state.revision(), Revision::ZERO);
}
