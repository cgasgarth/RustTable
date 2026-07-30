//! Cold-start selection restoration for the first darkroom presentation.

use crate::gtk_controller::CollectionController;
use rusttable_core::PhotoId;
use rusttable_ui::{GtkShell, LighttablePhotoState};

/// Returns the first persisted selection in the same deterministic order shown by the collection.
///
/// The collection projection is restored before GTK callbacks are installed during a cold start.
/// Keeping this lookup separate makes the installed-flow binding explicit and prevents startup
/// from inventing a second selection order that could disagree with the visible filmstrip.
pub(super) fn first_persisted_selected_photo(controller: &CollectionController) -> Option<PhotoId> {
    controller
        .snapshot()
        .photo_states()
        .find(|state| state.selected())
        .map(LighttablePhotoState::photo_id)
}

/// Binds a restored selection through the normal interactive path before first presentation.
///
/// `open_photo` is deliberately reused here: it updates the application selection token, starts
/// the generation-tagged preview, synchronizes the histogram/rails/filmstrip, and only then makes
/// the darkroom stack child visible. This keeps cold launch behavior identical to a user click.
pub(super) fn bind_persisted_darkroom_selection(shell: &GtkShell, photo_id: Option<PhotoId>) {
    if let Some(photo_id) = photo_id {
        tracing::debug!(
            target: "rusttable.gtk.darkroom",
            photo_id = %photo_id,
            "binding persisted darkroom selection before window presentation"
        );
        if !shell.open_photo(photo_id) {
            tracing::warn!(
                target: "rusttable.gtk.darkroom",
                photo_id = %photo_id,
                "persisted darkroom selection was not present in the rendered workspace"
            );
        }
    }
}
