//! Worker-to-GTK bridge for selected darkroom previews and histogram analysis.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use gtk4::glib::{self, ControlFlow};
use rusttable_ui::{
    GtkShell, HistogramData, HistogramError, PresentationText, PreviewDimensions,
    Rgba8PreviewMetadata, ViewportGeneration,
};

use crate::gtk_preview_controller::{GtkPreviewController, GtkPreviewFailureKind, GtkPreviewState};

use super::preview_lifecycle::{PreviewLifecycle, PreviewSelectionToken};

struct PreviewResult {
    token: PreviewSelectionToken,
    state: GtkPreviewState,
    histogram: Option<Result<HistogramData, HistogramError>>,
}

pub(super) fn start_selected_preview(
    shell: &GtkShell,
    catalog: crate::gtk_controller::GtkCatalogController,
    lifecycle: Rc<RefCell<PreviewLifecycle>>,
) {
    let Some(photo_id) = catalog.selected_photo() else {
        shell.clear_darkroom_selection(GtkPreviewFailureKind::NoSelection.message());
        return;
    };
    let token = lifecycle.borrow_mut().begin(photo_id);
    let generation = ViewportGeneration::new(token.generation());
    shell.begin_darkroom_selection(photo_id, generation);
    shell.set_darkroom_preview_loading();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("rusttable-preview".to_owned())
        .spawn(move || {
            let state = GtkPreviewController::new().render_selected(&catalog);
            let histogram = histogram_for_preview(&state);
            let _ = sender.send(PreviewResult {
                token,
                state,
                histogram,
            });
        });
    if worker.is_err() {
        shell.set_darkroom_preview_failure(
            generation,
            GtkPreviewFailureKind::RenderUnavailable.message(),
        );
        return;
    }

    let shell = shell.clone();
    glib::source::timeout_add_local(Duration::from_millis(16), move || {
        match receiver.try_recv() {
            Ok(result) => {
                if lifecycle.borrow().is_current(result.token) {
                    install_preview_state(&shell, result.token, result.state, result.histogram);
                }
                ControlFlow::Break
            }
            Err(TryRecvError::Empty) => ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                if lifecycle.borrow().is_current(token) {
                    shell.set_darkroom_preview_failure(
                        generation,
                        GtkPreviewFailureKind::RenderUnavailable.message(),
                    );
                }
                ControlFlow::Break
            }
        }
    });
}

fn histogram_for_preview(state: &GtkPreviewState) -> Option<Result<HistogramData, HistogramError>> {
    let GtkPreviewState::Ready(rendered) = state else {
        return None;
    };
    let dimensions = PreviewDimensions::new(
        rendered.dimensions().width(),
        rendered.dimensions().height(),
    )
    .map_err(|_| HistogramError::SizeOverflow);
    Some(dimensions.and_then(|dimensions| HistogramData::from_rgba8(dimensions, rendered.pixels())))
}

fn install_preview_state(
    shell: &GtkShell,
    token: PreviewSelectionToken,
    state: GtkPreviewState,
    histogram: Option<Result<HistogramData, HistogramError>>,
) {
    let generation = ViewportGeneration::new(token.generation());
    let GtkPreviewState::Ready(rendered) = state else {
        if let GtkPreviewState::Failed(failure) = state {
            shell.set_darkroom_preview_failure(generation, failure.message());
        }
        return;
    };

    let Ok(dimensions) = PreviewDimensions::new(
        rendered.dimensions().width(),
        rendered.dimensions().height(),
    ) else {
        shell.set_darkroom_preview_failure(
            generation,
            GtkPreviewFailureKind::InvalidRgba8.message(),
        );
        return;
    };
    let Ok(status) = PresentationText::new("rendered") else {
        shell.set_darkroom_preview_failure(
            generation,
            GtkPreviewFailureKind::RenderUnavailable.message(),
        );
        return;
    };
    let Ok(metadata) = Rgba8PreviewMetadata::new(dimensions, status, rendered.pixels().to_vec())
    else {
        shell.set_darkroom_preview_failure(
            generation,
            GtkPreviewFailureKind::InvalidRgba8.message(),
        );
        return;
    };
    let histogram = histogram.unwrap_or(Err(HistogramError::PreviewUnavailable));
    if shell
        .set_darkroom_preview_result(generation, &metadata, histogram)
        .is_err()
    {
        shell.set_darkroom_preview_failure(
            generation,
            GtkPreviewFailureKind::InvalidRgba8.message(),
        );
    }
}
