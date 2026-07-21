//! Darktable lighttable surface selectors.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::{LighttableLayout, ThemeRole, apply_theme_role};

/// GTK controls for switching between Darktable's lighttable surfaces.
#[derive(Clone)]
pub struct LighttableLayoutControls {
    root: gtk4::Box,
    buttons: Rc<Vec<(LighttableLayout, gtk4::ToggleButton)>>,
    projecting: Rc<Cell<bool>>,
    layout: Rc<RefCell<LighttableLayout>>,
}

impl LighttableLayoutControls {
    #[must_use]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 1);
        root.set_widget_name("lighttable-layout-controls");
        root.set_accessible_role(gtk4::AccessibleRole::Toolbar);
        root.update_property(&[Property::Label("Lighttable layout")]);
        apply_theme_role(&root, ThemeRole::Toolbar);

        let layouts = [
            (LighttableLayout::FileManager, "grid", "File manager grid"),
            (LighttableLayout::Zoomable, "zoom", "Zoomable grid"),
            (LighttableLayout::Culling, "cull", "Culling view"),
            (
                LighttableLayout::CullingDynamic,
                "dynamic",
                "Dynamic culling view",
            ),
            (LighttableLayout::Preview, "preview", "Full preview"),
        ];
        let buttons = layouts
            .into_iter()
            .map(|(layout, suffix, accessible_name)| {
                let button = gtk4::ToggleButton::with_label(layout.label());
                button.set_widget_name(&format!("lighttable-layout-{suffix}"));
                button.set_focus_on_click(false);
                button.set_accessible_role(gtk4::AccessibleRole::Radio);
                button.update_property(&[Property::Label(accessible_name)]);
                button.set_tooltip_text(Some(accessible_name));
                root.append(&button);
                (layout, button)
            })
            .collect::<Vec<_>>();
        let controls = Self {
            root,
            buttons: Rc::new(buttons),
            projecting: Rc::new(Cell::new(false)),
            layout: Rc::new(RefCell::new(LighttableLayout::default())),
        };
        controls.set_layout(LighttableLayout::default());
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

    pub fn connect_layout<F>(&self, handler: F)
    where
        F: Fn(LighttableLayout) + 'static,
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
                handler(selected);
            });
        }
    }
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
        let source = include_str!("lighttable_layout_controls.rs");
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
    }
}
