//! Lighttable-only GTK composition shared by the shell runtime.

use gtk4::prelude::*;

use super::{ThemeRole, apply_theme_role};

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
    use super::EMPTY_STATE_ROWS;

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
}
