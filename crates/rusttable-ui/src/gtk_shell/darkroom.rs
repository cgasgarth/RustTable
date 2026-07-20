//! Darktable-shaped GTK4 darkroom composition.

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::{ExposurePanel, PhotoPreview, ThemeRole, apply_theme_role};

/// Stable widget identifiers for the initial darkroom surface.
pub const DARKROOM_WIDGET_IDS: [&str; 13] = [
    "darkroom-page",
    "darkroom-toolbar-top",
    "darkroom-photo-preview",
    "darkroom-toolbar-bottom",
    "darkroom-left-panel",
    "darkroom-navigation",
    "darkroom-snapshots",
    "darkroom-history",
    "darkroom-image-information",
    "darkroom-right-panel",
    "darkroom-histogram",
    "darkroom-module-groups",
    "exposure",
];

/// Native GTK widgets owned by the darkroom view.
pub struct DarkroomView {
    page: gtk4::Box,
    preview: PhotoPreview,
    left_panel: gtk4::Box,
    left_modules: gtk4::Box,
    right_panel: gtk4::Box,
    right_modules: gtk4::Box,
    exposure: ExposurePanel,
}

impl DarkroomView {
    /// Builds the initial Darktable darkroom around the immutable preview boundary.
    #[must_use]
    pub fn new(panel_width: i32) -> Self {
        let preview = PhotoPreview::new();
        let page = darkroom_page(&preview);
        let (left_panel, left_modules) = left_panel(panel_width);
        let (right_panel, right_modules, exposure) = right_panel(panel_width);
        Self {
            page,
            preview,
            left_panel,
            left_modules,
            right_panel,
            right_modules,
            exposure,
        }
    }

    #[must_use]
    pub fn page(&self) -> &gtk4::Box {
        &self.page
    }

    #[must_use]
    pub fn preview(&self) -> &PhotoPreview {
        &self.preview
    }

    #[must_use]
    pub fn left_panel(&self) -> &gtk4::Box {
        &self.left_panel
    }

    #[must_use]
    pub fn left_modules(&self) -> &gtk4::Box {
        &self.left_modules
    }

    #[must_use]
    pub fn right_panel(&self) -> &gtk4::Box {
        &self.right_panel
    }

    #[must_use]
    pub fn right_modules(&self) -> &gtk4::Box {
        &self.right_modules
    }

    #[must_use]
    pub fn exposure(&self) -> &ExposurePanel {
        &self.exposure
    }
}

fn darkroom_page(preview: &PhotoPreview) -> gtk4::Box {
    let page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    page.set_widget_name("darkroom-page");
    page.set_hexpand(true);
    page.set_vexpand(true);
    apply_theme_role(&page, ThemeRole::Darkroom);

    let top = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    top.set_widget_name("darkroom-toolbar-top");
    top.add_css_class("dt_darkroom_toolbar");
    top.append(&chrome_button(
        "darkroom-soft-proof",
        "soft proof",
        "Toggle soft proof",
    ));
    top.append(&chrome_button(
        "darkroom-gamut-check",
        "gamut check",
        "Toggle gamut warning",
    ));

    let bottom = gtk4::Box::new(gtk4::Orientation::Horizontal, 2);
    bottom.set_widget_name("darkroom-toolbar-bottom");
    bottom.add_css_class("dt_darkroom_toolbar");
    bottom.append(&chrome_button("darkroom-zoom", "100%", "Set darkroom zoom"));
    bottom.append(&chrome_button(
        "darkroom-fit",
        "fit",
        "Fit image to viewport",
    ));
    bottom.append(&chrome_button(
        "darkroom-before-after",
        "before/after",
        "Compare before and after",
    ));

    page.append(&top);
    page.append(preview.widget());
    page.append(&bottom);
    page
}

fn left_panel(width: i32) -> (gtk4::Box, gtk4::Box) {
    let panel = rail("darkroom-left-panel", width, "Darkroom left module rail");
    let modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    modules.set_widget_name("darkroom-left-modules");
    for (id, title, expanded) in [
        ("darkroom-navigation", "navigation", true),
        ("darkroom-snapshots", "snapshots", false),
        ("darkroom-history", "history", false),
        ("darkroom-image-information", "image information", false),
    ] {
        modules.append(&module(id, title, expanded));
    }
    let controller_modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    controller_modules.set_widget_name("darkroom-left-controller-modules");
    modules.append(&controller_modules);
    let scroll = gtk4::ScrolledWindow::builder()
        .child(&modules)
        .hexpand(true)
        .vexpand(true)
        .build();
    panel.append(&scroll);
    (panel, controller_modules)
}

fn right_panel(width: i32) -> (gtk4::Box, gtk4::Box, ExposurePanel) {
    let panel = rail(
        "darkroom-right-panel",
        width,
        "Darkroom processing module rail",
    );
    let histogram = gtk4::DrawingArea::new();
    histogram.set_widget_name("darkroom-histogram");
    histogram.set_height_request(92);
    histogram.set_accessible_role(gtk4::AccessibleRole::Img);
    histogram.update_property(&[Property::Label("Image histogram")]);
    panel.append(&histogram);

    let groups = gtk4::Box::new(gtk4::Orientation::Horizontal, 1);
    groups.set_widget_name("darkroom-module-groups");
    groups.set_accessible_role(gtk4::AccessibleRole::Toolbar);
    groups.update_property(&[Property::Label("Processing module groups")]);
    for (id, icon, label) in [
        ("group-active", "●", "Active modules"),
        ("group-favorites", "★", "Favorite modules"),
        ("group-technical", "○", "Technical modules"),
        ("group-grading", "◐", "Grading modules"),
    ] {
        groups.append(&chrome_button(id, icon, label));
    }
    panel.append(&groups);

    let modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    modules.set_widget_name("darkroom-right-modules");
    let exposure = ExposurePanel::new();
    modules.append(exposure.widget());
    let controller_modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    controller_modules.set_widget_name("darkroom-right-controller-modules");
    modules.append(&controller_modules);
    let scroll = gtk4::ScrolledWindow::builder()
        .child(&modules)
        .hexpand(true)
        .vexpand(true)
        .build();
    panel.append(&scroll);
    (panel, controller_modules, exposure)
}

fn rail(id: &str, width: i32, accessible_name: &str) -> gtk4::Box {
    let panel = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    panel.set_widget_name(id);
    panel.set_width_request(width);
    panel.set_accessible_role(gtk4::AccessibleRole::Group);
    panel.update_property(&[Property::Label(accessible_name)]);
    apply_theme_role(&panel, ThemeRole::Panel);
    panel
}

fn module(id: &str, title: &str, expanded: bool) -> gtk4::Box {
    let module = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    module.set_widget_name(id);
    module.set_accessible_role(gtk4::AccessibleRole::Group);
    module.update_property(&[Property::Label(title)]);
    apply_theme_role(&module, ThemeRole::ModuleGroup);
    let disclosure = if expanded { "⌄" } else { "›" };
    let header = gtk4::Button::with_label(&format!("{disclosure} {title}"));
    header.add_css_class("dt_darkroom_module_header");
    header.set_focus_on_click(false);
    header.update_property(&[Property::Label(title)]);
    module.append(&header);
    module
}

fn chrome_button(id: &str, label: &str, accessible_name: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.add_css_class("dt_button");
    button.set_focus_on_click(false);
    button.update_property(&[Property::Label(accessible_name)]);
    button
}

#[cfg(test)]
mod tests {
    use super::DARKROOM_WIDGET_IDS;

    #[test]
    fn darkroom_contract_has_stable_unique_roles_and_initial_exposure() {
        let unique = DARKROOM_WIDGET_IDS
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(unique.len(), DARKROOM_WIDGET_IDS.len());
        assert_eq!(DARKROOM_WIDGET_IDS[0], "darkroom-page");
        assert_eq!(DARKROOM_WIDGET_IDS.last(), Some(&"exposure"));
    }
}
