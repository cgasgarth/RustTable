use iced::Task;
use rusttable_core::{Edit, PhotoId};
use rusttable_ui::{
    LibraryState, PresentationText, PreviewDimensions, Rgba8PreviewMetadata,
    SelectedPreviewFailure, SelectedPreviewState, WorkspaceRoute,
};

use crate::workspace::{
    SelectedPreview, load_selected_preview, preview_loader::load_preview_for_edit,
};

use super::{Message, PreviewLoadResult, Shell};

pub(super) fn handle_loaded(
    shell: &mut Shell,
    generation: u64,
    photo_id: PhotoId,
    result: PreviewLoadResult,
) {
    if shell.active_preview != Some((generation, photo_id))
        || shell.ui.route() != WorkspaceRoute::PhotoDetail(photo_id)
    {
        return;
    }
    shell.active_preview = None;
    match result {
        PreviewLoadResult::Ready(preview) => {
            publish(shell, photo_id, preview, "Current persisted edit");
        }
        PreviewLoadResult::Draft(preview) => {
            publish(shell, photo_id, preview, "Unsaved edit preview");
        }
        PreviewLoadResult::Failed => replace(shell, photo_id, failed_state()),
    }
}

pub(super) fn reconcile_route(shell: &mut Shell, previous_route: WorkspaceRoute) {
    if previous_route != shell.ui.route() && matches!(shell.ui.route(), WorkspaceRoute::Library) {
        shell.active_preview = None;
    }
}

pub(super) fn start_persisted(shell: &mut Shell, photo_id: PhotoId) -> Task<Message> {
    let Some(generation) = begin_request(shell, photo_id) else {
        return Task::none();
    };
    let (Ok(catalog_path), Ok(source_root)) = (&shell.catalog_path, &shell.source_root) else {
        shell.active_preview = None;
        replace(shell, photo_id, failed_state());
        return Task::none();
    };
    let catalog_path = catalog_path.clone();
    let source_root = source_root.clone();
    Task::perform(
        async move {
            load_selected_preview(&catalog_path, &source_root, photo_id)
                .map_or(PreviewLoadResult::Failed, PreviewLoadResult::Ready)
        },
        move |result| Message::PreviewLoaded {
            generation,
            photo_id,
            result,
        },
    )
}

pub(super) fn start_draft(shell: &mut Shell, draft: &Edit) -> Task<Message> {
    let photo_id = draft.photo_id();
    let Some(generation) = begin_request(shell, photo_id) else {
        return Task::none();
    };
    let (Ok(catalog_path), Ok(source_root)) = (&shell.catalog_path, &shell.source_root) else {
        shell.active_preview = None;
        replace(shell, photo_id, failed_state());
        return Task::none();
    };
    let catalog_path = catalog_path.clone();
    let source_root = source_root.clone();
    let draft = draft.clone();
    Task::perform(
        async move {
            load_preview_for_edit(&catalog_path, &source_root, photo_id, &draft)
                .map_or(PreviewLoadResult::Failed, PreviewLoadResult::Draft)
        },
        move |result| Message::PreviewLoaded {
            generation,
            photo_id,
            result,
        },
    )
}

fn begin_request(shell: &mut Shell, photo_id: PhotoId) -> Option<u64> {
    let generation = shell.preview_generation.checked_add(1)?;
    shell.preview_generation = generation;
    shell.active_preview = Some((generation, photo_id));
    if !has_rendered_preview(shell, photo_id) {
        replace(shell, photo_id, SelectedPreviewState::Loading);
    }
    Some(generation)
}

fn has_rendered_preview(shell: &Shell, photo_id: PhotoId) -> bool {
    shell
        .ui
        .library_state()
        .ready_workspace()
        .and_then(|workspace| workspace.detail(photo_id))
        .is_some_and(|detail| matches!(detail.selected_preview(), SelectedPreviewState::Ready(_)))
}

fn replace(shell: &mut Shell, photo_id: PhotoId, preview: SelectedPreviewState) {
    let Some(workspace) = shell.ui.library_state().ready_workspace().cloned() else {
        return;
    };
    let Some(workspace) = workspace.with_selected_preview(photo_id, preview) else {
        return;
    };
    shell.ui.set_library_state(LibraryState::Ready(workspace));
}

fn publish(shell: &mut Shell, photo_id: PhotoId, preview: SelectedPreview, status: &str) {
    let (_, dimensions, pixels) = preview.into_parts();
    let ready = PreviewDimensions::new(dimensions.width(), dimensions.height())
        .ok()
        .and_then(|dimensions| {
            Rgba8PreviewMetadata::new(
                dimensions,
                PresentationText::new(status).expect("constant preview status is valid"),
                pixels,
            )
            .ok()
        })
        .map_or_else(failed_state, SelectedPreviewState::Ready);
    replace(shell, photo_id, ready);
}

pub(super) fn failed_state() -> SelectedPreviewState {
    SelectedPreviewState::Failed(SelectedPreviewFailure::new(
        PresentationText::new("The selected preview could not be rendered.")
            .expect("constant failure text is valid"),
    ))
}
