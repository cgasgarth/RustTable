mod support;

use rusttable_catalog::{ImportRepository, RepositoryError};
use rusttable_catalog_store::RedbImportRepository;

#[test]
fn source_photo_and_asset_conflicts_are_typed_and_atomic() {
    let path = support::temp_path("conflicts");
    let first = support::record("one.raw", 1, 2, 1);
    let source = support::record("one.raw", 3, 4, 2);
    let photo = support::record("two.raw", 1, 5, 3);
    let asset = support::record("three.raw", 6, 2, 4);
    let mut repository = RedbImportRepository::open(&path).unwrap();
    repository.commit(&first).unwrap();
    assert_eq!(
        repository.commit(&source),
        Err(RepositoryError::SourceConflict {
            source: source.source().clone()
        })
    );
    assert_eq!(
        repository.commit(&photo),
        Err(RepositoryError::PhotoIdConflict {
            photo_id: photo.photo().id()
        })
    );
    assert_eq!(
        repository.commit(&asset),
        Err(RepositoryError::AssetIdConflict {
            asset_id: asset.photo().primary_asset_id()
        })
    );
    assert_eq!(repository.list().unwrap(), vec![first]);
    support::remove(&path);
}
