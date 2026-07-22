use crate::presentation::{
    PresentationText, Rgba8PreviewMetadata, SelectedPreviewFailure, SelectedPreviewState,
};
use crate::viewport_presentation::DisplayPresentationFrame;
use crate::{HistogramData, HistogramError, ViewportGeneration};
use rusttable_core::{EditId, Revision};
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

    /// Clears the old full-size texture while the selected preview is computed off-loop.
    ///
    /// A ready catalog thumbnail is real image data from the same photo, so it remains visible in
    /// the navigation/filmstrip surfaces and provides a provisional RGB histogram. The completed
    /// preview still replaces that lower-resolution histogram through the generation-checked path.
    pub fn set_darkroom_preview_loading(&self, generation: ViewportGeneration) {
        let selected_photo = self.darkroom.viewport_state().photo_id();
        self.replace_selected_preview_projection(generation, SelectedPreviewState::Loading);
        self.darkroom_preview.set_loading();
        self.darkroom.set_status("loading preview");
        if let Some(photo_id) = selected_photo {
            self.invalidate_photo_thumbnail_edit_identity(photo_id);
            if let Some(metadata) = self.photo_thumbnail_metadata(photo_id) {
                if let Err(error) = self.darkroom.set_navigation_preview(&metadata) {
                    tracing::warn!(
                        target: "rusttable.gtk.preview",
                        generation = generation.get(),
                        cause = ?error,
                        "could not retain selected thumbnail in darkroom navigation"
                    );
                }
                let histogram = HistogramData::from_rgba8(metadata.dimensions(), metadata.pixels());
                let _ = self.darkroom.set_histogram_result(generation, histogram);
            } else {
                self.set_photo_thumbnail_loading(photo_id);
            }
        }
    }

    /// Restores the truthful empty darkroom surface when no catalog photo is selected.
    pub fn clear_darkroom_selection(&self, message: &str) {
        let selected_photo = self.darkroom.viewport_state().photo_id();
        self.darkroom.clear_viewport_selection();
        self.darkroom_preview.set_failure(message);
        self.darkroom.set_status(message);
        if let Some(photo_id) = selected_photo {
            self.set_photo_thumbnail_unavailable(photo_id);
        }
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

    /// Installs a completed unprofiled preview while recording its edit identity.
    ///
    /// # Errors
    ///
    /// Returns a texture adaptation error when the validated RGBA8 payload cannot be installed.
    pub fn set_darkroom_preview_result_for_edit(
        &self,
        generation: ViewportGeneration,
        metadata: &Rgba8PreviewMetadata,
        histogram: Result<HistogramData, HistogramError>,
        edit_id: EditId,
        edit_revision: Revision,
    ) -> Result<(), crate::gtk_shell::PhotoPreviewTextureError> {
        self.darkroom
            .set_viewport_edit_revision(edit_revision, generation);
        tracing::debug!(
            target: "rusttable.gtk.preview",
            photo_id = ?self.darkroom.viewport_state().photo_id(),
            edit_id = %edit_id,
            edit_revision = %edit_revision,
            "publishing selected preview edit identity"
        );
        self.set_darkroom_preview_result(generation, metadata, histogram)
    }

    /// Publishes the thumbnail generated from the same completed presented preview as darkroom.
    ///
    /// The viewport generation and edit identity are checked before touching either shared
    /// thumbnail surface, so a late result cannot restore pixels from an older edit.
    ///
    /// # Errors
    ///
    /// Returns a texture adaptation error when the bounded thumbnail cannot be installed.
    pub fn set_darkroom_preview_thumbnail_for_edit(
        &self,
        generation: ViewportGeneration,
        metadata: &Rgba8PreviewMetadata,
        edit_id: EditId,
        edit_revision: Revision,
    ) -> Result<(), crate::gtk_shell::PhotoPreviewTextureError> {
        if !self.accepts_darkroom_preview_generation(generation) {
            tracing::warn!(
                target: "rusttable.gtk.preview",
                generation = generation.get(),
                edit_id = %edit_id,
                edit_revision = %edit_revision,
                "ignored stale selected-preview thumbnail"
            );
            return Ok(());
        }
        let viewport = self.darkroom.viewport_state();
        if viewport.edit_revision() != Some(edit_revision) {
            tracing::warn!(
                target: "rusttable.gtk.preview",
                generation = generation.get(),
                edit_id = %edit_id,
                edit_revision = %edit_revision,
                viewport_revision = ?viewport.edit_revision(),
                "ignored selected-preview thumbnail for another edit"
            );
            return Ok(());
        }
        let Some(photo_id) = viewport.photo_id() else {
            return Ok(());
        };
        self.set_photo_thumbnail_for_edit(photo_id, metadata, edit_id, edit_revision)
    }

    /// Installs a generation-checked, typed color-presentation frame and its histogram.
    ///
    /// The frame carries the selected photo and active monitor-profile generation. GTK receives
    /// only the validated presentation boundary; scene/edit cache identities remain in the
    /// application preview receipt.
    ///
    /// # Errors
    ///
    /// Returns a texture error when the validated frame dimensions cannot be represented by GTK.
    pub fn set_darkroom_presentation_result(
        &self,
        generation: ViewportGeneration,
        frame: &DisplayPresentationFrame,
        histogram: Result<HistogramData, HistogramError>,
    ) -> Result<(), crate::gtk_shell::PhotoPreviewTextureError> {
        if !self.accepts_darkroom_preview_generation(generation) {
            tracing::warn!(
                target: "rusttable.gtk.preview",
                generation = generation.get(),
                stage = "stale_generation",
                cause = "viewport_generation_mismatch",
                "ignored stale presentation result"
            );
            return Ok(());
        }
        let expected_photo = self.darkroom.viewport_state().photo_id();
        if expected_photo != Some(frame.ticket().request().photo_id()) {
            tracing::warn!(
                target: "rusttable.gtk.preview",
                generation = generation.get(),
                stage = "stale_photo",
                cause = "presentation_photo_mismatch",
                "ignored presentation result for another photo"
            );
            return Ok(());
        }
        self.set_darkroom_presentation(frame).inspect_err(|error| {
            tracing::error!(
                target: "rusttable.gtk.preview",
                generation = generation.get(),
                stage = "texture",
                cause = ?error,
                width = frame.metadata().dimensions().width(),
                height = frame.metadata().dimensions().height(),
                "presentation texture adaptation failed"
            );
        })?;
        self.replace_selected_preview_projection(
            generation,
            SelectedPreviewState::Ready(frame.metadata().clone()),
        );
        if let Err(error) = &histogram {
            tracing::warn!(
                target: "rusttable.gtk.preview",
                generation = generation.get(),
                stage = "histogram",
                cause = ?error,
                width = frame.metadata().dimensions().width(),
                height = frame.metadata().dimensions().height(),
                "preview histogram unavailable"
            );
        }
        let _ = self.darkroom.set_histogram_result(generation, histogram);
        self.darkroom.sync_viewport_projection();
        self.darkroom.set_status(&frame.status().label());
        Ok(())
    }

    /// Installs a completed preview and records the edit identity that owns the shared viewport.
    ///
    /// # Errors
    ///
    /// Returns a texture adaptation error when the validated presentation frame cannot be
    /// installed.
    pub fn set_darkroom_presentation_result_for_edit(
        &self,
        generation: ViewportGeneration,
        frame: &DisplayPresentationFrame,
        histogram: Result<HistogramData, HistogramError>,
        edit_id: EditId,
        edit_revision: Revision,
    ) -> Result<(), crate::gtk_shell::PhotoPreviewTextureError> {
        self.darkroom
            .set_viewport_edit_revision(edit_revision, generation);
        tracing::debug!(
            target: "rusttable.gtk.preview",
            photo_id = %frame.ticket().request().photo_id(),
            edit_id = %edit_id,
            edit_revision = %edit_revision,
            "publishing selected preview edit identity"
        );
        self.set_darkroom_presentation_result(generation, frame, histogram)
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
        if let Some(photo_id) = self.darkroom.viewport_state().photo_id() {
            self.set_photo_thumbnail_unavailable(photo_id);
        }
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
