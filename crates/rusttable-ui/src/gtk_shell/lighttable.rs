//! Lighttable-only GTK composition shared by the shell runtime.

use gtk4::prelude::*;

use super::{
    THUMBNAIL_METRICS, ThemeRole, apply_theme_role, lighttable_interaction::LighttableZoom,
};

/// Geometry shared by the collection grid and its focused layout tests.
///
/// The values originate in `darktable_spec`; this type only selects one of the
/// bounded lighttable densities and never invents a second geometry contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LighttableGridSpec {
    zoom: LighttableZoom,
    columns: usize,
    thumbnail_width_px: u16,
    thumbnail_height_px: u16,
}

impl LighttableGridSpec {
    #[must_use]
    pub(super) const fn for_zoom(zoom: LighttableZoom) -> Self {
        Self {
            zoom,
            columns: zoom.columns(),
            thumbnail_width_px: scale_dimension(THUMBNAIL_METRICS.grid_width_px, zoom),
            thumbnail_height_px: scale_dimension(THUMBNAIL_METRICS.grid_height_px, zoom),
        }
    }

    #[must_use]
    pub(super) const fn columns(self) -> usize {
        self.columns
    }

    #[must_use]
    pub(super) const fn thumbnail_width_px(self) -> u16 {
        self.thumbnail_width_px
    }

    #[must_use]
    pub(super) const fn thumbnail_height_px(self) -> u16 {
        self.thumbnail_height_px
    }
}

const fn scale_dimension(value: u16, zoom: LighttableZoom) -> u16 {
    match zoom {
        LighttableZoom::Small => value.saturating_mul(3) / 4,
        LighttableZoom::Normal => value,
        LighttableZoom::Large => value.saturating_mul(5) / 4,
    }
}

/// Catalog states that the central lighttable can present without exposing
/// filesystem or catalog-internal error details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LighttableCollectionState {
    #[allow(dead_code)] // constructed when catalog loading becomes asynchronous
    Loading,
    Empty,
    Ready(usize),
    #[allow(dead_code)] // constructed when catalog failures reach the shell
    Failed,
}

impl LighttableCollectionState {
    #[must_use]
    pub(super) const fn status_text(&self) -> &'static str {
        match self {
            Self::Loading => "loading collection…",
            Self::Empty => "there are no images in this collection",
            Self::Ready(_) => "",
            Self::Failed => "unable to load this collection",
        }
    }

    #[must_use]
    pub(super) const fn rendered_count(&self) -> usize {
        match self {
            Self::Ready(count) => *count,
            Self::Loading | Self::Empty | Self::Failed => 0,
        }
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

pub(super) fn empty_collection_state() -> gtk4::Grid {
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
    use super::super::lighttable_interaction::LighttableZoom;
    use super::{EMPTY_STATE_ROWS, LighttableCollectionState, LighttableGridSpec};
    use crate::gtk_shell::{LIGHTTABLE_COMPOSITION, THUMBNAIL_METRICS};

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
        assert_eq!(normal.columns(), 6);
        assert_eq!(
            (normal.thumbnail_width_px(), normal.thumbnail_height_px()),
            (
                THUMBNAIL_METRICS.grid_width_px,
                THUMBNAIL_METRICS.grid_height_px
            )
        );

        let large = LighttableGridSpec::for_zoom(LighttableZoom::Large);
        assert_eq!(large.columns(), 4);
        assert!(large.thumbnail_width_px() > normal.thumbnail_width_px());
        assert!(large.thumbnail_height_px() > normal.thumbnail_height_px());
        assert_eq!(LIGHTTABLE_COMPOSITION.top_toolbar_rows, 1);
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
