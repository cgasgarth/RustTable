mod support;

use rusttable_catalog::{ImportRepository, PhotoGroupCommand, PhotoGroupId};
use rusttable_catalog_store::{AtomicCatalogStoreError, RedbCatalogRepository};
use rusttable_core::PhotoId;

fn seed(path: &std::path::Path) {
    let mut repository = RedbCatalogRepository::open(path).unwrap();
    for (index, source) in ["a.raw", "b.raw", "c.raw"].into_iter().enumerate() {
        repository
            .commit(&support::record(
                source,
                u128::try_from(index + 1).unwrap(),
                u128::try_from(index + 11).unwrap(),
                u8::try_from(index + 1).unwrap(),
            ))
            .unwrap();
    }
}

fn photo(id: u128) -> PhotoId {
    PhotoId::new(id).unwrap()
}

#[test]
fn groups_persist_identity_order_and_representative_across_restart() {
    let path = support::temp_path("photo-groups-restart");
    seed(&path);
    let group_id = PhotoGroupId::new(700).unwrap();
    {
        let mut repository = RedbCatalogRepository::open(&path).unwrap();
        repository
            .apply_photo_group_command(&PhotoGroupCommand::Create {
                group_id,
                photo_ids: vec![photo(3), photo(1)],
                representative: Some(photo(3)),
            })
            .unwrap();
        repository
            .apply_photo_group_command(&PhotoGroupCommand::SetRepresentative {
                group_id,
                photo_id: photo(1),
            })
            .unwrap();
    }

    let repository = RedbCatalogRepository::open(&path).unwrap();
    assert_eq!(
        repository.photo_group_projections().unwrap(),
        [rusttable_catalog::PhotoGroupProjection {
            group_id,
            member_ids: vec![photo(1), photo(3)],
            representative: Some(photo(1)),
        }]
    );
    assert_eq!(
        repository.photo_group_for(photo(3)).unwrap(),
        Some(group_id)
    );
    drop(repository);
    support::remove(&path);
}

#[test]
fn empty_creation_membership_and_deletion_are_transactional() {
    let path = support::temp_path("photo-groups-empty");
    seed(&path);
    let group_id = PhotoGroupId::new(701).unwrap();
    let mut repository = RedbCatalogRepository::open(&path).unwrap();
    repository
        .apply_photo_group_command(&PhotoGroupCommand::Create {
            group_id,
            photo_ids: Vec::new(),
            representative: None,
        })
        .unwrap();
    repository
        .apply_photo_group_command(&PhotoGroupCommand::AddMembers {
            group_id,
            photo_ids: vec![photo(2)],
        })
        .unwrap();
    assert_eq!(
        repository
            .photo_group(group_id)
            .unwrap()
            .unwrap()
            .representative(),
        Some(photo(2))
    );
    repository
        .apply_photo_group_command(&PhotoGroupCommand::Delete { group_id })
        .unwrap();
    assert!(repository.photo_group(group_id).unwrap().is_none());
    assert_eq!(repository.photo_group_for(photo(2)).unwrap(), None);
    drop(repository);
    support::remove(&path);
}

#[test]
fn failed_group_commit_rolls_back_rows_indexes_and_revision() {
    let path = support::temp_path("photo-groups-rollback");
    seed(&path);
    let group_id = PhotoGroupId::new(702).unwrap();
    let mut repository = RedbCatalogRepository::open(&path).unwrap();
    repository
        .apply_photo_group_command(&PhotoGroupCommand::Create {
            group_id,
            photo_ids: vec![photo(1), photo(2)],
            representative: Some(photo(1)),
        })
        .unwrap();
    let before = repository.photo_group_projections().unwrap();
    let before_revision = repository.organization_revision().unwrap();
    drop(repository);

    let mut failing = RedbCatalogRepository::open_with_before_commit_hook(&path, || {
        Err(AtomicCatalogStoreError::CommitFailed)
    })
    .unwrap();
    assert_eq!(
        failing.apply_photo_group_command(&PhotoGroupCommand::SetRepresentative {
            group_id,
            photo_id: photo(2),
        }),
        Err(AtomicCatalogStoreError::CommitFailed)
    );
    drop(failing);

    let repository = RedbCatalogRepository::open(&path).unwrap();
    assert_eq!(repository.photo_group_projections().unwrap(), before);
    assert_eq!(repository.organization_revision().unwrap(), before_revision);
    assert_eq!(
        repository.photo_group_for(photo(2)).unwrap(),
        Some(group_id)
    );
    drop(repository);
    support::remove(&path);
}
