use std::fs;
use std::path::{Path, PathBuf};

use rusttable_catalog::{CatalogState, ImportOutcome, SourcePath};
use rusttable_catalog_store::RedbImportRepository;
use rusttable_core::{AssetId, ImageMetadata, PhotoId, Revision};
use rusttable_image::{
    DecodedImage, ImageDimensions, ImageInput, ImageInputError, ImageProbe, InputFormat,
};
use rusttable_import::{
    FileSourceSnapshotReader, ImportSourceLimits, SourceImportRequest, SourceImportService,
};
use rusttable_metadata::{MetadataInput, MetadataInputError};

fn fixture() -> Vec<u8> {
    decode(
        "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAADklEQVR4nGP4z8DwHwQBEPgD/U6VwW8AAAAASUVORK5CYII=",
    )
}

fn changed_fixture() -> Vec<u8> {
    decode(
        "/9j/4AAQSkZJRgABAQAAAQABAAD/2wBDAAEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQH/2wBDAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQH/wAARCAABAAIDASIAAhEBAxEB/8QAFQABAQAAAAAAAAAAAAAAAAAAAAf/xAAaEAEAAQUAAAAAAAAAAAAAAAAABwUGN3e2/8QAFAEBAAAAAAAAAAAAAAAAAAAACv/EAB8RAAADCQAAAAAAAAAAAAAAAAAFCAMGBzU4dXe0tv/aAAwDAQACEQMRAD8As0MYeijWti8vSgBczydHF0MNtsCOKiqZUVnWLnfvAP/Z",
    )
}

fn decode(encoded: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut quartet = [0u8; 4];
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

struct EmptyMetadata;
impl MetadataInput for EmptyMetadata {
    fn read_bytes(
        &self,
        _format: rusttable_image::InputFormat,
        _bytes: &[u8],
    ) -> Result<ImageMetadata, MetadataInputError> {
        Ok(ImageMetadata::empty())
    }
}

struct FixtureImageInput;

impl ImageInput for FixtureImageInput {
    fn probe_bytes(&self, bytes: &[u8]) -> Result<ImageProbe, ImageInputError> {
        let format = if bytes.starts_with(b"\x89PNG") {
            InputFormat::Png
        } else if bytes.starts_with(&[0xff, 0xd8]) {
            InputFormat::Jpeg
        } else {
            return Err(ImageInputError::UnsupportedSignature {
                signature: bytes.iter().copied().take(8).collect(),
            });
        };
        Ok(ImageProbe::new(
            format,
            ImageDimensions::new(2, 1).expect("fixture dimensions"),
        ))
    }

    fn decode_bytes(&self, _bytes: &[u8]) -> Result<DecodedImage, ImageInputError> {
        panic!("decode is outside import scope")
    }

    fn probe_path(&self, _path: &Path) -> Result<ImageProbe, ImageInputError> {
        panic!("path probe is outside import scope")
    }

    fn decode_path(&self, _path: &Path) -> Result<DecodedImage, ImageInputError> {
        panic!("path decode is outside import scope")
    }
}

fn request(path: &Path) -> SourceImportRequest {
    SourceImportRequest::new(
        PhotoId::new(11).expect("photo ID"),
        AssetId::new(12).expect("asset ID"),
        SourcePath::new("Camera/import.png").expect("logical source"),
        path.to_owned(),
    )
}

fn paths() -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!("rusttable-import-store-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).expect("temporary store directory");
    (root.join("photo.jpg"), root.join("catalog.redb"))
}

#[test]
fn durable_store_round_trip_is_idempotent_and_detects_changed_bytes() {
    let (source, database) = paths();
    let bytes = fixture();
    fs::write(&source, &bytes).expect("fixture writes");
    let image = FixtureImageInput;
    let mut state = CatalogState::new();
    let mut repository = RedbImportRepository::open(&database).expect("store opens");
    let first = SourceImportService::inspect_and_register(
        &mut state,
        Revision::ZERO,
        &request(&source),
        ImportSourceLimits::new(1_000_000).unwrap(),
        &mut repository,
        &FileSourceSnapshotReader,
        &image,
        &EmptyMetadata,
    )
    .expect("first import");
    assert!(matches!(first, ImportOutcome::Imported { .. }));
    drop(repository);

    let mut reopened = RedbImportRepository::open(&database).expect("store reopens");
    let retry_revision = state.revision();
    let second = SourceImportService::inspect_and_register(
        &mut state,
        retry_revision,
        &request(&source),
        ImportSourceLimits::new(1_000_000).unwrap(),
        &mut reopened,
        &FileSourceSnapshotReader,
        &image,
        &EmptyMetadata,
    )
    .expect("retry import");
    assert!(matches!(second, ImportOutcome::AlreadyPresent { .. }));

    fs::write(&source, changed_fixture()).expect("changed fixture writes");
    let changed_revision = state.revision();
    let changed = SourceImportService::inspect_and_register(
        &mut state,
        changed_revision,
        &request(&source),
        ImportSourceLimits::new(1_000_000).unwrap(),
        &mut reopened,
        &FileSourceSnapshotReader,
        &image,
        &EmptyMetadata,
    );
    assert!(matches!(
        changed,
        Err(rusttable_import::SourceImportError::Import(
            rusttable_catalog::ImportError::SourceContentChanged { .. }
        ))
    ));
    drop(reopened);
    fs::remove_dir_all(database.parent().expect("store parent")).expect("temporary data removes");
}
