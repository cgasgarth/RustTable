#![cfg(not(target_os = "macos"))]

use super::GtkShell;
use crate::CollectionProperty;
use crate::gtk_shell::{
    CollectionControlState, CollectionFilterState, LighttablePhotoState, LighttableToolbarState,
    WorkspaceRole,
};
use crate::presentation::{
    PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText,
    PreviewDimensions, Rgba8PreviewMetadata, SelectedPreviewState,
};
use crate::{HistogramData, ViewportGeneration};
use gtk4::prelude::ListModelExt;
use rusttable_core::PhotoId;

fn id(value: u128) -> PhotoId {
    PhotoId::new(value).expect("non-zero test photo ID")
}

fn workspace(photo_id: PhotoId) -> PhotoWorkspaceViewModel {
    let title = PresentationText::new(format!("photo-{photo_id}")).expect("test title");
    PhotoWorkspaceViewModel::new(
        vec![PhotoCardViewModel::new(photo_id, title.clone(), None)],
        vec![PhotoDetailViewModel::new(photo_id, title, Vec::new())],
    )
    .expect("matching test workspace")
}

fn workspace_with_photos(photo_ids: &[u128]) -> PhotoWorkspaceViewModel {
    let cards = photo_ids
        .iter()
        .map(|value| {
            let photo_id = id(*value);
            let title = PresentationText::new(format!("photo-{photo_id}")).expect("test title");
            PhotoCardViewModel::new(photo_id, title, None)
        })
        .collect::<Vec<_>>();
    let details = photo_ids
        .iter()
        .map(|value| {
            let photo_id = id(*value);
            let title = PresentationText::new(format!("photo-{photo_id}")).expect("test title");
            PhotoDetailViewModel::new(photo_id, title, Vec::new())
        })
        .collect::<Vec<_>>();
    PhotoWorkspaceViewModel::new(cards, details).expect("matching test workspace")
}

fn collection_state(photo_id: PhotoId, generation: u64) -> CollectionFilterState {
    let controls =
        CollectionControlState::new(CollectionProperty::Filename, 1).with_generation(generation);
    CollectionFilterState::new(controls, vec![photo_id]).with_lighttable_state(
        vec![LighttablePhotoState::new(
            photo_id,
            true,
            super::super::LighttableRating::Zero,
            [],
        )],
        LighttableToolbarState::new(1),
    )
}

fn assert_thumbnail_pair_is_ready(shell: &GtkShell, photo_id: PhotoId) {
    let pair = shell
        .photo_tiles
        .borrow()
        .get(&photo_id)
        .cloned()
        .expect("photo tile");
    assert!(pair.thumbnails.state().is_ready());
    assert!(pair.thumbnails.filmstrip().state().is_ready());
}

#[cfg(not(target_os = "macos"))]
#[gtk4::test]
fn grid_model_keeps_complete_collections_while_gtk_virtualizes_cards() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.complete-lighttable-model"),
        Default::default(),
    );
    let shell = GtkShell::new(&application);
    let ids = (1..=201).collect::<Vec<_>>();

    shell.set_photo_workspace(&workspace_with_photos(&ids));

    let selection = shell.lighttable.model().expect("grid selection model");
    let no_selection = selection
        .downcast::<gtk4::NoSelection>()
        .expect("grid uses non-selecting model");
    let model = no_selection.model().expect("grid string model");
    assert_eq!(model.n_items(), 201);
    assert!(shell.photo_tiles.borrow().len() <= 201);
}

#[cfg(not(target_os = "macos"))]
#[gtk4::test]
fn grid_model_preserves_controller_order_across_filtered_refreshes() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.stable-lighttable-order"),
        Default::default(),
    );
    let shell = GtkShell::new(&application);
    let source = workspace_with_photos(&[1, 2, 3]);

    shell.set_lighttable_workspace_filtered(&source, [3, 1]);

    let selection = shell.lighttable.model().expect("grid selection model");
    let no_selection = selection
        .downcast::<gtk4::NoSelection>()
        .expect("grid uses non-selecting model");
    let model = no_selection
        .model()
        .expect("grid string model")
        .downcast::<gtk4::StringList>()
        .expect("grid IDs are ordered strings");
    assert_eq!(model.string(0).as_deref(), Some("3"));
    assert_eq!(model.string(1).as_deref(), Some("1"));
}

// GTK4's test helper uses a GLib worker thread, which is not the Darwin
// process main thread. The real macOS validation is performed by the installed
// app smoke test; keep this widget-level test on platforms where GTK permits
// that harness.
#[cfg(not(target_os = "macos"))]
#[gtk4::test]
fn native_open_projection_survives_workspace_reset_and_mode_switch() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.native-open-projection"),
        Default::default(),
    );
    let shell = GtkShell::new(&application);
    let first = id(1);
    let second = id(2);

    shell.set_photo_workspace(&workspace(first));
    shell.set_collection_filter_state(&collection_state(first, 4));
    assert_eq!(shell.lighttable_toolbar.state().selected_count(), 1);

    shell.set_photo_workspace(&workspace(second));
    shell.set_collection_filter_state(&collection_state(second, 0));

    assert_eq!(shell.lighttable_generation.get(), 0);
    assert_eq!(
        shell
            .lighttable_interaction
            .borrow()
            .selected()
            .collect::<Vec<_>>(),
        [second]
    );
    assert_eq!(shell.lighttable_toolbar.state().selected_count(), 1);
    assert!(shell.photo_tiles.borrow().contains_key(&second));
    assert!(shell.lighttable.first_child().is_some());
    assert!(shell.lighttable.is_visible());
    assert_eq!(
        shell.lighttable_empty_state.visible_child_name().as_deref(),
        Some("grid")
    );

    let dimensions = PreviewDimensions::new(1, 1).expect("test dimensions");
    let status = PresentationText::new("thumbnail ready").expect("test status");
    let thumbnail = Rgba8PreviewMetadata::new(dimensions, status, vec![255, 0, 0, 255])
        .expect("test thumbnail");
    shell
        .set_photo_thumbnail(second, &thumbnail)
        .expect("install test thumbnail");
    assert!(
        shell
            .photo_tiles
            .borrow()
            .get(&second)
            .expect("second tile")
            .thumbnails
            .state()
            .is_ready()
    );

    assert!(shell.open_photo(second));
    assert_eq!(
        shell.workspace.visible_child_name().as_deref(),
        Some(WorkspaceRole::Darkroom.stack_name())
    );
    shell.show_workspace(WorkspaceRole::Lighttable);
    shell.show_workspace(WorkspaceRole::Darkroom);
    assert_eq!(
        shell.darkroom.preview().selection_state(),
        super::super::DarkroomSelectionState::Selected(second)
    );
    assert_eq!(shell.darkroom.preview().title_label().text(), "photo-2");
    shell.show_workspace(WorkspaceRole::Lighttable);
    assert_eq!(
        shell.workspace.visible_child_name().as_deref(),
        Some(WorkspaceRole::Lighttable.stack_name())
    );
    assert!(shell.lighttable.first_child().is_some());
    assert_eq!(shell.lighttable_toolbar.state().selected_count(), 1);
    assert!(
        shell
            .photo_tiles
            .borrow()
            .get(&second)
            .expect("second tile after mode switch")
            .thumbnails
            .state()
            .is_ready()
    );
}

#[gtk4::test]
fn thumbnail_publication_keeps_grid_and_filmstrip_ready_through_rerenders() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.thumbnail-publication-reconciliation"),
        Default::default(),
    );
    let shell = GtkShell::new(&application);
    let photo_id = id(4);
    let source = workspace(photo_id);
    shell.set_photo_workspace(&source);

    let dimensions = PreviewDimensions::new(1, 1).expect("test dimensions");
    let metadata = Rgba8PreviewMetadata::new(
        dimensions,
        PresentationText::new("thumbnail ready").expect("test status"),
        vec![255, 0, 0, 255],
    )
    .expect("test thumbnail");
    shell
        .set_photo_thumbnail(photo_id, &metadata)
        .expect("publish thumbnail");
    assert_thumbnail_pair_is_ready(&shell, photo_id);

    shell.set_lighttable_workspace_filtered(&source, [photo_id]);
    assert_thumbnail_pair_is_ready(&shell, photo_id);

    shell.show_workspace(WorkspaceRole::Darkroom);
    shell.show_workspace(WorkspaceRole::Lighttable);
    shell.set_lighttable_workspace_filtered(&source, [photo_id]);
    assert_thumbnail_pair_is_ready(&shell, photo_id);
}

#[gtk4::test]
fn thumbnail_candidates_include_unrealized_active_filmstrip_selection() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.bounded-thumbnail-candidates"),
        Default::default(),
    );
    let shell = GtkShell::new(&application);
    let selected = id(2);
    shell.set_photo_workspace(&workspace_with_photos(&[1, 2, 3]));
    shell.begin_darkroom_selection(selected, ViewportGeneration::new(1));

    {
        let mut tiles = shell.photo_tiles.borrow_mut();
        tiles.get_mut(&id(1)).expect("first tile").lighttable_button = Some(gtk4::Button::new());
        tiles
            .get_mut(&selected)
            .expect("selected tile")
            .lighttable_button = None;
        tiles.get_mut(&id(3)).expect("third tile").lighttable_button = None;
    }

    assert_eq!(
        shell.lighttable_thumbnail_photo_ids(),
        vec![id(1), selected]
    );

    shell.show_workspace(WorkspaceRole::Darkroom);
    assert_eq!(
        shell.lighttable_thumbnail_photo_ids(),
        vec![id(1), selected]
    );
}

#[cfg(not(target_os = "macos"))]
#[gtk4::test]
fn edited_main_preview_and_filmstrip_publish_for_the_same_selection() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.edited-preview-filmstrip-publication"),
        Default::default(),
    );
    let shell = GtkShell::new(&application);
    let photo_id = id(5);
    shell.set_photo_workspace(&workspace(photo_id));
    let generation = ViewportGeneration::new(3);
    shell.begin_darkroom_selection(photo_id, generation);
    shell.set_darkroom_preview_loading(generation);

    let dimensions = PreviewDimensions::new(1, 1).expect("test dimensions");
    let thumbnail = Rgba8PreviewMetadata::new(
        dimensions,
        PresentationText::new("thumbnail").expect("test status"),
        vec![16, 32, 48, 255],
    )
    .expect("test thumbnail");
    shell
        .set_photo_thumbnail(photo_id, &thumbnail)
        .expect("publish thumbnail");

    let edited = Rgba8PreviewMetadata::new(
        dimensions,
        PresentationText::new("edited preview").expect("test status"),
        vec![192, 160, 128, 255],
    )
    .expect("test edited preview");
    let histogram = HistogramData::from_rgba8(dimensions, edited.pixels()).expect("test histogram");
    shell
        .set_darkroom_preview_result(generation, &edited, Ok(histogram))
        .expect("publish edited preview");

    assert_eq!(
        shell.darkroom_preview().status_label().text(),
        "edited preview"
    );
    assert!(shell.darkroom_preview().texture().is_some());
    assert_thumbnail_pair_is_ready(&shell, photo_id);
    assert!(matches!(
        shell
            .lighttable_workspace
            .borrow()
            .as_ref()
            .and_then(|workspace| workspace.detail(photo_id))
            .map(PhotoDetailViewModel::selected_preview),
        Some(SelectedPreviewState::Ready(_))
    ));
}

// Darktable's `src/views/darkroom.c` and `src/libs/histogram.c` treat the completed preview pipe
// as one terminal projection. Reopening the selected photo must not replay its earlier loading
// detail after the texture and histogram have both completed.
#[gtk4::test]
fn completed_preview_survives_native_open_projection_replay() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.preview-projection-replay"),
        Default::default(),
    );
    let shell = GtkShell::new(&application);
    let photo_id = id(3);
    let generation = ViewportGeneration::new(1);
    let loading = workspace(photo_id)
        .with_selected_preview(photo_id, SelectedPreviewState::Loading)
        .expect("selected detail");
    shell.set_photo_workspace(&loading);
    shell.begin_darkroom_selection(photo_id, generation);
    shell.set_darkroom_preview_loading(generation);

    let dimensions = PreviewDimensions::new(1, 1).expect("test dimensions");
    let pixels = vec![64, 128, 192, 255];
    let metadata = Rgba8PreviewMetadata::new(
        dimensions,
        PresentationText::new("rendered").expect("test status"),
        pixels.clone(),
    )
    .expect("test preview");
    let histogram = HistogramData::from_rgba8(dimensions, &pixels).expect("test histogram");
    shell
        .set_darkroom_preview_result(generation, &metadata, Ok(histogram))
        .expect("install completed preview");

    assert!(shell.darkroom_preview().texture().is_some());
    assert_eq!(shell.darkroom_preview().status_label().text(), "rendered");
    assert!(shell.darkroom.histogram_available());
    assert!(matches!(
        shell
            .lighttable_workspace
            .borrow()
            .as_ref()
            .and_then(|workspace| workspace.detail(photo_id))
            .map(PhotoDetailViewModel::selected_preview),
        Some(SelectedPreviewState::Ready(_))
    ));

    shell.show_workspace(WorkspaceRole::Lighttable);
    assert!(shell.open_photo(photo_id));
    assert!(shell.darkroom_preview().texture().is_some());
    assert_eq!(shell.darkroom_preview().status_label().text(), "rendered");
    assert!(shell.darkroom.histogram_available());
}

#[gtk4::test]
fn immediate_darkroom_activation_binds_before_presentation_and_rejects_stale_reselection() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.darkroom-transition-race"),
        Default::default(),
    );
    let shell = GtkShell::new(&application);
    let first = id(10);
    let second = id(11);
    let generation = std::rc::Rc::new(std::cell::Cell::new(0_u64));

    shell.set_photo_workspace(&workspace_with_photos(&[10, 11]));
    shell.set_photo_selected_handler({
        let shell = shell.clone();
        let generation = std::rc::Rc::clone(&generation);
        move |photo_id, _| {
            let next = generation.get().saturating_add(1);
            generation.set(next);
            let generation = ViewportGeneration::new(next);
            shell.begin_darkroom_selection(photo_id, generation);
            shell.set_darkroom_preview_loading(generation);
        }
    });

    assert!(shell.open_photo(first));
    assert_eq!(
        shell.workspace.visible_child_name().as_deref(),
        Some(WorkspaceRole::Darkroom.stack_name())
    );
    assert_eq!(shell.darkroom.viewport_state().photo_id(), Some(first));
    assert_eq!(shell.darkroom.viewport_state().generation().get(), 1);
    assert_eq!(shell.darkroom.filmstrip_selection(), Some(first));
    assert_eq!(
        shell.darkroom_preview().selection_state(),
        super::super::DarkroomSelectionState::Selected(first)
    );
    assert_eq!(
        shell.darkroom_preview().status_label().text(),
        "loading preview"
    );
    assert!(!shell.darkroom.histogram_available());
    assert_eq!(
        shell.left_panel_stack.visible_child_name().as_deref(),
        Some(WorkspaceRole::Darkroom.stack_name())
    );
    assert_eq!(
        shell.right_panel_stack.visible_child_name().as_deref(),
        Some(WorkspaceRole::Darkroom.stack_name())
    );
    assert!(shell.filmstrip_root.is_visible());

    assert!(shell.open_photo(second));
    assert_eq!(shell.darkroom.viewport_state().photo_id(), Some(second));
    assert_eq!(shell.darkroom.viewport_state().generation().get(), 2);
    assert_eq!(shell.darkroom.filmstrip_selection(), Some(second));

    let dimensions = PreviewDimensions::new(1, 1).expect("test dimensions");
    let stale_pixels = vec![255, 0, 0, 255];
    let stale = Rgba8PreviewMetadata::new(
        dimensions,
        PresentationText::new("stale").expect("test status"),
        stale_pixels.clone(),
    )
    .expect("stale preview");
    let stale_histogram =
        HistogramData::from_rgba8(dimensions, &stale_pixels).expect("stale histogram");
    shell
        .set_darkroom_preview_result(ViewportGeneration::new(1), &stale, Ok(stale_histogram))
        .expect("stale result is ignored");
    assert_eq!(shell.darkroom.viewport_state().photo_id(), Some(second));
    assert!(shell.darkroom_preview().texture().is_none());

    let current_pixels = vec![0, 255, 0, 255];
    let current = Rgba8PreviewMetadata::new(
        dimensions,
        PresentationText::new("rendered").expect("test status"),
        current_pixels.clone(),
    )
    .expect("current preview");
    let current_histogram =
        HistogramData::from_rgba8(dimensions, &current_pixels).expect("current histogram");
    shell
        .set_darkroom_preview_result(ViewportGeneration::new(2), &current, Ok(current_histogram))
        .expect("current result");
    assert!(shell.darkroom_preview().texture().is_some());
    assert!(shell.darkroom.histogram_available());
    assert_eq!(shell.darkroom.viewport_state().photo_id(), Some(second));
}
