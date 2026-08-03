//! GTK4 adapter for the labeled Bauhaus combobox responsibilities in
//! `src/bauhaus/bauhaus.c` and `src/develop/imageop_gui.c`.
//!
//! Darktable keeps insertion order, renders the control label and selected
//! entry inside one full-width control, applies one tooltip to that control,
//! and emits one value-change signal for a completed selection. GTK4's
//! `DropDown` owns the popup and selection model here while this adapter owns
//! the source-style composite boundary.

use std::rc::Rc;

use gtk4::prelude::*;

// Selected by Darktable's non-condensed Bauhaus default.
const INNER_PADDING: i32 = 4;

/// Immutable source metadata used to construct a Bauhaus combobox.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BauhausComboBoxSpec<'a> {
    pub(crate) label: &'a str,
    pub(crate) tooltip: &'a str,
    pub(crate) options: &'a [&'a str],
}

impl<'a> BauhausComboBoxSpec<'a> {
    pub(crate) const fn new(label: &'a str, tooltip: &'a str, options: &'a [&'a str]) -> Self {
        Self {
            label,
            tooltip,
            options,
        }
    }
}

/// Full-width GTK4 Bauhaus combobox with an internal label.
#[derive(Clone, Debug)]
pub struct BauhausComboBox {
    root: gtk4::Box,
    dropdown: gtk4::DropDown,
    options: Rc<[String]>,
    tooltip: Rc<str>,
}

impl BauhausComboBox {
    #[must_use]
    pub(crate) fn new(widget_name: &str, spec: BauhausComboBoxSpec<'_>) -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, INNER_PADDING);
        root.set_widget_name(widget_name);
        root.set_hexpand(true);
        root.set_halign(gtk4::Align::Fill);
        root.set_tooltip_text(Some(spec.tooltip));
        root.add_css_class("dt_bauhaus");

        let label = gtk4::Label::new(Some(spec.label));
        label.set_widget_name("bauhaus-combobox-label");
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        label.set_width_chars(1);
        label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        label.add_css_class("dt_bauhaus_label");
        root.append(&label);

        let dropdown = gtk4::DropDown::from_strings(spec.options);
        dropdown.set_widget_name(&format!("{widget_name}-selection"));
        dropdown.set_halign(gtk4::Align::End);
        dropdown.set_tooltip_text(Some(spec.tooltip));
        dropdown.set_accessible_role(gtk4::AccessibleRole::ComboBox);
        dropdown.update_property(&[
            gtk4::accessible::Property::Label(spec.label),
            gtk4::accessible::Property::Description(spec.tooltip),
        ]);
        dropdown.add_css_class("dt_field");
        label.set_mnemonic_widget(Some(&dropdown));
        root.append(&dropdown);

        Self {
            root,
            dropdown,
            options: spec
                .options
                .iter()
                .map(|option| (*option).to_owned())
                .collect::<Vec<_>>()
                .into(),
            tooltip: Rc::from(spec.tooltip),
        }
    }

    pub(crate) const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub(crate) const fn dropdown(&self) -> &gtk4::DropDown {
        &self.dropdown
    }

    #[must_use]
    pub(crate) fn selected(&self) -> u32 {
        self.dropdown.selected()
    }

    pub(crate) fn set_selected(&self, selected: u32) {
        self.dropdown.set_selected(selected);
    }

    /// Connects the completed-selection signal exposed by GTK4's `DropDown`.
    pub(crate) fn connect_selection_changed(&self, callback: impl Fn(&Self) + 'static) {
        let control = self.clone();
        self.dropdown
            .connect_selected_notify(move |_| callback(&control));
    }

    #[must_use]
    pub(crate) fn option_count(&self) -> usize {
        self.options.len()
    }

    #[must_use]
    pub(crate) fn option_label(&self, index: usize) -> Option<&str> {
        self.options.get(index).map(String::as_str)
    }

    #[must_use]
    pub(crate) fn tooltip(&self) -> &str {
        &self.tooltip
    }
}

#[cfg(test)]
mod tests {
    use super::BauhausComboBoxSpec;

    #[test]
    fn source_metadata_preserves_option_order_and_control_tooltip() {
        const OPTIONS: [&str; 3] = ["cubic spline", "centripetal spline", "monotonic spline"];
        const TOOLTIP: &str = "change this method if you see oscillations or cusps in the curve\n\
                                - cubic spline is smoother";
        const SPEC: BauhausComboBoxSpec<'_> =
            BauhausComboBoxSpec::new("interpolation method", TOOLTIP, &OPTIONS);

        assert_eq!(SPEC.label, "interpolation method");
        assert_eq!(SPEC.options, &OPTIONS);
        assert_eq!(SPEC.tooltip, TOOLTIP);
    }

    #[test]
    fn repeated_labels_remain_distinct_ordered_entries() {
        const OPTIONS: [&str; 4] = ["hue", "saturation", "hue", "brightness"];
        const SPEC: BauhausComboBoxSpec<'_> =
            BauhausComboBoxSpec::new("select by", "selection criterion", &OPTIONS);

        assert_eq!(SPEC.options, &["hue", "saturation", "hue", "brightness"]);
    }
}
