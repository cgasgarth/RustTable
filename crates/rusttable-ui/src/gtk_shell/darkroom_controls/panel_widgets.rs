//! Darkroom rail and panel widget composition.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::super::darkroom_modules::{
    DarkroomModuleActionHandler, DarkroomModuleSide, DarkroomModulesViewModel,
    build_module_column_with_filter,
};
use super::{
    DARKROOM_GEOMETRY, DarkroomModuleGroup, DarkroomModuleGroupHandler, DarkroomRailStatus,
    chrome_toggle,
};
use super::{ExposurePanel, ThemeRole, apply_theme_role};

pub(super) fn left_panel(width: i32) -> (gtk4::Box, gtk4::Box, DarkroomRailStatus) {
    let panel = rail("darkroom-left-panel", width, "Darkroom left module rail");
    let modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    modules.set_widget_name("darkroom-left-modules");
    let (navigation, navigation_state) = rail_module(
        "darkroom-navigation",
        "navigation",
        true,
        "select a photo to navigate",
    );
    let (snapshots, snapshots_state) = rail_module(
        "darkroom-snapshots",
        "snapshots",
        false,
        "select a photo to view snapshots",
    );
    let (history, history_state) = rail_module(
        "darkroom-history",
        "history",
        false,
        "select a photo to view edit history",
    );
    let (image_information, image_information_state) = rail_module(
        "darkroom-image-information",
        "image information",
        false,
        "image information unavailable",
    );
    for module in [navigation, snapshots, history, image_information] {
        modules.append(&module);
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
    (
        panel,
        controller_modules,
        DarkroomRailStatus {
            navigation: navigation_state,
            snapshots: snapshots_state,
            history: history_state,
            image_information: image_information_state,
        },
    )
}

pub(super) fn right_panel(width: i32) -> super::DarkroomPanelBuild {
    let panel = rail(
        "darkroom-right-panel",
        width,
        "Darkroom processing module rail",
    );
    let histogram = histogram();
    panel.append(&histogram);

    let groups = gtk4::Box::new(gtk4::Orientation::Horizontal, 1);
    groups.set_widget_name("darkroom-module-groups");
    groups.set_accessible_role(gtk4::AccessibleRole::Toolbar);
    groups.update_property(&[Property::Label("Processing module groups")]);
    let module_group = Rc::new(Cell::new(DarkroomModuleGroup::Active));
    let module_group_handler = Rc::new(RefCell::new(None));
    add_group_buttons(&groups, &module_group, &module_group_handler);
    panel.append(&groups);

    let search = gtk4::SearchEntry::new();
    search.set_widget_name("darkroom-module-search");
    search.set_placeholder_text(Some("search modules"));
    search.set_accessible_role(gtk4::AccessibleRole::SearchBox);
    search.update_property(&[Property::Label("Search darkroom modules")]);
    search.set_hexpand(true);
    panel.append(&search);

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
    (
        panel,
        controller_modules,
        exposure,
        histogram,
        search,
        module_group,
        module_group_handler,
    )
}

pub(super) fn render_typed_modules_into(
    left_modules: &gtk4::Box,
    right_modules: &gtk4::Box,
    exposure: &ExposurePanel,
    typed_modules: &Rc<RefCell<Option<DarkroomModulesViewModel>>>,
    action_handler: &Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    group: DarkroomModuleGroup,
    query: &str,
) {
    let Some(modules) = typed_modules.borrow().as_ref().cloned() else {
        return;
    };
    let action_handler = action_handler.borrow();
    clear_children(left_modules);
    left_modules.append(&build_module_column_with_filter(
        modules.left_modules(),
        DarkroomModuleSide::Left,
        query,
        action_handler.as_ref(),
    ));

    clear_children(right_modules);
    right_modules.append(exposure.widget());
    let filtered_right = modules
        .right_modules()
        .filter(|module| module.id() != "exposure" && group.matches_title(module.title()));
    right_modules.append(&build_module_column_with_filter(
        filtered_right,
        DarkroomModuleSide::Right,
        query,
        action_handler.as_ref(),
    ));
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}

fn histogram() -> gtk4::Stack {
    let histogram = gtk4::Stack::new();
    histogram.set_widget_name("darkroom-histogram");
    histogram.set_height_request(i32::from(DARKROOM_GEOMETRY.histogram_height_px));
    histogram.set_accessible_role(gtk4::AccessibleRole::Img);
    histogram.update_property(&[Property::Label("Image histogram")]);
    for (name, text) in [
        ("empty", "select a photo to show the histogram"),
        ("unavailable", "histogram unavailable for this preview"),
    ] {
        let state = gtk4::Label::new(Some(text));
        state.set_widget_name(&format!("darkroom-histogram-{name}"));
        state.set_halign(gtk4::Align::Center);
        state.set_valign(gtk4::Align::Center);
        state.add_css_class("dim-label");
        state.set_hexpand(true);
        state.set_vexpand(true);
        state.set_accessible_role(gtk4::AccessibleRole::Status);
        histogram.add_named(&state, Some(name));
    }
    histogram.set_visible_child_name("empty");
    histogram
}

pub(super) fn add_group_buttons(
    groups: &gtk4::Box,
    state: &Rc<Cell<DarkroomModuleGroup>>,
    handler: &Rc<RefCell<Option<DarkroomModuleGroupHandler>>>,
) {
    let guard = Rc::new(Cell::new(false));
    let buttons = [
        (
            DarkroomModuleGroup::Active,
            "group-active",
            "●",
            "Active modules",
        ),
        (
            DarkroomModuleGroup::Favorites,
            "group-favorites",
            "★",
            "Favorite modules",
        ),
        (
            DarkroomModuleGroup::Technical,
            "group-technical",
            "○",
            "Technical modules",
        ),
        (
            DarkroomModuleGroup::Grading,
            "group-grading",
            "◐",
            "Grading modules",
        ),
    ]
    .into_iter()
    .map(|(group, id, icon, label)| {
        let button = chrome_toggle(id, icon, label);
        button.set_active(group == DarkroomModuleGroup::Active);
        let state = Rc::clone(state);
        let handler = Rc::clone(handler);
        let guard_for_callback = Rc::clone(&guard);
        button.connect_toggled(move |button| {
            if guard_for_callback.get() {
                return;
            }
            if !button.is_active() {
                guard_for_callback.set(true);
                button.set_active(true);
                guard_for_callback.set(false);
                return;
            }
            state.set(group);
            if let Some(handler) = handler.borrow().as_ref() {
                handler(group);
            }
        });
        button
    })
    .collect::<Vec<_>>();
    for button in buttons {
        groups.append(&button);
    }
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

fn rail_module(
    id: &str,
    title: &str,
    initially_expanded: bool,
    state_text: &str,
) -> (gtk4::Expander, gtk4::Label) {
    let state = gtk4::Label::new(Some(state_text));
    state.set_widget_name(&format!("{id}-state"));
    state.set_halign(gtk4::Align::Start);
    state.add_css_class("dim-label");
    state.set_accessible_role(gtk4::AccessibleRole::Status);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.append(&state);
    let expander = gtk4::Expander::builder()
        .label(title)
        .expanded(initially_expanded)
        .child(&content)
        .build();
    expander.set_widget_name(id);
    expander.set_focusable(true);
    expander.update_property(&[Property::Label(title)]);
    apply_theme_role(&expander, ThemeRole::ModuleGroup);
    (expander, state)
}
