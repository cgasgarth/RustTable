use rusttable_catalog::{CatalogRestoreError, CatalogState};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, Photo, PhotoId, Revision,
};

fn photo(id: u128, revision: u64, asset_id: u128) -> Photo {
    Photo::from_parts(
        PhotoId::new(id).unwrap(),
        Revision::from_u64(revision),
        [Asset::new(
            AssetId::new(asset_id).unwrap(),
            AssetRole::Primary,
            ContentHash::Sha256([u8::try_from(asset_id).expect("test asset ID fits"); 32]),
            ByteLength::from_bytes(1),
        )],
    )
    .unwrap()
}

fn edit(id: u128, photo_id: u128, base_revision: u64, revision: u64) -> Edit {
    Edit::from_parts(
        EditId::new(id).unwrap(),
        PhotoId::new(photo_id).unwrap(),
        Revision::from_u64(base_revision),
        Revision::from_u64(revision),
        [],
    )
    .unwrap()
}

#[test]
fn restoration_errors_follow_the_documented_precedence() {
    let duplicate_photo =
        CatalogState::restore([photo(1, 0, 11), photo(1, 4, 12)], Vec::<Edit>::new()).unwrap_err();
    assert_eq!(
        duplicate_photo,
        CatalogRestoreError::DuplicatePhoto {
            photo_id: PhotoId::new(1).unwrap()
        }
    );

    let duplicate_asset =
        CatalogState::restore([photo(1, 0, 11), photo(2, 0, 11)], Vec::<Edit>::new()).unwrap_err();
    assert!(matches!(
        duplicate_asset,
        CatalogRestoreError::AssetIdConflict {
            asset_id,
            existing_photo_id,
            conflicting_photo_id
        } if asset_id == AssetId::new(11).unwrap()
            && existing_photo_id == PhotoId::new(1).unwrap()
            && conflicting_photo_id == PhotoId::new(2).unwrap()
    ));

    let duplicate_edit = CatalogState::restore(
        [photo(1, 0, 11), photo(2, 0, 22)],
        [edit(5, 1, 0, 0), edit(5, 2, 0, 0)],
    )
    .unwrap_err();
    assert_eq!(
        duplicate_edit,
        CatalogRestoreError::DuplicateEdit {
            edit_id: EditId::new(5).unwrap()
        }
    );

    let unknown_photo = CatalogState::restore([photo(1, 0, 11)], [edit(5, 9, 0, 0)]).unwrap_err();
    assert_eq!(
        unknown_photo,
        CatalogRestoreError::UnknownPhoto {
            edit_id: EditId::new(5).unwrap(),
            photo_id: PhotoId::new(9).unwrap(),
        }
    );

    let base_conflict = CatalogState::restore([photo(1, 4, 11)], [edit(5, 1, 3, 0)]).unwrap_err();
    assert_eq!(
        base_conflict,
        CatalogRestoreError::EditBasePhotoRevisionConflict {
            edit_id: EditId::new(5).unwrap(),
            photo_id: PhotoId::new(1).unwrap(),
            expected: Revision::from_u64(4),
            actual: Revision::from_u64(3),
        }
    );
}

#[test]
fn restoration_revision_overflow_is_typed_and_atomic() {
    let photo_overflow =
        CatalogState::restore([photo(1, u64::MAX, 11)], Vec::<Edit>::new()).unwrap_err();
    assert!(matches!(
        photo_overflow,
        CatalogRestoreError::RevisionOverflow { .. }
    ));

    let cumulative_overflow = CatalogState::restore(
        [photo(1, u64::MAX - 1, 11), photo(2, 0, 22)],
        Vec::<Edit>::new(),
    )
    .unwrap_err();
    assert!(matches!(
        cumulative_overflow,
        CatalogRestoreError::RevisionOverflow { .. }
    ));
}

#[test]
fn unknown_photo_precedes_any_base_revision_mismatch() {
    let error = CatalogState::restore(
        [photo(1, 4, 11)],
        [edit(1, 1, 3, 0), edit(2, 9, 0, u64::MAX)],
    )
    .unwrap_err();

    assert_eq!(
        error,
        CatalogRestoreError::UnknownPhoto {
            edit_id: EditId::new(2).unwrap(),
            photo_id: PhotoId::new(9).unwrap(),
        }
    );
}
