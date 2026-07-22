//! Small, concrete GTK building blocks shared by the Darktable workspaces.
//!
//! These helpers intentionally describe Darktable chrome rather than a generic
//! widget framework.  Keeping the spacing, sizing, and semantic classes here
//! makes the lighttable and darkroom rails look like one application while
//! preserving each subsystem's module ownership.

use gtk4::prelude::*;

use super::{DARKTABLE_DESKTOP_SPEC, DARKTABLE_UI_TOKENS, ThemeRole, apply_theme_role};

pub(crate) const CONTROL_GAP: i32 = DARKTABLE_UI_TOKENS.controls.control_gap;
pub(crate) const MODULE_GAP: i32 = DARKTABLE_UI_TOKENS.controls.module_gap;
pub(crate) const MODULE_ROW_HEIGHT: i32 = DARKTABLE_UI_TOKENS.controls.module_row_height;
pub(crate) const TOOLBAR_HEIGHT: i32 = DARKTABLE_UI_TOKENS.controls.toolbar_height;
pub(crate) const RAIL_SCROLLBAR_RESERVE: i32 = DARKTABLE_UI_TOKENS.controls.rail_scrollbar_reserve;
const MODULE_CONTROL_MIN_WIDTH: i32 = DARKTABLE_UI_TOKENS.controls.module_control_min_width;

pub(crate) fn toolbar(id: &str, role: ThemeRole) -> gtk4::Box {
    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, CONTROL_GAP);
    toolbar.set_widget_name(id);
    toolbar.set_height_request(TOOLBAR_HEIGHT);
    toolbar.set_hexpand(true);
    toolbar.set_valign(gtk4::Align::Center);
    toolbar.add_css_class("dt_toolbar");
    apply_theme_role(&toolbar, role);
    toolbar
}

pub(crate) fn module_expander(
    id: &str,
    title: &str,
    expanded: bool,
    child: Option<&impl IsA<gtk4::Widget>>,
) -> gtk4::Expander {
    let module_widget = gtk4::Expander::builder()
        .label(title)
        .expanded(expanded)
        .build();
    if let Some(child) = child {
        module_widget.set_child(Some(child));
    }
    module_widget.set_widget_name(id);
    module_widget.set_focusable(true);
    module_widget.add_css_class("dt_module_expander");
    apply_theme_role(&module_widget, ThemeRole::Module);
    module_widget
}

pub(crate) fn module_row<W: IsA<gtk4::Widget>>(label: &str, widget: &W) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, CONTROL_GAP);
    row.set_height_request(MODULE_ROW_HEIGHT);
    row.set_width_request(0);
    row.set_hexpand(true);
    row.add_css_class("dt_module_row");
    let label_widget = gtk4::Label::new(Some(label));
    label_widget.set_halign(gtk4::Align::Start);
    label_widget.set_hexpand(true);
    label_widget.set_width_chars(1);
    label_widget.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    row.append(&label_widget);
    row.append(widget);
    row
}

pub(crate) fn scale_row(
    label: &str,
    scale: &gtk4::Scale,
    value: &gtk4::Label,
    unit: &str,
) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Vertical, MODULE_GAP);
    row.set_width_request(0);
    row.set_hexpand(true);
    row.add_css_class("dt_module_row");
    let heading = gtk4::Box::new(gtk4::Orientation::Horizontal, CONTROL_GAP);
    let label_text = if unit.is_empty() {
        label.to_owned()
    } else {
        format!("{label} ({unit})")
    };
    let label_widget = gtk4::Label::new(Some(&label_text));
    label_widget.set_halign(gtk4::Align::Start);
    label_widget.set_hexpand(true);
    heading.append(&label_widget);
    heading.append(value);
    row.append(&heading);
    row.append(scale);
    row
}

pub(crate) fn button(id: &str, label: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.set_height_request(DARKTABLE_UI_TOKENS.controls.control_height);
    button.add_css_class("dt_button");
    button.set_focus_on_click(false);
    button
}

pub(crate) fn toggle_button(id: &str, label: &str) -> gtk4::ToggleButton {
    let button = gtk4::ToggleButton::with_label(label);
    button.set_widget_name(id);
    button.set_height_request(DARKTABLE_UI_TOKENS.controls.control_height);
    button.add_css_class("dt_button");
    button.set_focus_on_click(false);
    button
}

/// Disabled until the owning module exposes an action handler, but retained in
/// the title row so rail geometry matches Darktable's action affordance slot.
pub(crate) fn module_action_button(id: &str, accessible_name: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label("⋮");
    button.set_widget_name(id);
    button.set_size_request(20, 20);
    button.set_focusable(false);
    button.set_sensitive(false);
    button.add_css_class("dt_module_action");
    button.update_property(&[gtk4::accessible::Property::Label(accessible_name)]);
    button.set_tooltip_text(Some(accessible_name));
    button
}

pub(crate) fn dropdown(id: &str, values: &[&str]) -> gtk4::DropDown {
    let dropdown = gtk4::DropDown::from_strings(values);
    dropdown.set_widget_name(id);
    dropdown.set_height_request(DARKTABLE_UI_TOKENS.controls.control_height);
    dropdown.set_width_request(MODULE_CONTROL_MIN_WIDTH);
    dropdown.set_hexpand(true);
    dropdown.add_css_class("dt_field");
    dropdown
}

pub(crate) fn slider(
    id: &str,
    minimum: f64,
    maximum: f64,
    step: f64,
    draw_value: bool,
) -> gtk4::Scale {
    let slider = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, minimum, maximum, step);
    slider.set_widget_name(id);
    slider.set_height_request(DARKTABLE_UI_TOKENS.controls.control_height);
    slider.set_width_request(MODULE_CONTROL_MIN_WIDTH);
    slider.set_hexpand(true);
    slider.set_draw_value(draw_value);
    slider.add_css_class("dt_slider");
    slider
}

pub(crate) fn switch(id: &str) -> gtk4::Switch {
    let control = gtk4::Switch::new();
    control.set_widget_name(id);
    control.set_height_request(DARKTABLE_UI_TOKENS.controls.control_height);
    control.set_valign(gtk4::Align::Center);
    control.add_css_class("dt_switch");
    control
}

pub(crate) fn rail(id: &str, width: i32, accessible_name: &str) -> gtk4::Box {
    let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    panel.set_widget_name(id);
    let minimum = i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.minimum_px);
    panel.set_size_request(width.max(minimum), -1);
    panel.set_hexpand(false);
    panel.set_vexpand(true);
    panel.set_halign(gtk4::Align::Fill);
    panel.set_valign(gtk4::Align::Fill);
    panel.set_accessible_role(gtk4::AccessibleRole::Group);
    panel.update_property(&[gtk4::accessible::Property::Label(accessible_name)]);
    panel.add_css_class("dt_rail");
    apply_theme_role(&panel, ThemeRole::Panel);
    panel
}

pub(crate) fn rail_scroll<W: IsA<gtk4::Widget>>(
    child: &W,
    width: i32,
    id: &str,
) -> gtk4::ScrolledWindow {
    let scroll = gtk4::ScrolledWindow::builder()
        .child(child)
        .hexpand(true)
        .vexpand(true)
        .build();
    scroll.set_widget_name(id);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    let minimum = i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.minimum_px);
    let allocated_content = width.max(minimum).saturating_sub(RAIL_SCROLLBAR_RESERVE);
    scroll.set_min_content_width(allocated_content);
    scroll.set_propagate_natural_width(false);
    scroll.set_propagate_natural_height(false);
    scroll.add_css_class("dt_rail_scroll");
    child.set_width_request(0);
    child.set_hexpand(true);
    scroll
}
