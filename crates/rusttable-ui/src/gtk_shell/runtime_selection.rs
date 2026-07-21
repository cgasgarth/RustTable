//! Programmatic photo selection shared by native-open and interactive GTK paths.

use rusttable_core::PhotoId;

use super::GtkShell;

impl GtkShell {
    /// Opens a photo through the same selection path used by a lighttable click.
    ///
    /// Native file-open delivery uses this entry point after importing a source so the
    /// application presents the imported photo instead of leaving the catalog unselected.
    /// Returns `false` when the current workspace does not contain `photo_id`.
    pub fn open_photo(&self, photo_id: PhotoId) -> bool {
        self.workspace_render_handle().open_photo_by_id(photo_id)
    }
}
