//! Darktable-shaped GTK4 header and lighttable mode chrome.

use crate::gui::display_profile::DisplayProfileBanner;
use gtk4::accessible::{Property, State};
use gtk4::prelude::*;
use rusttable_i18n::{I18n, MessageArgs, MessageId};

use super::{
    DARKTABLE_DESKTOP_SPEC, PanelSlot, ShellRegion, ThemeRole, WorkspaceRole, apply_theme_role,
};

/// Native GTK scrolled-window chrome outside its content-height allocation.
const HEADER_VIEWPORT_CHROME_PX: i32 = 5;
const BRAND_MARK_SIZE_PX: i32 = 40;
const BRAND_WORDMARK_RESERVE_PX: i32 = 180;
const BRAND_MARK_GAP_PX: i32 = 5;
const BRAND_LEADING_INSET_PX: i32 = 3;
const MODE_LABEL_PADDING_PX: i32 = 6;
const MODE_LABEL_SCALE: f64 = 1.5;
const OTHER_SELECTOR_WIDTH_PX: i32 = 64;
const MODE_INACTIVE_OPACITY: f64 = 0.64;
const MODE_HOVER_OPACITY: f64 = 0.82;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModeSwitcherItem {
    Lighttable,
    Separator,
    Darkroom,
    OtherSeparator,
    Other,
}

const MODE_SWITCHER_ORDER: [ModeSwitcherItem; 5] = [
    ModeSwitcherItem::Lighttable,
    ModeSwitcherItem::Separator,
    ModeSwitcherItem::Darkroom,
    ModeSwitcherItem::OtherSeparator,
    ModeSwitcherItem::Other,
];

#[cfg(test)]
const HEADER_WIDGET_IDS: [&str; 12] = [
    "header",
    "header-left",
    "rusttable-aperture-mark",
    "rusttable-brand-title",
    "rusttable-brand-version",
    "header-center",
    "header-right",
    "view-lighttable",
    "view-separator-lighttable-darkroom",
    "view-darkroom",
    "view-separator-darkroom-other",
    "view-other",
];

/// Widgets and actions owned by the persistent top panel.
pub(super) struct HeaderChrome {
    surface: gtk4::ScrolledWindow,
    preferences: gtk4::Button,
    import: gtk4::Button,
}

impl HeaderChrome {
    pub(super) fn new(
        workspace: &gtk4::Stack,
        i18n: &I18n,
        _display_profile: &DisplayProfileBanner,
    ) -> Self {
        // GTK4's CenterBox is the direct equivalent of Darktable's start-packed
        // logo, expandable center, and end-packed view switcher. Keeping a real
        // center child also aligns the header composition with the shell-owned
        // top collapse indicator regardless of unequal side widths.
        let root = gtk4::CenterBox::new();
        root.set_orientation(gtk4::Orientation::Horizontal);
        root.set_widget_name(ShellRegion::Header.identifier());
        root.set_vexpand(false);
        apply_theme_role(&root, ThemeRole::Header);

        root.set_start_widget(Some(&brand(i18n)));
        let (import, preferences) = header_actions(i18n);
        let center = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        center.set_widget_name(PanelSlot::HeaderCenter.identifier());
        center.set_width_request(i32::from(DARKTABLE_DESKTOP_SPEC.layout.outer_border_px) * 4);
        center.set_halign(gtk4::Align::Center);
        root.set_center_widget(Some(&center));
        root.set_end_widget(Some(&mode_switcher(workspace, i18n)));

        // A GTK height request is only a minimum. The stacked brand labels and
        // native button metrics otherwise make the shared header taller than
        // Darktable's fixed product chrome in both workspaces. Clip that
        // natural height behind the exact visual-contract allocation.
        let header_height = i32::from(DARKTABLE_DESKTOP_SPEC.layout.header_height_px);
        let content_height = header_height.saturating_sub(HEADER_VIEWPORT_CHROME_PX);
        root.set_height_request(content_height);
        let surface = gtk4::ScrolledWindow::builder()
            .child(&root)
            .has_frame(false)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Never)
            .min_content_height(content_height)
            .max_content_height(content_height)
            // Do not propagate the child's larger natural height: Darktable
            // clips its product chrome to the fixed header allocation.
            .propagate_natural_height(false)
            .build();
        surface.set_height_request(header_height);
        surface.set_widget_name("header-clip");
        surface.set_vexpand(false);

        Self {
            surface,
            preferences,
            import,
        }
    }

    pub(super) const fn widget(&self) -> &gtk4::ScrolledWindow {
        &self.surface
    }

    pub(super) const fn preferences_button(&self) -> &gtk4::Button {
        &self.preferences
    }

    pub(super) const fn import_button(&self) -> &gtk4::Button {
        &self.import
    }
}

fn brand(i18n: &I18n) -> gtk4::Box {
    let brand = gtk4::Box::new(gtk4::Orientation::Horizontal, BRAND_MARK_GAP_PX);
    brand.set_widget_name(PanelSlot::HeaderLeft.identifier());
    brand.set_width_request(BRAND_MARK_SIZE_PX + BRAND_WORDMARK_RESERVE_PX);
    brand.set_margin_start(BRAND_LEADING_INSET_PX);
    brand.set_hexpand(false);
    brand.set_halign(gtk4::Align::Start);
    brand.set_valign(gtk4::Align::Center);
    apply_theme_role(&brand, ThemeRole::Toolbar);

    let mark = aperture_mark();
    brand.append(&mark);

    let labels = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    labels.set_valign(gtk4::Align::Center);
    let title_text = i18n
        .text(MessageId::AppTitle, &MessageArgs::new())
        .to_lowercase();
    let title = gtk4::Label::new(Some(&title_text));
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
    mark.set_content_width(BRAND_MARK_SIZE_PX);
    mark.set_content_height(BRAND_MARK_SIZE_PX);
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

fn header_actions(i18n: &I18n) -> (gtk4::Button, gtk4::Button) {
    // Darktable keeps the center of the persistent product header empty. These actions remain
    // controller-owned for the left-rail import route and keyboard/preferences commands, but do
    // not create a second plus/wrench toolbar in the header.
    let import = gtk4::Button::from_icon_name("list-add-symbolic");
    import.set_widget_name("header-import");
    import.set_tooltip_text(Some(
        &i18n.text(MessageId::ToolbarImport, &MessageArgs::new()),
    ));
    import.update_property(&[Property::Label("Import images")]);
    import.add_css_class("dt_header_icon");

    let preferences = gtk4::Button::from_icon_name("preferences-system-symbolic");
    preferences.set_widget_name("header-preferences");
    preferences.set_tooltip_text(Some(
        &i18n.text(MessageId::ToolbarPreferences, &MessageArgs::new()),
    ));
    preferences.update_property(&[Property::Label("Open preferences")]);
    preferences.add_css_class("dt_header_icon");
    (import, preferences)
}

fn mode_switcher(workspace: &gtk4::Stack, i18n: &I18n) -> gtk4::Box {
    let modes = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    modes.set_widget_name(PanelSlot::HeaderRight.identifier());
    modes.set_hexpand(false);
    modes.set_halign(gtk4::Align::End);
    modes.set_valign(gtk4::Align::Center);
    apply_theme_role(&modes, ThemeRole::ViewSwitcher);

    let lighttable_label = i18n
        .text(MessageId::WorkspaceLighttable, &MessageArgs::new())
        .to_lowercase();
    let lighttable = mode_link(
        &lighttable_label,
        "view-lighttable",
        "Switch to lighttable",
        workspace,
        WorkspaceRole::Lighttable,
    );
    let darkroom_label = i18n
        .text(MessageId::WorkspaceDarkroom, &MessageArgs::new())
        .to_lowercase();
    let darkroom = mode_link(
        &darkroom_label,
        "view-darkroom",
        "Switch to darkroom",
        workspace,
        WorkspaceRole::Darkroom,
    );
    sync_mode_classes(workspace, &lighttable, &darkroom);
    workspace.connect_visible_child_name_notify({
        let lighttable = lighttable.clone();
        let darkroom = darkroom.clone();
        move |stack| sync_mode_classes(stack, &lighttable, &darkroom)
    });

    let primary_separator = separator("view-separator-lighttable-darkroom");
    let other_separator = separator("view-separator-darkroom-other");
    let other = other_mode_selector();
    for item in MODE_SWITCHER_ORDER {
        match item {
            ModeSwitcherItem::Lighttable => modes.append(&lighttable),
            ModeSwitcherItem::Separator => modes.append(&primary_separator),
            ModeSwitcherItem::Darkroom => modes.append(&darkroom),
            ModeSwitcherItem::OtherSeparator => modes.append(&other_separator),
            ModeSwitcherItem::Other => modes.append(&other),
        }
    }
    modes
}

fn mode_link(
    text: &str,
    id: &str,
    accessible_name: &'static str,
    workspace: &gtk4::Stack,
    role: WorkspaceRole,
) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_widget_name(id);
    label.add_css_class("dt_mode_button");
    label.set_margin_start(MODE_LABEL_PADDING_PX);
    label.set_margin_end(MODE_LABEL_PADDING_PX);
    label.set_focusable(true);
    label.set_accessible_role(gtk4::AccessibleRole::Radio);
    label.update_property(&[Property::Label(accessible_name)]);

    let click = gtk4::GestureClick::new();
    click.connect_released({
        let stack = workspace.clone();
        move |_, _, _, _| stack.set_visible_child_name(role.stack_name())
    });
    label.add_controller(click);

    let key = gtk4::EventControllerKey::new();
    key.connect_key_pressed({
        let stack = workspace.clone();
        move |_, key, _, _| {
            if matches!(
                key,
                gtk4::gdk::Key::Return | gtk4::gdk::Key::KP_Enter | gtk4::gdk::Key::space
            ) {
                stack.set_visible_child_name(role.stack_name());
                gtk4::glib::Propagation::Stop
            } else {
                gtk4::glib::Propagation::Proceed
            }
        }
    });
    label.add_controller(key);

    let motion = gtk4::EventControllerMotion::new();
    motion.connect_enter({
        let label = label.clone();
        move |_, _, _| {
            if !label.has_css_class("active") {
                label.set_opacity(MODE_HOVER_OPACITY);
            }
        }
    });
    motion.connect_leave({
        let label = label.clone();
        move |_| {
            if !label.has_css_class("active") {
                label.set_opacity(MODE_INACTIVE_OPACITY);
            }
        }
    });
    label.add_controller(motion);
    label
}

fn sync_mode_classes(workspace: &gtk4::Stack, lighttable: &gtk4::Label, darkroom: &gtk4::Label) {
    match workspace.visible_child_name().as_deref() {
        Some(name) if name == WorkspaceRole::Lighttable.stack_name() => {
            set_mode_active(lighttable, true);
            set_mode_active(darkroom, false);
        }
        Some(name) if name == WorkspaceRole::Darkroom.stack_name() => {
            set_mode_active(lighttable, false);
            set_mode_active(darkroom, true);
        }
        _ => {
            set_mode_active(lighttable, false);
            set_mode_active(darkroom, false);
        }
    }
}

fn set_mode_active(label: &gtk4::Label, active: bool) {
    if active {
        label.add_css_class("active");
    } else {
        label.remove_css_class("active");
    }
    label.set_opacity(if active { 1.0 } else { MODE_INACTIVE_OPACITY });
    let attributes = gtk4::pango::AttrList::new();
    attributes.insert(gtk4::pango::AttrFloat::new_scale(MODE_LABEL_SCALE));
    if active {
        attributes.insert(gtk4::pango::AttrInt::new_weight(gtk4::pango::Weight::Bold));
    }
    label.set_attributes(Some(&attributes));
    label.update_state(&[State::Selected(Some(active))]);
}

fn separator(id: &str) -> gtk4::Label {
    let separator = gtk4::Label::new(Some("|"));
    separator.set_widget_name(id);
    separator.add_css_class("dt_mode_separator");
    separator.set_margin_start(MODE_LABEL_PADDING_PX);
    separator.set_margin_end(MODE_LABEL_PADDING_PX);
    separator
}

fn other_mode_selector() -> gtk4::Box {
    let other = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
    other.set_widget_name("view-other");
    other.add_css_class("dt_mode_button");
    other.set_width_request(OTHER_SELECTOR_WIDTH_PX);
    other.set_margin_start(MODE_LABEL_PADDING_PX);
    other.set_margin_end(MODE_LABEL_PADDING_PX);
    other.set_valign(gtk4::Align::Center);
    other.set_opacity(MODE_INACTIVE_OPACITY);
    other.set_tooltip_text(Some(
        "Other Darktable workspaces are not implemented in RustTable",
    ));
    other.set_accessible_role(gtk4::AccessibleRole::ComboBox);
    other.update_property(&[Property::Label("Other workspaces are not implemented")]);

    let label = gtk4::Label::new(Some("other"));
    label.set_hexpand(true);
    label.set_halign(gtk4::Align::Start);
    let arrow = gtk4::Image::from_icon_name("pan-down-symbolic");
    arrow.set_pixel_size(8);
    other.append(&label);
    other.append(&arrow);
    other
}

#[cfg(test)]
mod tests {
    use super::{
        BRAND_LEADING_INSET_PX, BRAND_MARK_GAP_PX, BRAND_MARK_SIZE_PX, BRAND_WORDMARK_RESERVE_PX,
        HEADER_WIDGET_IDS, MODE_LABEL_PADDING_PX, MODE_LABEL_SCALE, MODE_SWITCHER_ORDER,
        ModeSwitcherItem, OTHER_SELECTOR_WIDTH_PX,
    };
    use crate::gui::DARKTABLE_DESKTOP_SPEC;

    #[test]
    fn header_contract_keeps_darktable_slots_and_modes() {
        assert_eq!(
            HEADER_WIDGET_IDS,
            [
                "header",
                "header-left",
                "rusttable-aperture-mark",
                "rusttable-brand-title",
                "rusttable-brand-version",
                "header-center",
                "header-right",
                "view-lighttable",
                "view-separator-lighttable-darkroom",
                "view-darkroom",
                "view-separator-darkroom-other",
                "view-other",
            ]
        );
    }

    #[test]
    fn header_geometry_maps_darktable_brand_and_view_spacing() {
        assert_eq!(BRAND_MARK_SIZE_PX, 40);
        assert_eq!(BRAND_WORDMARK_RESERVE_PX, 180);
        assert_eq!(
            DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.preferred_px,
            265
        );
        assert_eq!(BRAND_LEADING_INSET_PX, 3);
        assert_eq!(BRAND_MARK_GAP_PX, 5);
        assert_eq!(MODE_LABEL_PADDING_PX, 6);
        assert!((MODE_LABEL_SCALE - 1.5).abs() < f64::EPSILON);
        assert_eq!(OTHER_SELECTOR_WIDTH_PX, 64);
    }

    #[test]
    fn mode_switcher_keeps_primary_views_before_the_other_selector() {
        assert_eq!(
            MODE_SWITCHER_ORDER,
            [
                ModeSwitcherItem::Lighttable,
                ModeSwitcherItem::Separator,
                ModeSwitcherItem::Darkroom,
                ModeSwitcherItem::OtherSeparator,
                ModeSwitcherItem::Other,
            ]
        );
    }

    #[test]
    fn shell_does_not_install_the_product_chrome_as_a_native_header_bar() {
        let runtime = include_str!("runtime/mod.rs");

        assert!(!runtime.contains("gtk4::HeaderBar"));
        assert!(!runtime.contains("set_titlebar"));
        assert!(runtime.contains("shell.append(header.widget())"));
    }
}
