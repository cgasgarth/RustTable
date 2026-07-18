use rusttable_catalog::{CatalogCommand, CatalogState};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, Operation, OperationId,
    OperationKey, Photo, PhotoId, Revision,
};

fn photo(id: u128, revision: u64) -> Photo {
    Photo::from_parts(
        PhotoId::new(id).expect("nonzero photo ID"),
        Revision::from_u64(revision),
        [Asset::new(
            AssetId::new(id).expect("nonzero asset ID"),
            AssetRole::Primary,
            ContentHash::Sha256([id.to_le_bytes()[0]; 32]),
            ByteLength::from_bytes(1),
        )],
    )
    .expect("valid photo")
}

fn edit(id: u128, photo_id: u128, base_revision: u64) -> Edit {
    Edit::new(
        EditId::new(id).expect("nonzero edit ID"),
        PhotoId::new(photo_id).expect("nonzero photo ID"),
        Revision::from_u64(base_revision),
        [Operation::new(
            OperationId::new(id).expect("nonzero operation ID"),
            OperationKey::new("rusttable.exposure").expect("valid operation key"),
            true,
            [],
        )
        .expect("valid operation")],
    )
    .expect("valid edit")
}

#[test]
fn new_state_registers_a_photo_atomically() {
    let mut state = CatalogState::new();
    let value = photo(1, 0);
    let photo_id = value.id();

    let revision = state
        .apply(Revision::ZERO, CatalogCommand::RegisterPhoto(value))
        .expect("registration succeeds");

    assert_eq!(revision, Revision::from_u64(1));
    assert_eq!(state.revision(), revision);
    assert_eq!(state.photo(photo_id), Some(&photo(1, 0)));
    assert_eq!(
        state.photos().map(Photo::id).collect::<Vec<_>>(),
        vec![photo_id]
    );
    assert!(state.edits().next().is_none());
}

#[test]
fn valid_initial_edit_requires_the_registered_photo_revision() {
    let mut state = CatalogState::new();
    let value = photo(1, 4);
    state
        .apply(Revision::ZERO, CatalogCommand::RegisterPhoto(value))
        .expect("registration succeeds");
    let value = edit(2, 1, 4);
    let edit_id = value.id();

    let revision = state
        .apply(Revision::from_u64(1), CatalogCommand::CreateEdit(value))
        .expect("edit registration succeeds");

    assert_eq!(revision, Revision::from_u64(2));
    assert_eq!(state.edit(edit_id), Some(&edit(2, 1, 4)));
    assert_eq!(
        state.edits().map(Edit::id).collect::<Vec<_>>(),
        vec![edit_id]
    );
}

#[test]
fn equal_registrations_are_canonical_across_input_order() {
    let first = photo(1, 0);
    let second = photo(2, 0);
    let mut left = CatalogState::new();
    let mut right = CatalogState::new();

    left.apply(Revision::ZERO, CatalogCommand::RegisterPhoto(first))
        .expect("registration succeeds");
    left.apply(Revision::from_u64(1), CatalogCommand::RegisterPhoto(second))
        .expect("registration succeeds");
    right
        .apply(Revision::ZERO, CatalogCommand::RegisterPhoto(photo(2, 0)))
        .expect("registration succeeds");
    right
        .apply(
            Revision::from_u64(1),
            CatalogCommand::RegisterPhoto(photo(1, 0)),
        )
        .expect("registration succeeds");

    assert_eq!(
        left.photos().map(Photo::id).collect::<Vec<_>>(),
        right.photos().map(Photo::id).collect::<Vec<_>>()
    );
    assert_eq!(left, right);
}
