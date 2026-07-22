//! Worker-to-GTK bridge for selected darkroom previews and histogram analysis.

mod lifecycle;

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

use crate::diagnostics::AppDiagnostics;
use crate::gtk_preview_controller::{GtkPreviewController, GtkPreviewFailureKind, GtkPreviewState};

pub(crate) use lifecycle::PreviewLifecycle;
use lifecycle::PreviewSelectionToken;

struct PreviewResult {
    token: PreviewSelectionToken,
    state: GtkPreviewState,
    histogram: Option<Result<HistogramData, HistogramError>>,
}

#[expect(
    clippy::too_many_lines,
    reason = "selected preview keeps worker, generation, and GTK publication failure boundaries together"
)]
pub(crate) fn start_selected_preview(
    shell: &GtkShell,
    catalog: crate::gtk_controller::GtkCatalogController,
    lifecycle: Rc<RefCell<PreviewLifecycle>>,
    diagnostics: AppDiagnostics,
) {
    let Some(photo_id) = catalog.selected_photo() else {
        diagnostics.preview_failure(
            "start_selected_preview",
            "catalog_lookup",
            "no_selection",
            None,
            None,
            None,
            None,
            None,
        );
        shell.clear_darkroom_selection(GtkPreviewFailureKind::NoSelection.message());
        return;
    };
    let token = lifecycle.borrow_mut().begin(photo_id);
    let generation = ViewportGeneration::new(token.generation());
    shell.begin_darkroom_selection(photo_id, generation);
    shell.set_darkroom_preview_loading(generation);
    let (sender, receiver) = mpsc::channel();
    let worker_diagnostics = diagnostics.clone();
    let worker = thread::Builder::new()
        .name("rusttable-preview".to_owned())
        .spawn(move || {
            let state = GtkPreviewController::render_selected_with_diagnostics(
                &catalog,
                &worker_diagnostics,
            );
            let histogram = histogram_for_preview(&state);
            let _ = sender.send(PreviewResult {
                token,
                state,
                histogram,
            });
        });
    if worker.is_err() {
        diagnostics.preview_failure(
            "start_selected_preview",
            "processing",
            "worker_spawn",
            Some(photo_id),
            None,
            Some(generation.get()),
            None,
            None,
        );
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
                let token = result.token;
                let accepted = install_if_current(&lifecycle, token, || {
                    install_preview_state(
                        &shell,
                        result.token,
                        result.state,
                        result.histogram,
                        &diagnostics,
                    );
                });
                if !accepted {
                    diagnostics.preview_failure(
                        "install_preview_state",
                        "stale_generation",
                        "viewport_generation_mismatch",
                        Some(token.photo_id()),
                        None,
                        Some(token.generation()),
                        None,
                        None,
                    );
                }
                ControlFlow::Break
            }
            Err(TryRecvError::Empty) => ControlFlow::Continue,
            Err(TryRecvError::Disconnected) => {
                if lifecycle.borrow().is_current(token) {
                    diagnostics.preview_failure(
                        "start_selected_preview",
                        "processing",
                        "worker_disconnected",
                        Some(token.photo_id()),
                        None,
                        Some(generation.get()),
                        None,
                        None,
                    );
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

fn install_if_current(
    lifecycle: &RefCell<PreviewLifecycle>,
    token: PreviewSelectionToken,
    install: impl FnOnce(),
) -> bool {
    let is_current = lifecycle.borrow().is_current(token);
    if is_current {
        install();
    } else {
        tracing::warn!(
            target: "rusttable.gtk.preview",
            generation = token.generation(),
            "discarding stale preview result"
        );
    }
    is_current
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
    diagnostics: &AppDiagnostics,
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
        diagnostics.preview_failure(
            "install_preview_state",
            "texture",
            "dimension_conversion",
            Some(rendered.photo_id()),
            None,
            Some(generation.get()),
            None,
            Some(rendered.dimensions()),
        );
        shell.set_darkroom_preview_failure(
            generation,
            GtkPreviewFailureKind::InvalidRgba8.message(),
        );
        return;
    };
    let Ok(status) = PresentationText::new(shell.darkroom_preview_status()) else {
        diagnostics.preview_failure(
            "install_preview_state",
            "texture",
            "status_text",
            Some(rendered.photo_id()),
            None,
            Some(generation.get()),
            None,
            Some(rendered.dimensions()),
        );
        shell.set_darkroom_preview_failure(
            generation,
            GtkPreviewFailureKind::RenderUnavailable.message(),
        );
        return;
    };
    let Ok(metadata) = Rgba8PreviewMetadata::new(dimensions, status, rendered.pixels().to_vec())
    else {
        diagnostics.preview_failure(
            "install_preview_state",
            "texture",
            "metadata_validation",
            Some(rendered.photo_id()),
            None,
            Some(generation.get()),
            None,
            Some(rendered.dimensions()),
        );
        shell.set_darkroom_preview_failure(
            generation,
            GtkPreviewFailureKind::InvalidRgba8.message(),
        );
        return;
    };
    let histogram = histogram.unwrap_or(Err(HistogramError::PreviewUnavailable));
    if let Err(error) = &histogram {
        diagnostics.preview_failure(
            "install_preview_state",
            "histogram",
            histogram_cause(error),
            Some(rendered.photo_id()),
            None,
            Some(generation.get()),
            None,
            Some(rendered.dimensions()),
        );
    }
    if shell
        .set_darkroom_preview_result(generation, &metadata, histogram)
        .is_err()
    {
        diagnostics.preview_failure(
            "install_preview_state",
            "texture",
            "gtk_texture_adaptation",
            Some(rendered.photo_id()),
            None,
            Some(generation.get()),
            None,
            Some(rendered.dimensions()),
        );
        shell.set_darkroom_preview_failure(
            generation,
            GtkPreviewFailureKind::InvalidRgba8.message(),
        );
    }
}

fn histogram_cause(error: &HistogramError) -> &'static str {
    match error {
        HistogramError::SizeOverflow => "size_overflow",
        HistogramError::Empty => "empty",
        HistogramError::PreviewUnavailable => "preview_unavailable",
        HistogramError::IncorrectByteLength { .. } => "incorrect_byte_length",
        HistogramError::IncorrectSampleLength { .. } => "incorrect_sample_length",
        HistogramError::NonFinite { .. } => "non_finite",
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use rusttable_core::PhotoId;
    use rusttable_ui::HistogramError;

    use super::{PreviewLifecycle, histogram_cause, install_if_current};

    fn photo_id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    #[test]
    fn completion_callback_releases_lifecycle_borrow_before_reentry() {
        let lifecycle = RefCell::new(PreviewLifecycle::default());
        let completed = lifecycle.borrow_mut().begin(photo_id(1));

        install_if_current(&lifecycle, completed, || {
            lifecycle.borrow_mut().begin(photo_id(2));
        });

        assert!(!lifecycle.borrow().is_current(completed));
    }

    #[test]
    fn histogram_failures_have_stable_causes() {
        assert_eq!(
            histogram_cause(&HistogramError::IncorrectByteLength {
                expected: 8,
                actual: 4,
            }),
            "incorrect_byte_length"
        );
        assert_eq!(
            histogram_cause(&HistogramError::PreviewUnavailable),
            "preview_unavailable"
        );
    }
}
