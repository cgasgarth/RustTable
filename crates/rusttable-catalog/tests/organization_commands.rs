use rusttable_catalog::{CatalogCommand, CatalogQuery, CatalogState, ColorLabel, Rating};
use rusttable_core::{
    Asset, AssetId, AssetRole, ByteLength, ContentHash, Photo, PhotoId, Revision,
};

fn photo(id: u128) -> Photo {
    let id = PhotoId::new(id).unwrap();
    Photo::new(
        id,
        [Asset::new(
            AssetId::new(id.get() + 100).unwrap(),
            AssetRole::Primary,
            ContentHash::Sha256([u8::try_from(id.get()).expect("test ID fits in a byte"); 32]),
            ByteLength::ZERO,
        )],
    )
    .unwrap()
}

fn state() -> CatalogState {
    let mut state = CatalogState::new();
    state
        .apply(Revision::ZERO, CatalogCommand::RegisterPhoto(photo(1)))
        .unwrap();
    state
        .apply(
            Revision::from_u64(1),
            CatalogCommand::RegisterPhoto(photo(2)),
        )
        .unwrap();
    state
}

#[test]
fn batch_commands_are_atomic_and_indexes_are_deterministic() {
    let mut state = state();
    let revision = state
        .apply(
            state.revision(),
            CatalogCommand::SetRating {
                photo_ids: vec![PhotoId::new(2).unwrap(), PhotoId::new(1).unwrap()],
                rating: Rating::Five,
            },
        )
        .unwrap();
    state
        .apply(
            revision,
            CatalogCommand::SetRejection {
                photo_ids: vec![PhotoId::new(1).unwrap()],
                rejected: true,
            },
        )
        .unwrap();
    state
        .apply(
            state.revision(),
            CatalogCommand::SetColorLabel {
                photo_ids: vec![PhotoId::new(1).unwrap(), PhotoId::new(2).unwrap()],
                label: ColorLabel::Red,
                enabled: true,
            },
        )
        .unwrap();

    assert_eq!(
        state.photos_with_rating(Rating::Five).collect::<Vec<_>>(),
        [PhotoId::new(1).unwrap(), PhotoId::new(2).unwrap()]
    );
    assert_eq!(
        state.rejected_photos().collect::<Vec<_>>(),
        [PhotoId::new(1).unwrap()]
    );
    assert_eq!(
        state
            .photos_with_color_label(ColorLabel::Red)
            .collect::<Vec<_>>(),
        [PhotoId::new(1).unwrap(), PhotoId::new(2).unwrap()]
    );
    let projection = state.query(CatalogQuery {
        rating: Some(Rating::Five),
        rejected: Some(true),
        color_label: Some(ColorLabel::Red),
    });
    assert_eq!(projection[0].photo_id, PhotoId::new(1).unwrap());
}

#[test]
fn invalid_batch_does_not_mutate_state_or_revision() {
    let mut state = state();
    let before = state.clone();
    let error = state
        .apply(
            state.revision(),
            CatalogCommand::SetRating {
                photo_ids: vec![PhotoId::new(1).unwrap(), PhotoId::new(1).unwrap()],
                rating: Rating::One,
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        rusttable_catalog::CatalogError::DuplicatePhotoInOrganizationBatch { .. }
    ));
    assert_eq!(state, before);
}

#[test]
fn toggle_label_has_stable_projection() {
    let mut state = state();
    let id = PhotoId::new(2).unwrap();
    state
        .apply(
            state.revision(),
            CatalogCommand::ToggleColorLabel {
                photo_ids: vec![id],
                label: ColorLabel::Blue,
            },
        )
        .unwrap();
    assert_eq!(
        state
            .organization(id)
            .unwrap()
            .color_labels
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [ColorLabel::Blue]
    );
    state
        .apply(
            state.revision(),
            CatalogCommand::ToggleColorLabel {
                photo_ids: vec![id],
                label: ColorLabel::Blue,
            },
        )
        .unwrap();
    assert!(state.organization(id).unwrap().color_labels.is_empty());
}

#[test]
fn numeric_rating_unrejects_without_losing_the_previous_rejection_transition() {
    let mut state = state();
    let id = PhotoId::new(1).unwrap();
    state
        .apply(
            state.revision(),
            CatalogCommand::SetRating {
                photo_ids: vec![id],
                rating: Rating::Five,
            },
        )
        .unwrap();
    state
        .apply(
            state.revision(),
            CatalogCommand::SetRejection {
                photo_ids: vec![id],
                rejected: true,
            },
        )
        .unwrap();
    state
        .apply(
            state.revision(),
            CatalogCommand::SetRating {
                photo_ids: vec![id],
                rating: Rating::Two,
            },
        )
        .unwrap();

    let organization = state.organization(id).unwrap();
    assert_eq!(organization.rating, Rating::Two);
    assert!(!organization.rejected);
}
