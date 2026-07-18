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
            ByteLength::ZERO,
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
        .apply(Revision::ZERO, CatalogCommand::RegisterPhoto(photo(1, 4)))
        .unwrap();
    state
        .apply(
            Revision::from_u64(1),
            CatalogCommand::CreateEdit(edit(2, 1, 0, 4)),
        )
        .unwrap();
    state
}

#[test]
fn replacement_advances_edit_and_catalog_revisions_atomically() {
    let mut state = registered_state();
    let replacement = edit(2, 1, 1, 4);

    let revision = state
        .apply(
            Revision::from_u64(2),
            CatalogCommand::ReplaceEdit {
                edit_id: EditId::new(2).unwrap(),
                expected_edit_revision: Revision::ZERO,
                replacement,
            },
        )
        .unwrap();

    assert_eq!(revision, Revision::from_u64(3));
    assert_eq!(state.edit(EditId::new(2).unwrap()), Some(&edit(2, 1, 1, 4)));
    assert_eq!(state.revision(), Revision::from_u64(3));
}

#[test]
fn replacement_validation_precedence_is_explicit_and_atomic() {
    let mut state = registered_state();
    let before = state.clone();
    let unknown = state
        .apply(
            Revision::from_u64(2),
            CatalogCommand::ReplaceEdit {
                edit_id: EditId::new(99).unwrap(),
                expected_edit_revision: Revision::ZERO,
                replacement: edit(3, 99, 9, 99),
            },
        )
        .unwrap_err();
    assert!(matches!(unknown, CatalogError::UnknownEdit { .. }));
    assert_eq!(state, before);

    let before = state.clone();
    let id_mismatch = state
        .apply(
            Revision::from_u64(2),
            CatalogCommand::ReplaceEdit {
                edit_id: EditId::new(2).unwrap(),
                expected_edit_revision: Revision::ZERO,
                replacement: edit(3, 1, 1, 4),
            },
        )
        .unwrap_err();
    assert!(matches!(id_mismatch, CatalogError::EditIdMismatch { .. }));
    assert_eq!(state, before);

    let before = state.clone();
    let photo_mismatch = state
        .apply(
            Revision::from_u64(2),
            CatalogCommand::ReplaceEdit {
                edit_id: EditId::new(2).unwrap(),
                expected_edit_revision: Revision::ZERO,
                replacement: edit(2, 9, 1, 4),
            },
        )
        .unwrap_err();
    assert!(matches!(
        photo_mismatch,
        CatalogError::EditPhotoMismatch { .. }
    ));
    assert_eq!(state, before);

    let before = state.clone();
    let base_mismatch = state
        .apply(
            Revision::from_u64(2),
            CatalogCommand::ReplaceEdit {
                edit_id: EditId::new(2).unwrap(),
                expected_edit_revision: Revision::ZERO,
                replacement: edit(2, 1, 1, 9),
            },
        )
        .unwrap_err();
    assert!(matches!(
        base_mismatch,
        CatalogError::EditBasePhotoRevisionMismatch { .. }
    ));
    assert_eq!(state, before);
}

#[test]
fn replacement_revision_checks_and_catalog_conflicts_leave_state_unchanged() {
    let mut state = registered_state();
    let before = state.clone();
    let stale_edit = state
        .apply(
            Revision::from_u64(2),
            CatalogCommand::ReplaceEdit {
                edit_id: EditId::new(2).unwrap(),
                expected_edit_revision: Revision::from_u64(4),
                replacement: edit(2, 1, 1, 4),
            },
        )
        .unwrap_err();
    assert!(matches!(
        stale_edit,
        CatalogError::EditRevisionConflict { .. }
    ));
    assert_eq!(state, before);

    let before = state.clone();
    let skipped = state
        .apply(
            Revision::from_u64(2),
            CatalogCommand::ReplaceEdit {
                edit_id: EditId::new(2).unwrap(),
                expected_edit_revision: Revision::ZERO,
                replacement: edit(2, 1, 2, 4),
            },
        )
        .unwrap_err();
    assert!(matches!(
        skipped,
        CatalogError::InvalidEditRevisionAdvance { .. }
    ));
    assert_eq!(state, before);

    let before = state.clone();
    let conflict = state
        .apply(
            Revision::ZERO,
            CatalogCommand::ReplaceEdit {
                edit_id: EditId::new(2).unwrap(),
                expected_edit_revision: Revision::ZERO,
                replacement: edit(2, 1, 1, 4),
            },
        )
        .unwrap_err();
    assert!(matches!(
        conflict,
        CatalogError::CatalogRevisionConflict { .. }
    ));
    assert_eq!(state, before);
}
