//! Lighttable-only GTK composition shared by the shell runtime.

pub(crate) mod interaction;
pub(crate) mod layout_controls;
pub(crate) mod toolbar;

use gtk4::prelude::*;

use crate::gui::darktable_spec::{
    DARKTABLE_UI_TOKENS, FILMSTRIP_ITEM_GAP_PX, FILMSTRIP_MAX_CHILDREN_PER_LINE, THUMBNAIL_METRICS,
};
use crate::gui::{ThemeRole, apply_theme_role};
use interaction::{LighttableLayout, LighttableZoom};

/// Geometry shared by the collection grid and its focused layout tests.
///
/// The values originate in `darktable_spec`; this type only selects one of the
/// bounded lighttable densities and never invents a second geometry contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LighttableGridSpec {
    zoom: LighttableZoom,
    columns: usize,
    card_width_px: u16,
    thumbnail_width_px: u16,
    thumbnail_height_px: u16,
    horizontal_offset_px: u16,
}

#[allow(dead_code)] // geometry helpers are consumed by the future viewport adapter
impl LighttableGridSpec {
    #[must_use]
    pub(crate) fn for_zoom(zoom: LighttableZoom) -> Self {
        let columns = zoom.columns();
        let (scale_numerator, scale_denominator) = match zoom {
            LighttableZoom::Small => (3_u32, 4_u32),
            LighttableZoom::Normal => (1_u32, 1_u32),
            LighttableZoom::Large => (5_u32, 4_u32),
        };
        let nominal_card_width = u32::from(DARKTABLE_UI_TOKENS.cards.preferred_width_px)
            .saturating_mul(scale_numerator)
            / scale_denominator;
        let card_width_px = u16::try_from(nominal_card_width).unwrap_or(u16::MAX);
        let thumbnail_width_px = DARKTABLE_UI_TOKENS.cards.image_width_px(card_width_px);
        Self {
            zoom,
            columns,
            card_width_px,
            thumbnail_width_px,
            thumbnail_height_px: DARKTABLE_UI_TOKENS
                .cards
                .image_height_px(thumbnail_width_px),
            horizontal_offset_px: 0,
        }
    }

    /// Computes the square thumbtable cells used by Darktable's file-manager
    /// layout.  Darktable derives the cell size from the viewport on every
    /// resize: the requested number of images per row controls width, while
    /// the viewport height prevents cells from becoming taller than the view.
    #[must_use]
    pub(crate) fn for_viewport(
        zoom: LighttableZoom,
        viewport_width_px: u16,
        viewport_height_px: u16,
    ) -> Self {
        let columns = zoom.columns();
        let card_width_px = DARKTABLE_UI_TOKENS
            .cards
            .width_for_viewport(viewport_width_px, columns);
        let thumbnail_width_px = DARKTABLE_UI_TOKENS.cards.image_width_px(card_width_px);
        let available_image_height = viewport_height_px
            .saturating_sub(DARKTABLE_UI_TOKENS.cards.metadata_height_px)
            .max(1);
        let thumbnail_height_px = DARKTABLE_UI_TOKENS
            .cards
            .image_height_px(thumbnail_width_px)
            .min(available_image_height);
        let columns_u32 = u32::try_from(columns).unwrap_or(u32::MAX);
        let used_width = u32::from(card_width_px)
            .saturating_mul(columns_u32)
            .saturating_add(
                u32::from(DARKTABLE_UI_TOKENS.cards.item_gap_px)
                    .saturating_mul(columns_u32.saturating_sub(1)),
            );
        let horizontal_offset =
            (u32::from(viewport_width_px).saturating_sub(used_width) / 2).min(u32::from(u16::MAX));
        Self {
            zoom,
            columns,
            card_width_px,
            thumbnail_width_px,
            thumbnail_height_px,
            horizontal_offset_px: u16::try_from(horizontal_offset).unwrap_or(u16::MAX),
        }
    }

    /// One 3:2 image fitted into Darktable's full-preview lighttable surface.
    #[must_use]
    pub(crate) fn for_preview_viewport(viewport_width_px: u16, viewport_height_px: u16) -> Self {
        // GridView's own padding, child margin, and card border provide the
        // ten-pixel Darktable inset. Deducting a second inset here makes the
        // full preview noticeably smaller and turns resize into double chrome.
        let available_width = viewport_width_px.saturating_sub(2).max(1);
        let available_height = viewport_height_px.max(1);
        let width_from_height = available_height.saturating_mul(3) / 2;
        let thumbnail_width_px = available_width.min(width_from_height).max(1);
        let thumbnail_height_px = thumbnail_width_px.saturating_mul(2) / 3;
        Self {
            zoom: LighttableZoom::Normal,
            columns: 1,
            card_width_px: thumbnail_width_px.saturating_add(2),
            thumbnail_width_px,
            thumbnail_height_px,
            horizontal_offset_px: viewport_width_px
                .saturating_sub(thumbnail_width_px.saturating_add(2))
                / 2,
        }
    }

    #[must_use]
    pub(crate) fn for_layout_viewport(
        layout: LighttableLayout,
        zoom: LighttableZoom,
        width: u16,
        height: u16,
    ) -> Self {
        if layout == LighttableLayout::Preview && width > 0 && height > 0 {
            Self::for_preview_viewport(width, height)
        } else if width > 0 && height > 0 {
            Self::for_viewport(zoom, width.saturating_sub(12), height)
        } else if layout == LighttableLayout::Preview {
            Self::for_preview_viewport(900, 590)
        } else {
            Self::for_zoom(zoom)
        }
    }

    /// Re-centers the realized first row without changing its Darktable density.
    #[must_use]
    pub(crate) fn centered_for_visible_count(
        self,
        viewport_width_px: u16,
        visible_count: usize,
    ) -> Self {
        let items = visible_count.min(self.columns).max(1);
        let items_u32 = u32::try_from(items).unwrap_or(u32::MAX);
        let used_width = u32::from(self.card_width_px)
            .saturating_mul(items_u32)
            .saturating_add(
                u32::from(DARKTABLE_UI_TOKENS.cards.item_gap_px)
                    .saturating_mul(items_u32.saturating_sub(1)),
            );
        let offset = u32::from(viewport_width_px).saturating_sub(used_width) / 2;
        Self {
            horizontal_offset_px: u16::try_from(offset.min(u32::from(u16::MAX)))
                .unwrap_or(u16::MAX),
            ..self
        }
    }

    #[must_use]
    pub(crate) const fn columns(self) -> usize {
        self.columns
    }

    #[must_use]
    pub(crate) const fn card_width_px(self) -> u16 {
        self.card_width_px
    }

    #[must_use]
    pub(crate) const fn thumbnail_width_px(self) -> u16 {
        self.thumbnail_width_px
    }

    #[must_use]
    pub(crate) const fn thumbnail_height_px(self) -> u16 {
        self.thumbnail_height_px
    }

    #[must_use]
    pub(crate) const fn horizontal_offset_px(self) -> u16 {
        self.horizontal_offset_px
    }

    #[must_use]
    pub(crate) fn cell_origin_x_px(self, column: usize) -> u32 {
        u32::from(self.horizontal_offset_px).saturating_add(
            (u32::from(self.card_width_px)
                .saturating_add(u32::from(DARKTABLE_UI_TOKENS.cards.item_gap_px)))
            .saturating_mul(u32::try_from(column).unwrap_or(u32::MAX)),
        )
    }

    #[must_use]
    pub(crate) const fn rows_for(self, photo_count: usize) -> usize {
        photo_count.saturating_add(self.columns.saturating_sub(1)) / self.columns
    }

    #[must_use]
    pub(crate) const fn position_for(self, index: usize) -> GridPosition {
        GridPosition {
            row: index / self.columns,
            column: index % self.columns,
        }
    }

    #[must_use]
    pub(crate) fn visible_range(
        self,
        first_row: usize,
        row_count: usize,
        photo_count: usize,
    ) -> std::ops::Range<usize> {
        let start = first_row.saturating_mul(self.columns).min(photo_count);
        let end = first_row
            .saturating_add(row_count)
            .saturating_mul(self.columns)
            .min(photo_count);
        start..end
    }
}

#[allow(dead_code)] // constructed by focused geometry tests and the viewport adapter
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GridPosition {
    pub(crate) row: usize,
    pub(crate) column: usize,
}

/// Geometry for the single horizontally scrolling filmstrip row.
#[allow(dead_code)] // consumed by the filmstrip viewport adapter
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FilmstripSpec {
    width_px: u16,
    height_px: u16,
    gap_px: u8,
    max_children_per_line: u32,
}

#[allow(dead_code)] // consumed by the filmstrip viewport adapter
impl FilmstripSpec {
    #[must_use]
    pub(crate) const fn darktable() -> Self {
        Self {
            // Darktable's filmstrip thumbtable uses the available panel
            // height for both dimensions; the image itself is fit inside it.
            width_px: THUMBNAIL_METRICS.filmstrip_height_px,
            height_px: THUMBNAIL_METRICS.filmstrip_height_px,
            gap_px: FILMSTRIP_ITEM_GAP_PX,
            max_children_per_line: FILMSTRIP_MAX_CHILDREN_PER_LINE,
        }
    }

    #[must_use]
    pub(crate) const fn for_viewport(viewport_height_px: u16) -> Self {
        Self {
            width_px: viewport_height_px,
            height_px: viewport_height_px,
            gap_px: FILMSTRIP_ITEM_GAP_PX,
            max_children_per_line: FILMSTRIP_MAX_CHILDREN_PER_LINE,
        }
    }

    #[must_use]
    pub(crate) const fn width_px(self) -> u16 {
        self.width_px
    }

    #[must_use]
    pub(crate) const fn height_px(self) -> u16 {
        self.height_px
    }

    #[must_use]
    pub(crate) const fn gap_px(self) -> u8 {
        self.gap_px
    }

    #[must_use]
    pub(crate) const fn max_children_per_line(self) -> u32 {
        self.max_children_per_line
    }

    #[must_use]
    pub(crate) fn content_width_px(self, item_count: usize) -> u32 {
        let items = u32::try_from(item_count).unwrap_or(u32::MAX);
        u32::from(self.width_px)
            .saturating_mul(items)
            .saturating_add(
                items
                    .saturating_sub(1)
                    .saturating_mul(u32::from(self.gap_px)),
            )
    }

    #[must_use]
    pub(crate) fn leading_offset_px(self, viewport_width_px: u16, item_count: usize) -> u16 {
        let items = u32::try_from(item_count).unwrap_or(u32::MAX);
        let content_width = u32::from(self.width_px)
            .saturating_mul(items)
            .saturating_add(
                items
                    .saturating_sub(1)
                    .saturating_mul(u32::from(self.gap_px)),
            );
        u16::try_from(
            u32::from(viewport_width_px)
                .saturating_sub(content_width)
                .saturating_div(2)
                .min(u32::from(u16::MAX)),
        )
        .unwrap_or(u16::MAX)
    }
}

/// Catalog states that the central lighttable can present without exposing
/// filesystem or catalog-internal error details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LighttableCollectionState {
    #[allow(dead_code)] // constructed when catalog loading becomes asynchronous
    Loading,
    Empty,
    Ready(usize),
    #[allow(dead_code)] // constructed when catalog failures reach the shell
    Failed,
}

impl LighttableCollectionState {
    #[allow(dead_code)] // consumed by the asynchronous runtime projection
    #[must_use]
    pub(crate) const fn from_rendered_count(count: usize) -> Self {
        if count == 0 {
            Self::Empty
        } else {
            Self::Ready(count)
        }
    }

    #[must_use]
    pub(crate) const fn status_text(&self) -> &'static str {
        match self {
            Self::Loading => "loading collection…",
            Self::Empty => "there are no images in this collection",
            Self::Ready(_) => "",
            Self::Failed => "unable to load this collection",
        }
    }

    #[must_use]
    pub(crate) const fn rendered_count(&self) -> usize {
        match self {
            Self::Ready(count) => *count,
            Self::Loading | Self::Empty | Self::Failed => 0,
        }
    }

    #[allow(dead_code)] // consumed by loading/error projections
    #[must_use]
    pub(crate) const fn is_terminal(&self) -> bool {
        matches!(self, Self::Empty | Self::Ready(_) | Self::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EmptyStateRow {
    left: &'static str,
    left_heading: bool,
    left_margin: i32,
    right: &'static str,
    right_heading: bool,
    right_margin: i32,
}

const EMPTY_STATE_ROWS: [EmptyStateRow; 8] = [
    EmptyStateRow {
        left: "if you have not imported any images yet",
        left_heading: false,
        left_margin: 14,
        right: "click on ? then an on-screen item to open manual page",
        right_heading: false,
        right_margin: 14,
    },
    EmptyStateRow {
        left: "you can do so in the import module",
        left_heading: false,
        left_margin: 0,
        right: "press and hold 'h' to show all active keyboard shortcuts",
        right_heading: false,
        right_margin: 0,
    },
    EmptyStateRow {
        left: "try to relax the filter settings in the top panel",
        left_heading: false,
        left_margin: 18,
        right: "personalize darktable",
        right_heading: true,
        right_margin: 18,
    },
    EmptyStateRow {
        left: "or add images in the collections module",
        left_heading: false,
        left_margin: 0,
        right: "click on the gear icon for global preferences",
        right_heading: false,
        right_margin: 0,
    },
    EmptyStateRow {
        left: "try the 'no-click' workflow",
        left_heading: true,
        left_margin: 18,
        right: "click on the keyboard icon to define shortcuts",
        right_heading: false,
        right_margin: 0,
    },
    EmptyStateRow {
        left: "hover over an image and use keyboard shortcuts",
        left_heading: false,
        left_margin: 0,
        right: "set module-specific preferences through a module's menu",
        right_heading: false,
        right_margin: 0,
    },
    EmptyStateRow {
        left: "to apply ratings, colors, styles, etc.",
        left_heading: false,
        left_margin: 0,
        right: "make default raw development look more like your",
        right_heading: false,
        right_margin: 18,
    },
    EmptyStateRow {
        left: "hover over any button for its description and shortcuts",
        left_heading: false,
        left_margin: 0,
        right: "camera's JPEG by applying a camera-specific style",
        right_heading: false,
        right_margin: 0,
    },
];

pub(crate) fn empty_collection_state() -> gtk4::Grid {
    let root = gtk4::Grid::new();
    root.set_widget_name("empty-collection-state");
    root.set_halign(gtk4::Align::Fill);
    root.set_valign(gtk4::Align::Start);
    root.set_hexpand(true);
    root.set_column_homogeneous(true);
    root.set_column_spacing(36);
    root.set_row_spacing(2);
    root.set_margin_start(36);
    root.set_margin_end(36);
    root.set_margin_top(110);
    apply_theme_role(&root, ThemeRole::EmptyState);

    attach_label(
        &root,
        "there are no images in this collection",
        0,
        0,
        true,
        0,
    );
    attach_label(&root, "need help?", 1, 0, true, 0);
    for (row, content) in EMPTY_STATE_ROWS.into_iter().enumerate() {
        let row = i32::try_from(row + 1).expect("empty-state rows fit i32");
        attach_label(
            &root,
            content.left,
            0,
            row,
            content.left_heading,
            content.left_margin,
        );
        attach_label(
            &root,
            content.right,
            1,
            row,
            content.right_heading,
            content.right_margin,
        );
    }
    root
}

fn attach_label(
    root: &gtk4::Grid,
    text: &str,
    column: i32,
    row: i32,
    heading: bool,
    margin_top: i32,
) {
    let label = gtk4::Label::new(Some(text));
    label.set_halign(if column == 0 {
        gtk4::Align::Start
    } else {
        gtk4::Align::End
    });
    label.set_hexpand(true);
    label.set_wrap(false);
    label.set_margin_top(margin_top);
    if heading {
        label.add_css_class("dt_empty_heading");
    }
    root.attach(&label, column, row, 1, 1);
}

#[cfg(test)]
mod tests {
    use super::interaction::LighttableZoom;
    use super::{EMPTY_STATE_ROWS, LighttableCollectionState, LighttableGridSpec};
    use crate::gtk_shell::{DARKTABLE_UI_TOKENS, LIGHTTABLE_COMPOSITION, THUMBNAIL_METRICS};

    #[test]
    fn guidance_keeps_darktable_two_column_sections() {
        assert_eq!(EMPTY_STATE_ROWS.len(), 8);
        assert!(EMPTY_STATE_ROWS[2].right_heading);
        assert!(EMPTY_STATE_ROWS[4].left_heading);
        assert_eq!(
            EMPTY_STATE_ROWS.last().map(|row| row.right),
            Some("camera's JPEG by applying a camera-specific style")
        );
    }

    #[test]
    fn grid_spec_uses_darktable_thumbnail_geometry_with_bounded_density() {
        let normal = LighttableGridSpec::for_zoom(LighttableZoom::Normal);
        assert_eq!(normal.columns(), 5);
        assert_eq!(
            (normal.thumbnail_width_px(), normal.thumbnail_height_px()),
            (184, 138)
        );

        let large = LighttableGridSpec::for_zoom(LighttableZoom::Large);
        assert_eq!(large.columns(), 3);
        assert!(large.thumbnail_width_px() > normal.thumbnail_width_px());
        assert!(large.thumbnail_height_px() > normal.thumbnail_height_px());
        let resized = LighttableGridSpec::for_viewport(LighttableZoom::Normal, 1_000, 200);
        assert_eq!(resized.card_width_px(), 195);
        assert_eq!(resized.thumbnail_width_px(), 183);
        assert_eq!(resized.thumbnail_height_px(), 137);
        assert_eq!(resized.horizontal_offset_px(), 0);
        assert_eq!(resized.cell_origin_x_px(4), 804);
        assert_eq!(DARKTABLE_UI_TOKENS.cards.preferred_width_px, 196);
        assert_eq!(THUMBNAIL_METRICS.grid_width_px, 196);
        assert_eq!(LIGHTTABLE_COMPOSITION.top_toolbar_rows, 1);
    }

    #[test]
    fn preview_geometry_fits_one_three_by_two_surface_inside_the_canvas() {
        let preview = LighttableGridSpec::for_preview_viewport(900, 600);

        assert_eq!(preview.columns(), 1);
        assert_eq!(preview.thumbnail_width_px(), 898);
        assert_eq!(preview.thumbnail_height_px(), 598);
        assert_eq!(preview.card_width_px(), 900);
        assert_eq!(preview.horizontal_offset_px(), 0);
    }

    #[test]
    fn grid_geometry_culls_rows_without_reordering_items() {
        let spec = LighttableGridSpec::for_zoom(LighttableZoom::Normal);
        assert_eq!(spec.rows_for(7), 2);
        assert_eq!(
            spec.position_for(7),
            super::GridPosition { row: 1, column: 2 }
        );
        assert_eq!(spec.visible_range(1, 1, 20), 5..10);
        assert_eq!(spec.visible_range(8, 2, 20), 20..20);
    }

    #[test]
    fn grid_geometry_centers_the_realized_last_row() {
        let spec = LighttableGridSpec::for_viewport(LighttableZoom::Normal, 1_000, 600);
        let centered = spec.centered_for_visible_count(1_000, 2);

        assert_eq!(centered.card_width_px(), spec.card_width_px());
        assert_eq!(centered.columns(), spec.columns());
        assert!(centered.horizontal_offset_px() > 0);
        assert_eq!(
            centered.cell_origin_x_px(0),
            u32::from(centered.horizontal_offset_px())
        );
    }

    #[test]
    fn filmstrip_geometry_is_one_unwrapped_row() {
        let spec = super::FilmstripSpec::darktable();
        assert_eq!(spec.width_px(), THUMBNAIL_METRICS.filmstrip_height_px);
        assert_eq!(spec.height_px(), THUMBNAIL_METRICS.filmstrip_height_px);
        assert_eq!(spec.gap_px(), 4);
        assert_eq!(spec.max_children_per_line(), u32::MAX);
        assert_eq!(spec.content_width_px(3), 368);
        assert_eq!(spec.leading_offset_px(400, 1), 140);
        assert_eq!(super::FilmstripSpec::for_viewport(80).width_px(), 80);
    }

    #[test]
    fn collection_states_are_safe_and_do_not_leak_catalog_errors() {
        assert_eq!(LighttableCollectionState::Loading.rendered_count(), 0);
        assert_eq!(LighttableCollectionState::Empty.rendered_count(), 0);
        assert_eq!(LighttableCollectionState::Ready(3).rendered_count(), 3);
        assert_eq!(
            LighttableCollectionState::Failed.status_text(),
            "unable to load this collection"
        );
        assert!(
            !LighttableCollectionState::Failed
                .status_text()
                .contains('/')
        );
    }
}
