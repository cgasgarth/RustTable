//! Shared thumbnail and darkroom-navigation publication.

use rusttable_core::{EditId, PhotoId, Revision};

use super::GtkShell;

impl GtkShell {
    pub(super) fn photo_thumbnail_metadata(
        &self,
        photo_id: PhotoId,
    ) -> Option<crate::presentation::Rgba8PreviewMetadata> {
        self.photo_tiles
            .borrow()
            .get(&photo_id)
            .and_then(|tile| tile.thumbnails.ready_metadata())
    }

    pub(super) fn invalidate_photo_thumbnail_edit_identity(&self, photo_id: PhotoId) {
        self.thumbnail_edit_identities
            .borrow_mut()
            .remove(&photo_id);
    }

    /// Installs a background-rendered thumbnail into the synchronized grid and filmstrip tiles.
    ///
    /// # Errors
    ///
    /// Returns a typed texture error when validated dimensions exceed GTK's representation.
    pub fn set_photo_thumbnail(
        &self,
        photo_id: PhotoId,
        metadata: &crate::presentation::Rgba8PreviewMetadata,
    ) -> Result<(), crate::widgets::preview::PhotoPreviewTextureError> {
        if let Some(tile) = self.photo_tiles.borrow().get(&photo_id) {
            tile.thumbnails.set_rgba8(metadata)?;
        }
        if self.darkroom.viewport_state().photo_id() == Some(photo_id) {
            self.darkroom.set_navigation_preview(metadata)?;
        }
        Ok(())
    }

    /// Publishes a thumbnail together with the exact edit identity it rendered.
    ///
    /// The identity is retained outside the GTK child tree so a GridView/filmstrip rebuild can
    /// reject a terminal tile that belongs to an older edit revision.
    ///
    /// # Errors
    ///
    /// Returns a texture adaptation error when the validated RGBA8 payload cannot be installed.
    pub fn set_photo_thumbnail_for_edit(
        &self,
        photo_id: PhotoId,
        metadata: &crate::presentation::Rgba8PreviewMetadata,
        edit_id: EditId,
        edit_revision: Revision,
    ) -> Result<(), crate::widgets::preview::PhotoPreviewTextureError> {
        let result = self.set_photo_thumbnail(photo_id, metadata);
        if result.is_ok() {
            self.thumbnail_edit_identities
                .borrow_mut()
                .insert(photo_id, (edit_id, edit_revision));
        }
        result
    }

    #[must_use]
    pub fn photo_thumbnail_edit_identity(&self, photo_id: PhotoId) -> Option<(EditId, Revision)> {
        self.thumbnail_edit_identities
            .borrow()
            .get(&photo_id)
            .copied()
    }

    #[must_use]
    pub fn photo_thumbnail_has_edit_identity(
        &self,
        photo_id: PhotoId,
        edit_id: EditId,
        edit_revision: Revision,
    ) -> bool {
        self.photo_thumbnail_edit_identity(photo_id) == Some((edit_id, edit_revision))
    }

    pub fn set_photo_thumbnail_loading(&self, photo_id: PhotoId) {
        self.invalidate_photo_thumbnail_edit_identity(photo_id);
        if let Some(tile) = self.photo_tiles.borrow().get(&photo_id) {
            tile.thumbnails.set_loading();
        }
        if self.darkroom.viewport_state().photo_id() == Some(photo_id) {
            self.darkroom.set_navigation_preview_loading();
        }
    }

    /// Projects a bounded background-rendering failure onto both thumbnail surfaces.
    pub fn set_photo_thumbnail_failed(&self, photo_id: PhotoId) {
        self.thumbnail_edit_identities
            .borrow_mut()
            .remove(&photo_id);
        if let Some(tile) = self.photo_tiles.borrow().get(&photo_id) {
            tile.thumbnails.set_failed();
        }
        if self.darkroom.viewport_state().photo_id() == Some(photo_id) {
            self.darkroom.set_navigation_preview_failed();
        }
    }

    /// Removes thumbnail pixels when the edited preview cannot be produced.
    pub fn set_photo_thumbnail_unavailable(&self, photo_id: PhotoId) {
        self.thumbnail_edit_identities
            .borrow_mut()
            .remove(&photo_id);
        if let Some(tile) = self.photo_tiles.borrow().get(&photo_id) {
            tile.thumbnails.set_unavailable();
        }
        if self.darkroom.viewport_state().photo_id() == Some(photo_id) {
            self.darkroom.set_navigation_preview_unavailable();
        }
    }
}
