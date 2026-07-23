//! Worker-to-GTK bridge for selected darkroom previews and histogram analysis.

mod lifecycle;
pub(crate) mod presentation;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use gtk4::glib::{self, ControlFlow};
use rusttable_display_profile::DisplayProfileSnapshot;
use rusttable_image::{CancellationToken, ColorEncoding, DecodedImage};
use rusttable_render::{MipmapLevel, ThumbnailGenerator, ThumbnailRequest, ThumbnailSize};
use rusttable_ui::{
    DisplayPresentationFrame, DisplayPresentationRequest, GtkShell, HistogramData, HistogramError,
    PresentationGeneration, PresentationMode, PresentationText, PresentationTicket,
    PreviewDimensions, Rgba8PreviewMetadata, ViewportGeneration,
};

use crate::composition::thumbnails::ThumbnailLifecycle;
use crate::diagnostics::AppDiagnostics;
use crate::gtk_preview_controller::{GtkPreviewController, GtkPreviewFailureKind, GtkPreviewState};

pub(crate) use lifecycle::PreviewLifecycle;
use lifecycle::PreviewSelectionToken;

struct PreviewResult {
    token: PreviewSelectionToken,
    state: GtkPreviewState,
    histogram: Option<Result<HistogramData, HistogramError>>,
    thumbnail: Option<Rgba8PreviewMetadata>,
}

#[expect(
    clippy::too_many_lines,
    reason = "selected preview keeps worker, generation, and GTK publication failure boundaries together"
)]
pub(crate) fn start_selected_preview(
    shell: &GtkShell,
    catalog: crate::gtk_controller::GtkCatalogController,
    lifecycle: Rc<RefCell<PreviewLifecycle>>,
    thumbnail_lifecycle: &Rc<RefCell<ThumbnailLifecycle>>,
    diagnostics: AppDiagnostics,
    display_profile: Option<&DisplayProfileSnapshot>,
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
    let edit = match catalog.current_edit(photo_id) {
        Ok(Some(edit)) => edit,
        Ok(None) => {
            diagnostics.preview_failure(
                "start_selected_preview",
                "edit_resolution",
                "missing_persisted_edit",
                Some(photo_id),
                None,
                None,
                None,
                None,
            );
            shell.clear_darkroom_selection(GtkPreviewFailureKind::MissingPersistedEdit.message());
            return;
        }
        Err(error) => {
            diagnostics.preview_failure(
                "start_selected_preview",
                "edit_resolution",
                "catalog_unavailable",
                Some(photo_id),
                None,
                None,
                None,
                None,
            );
            tracing::error!(
                target: "rusttable.gtk.preview",
                photo_id = %photo_id,
                cause = %error,
                "could not capture selected edit identity"
            );
            shell.clear_darkroom_selection(GtkPreviewFailureKind::CatalogUnavailable.message());
            return;
        }
    };
    let token = lifecycle
        .borrow_mut()
        .begin(photo_id, edit.id(), edit.revision());
    let generation = ViewportGeneration::new(token.generation());
    shell.begin_darkroom_selection(photo_id, generation);
    if !shell.photo_thumbnail_has_edit_identity(photo_id, edit.id(), edit.revision()) {
        thumbnail_lifecycle.borrow_mut().invalidate(photo_id);
        shell.set_photo_thumbnail_loading(photo_id);
    }
    shell.set_darkroom_preview_loading(generation);
    let (sender, receiver) = mpsc::channel();
    let worker_diagnostics = diagnostics.clone();
    let display_profile_for_worker = display_profile.cloned();
    let worker = thread::Builder::new()
        .name("rusttable-preview".to_owned())
        .spawn(move || {
            let state = GtkPreviewController::render_selected_with_generation_for_edit(
                &catalog,
                &worker_diagnostics,
                edit.id(),
                edit.revision(),
                token.generation(),
                display_profile_for_worker.as_ref(),
            );
            let histogram = histogram_for_preview(&state);
            let thumbnail = thumbnail_for_preview(&state);
            let _ = sender.send(PreviewResult {
                token,
                state,
                histogram,
                thumbnail,
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
                        result.thumbnail,
                        &diagnostics,
                    );
                });
                if !accepted {
                    diagnostics.preview_failure(
                        "install_preview_state",
                        "stale_generation",
                        "viewport_generation_mismatch",
                        Some(token.photo_id()),
                        Some(token.edit_id()),
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
            edit_id = %token.edit_id(),
            edit_revision = %token.edit_revision(),
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

fn thumbnail_for_preview(state: &GtkPreviewState) -> Option<Rgba8PreviewMetadata> {
    let rendered = state.ready()?;
    let source = DecodedImage::new_with_color_encoding(
        rendered.dimensions(),
        rendered.pixels().to_vec(),
        ColorEncoding::Srgb,
    )
    .ok()?;
    // Keep the shared publication below ThumbnailSurface's 2 MiB bound while retaining enough
    // pixels for Darktable's full-canvas Lighttable preview. Filmstrip and navigation consumers
    // deterministically downsample this same edit-identity-safe result to their smaller targets.
    let size = ThumbnailSize::fit(864, 576).ok()?;
    let request = ThumbnailRequest::new(MipmapLevel::zero(), size);
    let thumbnail =
        ThumbnailGenerator::generate(&source, request, 2 * 1024 * 1024, &CancellationToken::new())
            .ok()?;
    let dimensions = PreviewDimensions::new(
        thumbnail.dimensions().width(),
        thumbnail.dimensions().height(),
    )
    .ok()?;
    let status = PresentationText::new("edited preview ready").ok()?;
    Rgba8PreviewMetadata::new(dimensions, status, thumbnail.pixels().to_vec()).ok()
}

#[expect(
    clippy::too_many_lines,
    reason = "selected preview publication keeps validation, diagnostics, and GTK projection together"
)]
fn install_preview_state(
    shell: &GtkShell,
    token: PreviewSelectionToken,
    state: GtkPreviewState,
    histogram: Option<Result<HistogramData, HistogramError>>,
    thumbnail: Option<Rgba8PreviewMetadata>,
    diagnostics: &AppDiagnostics,
) {
    let generation = ViewportGeneration::new(token.generation());
    let GtkPreviewState::Ready(rendered) = state else {
        if let GtkPreviewState::Failed(failure) = state {
            shell.set_darkroom_preview_failure(generation, failure.message());
        }
        return;
    };
    if let Some(receipt) = rendered.presentation_receipt() {
        tracing::debug!(
            target: "rusttable.gtk.preview",
            scene_identity = ?receipt.scene_identity(),
            edit_identity = ?receipt.edit_identity(),
            presentation_identity = ?receipt.presentation_identity(),
            monitor = ?receipt.monitor(),
            profile_id = ?receipt.profile_id(),
            profile_generation = receipt.generation(),
            intent = ?receipt.intent(),
            fallback = ?receipt.fallback(),
            "selected preview presentation receipt"
        );
    }

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
    let receipt = rendered
        .presentation_receipt()
        .expect("presented selected previews carry a presentation receipt");
    let render_receipt = rendered
        .receipt()
        .expect("presented selected previews carry a render receipt");
    let request = DisplayPresentationRequest::new(
        rendered.photo_id(),
        render_receipt.edit_revision(),
        receipt.monitor(),
        PresentationGeneration::new(receipt.generation()),
        PresentationMode::Sdr,
    );
    let Ok(frame) = DisplayPresentationFrame::new(
        PresentationTicket::new(request),
        dimensions,
        rendered.pixels().to_vec(),
        rendered.presentation_status(),
    ) else {
        diagnostics.preview_failure(
            "install_preview_state",
            "display_presentation",
            "invalid_frame",
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
        .set_darkroom_presentation_result_for_edit(
            generation,
            &frame,
            histogram,
            render_receipt.edit_id(),
            render_receipt.edit_revision(),
        )
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
        return;
    }
    if let Some(thumbnail) = thumbnail {
        if shell
            .set_darkroom_preview_thumbnail_for_edit(
                generation,
                &thumbnail,
                render_receipt.edit_id(),
                render_receipt.edit_revision(),
            )
            .is_err()
        {
            diagnostics.preview_failure(
                "install_preview_state",
                "thumbnail",
                "gtk_texture_adaptation",
                Some(rendered.photo_id()),
                Some(render_receipt.edit_id()),
                Some(generation.get()),
                None,
                Some(rendered.dimensions()),
            );
            shell.set_photo_thumbnail_unavailable(rendered.photo_id());
        }
    } else {
        diagnostics.preview_failure(
            "install_preview_state",
            "thumbnail",
            "thumbnail_generation_unavailable",
            Some(rendered.photo_id()),
            Some(render_receipt.edit_id()),
            Some(generation.get()),
            None,
            Some(rendered.dimensions()),
        );
        shell.set_photo_thumbnail_unavailable(rendered.photo_id());
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

    use rusttable_core::{EditId, PhotoId, Revision};
    use rusttable_ui::HistogramError;

    use super::{PreviewLifecycle, histogram_cause, install_if_current};

    fn photo_id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    #[test]
    fn completion_callback_releases_lifecycle_borrow_before_reentry() {
        let lifecycle = RefCell::new(PreviewLifecycle::default());
        let completed = lifecycle.borrow_mut().begin(
            photo_id(1),
            EditId::new(2).unwrap(),
            Revision::from_u64(1),
        );

        install_if_current(&lifecycle, completed, || {
            lifecycle.borrow_mut().begin(
                photo_id(2),
                EditId::new(3).unwrap(),
                Revision::from_u64(1),
            );
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
