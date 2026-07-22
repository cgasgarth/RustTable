use std::cell::RefCell;
use std::rc::Rc;

use rusttable_core::PhotoId;

use super::GtkShell;

pub(super) type ThumbnailWindowChanged = Rc<RefCell<Option<Box<dyn Fn()>>>>;

impl GtkShell {
    /// Installs the application-owned thumbnail worker wake-up for `GridView` realization changes.
    pub fn connect_thumbnail_window_changed<F>(&self, handler: F)
    where
        F: Fn() + 'static,
    {
        self.thumbnail_window_changed
            .replace(Some(Box::new(handler)));
    }

    /// Returns only GTK-realized lighttable items eligible for thumbnail work.
    ///
    /// `GridView` owns realization and keeps this set bounded to the active viewport and
    /// GTK's prefetch window; the catalog size never expands thumbnail work.
    #[must_use]
    pub fn lighttable_thumbnail_photo_ids(&self) -> Vec<PhotoId> {
        self.photo_tiles
            .borrow()
            .iter()
            .filter_map(|(photo_id, tile)| tile.lighttable_button.is_some().then_some(*photo_id))
            .collect()
    }

    /// Reports whether the current tile has a published result that can survive a rebuild.
    #[must_use]
    pub fn photo_thumbnail_has_terminal_state(&self, photo_id: PhotoId) -> bool {
        self.photo_tiles
            .borrow()
            .get(&photo_id)
            .is_some_and(|tile| tile.thumbnails.state().is_terminal())
    }
}
