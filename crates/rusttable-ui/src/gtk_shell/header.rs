//! Darktable-shaped GTK4 header and lighttable mode chrome.

use crate::display_profile::DisplayProfileBanner;
use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_i18n::{I18n, MessageArgs, MessageId};

use super::{
    DARKTABLE_DESKTOP_SPEC, LIGHTTABLE_TOOLBAR, PanelSlot, ShellRegion, ThemeRole, WorkspaceRole,
    apply_theme_role,
};

#[cfg(test)]
const HEADER_WIDGET_IDS: [&str; 7] = [
    "header",
    "header-left",
    "header-center",
    "header-right",
    "view-lighttable",
    "view-darkroom",
    "view-other",
];

/// Widgets and actions owned by the persistent top panel.
pub(super) struct HeaderChrome {
    root: gtk4::Box,
    preferences: gtk4::Button,
    import: gtk4::Button,
}

impl HeaderChrome {
    pub(super) fn new(
        workspace: &gtk4::Stack,
        i18n: &I18n,
        display_profile: &DisplayProfileBanner,
    ) -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        root.set_widget_name(ShellRegion::Header.identifier());
        apply_theme_role(&root, ThemeRole::Header);

        root.append(&brand(i18n));
        let (toolbar, import, preferences) = lighttable_toolbar(i18n, display_profile);
        let center = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        center.set_widget_name(PanelSlot::HeaderCenter.identifier());
        center.set_hexpand(true);
        center.append(&toolbar);
        root.append(&center);
        root.append(&mode_switcher(workspace, i18n));

        Self {
            root,
            preferences,
            import,
        }
    }

    pub(super) const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub(super) const fn preferences_button(&self) -> &gtk4::Button {
        &self.preferences
    }

    pub(super) const fn import_button(&self) -> &gtk4::Button {
        &self.import
    }
}

fn brand(i18n: &I18n) -> gtk4::Box {
    let brand = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    brand.set_widget_name(PanelSlot::HeaderLeft.identifier());
    brand.set_width_request(i32::from(
        DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.preferred_px,
    ));
    brand.set_halign(gtk4::Align::Start);
    brand.set_valign(gtk4::Align::Center);
    apply_theme_role(&brand, ThemeRole::Toolbar);

    let mark = aperture_mark();
    brand.append(&mark);

    let labels = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let title = gtk4::Label::new(Some(&i18n.text(MessageId::AppTitle, &MessageArgs::new())));
    title.set_widget_name("rusttable-brand-title");
    title.set_halign(gtk4::Align::Start);
    title.add_css_class("dt_brand_title");
    labels.append(&title);
    let version = gtk4::Label::new(Some(env!("CARGO_PKG_VERSION")));
    version.set_widget_name("rusttable-brand-version");
    version.set_halign(gtk4::Align::Start);
    version.add_css_class("dt_brand_version");
    labels.append(&version);
    brand.append(&labels);
    brand
}

fn aperture_mark() -> gtk4::DrawingArea {
    let mark = gtk4::DrawingArea::new();
    mark.set_widget_name("rusttable-aperture-mark");
    mark.set_content_width(24);
    mark.set_content_height(24);
    mark.set_accessible_role(gtk4::AccessibleRole::Img);
    mark.update_property(&[Property::Label("RustTable aperture logo")]);
    mark.set_draw_func(|_, context, width, height| {
        let center_x = f64::from(width) / 2.0;
        let center_y = f64::from(height) / 2.0;
        let radius = f64::from(width.min(height)) * 0.45;
        let colors = [
            (0.91, 0.18, 0.16),
            (0.96, 0.55, 0.12),
            (0.88, 0.81, 0.12),
            (0.25, 0.70, 0.30),
            (0.12, 0.58, 0.82),
            (0.35, 0.30, 0.76),
            (0.72, 0.22, 0.66),
        ];
        let turn = std::f64::consts::TAU / 7.0;
        let mut start = -std::f64::consts::FRAC_PI_2;
        for (red, green, blue) in colors {
            context.move_to(center_x, center_y);
            context.arc(center_x, center_y, radius, start, start + turn);
            context.close_path();
            context.set_source_rgb(red, green, blue);
            context.fill().expect("draw aperture segment");
            start += turn;
        }
        context.set_source_rgb(0.12, 0.12, 0.12);
        context.arc(
            center_x,
            center_y,
            radius * 0.36,
            0.0,
            std::f64::consts::TAU,
        );
        context.fill().expect("draw aperture center");
    });
    mark
}

fn lighttable_toolbar(
    i18n: &I18n,
    display_profile: &DisplayProfileBanner,
) -> (gtk4::Box, gtk4::Button, gtk4::Button) {
    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
    toolbar.set_widget_name(LIGHTTABLE_TOOLBAR.widget_name);
    toolbar.set_hexpand(true);
    toolbar.set_valign(gtk4::Align::Center);
    apply_theme_role(&toolbar, ThemeRole::Toolbar);

    let import = gtk4::Button::with_label("+");
    import.set_widget_name("header-import");
    import.set_tooltip_text(Some(
        &i18n.text(MessageId::ToolbarImport, &MessageArgs::new()),
    ));
    import.update_property(&[Property::Label("Import images")]);
    import.add_css_class("dt_header_icon");
    toolbar.append(&import);

    let property =
        gtk4::DropDown::from_strings(&["filter all images", "filename", "folder", "capture date"]);
    property.set_widget_name("lighttable-filter-property");
    toolbar.append(&property);

    let filter = gtk4::SearchEntry::new();
    filter.set_widget_name(LIGHTTABLE_TOOLBAR.filter_entry_name);
    filter.set_placeholder_text(Some("search"));
    filter.set_hexpand(true);
    filter.set_max_width_chars(18);
    toolbar.append(&filter);

    for (index, label) in ["●", "●", "●", "●", "●", "○", "★", "★", "★", "★", "★"]
        .into_iter()
        .enumerate()
    {
        let button = gtk4::Button::with_label(label);
        button.set_widget_name(&format!("lighttable-rating-{index}"));
        button.add_css_class("dt_filter_button");
        button.add_css_class(if index < 6 {
            "dt_color_filter"
        } else {
            "dt_rating_filter"
        });
        toolbar.append(&button);
    }

    let sort = gtk4::DropDown::from_strings(&["filename", "capture date", "rating"]);
    sort.set_widget_name("lighttable-sort");
    toolbar.append(&sort);
    let count = gtk4::Label::new(Some("0 images selected of 0"));
    count.set_widget_name("lighttable-selection-count");
    count.add_css_class("dim-label");
    toolbar.append(&count);

    let profile = display_profile.widget();
    profile.set_width_chars(2);
    toolbar.append(profile);

    let preferences = gtk4::Button::with_label("⚙");
    preferences.set_widget_name("header-preferences");
    preferences.set_tooltip_text(Some(
        &i18n.text(MessageId::ToolbarPreferences, &MessageArgs::new()),
    ));
    preferences.update_property(&[Property::Label("Open preferences")]);
    preferences.add_css_class("dt_header_icon");
    toolbar.append(&preferences);
    (toolbar, import, preferences)
}

fn mode_switcher(workspace: &gtk4::Stack, i18n: &I18n) -> gtk4::Box {
    let modes = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    modes.set_widget_name(PanelSlot::HeaderRight.identifier());
    modes.set_valign(gtk4::Align::Center);
    apply_theme_role(&modes, ThemeRole::ViewSwitcher);

    let lighttable =
        gtk4::Button::with_label(&i18n.text(MessageId::WorkspaceLighttable, &MessageArgs::new()));
    lighttable.set_widget_name("view-lighttable");
    lighttable.update_property(&[Property::Label("Switch to lighttable")]);
    let darkroom =
        gtk4::Button::with_label(&i18n.text(MessageId::WorkspaceDarkroom, &MessageArgs::new()));
    darkroom.set_widget_name("view-darkroom");
    darkroom.update_property(&[Property::Label("Switch to darkroom")]);
    let other = gtk4::Button::with_label("other ⌄");
    other.set_widget_name("view-other");
    other.update_property(&[Property::Label("Open other views")]);

    connect_mode(&lighttable, workspace, WorkspaceRole::Lighttable);
    connect_mode(&darkroom, workspace, WorkspaceRole::Darkroom);
    sync_mode_classes(workspace, &lighttable, &darkroom);
    workspace.connect_visible_child_name_notify({
        let lighttable = lighttable.clone();
        let darkroom = darkroom.clone();
        move |stack| sync_mode_classes(stack, &lighttable, &darkroom)
    });

    modes.append(&lighttable);
    modes.append(&separator());
    modes.append(&darkroom);
    modes.append(&separator());
    modes.append(&other);
    modes
}

fn connect_mode(button: &gtk4::Button, workspace: &gtk4::Stack, role: WorkspaceRole) {
    let stack = workspace.clone();
    button.connect_clicked(move |_| stack.set_visible_child_name(role.stack_name()));
}

fn sync_mode_classes(workspace: &gtk4::Stack, lighttable: &gtk4::Button, darkroom: &gtk4::Button) {
    let lighttable_active =
        workspace.visible_child_name().as_deref() == Some(WorkspaceRole::Lighttable.stack_name());
    if lighttable_active {
        lighttable.add_css_class("active");
        darkroom.remove_css_class("active");
    } else {
        lighttable.remove_css_class("active");
        darkroom.add_css_class("active");
    }
}

fn separator() -> gtk4::Label {
    let separator = gtk4::Label::new(Some("|"));
    separator.add_css_class("dt_mode_separator");
    separator
}

#[cfg(test)]
mod tests {
    use super::HEADER_WIDGET_IDS;

    #[test]
    fn header_contract_keeps_darktable_slots_and_modes() {
        assert_eq!(
            HEADER_WIDGET_IDS,
            [
                "header",
                "header-left",
                "header-center",
                "header-right",
                "view-lighttable",
                "view-darkroom",
                "view-other",
            ]
        );
    }

    #[test]
    fn shell_does_not_install_the_product_chrome_as_a_native_header_bar() {
        let runtime = include_str!("runtime.rs");

        assert!(!runtime.contains("gtk4::HeaderBar"));
        assert!(!runtime.contains("set_titlebar"));
        assert!(runtime.contains("shell.append(header.widget())"));
    }
}
