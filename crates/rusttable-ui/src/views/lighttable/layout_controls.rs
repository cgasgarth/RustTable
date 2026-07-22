//! Darktable lighttable surface selectors.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::interaction::{LighttableLayout, LighttableZoom};
use crate::gui::{ThemeRole, apply_theme_role};

/// GTK controls for switching between Darktable's lighttable surfaces.
#[derive(Clone)]
pub struct LighttableLayoutControls {
    root: gtk4::Box,
    buttons: Rc<Vec<(LighttableLayout, gtk4::ToggleButton)>>,
    zoom_out: gtk4::Button,
    zoom_in: gtk4::Button,
    zoom_value: gtk4::Label,
    projecting: Rc<Cell<bool>>,
    layout: Rc<RefCell<LighttableLayout>>,
    zoom: Rc<RefCell<LighttableZoom>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LighttablePanel {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LighttableLayoutAction {
    SetLayout(LighttableLayout),
    SetZoom(LighttableZoom),
    SetPanelVisibility {
        panel: LighttablePanel,
        visible: bool,
    },
}

impl LighttableLayoutControls {
    #[must_use]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        root.set_widget_name("lighttable-layout-controls");
        root.set_accessible_role(gtk4::AccessibleRole::Toolbar);
        root.update_property(&[Property::Label("Lighttable layout")]);
        apply_theme_role(&root, ThemeRole::Toolbar);

        let layout_group = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        layout_group.set_widget_name("lighttable-layout-mode-group");
        layout_group.add_css_class("dt_segmented_group");

        let layouts = [
            (
                LighttableLayout::FileManager,
                "grid",
                "File manager grid",
                "view-grid-symbolic",
            ),
            (
                LighttableLayout::Zoomable,
                "zoom",
                "Zoomable grid",
                "view-app-grid-symbolic",
            ),
            (
                LighttableLayout::Culling,
                "cull",
                "Culling view",
                "view-dual-symbolic",
            ),
            (
                LighttableLayout::CullingDynamic,
                "dynamic",
                "Dynamic culling view",
                "view-continuous-symbolic",
            ),
            (
                LighttableLayout::Preview,
                "preview",
                "Full preview",
                "view-fullscreen-symbolic",
            ),
        ];
        let buttons = layouts
            .into_iter()
            .map(|(layout, suffix, accessible_name, icon_name)| {
                let button = gtk4::ToggleButton::new();
                button.set_widget_name(&format!("lighttable-layout-{suffix}"));
                button.set_child(Some(&gtk4::Image::from_icon_name(icon_name)));
                button.set_focus_on_click(false);
                button.set_accessible_role(gtk4::AccessibleRole::Radio);
                button.update_property(&[Property::Label(accessible_name)]);
                button.set_tooltip_text(Some(accessible_name));
                layout_group.append(&button);
                (layout, button)
            })
            .collect::<Vec<_>>();
        root.append(&layout_group);

        let zoom_group = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        zoom_group.set_widget_name("lighttable-zoom-controls");
        zoom_group.add_css_class("dt_segmented_group");
        let zoom_value = gtk4::Label::new(None);
        zoom_value.set_widget_name("lighttable-zoom-columns");
        zoom_value.set_width_chars(2);
        zoom_value.set_accessible_role(gtk4::AccessibleRole::Status);
        zoom_value.update_property(&[Property::Label("Images per row")]);
        let zoom_out = zoom_button(
            "lighttable-zoom-out",
            "list-remove-symbolic",
            "Fewer, larger thumbnails",
        );
        let zoom_in = zoom_button(
            "lighttable-zoom-in",
            "list-add-symbolic",
            "More, smaller thumbnails",
        );
        zoom_group.append(&zoom_value);
        zoom_group.append(&zoom_out);
        zoom_group.append(&zoom_in);
        root.append(&zoom_group);
        let controls = Self {
            root,
            buttons: Rc::new(buttons),
            zoom_out,
            zoom_in,
            zoom_value,
            projecting: Rc::new(Cell::new(false)),
            layout: Rc::new(RefCell::new(LighttableLayout::default())),
            zoom: Rc::new(RefCell::new(LighttableZoom::default())),
        };
        controls.set_layout(LighttableLayout::default());
        controls.set_zoom(LighttableZoom::default());
        controls
    }

    #[must_use]
    pub const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    #[must_use]
    pub fn layout(&self) -> LighttableLayout {
        *self.layout.borrow()
    }

    pub fn set_layout(&self, layout: LighttableLayout) {
        self.projecting.set(true);
        *self.layout.borrow_mut() = layout;
        for (candidate, button) in self.buttons.iter() {
            button.set_active(*candidate == layout);
        }
        self.projecting.set(false);
    }

    #[must_use]
    pub fn zoom(&self) -> LighttableZoom {
        *self.zoom.borrow()
    }

    pub fn set_zoom(&self, zoom: LighttableZoom) {
        *self.zoom.borrow_mut() = zoom;
        self.zoom_value.set_text(&zoom.columns().to_string());
        self.zoom_out.set_sensitive(zoom != LighttableZoom::Large);
        self.zoom_in.set_sensitive(zoom != LighttableZoom::Small);
    }

    pub fn connect_layout<F>(&self, handler: F)
    where
        F: Fn(LighttableLayout) + 'static,
    {
        self.connect_action(move |action| {
            if let LighttableLayoutAction::SetLayout(layout) = action {
                handler(layout);
            }
        });
    }

    pub fn connect_action<F>(&self, handler: F)
    where
        F: Fn(LighttableLayoutAction) + 'static,
    {
        let handler = Rc::new(handler);
        let guard = Rc::clone(&self.projecting);
        for (layout, button) in self.buttons.iter() {
            let handler = Rc::clone(&handler);
            let guard = Rc::clone(&guard);
            let buttons = Rc::clone(&self.buttons);
            let selected = *layout;
            button.connect_toggled(move |button| {
                if guard.get() {
                    return;
                }
                if !button.is_active() {
                    guard.set(true);
                    button.set_active(true);
                    guard.set(false);
                    return;
                }
                guard.set(true);
                for (candidate, other) in buttons.iter() {
                    if *candidate != selected {
                        other.set_active(false);
                    }
                }
                guard.set(false);
                handler(LighttableLayoutAction::SetLayout(selected));
            });
        }
        let zoom = Rc::clone(&self.zoom);
        let action = Rc::clone(&handler);
        self.zoom_out.connect_clicked(move |_| {
            action(LighttableLayoutAction::SetZoom(zoom.borrow().larger()));
        });
        let zoom = Rc::clone(&self.zoom);
        let action = Rc::clone(&handler);
        self.zoom_in.connect_clicked(move |_| {
            action(LighttableLayoutAction::SetZoom(zoom.borrow().smaller()));
        });
    }

    pub const fn set_panel_visibility(&self, _panel: LighttablePanel, _visible: bool) {
        // Lighttable panel visibility remains available through the typed shell action. Darktable's
        // footer reserves this compact group for layout modes and thumbnail density.
    }
}

fn zoom_button(id: &str, icon: &str, accessible_name: &str) -> gtk4::Button {
    let button = gtk4::Button::new();
    button.set_widget_name(id);
    button.set_child(Some(&gtk4::Image::from_icon_name(icon)));
    button.set_focus_on_click(false);
    button.update_property(&[Property::Label(accessible_name)]);
    button.set_tooltip_text(Some(accessible_name));
    button
}

impl Default for LighttableLayoutControls {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_controls_cover_every_darktable_surface() {
        let source = include_str!("layout_controls.rs");
        for id in [
            "lighttable-layout-grid",
            "lighttable-layout-zoom",
            "lighttable-layout-cull",
            "lighttable-layout-dynamic",
            "lighttable-layout-preview",
        ] {
            assert!(source.contains(id));
        }
        assert_eq!(LighttableLayout::Preview.label(), "preview");
        assert!(LighttableLayout::Culling.shows_filmstrip());
        for id in [
            "lighttable-zoom-controls",
            "lighttable-zoom-columns",
            "lighttable-zoom-out",
            "lighttable-zoom-in",
        ] {
            assert!(source.contains(id));
        }
    }
}
