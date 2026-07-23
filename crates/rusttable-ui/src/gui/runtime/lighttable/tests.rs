use super::{ThumbnailState, retained_thumbnail_state};
use crate::presentation::{
    PhotoDetailViewModel, PresentationText, PreviewDimensions, Rgba8PreviewMetadata,
};
use rusttable_core::PhotoId;
use std::collections::BTreeMap;

fn id(value: u128) -> PhotoId {
    PhotoId::new(value).expect("non-zero test photo ID")
}

fn text(value: &str) -> PresentationText {
    PresentationText::new(value).expect("valid test text")
}

fn detail(photo_id: PhotoId, title: &str) -> PhotoDetailViewModel {
    PhotoDetailViewModel::new(photo_id, text(title), Vec::new())
}

fn ready_thumbnail() -> ThumbnailState {
    ThumbnailState::Ready(
        Rgba8PreviewMetadata::new(
            PreviewDimensions::new(2, 1).expect("non-zero dimensions"),
            text("thumbnail ready"),
            vec![0; 8],
        )
        .expect("valid RGBA8 thumbnail"),
    )
}

#[test]
fn rerender_retains_completed_thumbnail_for_an_unchanged_detail() {
    let photo_id = id(1);
    let current = detail(photo_id, "photo.png");
    let previous_states = BTreeMap::from([(photo_id, ready_thumbnail())]);
    let previous_details = BTreeMap::from([(photo_id, current.clone())]);

    assert_eq!(
        retained_thumbnail_state(photo_id, &current, &previous_details, &previous_states),
        ready_thumbnail()
    );
    assert_eq!(previous_states.get(&photo_id), Some(&ready_thumbnail()));
}

#[test]
fn rerender_resets_thumbnail_when_catalog_detail_changes() {
    let photo_id = id(1);
    let current = detail(photo_id, "new-photo.png");
    let previous_details = BTreeMap::from([(photo_id, detail(photo_id, "old-photo.png"))]);
    let previous_states = BTreeMap::from([(photo_id, ready_thumbnail())]);

    assert_eq!(
        retained_thumbnail_state(photo_id, &current, &previous_details, &previous_states),
        ThumbnailState::Loading
    );
    assert_eq!(previous_states.get(&photo_id), Some(&ready_thumbnail()));
}

#[test]
fn rerender_retains_unavailable_and_failed_states() {
    let photo_id = id(1);
    let current = detail(photo_id, "photo.png");
    let previous_details = BTreeMap::from([(photo_id, current.clone())]);

    for state in [ThumbnailState::Unavailable, ThumbnailState::Failed] {
        let previous_states = BTreeMap::from([(photo_id, state.clone())]);
        assert_eq!(
            retained_thumbnail_state(photo_id, &current, &previous_details, &previous_states),
            state
        );
    }
}
