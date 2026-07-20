use std::path::PathBuf;

use crate::library::{LibraryFailureKind, LibraryState};
use crate::library::{LibraryLoadRequestId, LibraryLoadResult};
use rusttable_core::PhotoId;
use rusttable_import::{RasterImportProgress, RasterImportRequest, RasterImportStage};
use rusttable_ui::{
    ImportPanelViewModel, ImportRowState, ImportRowViewModel, NavigationIntent, PhotoCardViewModel,
    PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText, PreviewDimensions,
    Rgba8PreviewMetadata, SelectedPreviewState, UiState, WorkspaceRoute,
};

use super::{Message, PreviewLoadResult, Shell, preview::failed_state, update};

fn photo_id() -> PhotoId {
    PhotoId::new(1).expect("test photo ID is non-zero")
}

fn workspace() -> PhotoWorkspaceViewModel {
    workspace_for(photo_id())
}

fn workspace_for(photo_id: PhotoId) -> PhotoWorkspaceViewModel {
    PhotoWorkspaceViewModel::new(
        vec![PhotoCardViewModel::new(
            photo_id,
            PresentationText::new("Test photo").expect("test title is valid"),
            None,
        )],
        vec![PhotoDetailViewModel::new(
            photo_id,
            PresentationText::new("Test photo").expect("test title is valid"),
            Vec::new(),
        )],
    )
    .expect("test workspace is valid")
}

fn rendered_preview() -> SelectedPreviewState {
    SelectedPreviewState::Ready(
        Rgba8PreviewMetadata::new(
            PreviewDimensions::new(1, 1).expect("test dimensions are valid"),
            PresentationText::new("Current persisted edit").expect("test status is valid"),
            vec![0, 0, 0, 255],
        )
        .expect("test preview is valid"),
    )
}

#[test]
fn injected_photo_workspace_is_retained_read_only() {
    let workspace = PhotoWorkspaceViewModel::default();
    let shell = Shell::with_photo_workspace(workspace.clone());

    assert_eq!(shell.library_state(), &LibraryState::Ready(workspace));
}

#[test]
fn opening_a_photo_starts_a_loading_preview_request() {
    let photo_id = photo_id();
    let mut shell = Shell {
        ui: UiState::with_photo_workspace(workspace()),
        active_load_request_id: LibraryLoadRequestId::first(),
        load_in_flight: false,
        catalog_path: Ok(PathBuf::from("/tmp/catalog.redb")),
        source_root: Ok(PathBuf::from("/tmp")),
        preview_generation: 0,
        active_preview: None,
        import_paths: Vec::new(),
        import_cancellation: None,
        active_export: None,
        pending_export: None,
        pending_export_collision: None,
        pending_import_selection: None,
        basic_edit: None,
    };

    let task = update(
        &mut shell,
        Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
    );

    assert_eq!(shell.ui.route(), WorkspaceRoute::PhotoDetail(photo_id));
    assert_eq!(shell.active_preview, Some((1, photo_id)));
    assert_eq!(
        shell
            .library_state()
            .ready_workspace()
            .and_then(|workspace| workspace.detail(photo_id))
            .map(PhotoDetailViewModel::selected_preview),
        Some(&SelectedPreviewState::Loading)
    );
    let _ = task;
}

#[test]
fn stale_preview_completion_cannot_replace_the_active_photo_preview() {
    let photo_id = photo_id();
    let mut shell = Shell {
        ui: UiState::with_photo_workspace(workspace()),
        active_load_request_id: LibraryLoadRequestId::first(),
        load_in_flight: false,
        catalog_path: Ok(PathBuf::from("/tmp/catalog.redb")),
        source_root: Ok(PathBuf::from("/tmp")),
        preview_generation: 2,
        active_preview: None,
        import_paths: Vec::new(),
        import_cancellation: None,
        active_export: None,
        pending_export: None,
        pending_export_collision: None,
        pending_import_selection: None,
        basic_edit: None,
    };
    let _ = update(
        &mut shell,
        Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
    );

    let _ = update(
        &mut shell,
        Message::PreviewLoaded {
            generation: 1,
            photo_id,
            result: PreviewLoadResult::Failed,
        },
    );

    assert_eq!(shell.active_preview, Some((3, photo_id)));
    assert_eq!(
        shell
            .library_state()
            .ready_workspace()
            .and_then(|workspace| workspace.detail(photo_id))
            .map(PhotoDetailViewModel::selected_preview),
        Some(&SelectedPreviewState::Loading)
    );
}

#[test]
fn refreshing_a_photo_keeps_its_last_successful_preview_visible() {
    let photo_id = photo_id();
    let preview = rendered_preview();
    let workspace = workspace()
        .with_selected_preview(photo_id, preview.clone())
        .expect("test detail exists");
    let mut shell = Shell {
        ui: UiState::with_photo_workspace(workspace),
        active_load_request_id: LibraryLoadRequestId::first(),
        load_in_flight: false,
        catalog_path: Ok(PathBuf::from("/tmp/catalog.redb")),
        source_root: Ok(PathBuf::from("/tmp")),
        preview_generation: 0,
        active_preview: None,
        import_paths: Vec::new(),
        import_cancellation: None,
        active_export: None,
        pending_export: None,
        pending_export_collision: None,
        pending_import_selection: None,
        basic_edit: None,
    };

    let task = update(
        &mut shell,
        Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
    );

    assert_eq!(shell.active_preview, Some((1, photo_id)));
    assert_eq!(
        shell
            .library_state()
            .ready_workspace()
            .and_then(|workspace| workspace.detail(photo_id))
            .map(PhotoDetailViewModel::selected_preview),
        Some(&preview)
    );
    let _ = task;
}

#[test]
fn catalog_refresh_rerenders_a_selected_photo_that_still_exists() {
    let photo_id = photo_id();
    let mut shell = Shell {
        ui: UiState::with_photo_workspace(workspace()),
        active_load_request_id: LibraryLoadRequestId::first(),
        load_in_flight: true,
        catalog_path: Err(LibraryFailureKind::CatalogLocationUnavailable),
        source_root: Err(LibraryFailureKind::CatalogLocationUnavailable),
        preview_generation: 0,
        active_preview: None,
        import_paths: Vec::new(),
        import_cancellation: None,
        active_export: None,
        pending_export: None,
        pending_export_collision: None,
        pending_import_selection: None,
        basic_edit: None,
    };
    let _ = update(
        &mut shell,
        Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
    );

    let _ = update(
        &mut shell,
        Message::LibraryLoaded {
            request_id: LibraryLoadRequestId::first(),
            result: LibraryLoadResult::Ready(workspace()),
        },
    );

    assert_eq!(shell.ui.route(), WorkspaceRoute::PhotoDetail(photo_id));
    assert_eq!(shell.active_preview, None);
    assert_eq!(
        shell
            .library_state()
            .ready_workspace()
            .and_then(|workspace| workspace.detail(photo_id))
            .map(PhotoDetailViewModel::selected_preview),
        Some(&failed_state())
    );
}

#[test]
fn catalog_refresh_returns_to_library_when_the_selected_photo_is_removed() {
    let photo_id = photo_id();
    let mut shell = Shell {
        ui: UiState::with_photo_workspace(workspace()),
        active_load_request_id: LibraryLoadRequestId::first(),
        load_in_flight: true,
        catalog_path: Err(LibraryFailureKind::CatalogLocationUnavailable),
        source_root: Err(LibraryFailureKind::CatalogLocationUnavailable),
        preview_generation: 0,
        active_preview: None,
        import_paths: Vec::new(),
        import_cancellation: None,
        active_export: None,
        pending_export: None,
        pending_export_collision: None,
        pending_import_selection: None,
        basic_edit: None,
    };
    let _ = update(
        &mut shell,
        Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
    );

    let _ = update(
        &mut shell,
        Message::LibraryLoaded {
            request_id: LibraryLoadRequestId::first(),
            result: LibraryLoadResult::Ready(workspace_for(
                PhotoId::new(2).expect("test photo ID is non-zero"),
            )),
        },
    );

    assert_eq!(shell.ui.route(), WorkspaceRoute::Library);
    assert_eq!(shell.active_preview, None);
}

#[test]
fn boot_starts_loading_with_one_active_request() {
    let (shell, task) = super::boot();

    assert_eq!(shell.library_state(), &LibraryState::Loading);
    assert_eq!(shell.active_load_request_id().get(), 1);
    let _ = task;
}

#[test]
fn active_completion_replaces_loading_state_and_duplicate_is_ignored() {
    let (mut shell, _) = super::boot();
    let request_id = shell.active_load_request_id();

    let _ = update(
        &mut shell,
        Message::LibraryLoaded {
            request_id,
            result: LibraryLoadResult::Empty,
        },
    );
    assert_eq!(shell.library_state(), &LibraryState::Empty);

    let _ = update(&mut shell, Message::RetryLibrary);
    assert_eq!(shell.active_load_request_id(), request_id);

    let _ = update(
        &mut shell,
        Message::LibraryLoaded {
            request_id,
            result: LibraryLoadResult::Failed(LibraryFailureKind::RepositoryUnavailable),
        },
    );
    assert_eq!(shell.library_state(), &LibraryState::Empty);
}

#[test]
fn stale_completion_and_retry_while_loading_are_no_ops() {
    let (mut shell, _) = super::boot();
    let active = shell.active_load_request_id();
    let stale = active.next().expect("next request");

    let _ = update(
        &mut shell,
        Message::LibraryLoaded {
            request_id: stale,
            result: LibraryLoadResult::Empty,
        },
    );
    assert_eq!(shell.library_state(), &LibraryState::Loading);

    let _ = update(&mut shell, Message::RetryLibrary);
    assert_eq!(shell.active_load_request_id(), active);
    assert_eq!(shell.library_state(), &LibraryState::Loading);
}

#[test]
fn failed_retry_advances_request_and_returns_to_loading() {
    let (mut shell, _) = super::boot();
    let first = shell.active_load_request_id();
    let _ = update(
        &mut shell,
        Message::LibraryLoaded {
            request_id: first,
            result: LibraryLoadResult::Failed(LibraryFailureKind::RepositoryUnavailable),
        },
    );

    let _ = update(&mut shell, Message::RetryLibrary);
    let retried = shell.active_load_request_id();
    let _ = update(&mut shell, Message::RetryLibrary);

    assert_eq!(retried.get(), first.get() + 1);
    assert_eq!(shell.active_load_request_id(), retried);
    assert_eq!(shell.library_state(), &LibraryState::Loading);
}

#[test]
fn ordered_service_progress_updates_the_visible_import_row_live() {
    let request = RasterImportRequest::new([PathBuf::from("photo.png")]).unwrap();
    let item_id = request.items().next().unwrap().0;
    let mut shell = Shell::default();
    shell.ui.set_import_panel(ImportPanelViewModel::new(
        vec![ImportRowViewModel::new(
            item_id.get(),
            PresentationText::new("photo.png").unwrap(),
            ImportRowState::Queued,
        )],
        true,
    ));

    for (stage, expected) in [
        (RasterImportStage::Opening, ImportRowState::Opening),
        (RasterImportStage::Hashing, ImportRowState::Hashing),
        (RasterImportStage::Probing, ImportRowState::Probing),
        (
            RasterImportStage::DecodingHeader,
            ImportRowState::DecodingHeader,
        ),
        (RasterImportStage::Registering, ImportRowState::Registering),
        (
            RasterImportStage::GeneratingPreview,
            ImportRowState::GeneratingPreview,
        ),
        (RasterImportStage::Completed, ImportRowState::Completed),
    ] {
        let _ = update(
            &mut shell,
            Message::ImportProgress(RasterImportProgress { item_id, stage }),
        );
        assert_eq!(
            shell
                .ui
                .import_panel()
                .rows()
                .next()
                .map(rusttable_ui::ImportRowViewModel::state),
            Some(expected)
        );
    }
}
