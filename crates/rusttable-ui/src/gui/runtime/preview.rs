use crate::presentation::{
    PresentationText, Rgba8PreviewMetadata, SelectedPreviewFailure, SelectedPreviewState,
};
use crate::{HistogramData, HistogramError, ViewportGeneration};
use rusttable_display_profile::{
    DisplayProfileReceipt, DisplayProfileSnapshot, ProfileSelection, SelectionStatus,
};

use crate::libs::profiles::diagnostics::ProfileDiagnosticRequest;

use super::GtkShell;

impl GtkShell {
    /// Applies one display-profile decision to both the header and darkroom render status.
    pub fn set_display_profile_state(
        &self,
        snapshot: Option<&DisplayProfileSnapshot>,
        receipt: Option<DisplayProfileReceipt>,
    ) {
        self.display_profile_banner.set_snapshot(snapshot);
        self.darkroom.set_profile_diagnostic_state(
            snapshot,
            receipt,
            ProfileDiagnosticRequest::new(),
        );
        self.preview_profile_fallback
            .set(snapshot.is_none_or(|snapshot| {
                snapshot.status() != SelectionStatus::Active
                    || snapshot.selection() == ProfileSelection::Unprofiled
            }));
    }

    /// Returns the explicit display contract attached to the next selected preview.
    #[must_use]
    pub fn darkroom_preview_status(&self) -> &'static str {
        if self.preview_profile_fallback.get() {
            "rendered · sRGB display fallback · profile evidence unavailable"
        } else {
            "rendered"
        }
    }

    /// Clears the old texture while the selected preview and its histogram are computed off-loop.
    pub fn set_darkroom_preview_loading(&self, generation: ViewportGeneration) {
        self.replace_selected_preview_projection(generation, SelectedPreviewState::Loading);
        self.darkroom_preview.set_loading();
        self.darkroom.set_status("loading preview");
    }

    /// Restores the truthful empty darkroom surface when no catalog photo is selected.
    pub fn clear_darkroom_selection(&self, message: &str) {
        self.darkroom.clear_viewport_selection();
        self.darkroom_preview.set_failure(message);
        self.darkroom.set_status(message);
    }

    /// Installs a validated worker result and its already-computed histogram on the GTK surface.
    ///
    /// The darkroom checks the generation again before publishing histogram data, so a late
    /// worker cannot restore data from an older selection.
    ///
    /// # Errors
    ///
    /// Returns a texture error when the validated preview dimensions cannot be represented by
    /// GTK's memory texture API.
    pub fn set_darkroom_preview_result(
        &self,
        generation: ViewportGeneration,
        metadata: &Rgba8PreviewMetadata,
        histogram: Result<HistogramData, HistogramError>,
    ) -> Result<(), crate::gtk_shell::PhotoPreviewTextureError> {
        if !self.accepts_darkroom_preview_generation(generation) {
            tracing::warn!(
                target: "rusttable.gtk.preview",
                generation = generation.get(),
                stage = "stale_generation",
                cause = "viewport_generation_mismatch",
                "ignored stale preview result"
            );
            return Ok(());
        }
        self.darkroom_preview
            .set_rgba8(metadata)
            .inspect_err(|error| {
                tracing::error!(
                    target: "rusttable.gtk.preview",
                    generation = generation.get(),
                    stage = "texture",
                    cause = ?error,
                    width = metadata.dimensions().width(),
                    height = metadata.dimensions().height(),
                    "preview texture adaptation failed"
                );
            })?;
        self.replace_selected_preview_projection(
            generation,
            SelectedPreviewState::Ready(metadata.clone()),
        );
        if let Err(error) = &histogram {
            tracing::warn!(
                target: "rusttable.gtk.preview",
                generation = generation.get(),
                stage = "histogram",
                cause = ?error,
                width = metadata.dimensions().width(),
                height = metadata.dimensions().height(),
                "preview histogram unavailable"
            );
        }
        let _ = self.darkroom.set_histogram_result(generation, histogram);
        self.darkroom.sync_viewport_projection();
        self.darkroom.set_status(metadata.status().as_str());
        Ok(())
    }

    /// Projects a preview failure and removes any histogram that belonged to its frame.
    pub fn set_darkroom_preview_failure(&self, generation: ViewportGeneration, message: &str) {
        if !self.accepts_darkroom_preview_generation(generation) {
            tracing::warn!(
                target: "rusttable.gtk.preview",
                generation = generation.get(),
                stage = "stale_generation",
                cause = "failure_for_old_viewport",
                "ignored stale preview failure"
            );
            return;
        }
        if let Ok(detail) = PresentationText::new(message) {
            self.replace_selected_preview_projection(
                generation,
                SelectedPreviewState::Failed(SelectedPreviewFailure::new(detail)),
            );
        }
        self.darkroom_preview.set_failure(message);
        let _ = self
            .darkroom
            .set_histogram_result(generation, Err(HistogramError::PreviewUnavailable));
        self.darkroom.set_status(message);
    }

    fn accepts_darkroom_preview_generation(&self, generation: ViewportGeneration) -> bool {
        let viewport = self.darkroom.viewport_state();
        viewport.photo_id().is_some() && viewport.generation() == generation
    }

    fn replace_selected_preview_projection(
        &self,
        generation: ViewportGeneration,
        state: SelectedPreviewState,
    ) {
        if !self.accepts_darkroom_preview_generation(generation) {
            return;
        }
        let Some(photo_id) = self.darkroom.viewport_state().photo_id() else {
            return;
        };
        let Some(workspace) = self
            .lighttable_workspace
            .borrow()
            .as_ref()
            .cloned()
            .and_then(|workspace| workspace.with_selected_preview(photo_id, state))
        else {
            return;
        };
        let detail = workspace.detail(photo_id).cloned();
        self.lighttable_workspace.replace(Some(workspace));
        if let Some(detail) = detail {
            self.photo_details.borrow_mut().insert(photo_id, detail);
        }
    }
}
