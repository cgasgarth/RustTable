use rusttable_catalog::{CatalogCommand, CatalogState};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, Photo, PhotoId, Revision,
};

fn photo(id: u128, revision: u64, assets: &[(u128, AssetRole)]) -> Photo {
    Photo::from_parts(
        PhotoId::new(id).expect("nonzero photo ID"),
        Revision::from_u64(revision),
        assets.iter().map(|(asset_id, role)| {
            Asset::new(
                AssetId::new(*asset_id).expect("nonzero asset ID"),
                *role,
                ContentHash::Sha256([u8::try_from(*asset_id).expect("test asset ID fits"); 32]),
                ByteLength::from_bytes(8),
            )
        }),
    )
    .expect("valid photo")
}

fn edit(id: u128, photo_id: u128, revision: u64) -> Edit {
    Edit::from_parts(
        EditId::new(id).expect("nonzero edit ID"),
        PhotoId::new(photo_id).expect("nonzero photo ID"),
        Revision::ZERO,
        Revision::from_u64(revision),
        [],
    )
    .expect("valid edit")
}

#[test]
fn empty_restore_matches_new_state() {
    assert_eq!(
        CatalogState::restore(Vec::<Photo>::new(), Vec::<Edit>::new()).unwrap(),
        CatalogState::new()
    );
}

#[test]
fn restore_reproduces_current_state_from_shuffled_history_values() {
    let first = photo(1, 0, &[(11, AssetRole::Primary), (12, AssetRole::Sidecar)]);
    let second = photo(
        2,
        0,
        &[(21, AssetRole::Primary), (22, AssetRole::Auxiliary)],
    );
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

    let first_edit = edit(101, 1, 0);
    let second_edit = edit(202, 2, 0);
    state
        .apply(
            state.revision(),
            CatalogCommand::CreateEdit(first_edit.clone()),
        )
        .unwrap();
    state
        .apply(
            state.revision(),
            CatalogCommand::CreateEdit(second_edit.clone()),
        )
        .unwrap();
    let first_replacement = edit(101, 1, 1);
    state
        .apply(
            state.revision(),
            CatalogCommand::ReplaceEdit {
                edit_id: first_replacement.id(),
                expected_edit_revision: first_edit.revision(),
                replacement: first_replacement,
            },
        )
        .unwrap();
    let second_replacement = edit(101, 1, 2);
    state
        .apply(
            state.revision(),
            CatalogCommand::ReplaceEdit {
                edit_id: second_replacement.id(),
                expected_edit_revision: Revision::from_u64(1),
                replacement: second_replacement,
            },
        )
        .unwrap();

    let photos = state.photos().cloned().collect::<Vec<_>>();
    let edits = state.edits().cloned().collect::<Vec<_>>();
    let restored = CatalogState::restore(photos.into_iter().rev(), edits.into_iter().rev())
        .expect("current values restore");

    assert_eq!(restored, state);
    assert_eq!(restored.revision(), Revision::from_u64(6));
    assert_eq!(
        restored.photos().map(Photo::id).collect::<Vec<_>>(),
        vec![first.id(), second.id()]
    );
    assert_eq!(
        restored.edits().map(Edit::id).collect::<Vec<_>>(),
        vec![first_edit.id(), second_edit.id()]
    );
    assert_eq!(
        restored.asset_owner(AssetId::new(11).unwrap()),
        Some(first.id())
    );
    assert_eq!(
        restored.asset_owner(AssetId::new(12).unwrap()),
        Some(first.id())
    );
    assert_eq!(
        restored.asset_owner(AssetId::new(21).unwrap()),
        Some(second.id())
    );
    assert_eq!(
        restored.asset_owner(AssetId::new(22).unwrap()),
        Some(second.id())
    );
    assert_eq!(restored.asset_owner(AssetId::new(99).unwrap()), None);
}

#[test]
fn restore_accepts_nonzero_current_aggregate_revisions() {
    let photo = photo(1, 4, &[(11, AssetRole::Primary)]);
    let edit = Edit::from_parts(
        EditId::new(2).unwrap(),
        photo.id(),
        photo.revision(),
        Revision::from_u64(3),
        [],
    )
    .unwrap();

    let state = CatalogState::restore([photo.clone()], [edit.clone()]).unwrap();

    assert_eq!(state.photo(photo.id()), Some(&photo));
    assert_eq!(state.edit(edit.id()), Some(&edit));
    assert_eq!(state.revision(), Revision::from_u64(9));
}
