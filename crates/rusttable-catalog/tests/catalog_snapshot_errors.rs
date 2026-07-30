use std::cell::Cell;

use rusttable_catalog::{
    CatalogCommand, CatalogSnapshot, CatalogSnapshotError, CatalogState, ImportCandidate,
    ImportRecord, ImportRepository, RepositoryError, SourcePath,
};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, ImageMetadata, Photo, PhotoId,
};
use rusttable_image::{ImageDimensions, ImageProbe, InputFormat};

fn photo(photo_id: u128, asset_id: u128, byte: u8) -> Photo {
    Photo::new(
        PhotoId::new(photo_id).unwrap(),
        [Asset::new(
            AssetId::new(asset_id).unwrap(),
            AssetRole::Primary,
            ContentHash::Sha256([byte; 32]),
            ByteLength::from_bytes(8),
        )],
    )
    .unwrap()
}

fn record(source: &str, photo: Photo) -> ImportRecord {
    let candidate = ImportCandidate::new(
        photo.id(),
        photo.primary_asset_id(),
        SourcePath::new(source).unwrap(),
        photo.primary_asset().content_hash(),
        photo.primary_asset().byte_length(),
        ImageProbe::new(InputFormat::Png, ImageDimensions::new(2, 1).unwrap()),
        ImageMetadata::default(),
    )
    .unwrap();
    ImportRecord::new(&candidate, photo).unwrap()
}

fn state_with_photos(photos: impl IntoIterator<Item = Photo>) -> CatalogState {
    let mut state = CatalogState::new();
    for photo in photos {
        state
            .apply(state.revision(), CatalogCommand::RegisterPhoto(photo))
            .unwrap();
    }
    state
}

struct FakeRepository {
    records: Vec<ImportRecord>,
    error: Option<RepositoryError>,
    list_calls: Cell<usize>,
}

impl ImportRepository for FakeRepository {
    fn find_by_source(
        &self,
        _source: &SourcePath,
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        unreachable!("snapshot loading must not perform per-entry lookups")
    }

    fn find_by_photo_id(&self, _id: PhotoId) -> Result<Option<ImportRecord>, RepositoryError> {
        unreachable!("snapshot loading must not perform per-entry lookups")
    }

    fn find_by_asset_id(&self, _id: AssetId) -> Result<Option<ImportRecord>, RepositoryError> {
        unreachable!("snapshot loading must not perform per-entry lookups")
    }

    fn commit(&mut self, _record: &ImportRecord) -> Result<(), RepositoryError> {
        unreachable!("snapshot loading must not commit")
    }

    fn list(&self) -> Result<Vec<ImportRecord>, RepositoryError> {
        self.list_calls.set(self.list_calls.get() + 1);
        match &self.error {
            Some(error) => Err(error.clone()),
            None => Ok(self.records.clone()),
        }
    }
}

fn load(
    state: &CatalogState,
    records: Vec<ImportRecord>,
) -> Result<rusttable_catalog::CatalogSnapshot, CatalogSnapshotError> {
    let repository = FakeRepository {
        records,
        error: None,
        list_calls: Cell::new(0),
    };
    CatalogSnapshot::load(state, &repository)
}

#[test]
fn preserves_repository_failure_as_nested_source() {
    let repository = FakeRepository {
        records: Vec::new(),
        error: Some(RepositoryError::Unavailable),
        list_calls: Cell::new(0),
    };
    let error = CatalogSnapshot::load(&CatalogState::new(), &repository).unwrap_err();
    assert!(matches!(
        error,
        CatalogSnapshotError::Repository(RepositoryError::Unavailable)
    ));
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn validates_duplicate_source_before_other_identity_errors() {
    let first = photo(1, 11, 1);
    let second = photo(2, 22, 2);
    let state = state_with_photos([first.clone(), second.clone()]);
    let error = load(
        &state,
        vec![record("same.png", first), record("same.png", second)],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CatalogSnapshotError::DuplicateSource { .. }
    ));
}

#[test]
fn rejects_duplicate_photo_and_asset_identities() {
    let first = photo(1, 11, 1);
    let second_photo = photo(1, 22, 2);
    let state = state_with_photos([first.clone()]);
    let photo_error = load(
        &state,
        vec![
            record("one.png", first.clone()),
            record("two.png", second_photo),
        ],
    )
    .unwrap_err();
    assert!(matches!(
        photo_error,
        CatalogSnapshotError::DuplicatePhotoId { .. }
    ));

    let second_asset = photo(2, 11, 2);
    let state = state_with_photos([first.clone(), photo(2, 22, 2)]);
    let asset_error = load(
        &state,
        vec![record("one.png", first), record("two.png", second_asset)],
    )
    .unwrap_err();
    assert!(matches!(
        asset_error,
        CatalogSnapshotError::DuplicateAssetId { .. }
    ));
}

#[test]
fn rejects_persisted_only_photo_and_state_only_photo() {
    let persisted = photo(1, 11, 1);
    let persisted_only =
        load(&CatalogState::new(), vec![record("one.png", persisted)]).unwrap_err();
    assert!(matches!(
        persisted_only,
        CatalogSnapshotError::PersistedPhotoMissingFromState { .. }
    ));

    let state_only = photo(2, 22, 2);
    let state_only_error = load(&state_with_photos([state_only]), Vec::new()).unwrap_err();
    assert!(matches!(
        state_only_error,
        CatalogSnapshotError::StatePhotoMissingFromRepository { .. }
    ));
}

#[test]
fn rejects_persisted_state_photo_mismatch() {
    let state_photo = photo(1, 11, 1);
    let persisted_photo = photo(1, 11, 2);
    let error = load(
        &state_with_photos([state_photo]),
        vec![record("one.png", persisted_photo)],
    )
    .unwrap_err();
    assert!(matches!(
        error,
        CatalogSnapshotError::PersistedPhotoMismatch { .. }
    ));
}
