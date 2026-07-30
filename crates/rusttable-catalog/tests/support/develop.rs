use std::collections::BTreeMap;

use rusttable_catalog::{
    CatalogCommand, CatalogSnapshot, CatalogState, ImportCandidate, ImportRecord, ImportRepository,
    RepositoryError, SourcePath,
};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, ImageMetadata, Photo,
    PhotoId, Revision,
};
use rusttable_image::{ImageDimensions, ImageProbe, InputFormat};

pub fn photo(id: u128, asset_id: u128, byte: u8) -> Photo {
    Photo::new(
        PhotoId::new(id).unwrap(),
        [Asset::new(
            AssetId::new(asset_id).unwrap(),
            AssetRole::Primary,
            ContentHash::Sha256([byte; 32]),
            ByteLength::from_bytes(8),
        )],
    )
    .unwrap()
}

pub fn edit(id: u128, photo_id: u128) -> Edit {
    Edit::new(
        EditId::new(id).unwrap(),
        PhotoId::new(photo_id).unwrap(),
        Revision::ZERO,
        [],
    )
    .unwrap()
}

pub fn record(source: &str, photo: Photo) -> ImportRecord {
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

#[derive(Default)]
pub struct FakeRepository {
    pub records: BTreeMap<SourcePath, ImportRecord>,
}

impl ImportRepository for FakeRepository {
    fn find_by_source(
        &self,
        _source: &SourcePath,
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        unreachable!("snapshot resolution does not access the repository")
    }

    fn find_by_photo_id(
        &self,
        _photo_id: PhotoId,
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        unreachable!("snapshot resolution does not access the repository")
    }

    fn find_by_asset_id(
        &self,
        _asset_id: AssetId,
    ) -> Result<Option<ImportRecord>, RepositoryError> {
        unreachable!("snapshot resolution does not access the repository")
    }

    fn commit(&mut self, _record: &ImportRecord) -> Result<(), RepositoryError> {
        unreachable!("snapshot resolution does not access the repository")
    }

    fn list(&self) -> Result<Vec<ImportRecord>, RepositoryError> {
        Ok(self.records.values().cloned().collect())
    }
}

pub fn fixture() -> (
    CatalogSnapshot,
    Revision,
    PhotoId,
    EditId,
    EditId,
    PhotoId,
    EditId,
) {
    let first = photo(1, 11, 1);
    let second = photo(2, 22, 2);
    let mut state = CatalogState::new();
    state
        .apply(
            state.revision(),
            CatalogCommand::RegisterPhoto(first.clone()),
        )
        .unwrap();
    state
        .apply(
            state.revision(),
            CatalogCommand::RegisterPhoto(second.clone()),
        )
        .unwrap();
    state
        .apply(state.revision(), CatalogCommand::CreateEdit(edit(4, 1)))
        .unwrap();
    state
        .apply(state.revision(), CatalogCommand::CreateEdit(edit(2, 1)))
        .unwrap();
    state
        .apply(state.revision(), CatalogCommand::CreateEdit(edit(6, 2)))
        .unwrap();

    let mut repository = FakeRepository::default();
    repository.records.insert(
        SourcePath::new("second.png").unwrap(),
        record("second.png", second),
    );
    repository.records.insert(
        SourcePath::new("first.png").unwrap(),
        record("first.png", first),
    );
    let revision = state.revision();
    let snapshot = CatalogSnapshot::load(&state, &repository).unwrap();
    (
        snapshot,
        revision,
        PhotoId::new(1).unwrap(),
        EditId::new(2).unwrap(),
        EditId::new(4).unwrap(),
        PhotoId::new(2).unwrap(),
        EditId::new(6).unwrap(),
    )
}
