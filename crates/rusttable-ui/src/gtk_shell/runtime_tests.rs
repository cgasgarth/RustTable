#![cfg(not(target_os = "macos"))]

use super::GtkShell;
use crate::CollectionProperty;
use crate::gtk_shell::{
    CollectionControlState, CollectionFilterState, LighttablePhotoState, LighttableToolbarState,
    WorkspaceRole,
};
use crate::presentation::{
    PhotoCardViewModel, PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText,
    PreviewDimensions, Rgba8PreviewMetadata,
};
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
