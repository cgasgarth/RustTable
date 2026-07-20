use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use super::super::{ActiveExport, Message, Shell, update};
use super::{
    ExportCancellation, ExportCollisionSelection, ExportCollisionStatus, ExportRequest,
    ExportSettings, ExportSize, ExportSizeSelection, ExportStage, ExportStatus, ExportTaskResult,
    png_destination, run, start_request,
};
use crate::library::{LibraryFailureKind, LibraryLoadRequestId};
use rusttable_core::PhotoId;
use rusttable_ui::{
    ExportIntent, ExportSize as UiExportSize, InputIntent, NavigationIntent, PhotoCardViewModel,
    PhotoDetailViewModel, PhotoWorkspaceViewModel, PresentationText, UiState,
};

static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let sequence = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusttable-app-export-{}-{sequence}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("test export directory");
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn active_photo_shell() -> (Shell, PhotoId) {
    let photo_id = PhotoId::new(1).expect("test photo ID is non-zero");
    let workspace = PhotoWorkspaceViewModel::new(
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
    .expect("test workspace is valid");
    let mut shell = Shell::with_photo_workspace(workspace);
    let _ = update(
        &mut shell,
        Message::Navigate(NavigationIntent::ShowPhoto(photo_id)),
    );
    (shell, photo_id)
}

#[test]
fn default_shell_shows_the_sidebar() {
    assert_eq!(
        Shell::default(),
        Shell {
            ui: UiState::default(),
            active_load_request_id: LibraryLoadRequestId::first(),
            load_in_flight: false,
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
        }
    );
}

#[test]
fn toggle_sidebar_hides_it() {
    let mut shell = Shell::default();

    let _ = update(&mut shell, Message::ToggleSidebar);

    assert!(!shell.ui_state().sidebar_visible());
}

#[test]
fn toggling_sidebar_twice_restores_it() {
    let mut shell = Shell::default();

    let _ = update(&mut shell, Message::ToggleSidebar);
    let _ = update(&mut shell, Message::ToggleSidebar);

    assert!(shell.ui_state().sidebar_visible());
}

#[test]
fn adds_the_png_extension_when_the_picker_destination_omits_it() {
    assert_eq!(
        png_destination(Path::new("/tmp/rendered-copy")).expect("extension is appended"),
        Path::new("/tmp/rendered-copy.png")
    );
}

#[test]
fn rejects_a_conflicting_destination_extension() {
    assert!(png_destination(Path::new("/tmp/rendered-copy.jpg")).is_err());
}

#[test]
fn settings_map_ui_and_custom_selections_into_immutable_bounds() {
    let request = ExportRequest::new(
        PathBuf::from("catalog.redb"),
        PathBuf::from("sources"),
        PhotoId::new(7).expect("non-zero photo ID"),
        PathBuf::from("copy.png"),
        ExportSettings::from_selection(
            ExportSizeSelection::from_ui(UiExportSize::Fit4096),
            ExportCollisionSelection::ReplaceExisting,
        ),
    );

    assert_eq!(
        request.photo_id(),
        PhotoId::new(7).expect("non-zero photo ID")
    );
    assert_eq!(request.destination(), Path::new("copy.png"));
    assert_eq!(request.settings().size(), ExportSize::FitMaximum(4_096));
    assert_eq!(
        request.settings().collision(),
        ExportCollisionSelection::ReplaceExisting
    );
    assert_eq!(
        ExportSizeSelection::from_ui(UiExportSize::Fit2048).into_size(),
        ExportSize::FitMaximum(2_048)
    );
    assert_eq!(
        ExportSizeSelection::custom_maximum(16_384)
            .expect("maximum boundary is valid")
            .into_size(),
        ExportSize::FitMaximum(16_384)
    );
    assert!(ExportSizeSelection::custom_maximum(0).is_err());
    assert!(ExportSizeSelection::custom_maximum(16_385).is_err());
}

#[test]
fn app_snapshots_the_current_iced_size_when_destination_selection_begins() {
    let (mut shell, photo_id) = active_photo_shell();

    let _ = update(
        &mut shell,
        Message::Input(InputIntent::Export(ExportIntent::SelectSize(
            UiExportSize::Fit2048,
        ))),
    );
    let _ = update(&mut shell, Message::SaveRenderedCopy(photo_id));

    assert_eq!(
        shell
            .pending_export
            .as_ref()
            .map(|pending| pending.settings),
        Some(ExportSettings::from_selection(
            ExportSizeSelection::Fit2048,
            ExportCollisionSelection::CreateNew,
        ))
    );
}

#[test]
fn export_task_keeps_a_cancellation_handle_for_its_immutable_request() {
    let task = start_request(ExportRequest::new(
        PathBuf::from("catalog.redb"),
        PathBuf::from("sources"),
        PhotoId::new(7).expect("non-zero photo ID"),
        PathBuf::from("copy.png"),
        ExportSettings::original(),
    ));
    let cancellation = task.cancellation();

    assert!(!cancellation.is_cancelled());
    cancellation.cancel();
    assert!(task.cancellation().is_cancelled());
}

#[test]
fn cancellation_before_work_reports_preparing_then_cancelled() {
    let cancellation = ExportCancellation::default();
    cancellation.cancel();
    let request = ExportRequest::new(
        PathBuf::from("catalog.redb"),
        PathBuf::from("sources"),
        PhotoId::new(7).expect("non-zero photo ID"),
        PathBuf::from("copy.png"),
        ExportSettings::original(),
    );
    let mut stages = Vec::new();

    let result = run(&request, &cancellation, &mut |status| {
        assert_eq!(status.text(), status.stage().label());
        stages.push(status.stage());
    });

    assert!(result.is_err());
    assert_eq!(stages, vec![ExportStage::Preparing, ExportStage::Cancelled]);
}

#[test]
fn selected_imported_edit_renders_and_saves_a_verified_png() {
    let directory = TestDirectory::new();
    let catalog = directory.0.join("catalog.redb");
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root");
    let source = source_root.join("fixtures/corpus/assets/raster-png-16-alpha.png");
    let batch = crate::workspace::run_raster_import(
        &catalog,
        vec![source],
        &rusttable_import::RasterImportCancellation::default(),
        &|_| {},
    );
    let receipt = batch.receipts().next().expect("one fixture receipt");
    let photo_id = receipt.photo_id.expect("fixture import photo");
    let destination = directory.0.join("rendered.png");

    let request = ExportRequest::new(
        catalog,
        source_root,
        photo_id,
        destination.clone(),
        ExportSettings::original(),
    );
    let cancellation = ExportCancellation::default();
    let mut stages = Vec::new();
    let status = run(&request, &cancellation, &mut |status| {
        stages.push(status.stage());
    })
    .expect("selected edit is rendered and saved");

    assert!(status.summary.starts_with("Saved rendered.png (4×3,"));
    assert_eq!(status.collision, ExportCollisionStatus::CreatedNew);
    assert!(destination.is_file());
    assert_eq!(
        stages,
        vec![
            ExportStage::Preparing,
            ExportStage::Rendering,
            ExportStage::Publishing,
            ExportStage::Completed,
        ]
    );
}

#[test]
fn status_events_are_scoped_to_the_active_export() {
    let (mut shell, photo_id) = active_photo_shell();
    let other_photo_id = PhotoId::new(2).expect("test photo ID is non-zero");
    shell.active_export = Some(ActiveExport {
        photo_id,
        request: ExportRequest::new(
            PathBuf::from("catalog.redb"),
            PathBuf::from("sources"),
            photo_id,
            PathBuf::from("copy.png"),
            ExportSettings::original(),
        ),
        cancellation: ExportCancellation::default(),
    });

    let _ = update(
        &mut shell,
        Message::ExportStatus {
            photo_id: other_photo_id,
            status: ExportStatus::at(ExportStage::Rendering),
        },
    );
    assert!(shell.ui_state().export_status(photo_id).is_none());

    let _ = update(
        &mut shell,
        Message::ExportStatus {
            photo_id,
            status: ExportStatus::at(ExportStage::Rendering),
        },
    );
    assert_eq!(
        shell
            .ui_state()
            .export_status(photo_id)
            .map(PresentationText::as_str),
        Some("Rendering selected edit…")
    );

    let _ = update(
        &mut shell,
        Message::ExportFinished {
            photo_id,
            result: ExportTaskResult::Completed {
                summary: "Saved copy.png".to_owned(),
                collision: ExportCollisionStatus::CreatedNew,
            },
        },
    );
    assert!(shell.active_export.is_none());

    let _ = update(
        &mut shell,
        Message::ExportStatus {
            photo_id,
            status: ExportStatus::at(ExportStage::Failed),
        },
    );
    assert_eq!(
        shell
            .ui_state()
            .export_status(photo_id)
            .map(PresentationText::as_str),
        Some("Saved copy.png Created a new PNG.")
    );
}

#[test]
fn collision_choices_keep_size_immutable_and_require_an_explicit_replace() {
    let (mut shell, photo_id) = active_photo_shell();
    shell.catalog_path = Ok(PathBuf::from("catalog.redb"));
    shell.source_root = Ok(PathBuf::from("sources"));
    let request = ExportRequest::new(
        PathBuf::from("catalog.redb"),
        PathBuf::from("sources"),
        photo_id,
        PathBuf::from("copy.png"),
        ExportSettings::from_selection(
            ExportSizeSelection::custom_maximum(3_000).expect("custom maximum is valid"),
            ExportCollisionSelection::CreateNew,
        ),
    );
    shell.active_export = Some(ActiveExport {
        photo_id,
        request: request.clone(),
        cancellation: ExportCancellation::default(),
    });

    let _ = update(
        &mut shell,
        Message::ExportFinished {
            photo_id,
            result: ExportTaskResult::Collision(ExportCollisionStatus::AwaitingSelection),
        },
    );
    assert_eq!(
        shell
            .ui_state()
            .export_status(photo_id)
            .map(PresentationText::as_str),
        Some("A PNG already exists there. Choose another destination or replace it.")
    );

    let _ = update(
        &mut shell,
        Message::ExportCollisionSelected {
            photo_id,
            selection: ExportCollisionSelection::CreateNew,
        },
    );
    assert_eq!(
        shell
            .pending_export
            .as_ref()
            .map(|pending| pending.settings),
        Some(request.settings())
    );

    shell.active_export = Some(ActiveExport {
        photo_id,
        request: request.clone(),
        cancellation: ExportCancellation::default(),
    });
    let _ = update(
        &mut shell,
        Message::ExportFinished {
            photo_id,
            result: ExportTaskResult::Collision(ExportCollisionStatus::AwaitingSelection),
        },
    );
    let _ = update(
        &mut shell,
        Message::ExportCollisionSelected {
            photo_id,
            selection: ExportCollisionSelection::ReplaceExisting,
        },
    );

    assert_eq!(
        shell
            .active_export
            .as_ref()
            .map(|active| active.request.settings()),
        Some(ExportSettings::from_selection(
            ExportSizeSelection::custom_maximum(3_000).expect("custom maximum is valid"),
            ExportCollisionSelection::ReplaceExisting,
        ))
    );
}

#[test]
fn cancellation_is_retained_until_the_active_export_finishes() {
    let (mut shell, photo_id) = active_photo_shell();
    let cancellation = ExportCancellation::default();
    shell.active_export = Some(ActiveExport {
        photo_id,
        request: ExportRequest::new(
            PathBuf::from("catalog.redb"),
            PathBuf::from("sources"),
            photo_id,
            PathBuf::from("copy.png"),
            ExportSettings::original(),
        ),
        cancellation: cancellation.clone(),
    });

    let _ = update(&mut shell, Message::CancelExport(photo_id));

    assert!(cancellation.is_cancelled());
    assert!(shell.active_export.is_some());
    assert_eq!(
        shell
            .ui_state()
            .export_status(photo_id)
            .map(PresentationText::as_str),
        Some("Cancelling export…")
    );
}
