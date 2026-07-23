//! Darktable-shaped GTK4 lighttable navigation rail.

use gtk4::prelude::*;
use rusttable_i18n::{I18n, MessageArgs, MessageId};

use crate::gui::darktable_components::module_title;
use crate::gui::{
    CollectionControls, LIGHTTABLE_PANEL_WIDTHS, PanelSlot, ShellRegion, ThemeRole,
    apply_theme_role,
};

#[cfg(test)]
const LIGHTTABLE_LEFT_MODULE_IDS: [&str; 5] = [
    "image-information",
    "collection-filters",
    "collections",
    "import",
    "scripts",
];

/// Persistent left rail and the action-bearing import control it owns.
pub(crate) struct LeftPanel {
    root: gtk4::Box,
    import: gtk4::Button,
}

impl LeftPanel {
    pub(crate) fn new(collection_controls: &CollectionControls, i18n: &I18n) -> Self {
        let width = i32::from(LIGHTTABLE_PANEL_WIDTHS.left_px);
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        root.set_widget_name(ShellRegion::LeftPanel.identifier());
        root.set_size_request(width, -1);
        root.set_hexpand(false);
        apply_theme_role(&root, ThemeRole::Panel);

        let top = panel_slot(PanelSlot::LeftTop);
        let modules = panel_slot(PanelSlot::LeftCenter);
        modules.append(&image_information());
        modules.append(&module_separator("information-filters"));
        modules.append(&collection_filters());
        modules.append(&module_separator("filters-collections"));
        modules.append(&module_with_child(
            "collections",
            "collections",
            collection_controls.widget(),
            true,
        ));
        modules.append(&module_separator("collections-import"));
        let import = import_module(i18n);
        modules.append(&import.0);

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
    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
    actions.set_homogeneous(true);
    actions.add_css_class("dt_import_actions");
    let import = gtk4::Button::with_label("add to library…");
    import.set_widget_name("lighttable-import");
    import.set_tooltip_text(Some(
        &i18n.text(MessageId::ToolbarImport, &MessageArgs::new()),
    ));
    compact_import_action(&import);
    actions.append(&import);
    let copy_import = gtk4::Button::with_label("copy & import…");
    copy_import.set_widget_name("lighttable-copy-import");
    compact_import_action(&copy_import);
    copy_import.set_sensitive(false);
    copy_import.set_tooltip_text(Some(
        "Copy-and-import destinations are not implemented; add to library keeps source files in place",
    ));
    actions.append(&copy_import);
    content.append(&actions);
    let parameters = gtk4::Button::with_label("parameters");
    parameters.set_widget_name("lighttable-import-parameters");
    parameters.set_hexpand(true);
    parameters.set_sensitive(false);
    parameters.set_tooltip_text(Some(
        "Import parameters are configured in the source chooser",
    ));
    content.append(&parameters);
    (
        module_with_child("import", "import", &content, true),
        import,
    )
}

fn compact_import_action(button: &gtk4::Button) {
    button.set_width_request(0);
    button.set_hexpand(true);
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
    let title = module_title(id, label);
    let group = gtk4::Expander::builder()
        .label_widget(&title)
        .expanded(expanded)
        .build();
    group.set_widget_name(id);
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
    let title = module_title(id, label);
    let group = gtk4::Expander::builder()
        .label_widget(&title)
        .expanded(expanded)
        .child(&content)
        .build();
    group.set_widget_name(id);
    apply_theme_role(&group, ThemeRole::ModuleGroup);
    group
}

#[cfg(test)]
mod tests {
    use super::LIGHTTABLE_LEFT_MODULE_IDS;

    #[test]
    fn rail_modules_follow_darktable_lighttable_order() {
        assert_eq!(
            LIGHTTABLE_LEFT_MODULE_IDS,
            [
                "image-information",
                "collection-filters",
                "collections",
                "import",
                "scripts",
            ]
        );
    }

    #[test]
    fn rail_keeps_typed_action_widget_ids() {
        let source = include_str!("navigation.rs");
        let css = include_str!("../gui/theme.css");

        assert!(source.contains("lighttable-import"));
        assert!(source.contains("collection_controls.widget()"));
        assert!(source.contains("hscrollbar_policy(gtk4::PolicyType::Never)"));
        assert!(source.contains("actions.set_homogeneous(true)"));
        assert!(source.contains("fn compact_import_action"));
        assert!(source.contains("button.set_width_request(0)"));
        assert!(css.contains(".dt_import_actions > button"));
        assert!(css.contains("min-width: 0;"));
    }
}
