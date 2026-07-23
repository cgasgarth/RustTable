use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use gtk4::prelude::*;
use rusttable_core::PhotoId;

use super::GtkShell;
use crate::gui::darktable_spec::FILMSTRIP_ITEM_GAP_PX;
use crate::gui::{DARKTABLE_DESKTOP_SPEC, DARKTABLE_UI_TOKENS};
use crate::views::lighttable::LighttableGridSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LighttableGridGeometry {
    columns: usize,
    cell_width_px: u16,
    card_width_px: u16,
    card_height_px: u16,
    thumbnail_width_px: u16,
    thumbnail_height_px: u16,
    horizontal_offset_px: u16,
}

impl LighttableGridGeometry {
    pub(super) fn for_viewport(
        layout: crate::gui::LighttableLayout,
        zoom: crate::gui::LighttableZoom,
        viewport_width_px: u16,
        viewport_height_px: u16,
    ) -> Self {
        if !layout.shows_grid() {
            let spec = LighttableGridSpec::for_layout_viewport(
                layout,
                zoom,
                viewport_width_px,
                viewport_height_px,
            );
            return Self {
                columns: spec.columns(),
                cell_width_px: spec.card_width_px(),
                card_width_px: spec.card_width_px(),
                card_height_px: spec.thumbnail_height_px().saturating_add(
                    DARKTABLE_UI_TOKENS
                        .cards
                        .metadata_height_px
                        .saturating_mul(u16::from(layout != crate::gui::LighttableLayout::Preview)),
                ),
                thumbnail_width_px: spec.thumbnail_width_px(),
                thumbnail_height_px: spec.thumbnail_height_px(),
                horizontal_offset_px: spec.horizontal_offset_px(),
            };
        }

        // Darktable's file-manager thumbtable uses square cells and recomputes
        // their size from the current center allocation. The configured image
        // count remains authoritative even when that makes cards narrower than
        // RustTable's nominal card token.
        let columns = zoom.columns().max(1);
        let content_width_px = viewport_width_px.saturating_sub(12).max(1);
        let columns_u16 = u16::try_from(columns).unwrap_or(u16::MAX).max(1);
        let cell_width_px = (content_width_px / columns_u16)
            .min(viewport_height_px.max(1))
            .max(1);
        let used_width_px = cell_width_px.saturating_mul(columns_u16);
        let horizontal_offset_px = content_width_px.saturating_sub(used_width_px) / 2;
        let card_width_px = cell_width_px
            .saturating_sub(DARKTABLE_UI_TOKENS.cards.item_gap_px)
            .max(1);
        let thumbnail_width_px = DARKTABLE_UI_TOKENS.cards.image_width_px(card_width_px);
        let thumbnail_height_px = card_width_px
            .saturating_sub(DARKTABLE_UI_TOKENS.cards.metadata_height_px)
            .max(1);
        Self {
            columns,
            cell_width_px,
            card_width_px,
            card_height_px: card_width_px,
            thumbnail_width_px,
            thumbnail_height_px,
            horizontal_offset_px,
        }
    }

    pub(super) const fn columns(self) -> usize {
        self.columns
    }

    #[cfg(test)]
    pub(super) const fn cell_width_px(self) -> u16 {
        self.cell_width_px
    }

    pub(super) const fn card_width_px(self) -> u16 {
        self.card_width_px
    }

    pub(super) const fn card_height_px(self) -> u16 {
        self.card_height_px
    }

    pub(super) const fn thumbnail_width_px(self) -> u16 {
        self.thumbnail_width_px
    }

    pub(super) const fn thumbnail_height_px(self) -> u16 {
        self.thumbnail_height_px
    }

    #[cfg(test)]
    pub(super) const fn horizontal_offset_px(self) -> u16 {
        self.horizontal_offset_px
    }

    pub(super) fn cell_origin_x_px(self, column: usize) -> u32 {
        u32::from(self.horizontal_offset_px).saturating_add(
            u32::from(self.cell_width_px).saturating_mul(u32::try_from(column).unwrap_or(u32::MAX)),
        )
    }
}

pub(super) fn reveal_active_photo(
    lighttable: &gtk4::GridView,
    grid: LighttableGridGeometry,
    position: usize,
) {
    let lighttable = lighttable.clone();
    gtk4::glib::idle_add_local_once(move || {
        let Some(scrolled) = lighttable
            .ancestor(gtk4::ScrolledWindow::static_type())
            .and_then(|widget| widget.downcast::<gtk4::ScrolledWindow>().ok())
        else {
            return;
        };
        let adjustment = scrolled.vadjustment();
        let row = position / grid.columns().max(1);
        let row_top =
            f64::from(grid.card_height_px()) * f64::from(u32::try_from(row).unwrap_or(u32::MAX));
        let row_bottom = row_top + f64::from(grid.card_height_px());
        let viewport_top = adjustment.value();
        let viewport_bottom = viewport_top + adjustment.page_size();
        if row_top < viewport_top {
            adjustment.set_value(row_top);
        } else if row_bottom > viewport_bottom {
            adjustment.set_value(row_bottom - adjustment.page_size());
        }
    });
}

pub(super) const fn filmstrip_visible_for_workspace(
    layout: crate::gui::LighttableLayout,
    darkroom_visible: bool,
    darkroom_filmstrip_visible: bool,
) -> bool {
    if darkroom_visible {
        darkroom_filmstrip_visible
    } else {
        layout.shows_filmstrip()
    }
}

pub(super) fn active_photo_position(
    display_ids: &[PhotoId],
    active: Option<PhotoId>,
) -> Option<usize> {
    let active = active?;
    display_ids.iter().position(|photo_id| *photo_id == active)
}

pub(super) fn grid_model_strings(display_ids: &[PhotoId], columns: usize) -> Vec<String> {
    let columns = columns.max(1);
    let padding = if display_ids.is_empty() {
        0
    } else {
        columns.saturating_sub(display_ids.len() % columns) % columns
    };
    display_ids
        .iter()
        .map(|photo_id| photo_id.get().to_string())
        .chain(std::iter::repeat_n(String::new(), padding))
        .collect()
}

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
