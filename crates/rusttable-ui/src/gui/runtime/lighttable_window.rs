use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gtk4::prelude::WidgetExt;
use rusttable_core::PhotoId;

use super::GtkShell;
use crate::gui::DARKTABLE_DESKTOP_SPEC;
use crate::gui::darktable_spec::FILMSTRIP_ITEM_GAP_PX;

pub(super) type ThumbnailWindowChanged = Rc<RefCell<Option<Box<dyn Fn()>>>>;

const MAX_FILMSTRIP_THUMBNAIL_WINDOW: usize = 16;
const FILMSTRIP_PREFETCH_ITEMS: usize = 2;

impl GtkShell {
    /// Installs the application-owned thumbnail worker wake-up for `GridView` realization changes.
    pub fn connect_thumbnail_window_changed<F>(&self, handler: F)
    where
        F: Fn() + 'static,
    {
        self.thumbnail_window_changed
            .replace(Some(Box::new(handler)));
    }

    /// Returns realized lighttable items plus a bounded visible filmstrip window.
    ///
    /// `GridView` owns realization and keeps this set bounded to the active viewport and
    /// GTK's prefetch window. The filmstrip contributes only the selected/focused item's visible
    /// neighbors, so every painted strip tile can converge without expanding work to catalog size.
    #[must_use]
    pub fn lighttable_thumbnail_photo_ids(&self) -> Vec<PhotoId> {
        let tiles = self.photo_tiles.borrow();
        let mut photo_ids = tiles
            .iter()
            .filter_map(|(photo_id, tile)| tile.lighttable_button.is_some().then_some(*photo_id))
            .collect::<BTreeSet<_>>();
        let ordered = self
            .lighttable_interaction
            .borrow()
            .filmstrip_ids()
            .collect::<Vec<_>>();
        let active = self
            .darkroom
            .filmstrip_selection()
            .or_else(|| self.lighttable_interaction.borrow().focus());
        let item_width = usize::from(DARKTABLE_DESKTOP_SPEC.layout.filmstrip_heights.preferred_px)
            .saturating_add(usize::from(FILMSTRIP_ITEM_GAP_PX));
        let allocated_width = usize::try_from(self.filmstrip_root.allocated_width()).unwrap_or(0);
        let capacity = allocated_width
            .checked_div(item_width.max(1))
            .unwrap_or(0)
            .saturating_add(FILMSTRIP_PREFETCH_ITEMS)
            .clamp(1, MAX_FILMSTRIP_THUMBNAIL_WINDOW);
        for photo_id in bounded_filmstrip_window(&ordered, active, capacity) {
            if tiles.contains_key(&photo_id) {
                photo_ids.insert(photo_id);
            }
        }
        photo_ids.into_iter().collect()
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

fn bounded_filmstrip_window(
    ordered: &[PhotoId],
    active: Option<PhotoId>,
    capacity: usize,
) -> impl Iterator<Item = PhotoId> + '_ {
    let count = capacity.max(1).min(ordered.len());
    let active_index = active
        .and_then(|photo_id| ordered.iter().position(|candidate| *candidate == photo_id))
        .unwrap_or(0);
    let start = active_index
        .saturating_sub(count / 2)
        .min(ordered.len().saturating_sub(count));
    ordered[start..start.saturating_add(count)].iter().copied()
}

#[cfg(test)]
mod tests {
    use super::bounded_filmstrip_window;
    use rusttable_core::PhotoId;

    fn id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    #[test]
    fn filmstrip_thumbnail_window_is_bounded_and_centered_on_the_active_photo() {
        let ordered = (1..=40).map(id).collect::<Vec<_>>();
        let window = bounded_filmstrip_window(&ordered, Some(id(20)), 8).collect::<Vec<_>>();

        assert_eq!(window, (16..=23).map(id).collect::<Vec<_>>());
        assert!(window.contains(&id(20)));
    }

    #[test]
    fn short_filmstrips_publish_every_visible_item() {
        let ordered = [id(1), id(2)];

        assert_eq!(
            bounded_filmstrip_window(&ordered, Some(id(1)), 12).collect::<Vec<_>>(),
            ordered
        );
    }
}
