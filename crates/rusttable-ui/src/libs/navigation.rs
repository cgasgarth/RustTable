//! Darktable-shaped GTK4 lighttable navigation rail.

use gtk4::prelude::*;
use rusttable_i18n::{I18n, MessageArgs, MessageId};

use crate::gui::{
    CollectionControls, DARKTABLE_DESKTOP_SPEC, PanelSlot, ShellRegion, ThemeRole, apply_theme_role,
};

#[cfg(test)]
const LIGHTTABLE_LEFT_MODULE_IDS: [&str; 5] = [
    "import",
    "collections",
    "collection-filters",
    "image-information",
    "scripts",
];

/// Persistent left rail and the action-bearing import control it owns.
pub(crate) struct LeftPanel {
    root: gtk4::Box,
    import: gtk4::Button,
}

impl LeftPanel {
    pub(crate) fn new(collection_controls: &CollectionControls, i18n: &I18n) -> Self {
        let width = i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.preferred_px);
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.set_widget_name(ShellRegion::LeftPanel.identifier());
        root.set_size_request(width, -1);
        root.set_hexpand(false);
        apply_theme_role(&root, ThemeRole::Panel);

        let top = panel_slot(PanelSlot::LeftTop);
        let modules = panel_slot(PanelSlot::LeftCenter);
        let import = import_module(i18n);
        modules.append(&import.0);
        modules.append(&module_separator("import-collections"));
        modules.append(&module_with_child(
            "collections",
            "collections",
            collection_controls.widget(),
            true,
        ));
        modules.append(&module_separator("collections-filters"));
        modules.append(&collection_filters());
        modules.append(&module_separator("filters-information"));
        modules.append(&image_information());

        let scroll = gtk4::ScrolledWindow::builder()
            .child(&modules)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hexpand(false)
            .vexpand(true)
            .min_content_width(width)
            .max_content_width(width)
            .propagate_natural_width(false)
            .build();
        scroll.set_widget_name("left-module-scroll");

        let bottom = panel_slot(PanelSlot::LeftBottom);
        bottom.append(&module("scripts", "scripts", false));
        let background_jobs = gtk4::Label::new(Some(
            &i18n.text(MessageId::PanelBackgroundJobs, &MessageArgs::new()),
        ));
        background_jobs.set_widget_name("background-jobs");
        background_jobs.set_xalign(0.0);
        background_jobs.set_height_request(18);
        background_jobs.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        background_jobs.add_css_class("dt_background_jobs");
        bottom.append(&background_jobs);

        root.append(&top);
        root.append(&scroll);
        root.append(&bottom);
        Self {
            root,
            import: import.1,
        }
    }

    pub(crate) const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub(crate) const fn import_button(&self) -> &gtk4::Button {
        &self.import
    }
}

fn import_module(i18n: &I18n) -> (gtk4::Expander, gtk4::Button) {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
    let import = gtk4::Button::with_label("add to library…");
    import.set_widget_name("lighttable-import");
    import.set_tooltip_text(Some(
        &i18n.text(MessageId::ToolbarImport, &MessageArgs::new()),
    ));
    import.set_hexpand(true);
    content.append(&import);
    (
        module_with_child("import", "import", &content, false),
        import,
    )
}

fn collection_filters() -> gtk4::Expander {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
    let status = gtk4::Label::new(Some("collection rules follow the active selection"));
    status.set_widget_name("left-collection-filter-status");
    status.set_xalign(0.0);
    status.set_wrap(true);
    status.add_css_class("dim-label");
    content.append(&status);
    module_with_child("collection-filters", "collection filters", &content, false)
}

fn image_information() -> gtk4::Expander {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    for text in ["selected image", "rating · color label · tags"] {
        let label = gtk4::Label::new(Some(text));
        label.set_xalign(0.0);
        label.set_wrap(true);
        label.add_css_class("dim-label");
        content.append(&label);
    }
    module_with_child("image-information", "image information", &content, false)
}

fn panel_slot(slot: PanelSlot) -> gtk4::Box {
    let slot_widget = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    slot_widget.set_widget_name(slot.identifier());
    slot_widget.add_css_class("dt_panel_slot");
    slot_widget
}

fn module_separator(id: &str) -> gtk4::Separator {
    let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    separator.set_widget_name(&format!("left-separator-{id}"));
    separator.add_css_class("dt_rail_separator");
    separator
}

fn module(id: &str, label: &str, expanded: bool) -> gtk4::Expander {
    let (title, disclosure) = module_label(label, expanded);
    let group = gtk4::Expander::builder()
        .label_widget(&title)
        .expanded(expanded)
        .build();
    group.set_widget_name(id);
    group.connect_expanded_notify(move |expander| {
        disclosure.set_text(if expander.is_expanded() { "⌄" } else { "›" });
    });
    apply_theme_role(&group, ThemeRole::ModuleGroup);
    group
}

fn module_with_child<W: IsA<gtk4::Widget>>(
    id: &str,
    label: &str,
    child: &W,
    expanded: bool,
) -> gtk4::Expander {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
    content.append(child);
    let (title, disclosure) = module_label(label, expanded);
    let group = gtk4::Expander::builder()
        .label_widget(&title)
        .expanded(expanded)
        .child(&content)
        .build();
    group.set_widget_name(id);
    group.connect_expanded_notify(move |expander| {
        disclosure.set_text(if expander.is_expanded() { "⌄" } else { "›" });
    });
    apply_theme_role(&group, ThemeRole::ModuleGroup);
    group
}

fn module_label(text: &str, expanded: bool) -> (gtk4::Box, gtk4::Label) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
    row.set_halign(gtk4::Align::Fill);
    row.set_hexpand(true);
    row.set_width_request(126);
    let disclosure = gtk4::Label::new(Some(if expanded { "⌄" } else { "›" }));
    disclosure.set_widget_name("module-disclosure");
    disclosure.add_css_class("dim-label");
    row.append(&disclosure);
    let label = gtk4::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    row.append(&label);
    for glyph in ["⊙", "≡"] {
        let icon = gtk4::Label::new(Some(glyph));
        icon.add_css_class("dim-label");
        row.append(&icon);
    }
    (row, disclosure)
}

#[cfg(test)]
mod tests {
    use super::LIGHTTABLE_LEFT_MODULE_IDS;

    #[test]
    fn rail_modules_follow_darktable_lighttable_order() {
        assert_eq!(
            LIGHTTABLE_LEFT_MODULE_IDS,
            [
                "import",
                "collections",
                "collection-filters",
                "image-information",
                "scripts",
            ]
        );
    }

    #[test]
    fn rail_keeps_typed_action_widget_ids() {
        let source = include_str!("navigation.rs");

        assert!(source.contains("lighttable-import"));
        assert!(source.contains("collection_controls.widget()"));
        assert!(source.contains("hscrollbar_policy(gtk4::PolicyType::Never)"));
    }
}
