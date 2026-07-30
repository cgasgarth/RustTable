use std::cell::Cell;

use rusttable_catalog::{
    CatalogCommand, CatalogSnapshot, CatalogState, ImportCandidate, ImportRecord, ImportRepository,
    RepositoryError, SourcePath,
};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, ImageMetadata, Photo,
    PhotoId, Revision,
};
use rusttable_image::{ImageDimensions, ImageProbe, InputFormat};

fn probe() -> ImageProbe {
    ImageProbe::new(
        InputFormat::Png,
        ImageDimensions::new(2, 1).expect("valid dimensions"),
    )
}

fn photo(photo_id: u128, asset_id: u128, byte: u8) -> Photo {
    Photo::new(
        PhotoId::new(photo_id).expect("valid photo ID"),
        [Asset::new(
            AssetId::new(asset_id).expect("valid asset ID"),
            AssetRole::Primary,
            ContentHash::Sha256([byte; 32]),
            ByteLength::from_bytes(8),
        )],
    )
    .expect("valid photo")
}

fn record(source: &str, photo: Photo) -> ImportRecord {
    let candidate = ImportCandidate::new(
        photo.id(),
        photo.primary_asset_id(),
        SourcePath::new(source).expect("valid source"),
        photo.primary_asset().content_hash(),
        photo.primary_asset().byte_length(),
        probe(),
        ImageMetadata::default(),
    )
    .expect("valid candidate");
    ImportRecord::new(&candidate, photo).expect("valid record")
}

fn state_with_photos(photos: impl IntoIterator<Item = Photo>) -> CatalogState {
    let mut state = CatalogState::new();
    for photo in photos {
        state
            .apply(state.revision(), CatalogCommand::RegisterPhoto(photo))
            .expect("photo registration");
    }
    state
}

fn edit(id: u128, photo_id: u128) -> Edit {
    Edit::new(
        EditId::new(id).expect("valid edit ID"),
        PhotoId::new(photo_id).expect("valid photo ID"),
        Revision::ZERO,
        [],
    )
    .expect("valid edit")
}

#[derive(Default)]
struct ListingRepository {
    records: Vec<ImportRecord>,
    list_calls: Cell<usize>,
}

impl ImportRepository for ListingRepository {
    fn find_by_source(
        &self,
        _source: &SourcePath,
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        unreachable!("snapshot loading must not perform per-entry lookups")
    }

    fn find_by_photo_id(
        &self,
        _photo_id: PhotoId,
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        unreachable!("snapshot loading must not perform per-entry lookups")
    }

    fn find_by_asset_id(
        &self,
        _asset_id: AssetId,
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        unreachable!("snapshot loading must not perform per-entry lookups")
    }

    fn commit(&mut self, _record: &ImportRecord) -> Result<(), RepositoryError> {
        unreachable!("snapshot loading must not commit")
    }

    fn list(&self) -> Result<Vec<ImportRecord>, RepositoryError> {
        self.list_calls.set(self.list_calls.get() + 1);
        Ok(self.records.clone())
    }
}

#[test]
fn loads_one_owned_snapshot_in_canonical_source_order() {
    let first = photo(1, 11, 1);
    let second = photo(2, 22, 2);
    let first_record = record("z/second.png", first.clone());
    let second_record = record("a/first.png", second.clone());
    let state = state_with_photos([first, second]);
    let repository = ListingRepository {
        records: vec![first_record, second_record],
        list_calls: Cell::new(0),
    };

    let snapshot = CatalogSnapshot::load(&state, &repository).expect("snapshot loads");

    assert_eq!(repository.list_calls.get(), 1);
    assert_eq!(snapshot.revision(), state.revision());
    assert_eq!(
        snapshot
            .entries()
            .map(|entry| entry.source().to_string())
            .collect::<Vec<_>>(),
        ["a/first.png", "z/second.png"]
    );
    assert_eq!(
        snapshot
            .by_source(&SourcePath::new("a/first.png").unwrap())
            .unwrap()
            .photo(),
        &photo(2, 22, 2)
    );
    assert_eq!(
        snapshot
            .by_photo_id(PhotoId::new(1).unwrap())
            .unwrap()
            .source(),
        &SourcePath::new("z/second.png").unwrap()
    );
    assert!(
        snapshot
            .by_source(&SourcePath::new("missing.png").unwrap())
            .is_none()
    );
    assert!(snapshot.by_photo_id(PhotoId::new(99).unwrap()).is_none());
}

#[test]
fn associates_all_matching_edits_in_edit_id_order_without_selecting_current() {
    let photo = photo(1, 11, 1);
    let record = record("one.png", photo.clone());
    let mut state = state_with_photos([photo]);
    state
        .apply(state.revision(), CatalogCommand::CreateEdit(edit(4, 1)))
        .expect("first edit");
    state
        .apply(state.revision(), CatalogCommand::CreateEdit(edit(2, 1)))
        .expect("second edit");
    let repository = ListingRepository {
        records: vec![record],
        list_calls: Cell::new(0),
    };

    let snapshot = CatalogSnapshot::load(&state, &repository).expect("snapshot loads");
    let entry = snapshot.entries().next().expect("entry");

    assert_eq!(
        entry.edits().map(Edit::id).collect::<Vec<_>>(),
        [EditId::new(2).unwrap(), EditId::new(4).unwrap()]
    );
}

#[test]
fn snapshot_owns_values_after_inputs_are_dropped() {
    let snapshot = {
        let photo = photo(1, 11, 1);
        let state = state_with_photos([photo.clone()]);
        let repository = ListingRepository {
            records: vec![record("owned.png", photo)],
            list_calls: Cell::new(0),
        };
        CatalogSnapshot::load(&state, &repository).expect("snapshot loads")
    };

    let entry = snapshot
        .by_source(&SourcePath::new("owned.png").unwrap())
        .unwrap();
    assert_eq!(entry.photo().id(), PhotoId::new(1).unwrap());
    assert_eq!(entry.metadata().len(), 0);
}
