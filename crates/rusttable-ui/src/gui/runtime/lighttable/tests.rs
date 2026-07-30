use super::{
    LighttableGridGeometry, ThumbnailState, active_photo_position, filmstrip_visible_for_workspace,
    grid_model_strings, retained_thumbnail_state,
};
use crate::gui::{LighttableLayout, LighttableZoom};
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

#[test]
fn file_manager_grid_keeps_the_configured_count_and_top_left_row_origin() {
    let grid = LighttableGridGeometry::for_viewport(
        LighttableLayout::FileManager,
        LighttableZoom::Normal,
        720,
        600,
    );

    assert_eq!(grid.columns(), 5);
    assert_eq!(grid.cell_width_px(), 141);
    assert_eq!(grid.horizontal_offset_px(), 1);
    assert_eq!(grid.cell_origin_x_px(0), 1);
    assert_eq!(grid.cell_origin_x_px(1), 142);
    assert_eq!(grid.card_height_px(), grid.card_width_px());
}

#[test]
fn file_manager_grid_resizes_cells_instead_of_clamping_their_width() {
    let wide = LighttableGridGeometry::for_viewport(
        LighttableLayout::FileManager,
        LighttableZoom::Normal,
        1_000,
        600,
    );
    let narrow = LighttableGridGeometry::for_viewport(
        LighttableLayout::FileManager,
        LighttableZoom::Normal,
        620,
        600,
    );

    assert_eq!(wide.columns(), narrow.columns());
    assert_eq!(wide.cell_width_px(), 197);
    assert_eq!(narrow.cell_width_px(), 121);
    assert!(narrow.card_width_px() < 148);
}

#[test]
fn filmstrip_visibility_follows_the_active_view_and_lighttable_layout() {
    assert!(!filmstrip_visible_for_workspace(
        LighttableLayout::FileManager,
        false,
        true,
    ));
    assert!(!filmstrip_visible_for_workspace(
        LighttableLayout::Zoomable,
        false,
        true,
    ));
    assert!(filmstrip_visible_for_workspace(
        LighttableLayout::Culling,
        false,
        false,
    ));
    assert!(!filmstrip_visible_for_workspace(
        LighttableLayout::Preview,
        true,
        false,
    ));
    assert!(filmstrip_visible_for_workspace(
        LighttableLayout::FileManager,
        true,
        true,
    ));
}

#[test]
fn rerender_locates_the_active_photo_in_display_order() {
    let displayed = [id(11), id(4), id(19)];

    assert_eq!(active_photo_position(&displayed, Some(id(4))), Some(1));
    assert_eq!(active_photo_position(&displayed, Some(id(8))), None);
    assert_eq!(active_photo_position(&displayed, None), None);
}

#[test]
fn grid_model_pads_partial_rows_without_inventing_photo_ids() {
    let displayed = [id(11), id(4)];

    assert_eq!(grid_model_strings(&displayed, 5), ["11", "4", "", "", ""]);
    assert!(grid_model_strings(&[], 5).is_empty());
    assert_eq!(grid_model_strings(&displayed, 1), ["11", "4"]);
}
