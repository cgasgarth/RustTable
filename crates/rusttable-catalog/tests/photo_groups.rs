use rusttable_catalog::{
    CatalogCommand, CatalogError, CatalogState, PhotoGroupCommand, PhotoGroupId,
};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Photo, PhotoId, Revision,
};

fn photo(id: u128) -> Photo {
    let photo_id = PhotoId::new(id).unwrap();
    Photo::new(
        photo_id,
        [Asset::new(
            AssetId::new(id + 100).unwrap(),
            AssetRole::Primary,
            ContentHash::Sha256([u8::try_from(id).unwrap(); 32]),
            ByteLength::ZERO,
        )],
    )
    .unwrap()
}

fn state() -> CatalogState {
    let mut state = CatalogState::new();
    for id in 1..=4 {
        state
            .apply(state.revision(), CatalogCommand::RegisterPhoto(photo(id)))
            .unwrap();
    }
    state
}

#[test]
fn group_identity_survives_representative_changes() {
    let mut state = state();
    let group_id = PhotoGroupId::new(77).unwrap();
    state
        .apply_photo_group_command(
            state.revision(),
            PhotoGroupCommand::Create {
                group_id,
                photo_ids: vec![PhotoId::new(3).unwrap(), PhotoId::new(1).unwrap()],
                representative: Some(PhotoId::new(3).unwrap()),
            },
        )
        .unwrap();
    state
        .apply_photo_group_command(
            state.revision(),
            PhotoGroupCommand::SetRepresentative {
                group_id,
                photo_id: PhotoId::new(1).unwrap(),
            },
        )
        .unwrap();

    let group = state.photo_group(group_id).unwrap();
    assert_eq!(group.id(), group_id);
    assert_eq!(
        group.members(),
        [PhotoId::new(1).unwrap(), PhotoId::new(3).unwrap()]
    );
    assert_eq!(group.representative(), Some(PhotoId::new(1).unwrap()));
}

#[test]
fn membership_order_and_representative_invariants_are_deterministic() {
    let mut state = state();
    let group_id = PhotoGroupId::new(88).unwrap();
    state
        .apply_photo_group_command(
            state.revision(),
            PhotoGroupCommand::Create {
                group_id,
                photo_ids: vec![PhotoId::new(4).unwrap(), PhotoId::new(2).unwrap()],
                representative: Some(PhotoId::new(4).unwrap()),
            },
        )
        .unwrap();
    state
        .apply_photo_group_command(
            state.revision(),
            PhotoGroupCommand::AddMembers {
                group_id,
                photo_ids: vec![PhotoId::new(1).unwrap()],
            },
        )
        .unwrap();
    assert_eq!(
        state.photo_group(group_id).unwrap().members(),
        [
            PhotoId::new(1).unwrap(),
            PhotoId::new(2).unwrap(),
            PhotoId::new(4).unwrap()
        ]
    );
    assert_eq!(
        state
            .query(rusttable_catalog::CatalogQuery::default())
            .into_iter()
            .filter(|projection| projection.group_id == Some(group_id))
            .map(|projection| (projection.photo_id, projection.is_representative))
            .collect::<Vec<_>>(),
        [
            (PhotoId::new(1).unwrap(), false),
            (PhotoId::new(2).unwrap(), false),
            (PhotoId::new(4).unwrap(), true),
        ]
    );
}

#[test]
fn removing_the_representative_and_then_every_member_is_atomic_and_valid() {
    let mut state = state();
    let group_id = PhotoGroupId::new(99).unwrap();
    state
        .apply_photo_group_command(
            state.revision(),
            PhotoGroupCommand::Create {
                group_id,
                photo_ids: vec![PhotoId::new(1).unwrap(), PhotoId::new(2).unwrap()],
                representative: Some(PhotoId::new(1).unwrap()),
            },
        )
        .unwrap();
    state
        .apply_photo_group_command(
            state.revision(),
            PhotoGroupCommand::RemoveMembers {
                group_id,
                photo_ids: vec![PhotoId::new(1).unwrap()],
            },
        )
        .unwrap();
    assert_eq!(
        state.photo_group(group_id).unwrap().representative(),
        Some(PhotoId::new(2).unwrap())
    );
    let before = state.clone();
    let error = state
        .apply_photo_group_command(
            state.revision(),
            PhotoGroupCommand::RemoveMembers {
                group_id,
                photo_ids: vec![PhotoId::new(2).unwrap(), PhotoId::new(2).unwrap()],
            },
        )
        .unwrap_err();
    assert!(matches!(error, CatalogError::PhotoGroup(_)));
    assert_eq!(state, before);
    state
        .apply_photo_group_command(
            state.revision(),
            PhotoGroupCommand::RemoveMembers {
                group_id,
                photo_ids: vec![PhotoId::new(2).unwrap()],
            },
        )
        .unwrap();
    assert_eq!(state.photo_group(group_id).unwrap().members(), &[]);
    assert_eq!(state.photo_group(group_id).unwrap().representative(), None);
}

#[test]
fn empty_group_can_be_created_and_deleted_without_selection_state() {
    let mut state = state();
    let group_id = PhotoGroupId::new(123).unwrap();
    state
        .apply_photo_group_command(
            state.revision(),
            PhotoGroupCommand::Create {
                group_id,
                photo_ids: Vec::new(),
                representative: None,
            },
        )
        .unwrap();
    assert_eq!(state.photo_group(group_id).unwrap().representative(), None);
    state
        .apply_photo_group_command(state.revision(), PhotoGroupCommand::Delete { group_id })
        .unwrap();
    assert!(state.photo_group(group_id).is_none());
    assert_eq!(state.revision(), Revision::from_u64(6));
}
