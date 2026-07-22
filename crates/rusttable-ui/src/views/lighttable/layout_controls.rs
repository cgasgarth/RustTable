//! Darktable lighttable surface selectors.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::interaction::LighttableLayout;
use crate::gui::{ThemeRole, apply_theme_role};

/// GTK controls for switching between Darktable's lighttable surfaces.
#[derive(Clone)]
pub struct LighttableLayoutControls {
    root: gtk4::Box,
    buttons: Rc<Vec<(LighttableLayout, gtk4::ToggleButton)>>,
    panel_buttons: Rc<Vec<(LighttablePanel, gtk4::ToggleButton)>>,
    projecting: Rc<Cell<bool>>,
    layout: Rc<RefCell<LighttableLayout>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LighttablePanel {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LighttableLayoutAction {
    SetLayout(LighttableLayout),
    SetPanelVisibility {
        panel: LighttablePanel,
        visible: bool,
    },
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
                "zoom-in-symbolic",
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
                "view-refresh-symbolic",
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
                root.append(&button);
                (layout, button)
            })
            .collect::<Vec<_>>();
        let panel_separator = gtk4::Separator::new(gtk4::Orientation::Vertical);
        panel_separator.set_widget_name("lighttable-layout-separator-panels");
        panel_separator.add_css_class("dt_toolbar_separator");
        root.append(&panel_separator);
        let panel_buttons = [
            (LighttablePanel::Left, "left", "Show left panel"),
            (LighttablePanel::Right, "right", "Show right panel"),
        ]
        .into_iter()
        .map(|(panel, suffix, accessible_name)| {
            let button = gtk4::ToggleButton::new();
            button.set_child(Some(&gtk4::Image::from_icon_name(match panel {
                LighttablePanel::Left => "view-sidebar-start-symbolic",
                LighttablePanel::Right => "view-sidebar-end-symbolic",
            })));
            button.set_widget_name(&format!("lighttable-panel-{suffix}"));
            button.set_focus_on_click(false);
            button.set_active(true);
            button.set_accessible_role(gtk4::AccessibleRole::Button);
            button.update_property(&[Property::Label(accessible_name)]);
            button.set_tooltip_text(Some(accessible_name));
            root.append(&button);
            (panel, button)
        })
        .collect::<Vec<_>>();
        let controls = Self {
            root,
            buttons: Rc::new(buttons),
            panel_buttons: Rc::new(panel_buttons),
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
        for (panel, button) in self.panel_buttons.iter() {
            let handler = Rc::clone(&handler);
            let panel = *panel;
            let guard = Rc::clone(&guard);
            button.connect_toggled(move |button| {
                if !guard.get() {
                    handler(LighttableLayoutAction::SetPanelVisibility {
                        panel,
                        visible: button.is_active(),
                    });
                }
            });
        }
    }

    pub fn set_panel_visibility(&self, panel: LighttablePanel, visible: bool) {
        self.projecting.set(true);
        if let Some((_, button)) = self
            .panel_buttons
            .iter()
            .find(|(candidate, _)| *candidate == panel)
        {
            button.set_active(visible);
        }
        self.projecting.set(false);
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
        for id in ["lighttable-panel-left", "lighttable-panel-right"] {
            assert!(source.contains(id));
        }
        assert!(source.contains("lighttable-layout-separator-panels"));
    }
}
