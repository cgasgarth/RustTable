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
