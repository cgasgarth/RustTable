use rusttable_catalog::{CatalogCommand, CatalogError, CatalogState};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, Operation, OperationId,
    OperationKey, Photo, PhotoId, Revision,
};

fn photo(id: u128, revision: u64) -> Photo {
    Photo::from_parts(
        PhotoId::new(id).unwrap(),
        Revision::from_u64(revision),
        [Asset::new(
            AssetId::new(id).unwrap(),
            AssetRole::Primary,
            ContentHash::Sha256([id.to_le_bytes()[0]; 32]),
            ByteLength::from_bytes(1),
        )],
    )
    .unwrap()
}

fn edit(id: u128, photo_id: u128, edit_revision: u64, base_revision: u64) -> Edit {
    Edit::from_parts(
        EditId::new(id).unwrap(),
        PhotoId::new(photo_id).unwrap(),
        Revision::from_u64(base_revision),
        Revision::from_u64(edit_revision),
        [Operation::new(
            OperationId::new(id).unwrap(),
            OperationKey::new("rusttable.exposure").unwrap(),
            true,
            [],
        )
        .unwrap()],
    )
    .unwrap()
}

fn registered_state() -> CatalogState {
    let mut state = CatalogState::new();
    state
        .apply(Revision::ZERO, CatalogCommand::RegisterPhoto(photo(1, 0)))
        .unwrap();
    state
}

#[test]
fn duplicate_photo_and_stale_revision_fail_without_mutation() {
    let mut state = registered_state();
    let before = state.clone();
    let duplicate = state
        .apply(
            Revision::from_u64(1),
            CatalogCommand::RegisterPhoto(photo(1, 0)),
        )
        .unwrap_err();
    assert!(matches!(duplicate, CatalogError::DuplicatePhoto { .. }));
    assert_eq!(state, before);

    let before = state.clone();
    let conflict = state
        .apply(Revision::ZERO, CatalogCommand::RegisterPhoto(photo(2, 0)))
        .unwrap_err();
    assert!(matches!(
        conflict,
        CatalogError::CatalogRevisionConflict { .. }
    ));
    assert_eq!(state, before);
}

#[test]
fn create_edit_validation_has_documented_precedence() {
    let mut state = registered_state();
    let before = state.clone();

    let duplicate = state
        .apply(
            Revision::from_u64(1),
            CatalogCommand::CreateEdit(edit(2, 1, 4, 0)),
        )
        .unwrap_err();
    assert!(matches!(
        duplicate,
        CatalogError::InvalidInitialEditRevision { .. }
    ));
    assert_eq!(state, before);

    let unknown = state
        .apply(
            Revision::from_u64(1),
            CatalogCommand::CreateEdit(edit(2, 9, 0, 0)),
        )
        .unwrap_err();
    assert!(matches!(unknown, CatalogError::UnknownPhoto { .. }));
    assert_eq!(state, before);

    let base_conflict = state
        .apply(
            Revision::from_u64(1),
            CatalogCommand::CreateEdit(edit(2, 1, 0, 1)),
        )
        .unwrap_err();
    assert!(matches!(
        base_conflict,
        CatalogError::EditBasePhotoRevisionConflict { .. }
    ));
    assert_eq!(state, before);
}

#[test]
fn duplicate_edit_is_rejected_before_photo_and_revision_checks() {
    let mut state = registered_state();
    state
        .apply(
            Revision::from_u64(1),
            CatalogCommand::CreateEdit(edit(2, 1, 0, 0)),
        )
        .unwrap();
    let before = state.clone();

    let error = state
        .apply(
            Revision::from_u64(2),
            CatalogCommand::CreateEdit(edit(2, 999, 4, 99)),
        )
        .unwrap_err();
    assert!(matches!(error, CatalogError::DuplicateEdit { .. }));
    assert_eq!(state, before);
}
