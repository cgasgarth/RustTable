//! Darkroom rail and panel widget composition.

#![allow(clippy::too_many_arguments)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::Revision;

use super::super::darkroom_modules::{
    DarkroomModuleActionHandler, DarkroomModuleSide, DarkroomModulesViewModel,
    build_module_column_with_filter,
};
use super::{
    DARKROOM_GEOMETRY, DarkroomModuleGroup, DarkroomModuleGroupHandler, DarkroomRailStatus,
    chrome_toggle,
};
use super::{ExposurePanel, RawDenoisePanel, RgbDenoisePanel, ThemeRole, apply_theme_role};
use crate::presentation::{
    DarkroomHistoryViewModel, DarkroomImageInformationViewModel, DarkroomPanelProjection,
    DarkroomPanelTarget, DarkroomSnapshotsViewModel, PhotoDetailViewModel, build_history_panel,
    build_image_information_panel, build_snapshots_panel,
};

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
    let snapshots_projection = DarkroomPanelProjection::<DarkroomSnapshotsViewModel>::empty();
    let history_projection = DarkroomPanelProjection::<DarkroomHistoryViewModel>::empty();
    let image_projection = DarkroomPanelProjection::<DarkroomImageInformationViewModel>::empty();
    let snapshots = build_snapshots_panel(&snapshots_projection, None);
    let history = build_history_panel(&history_projection, None);
    let image_information = build_image_information_panel(&image_projection);
    let snapshots_body = projection_body(&snapshots);
    let history_body = projection_body(&history);
    let image_information_body = projection_body(&image_information);
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
    scroll.set_widget_name("darkroom-left-module-scroll");
    scroll.set_can_focus(true);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_propagate_natural_height(false);
    panel.append(&scroll);
    (
        panel,
        controller_modules,
        DarkroomRailStatus {
            navigation: navigation_state,
            snapshots_body,
            history_body,
            image_information_body,
        },
    )
}

fn projection_body(expander: &gtk4::Expander) -> gtk4::Box {
    let child = expander.child().expect("panel projection has a body");
    child
        .downcast::<gtk4::Box>()
        .expect("panel projection body is a GTK box")
}

pub(super) fn right_panel(width: i32) -> super::DarkroomPanelBuild {
    let panel = rail(
        "darkroom-right-panel",
        width,
        "Darkroom processing module rail",
    );
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

    let histogram = histogram();
    panel.append(&histogram);

    let modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    modules.set_widget_name("darkroom-right-modules");
    let exposure = ExposurePanel::new();
    let rgb_denoise = RgbDenoisePanel::new();
    let raw_denoise = RawDenoisePanel::new();
    modules.append(exposure.widget());
    modules.append(rgb_denoise.widget());
    modules.append(raw_denoise.widget());
    let controller_modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    controller_modules.set_widget_name("darkroom-right-controller-modules");
    modules.append(&controller_modules);
    let scroll = gtk4::ScrolledWindow::builder()
        .child(&modules)
        .hexpand(true)
        .vexpand(true)
        .build();
    scroll.set_widget_name("darkroom-right-module-scroll");
    scroll.set_can_focus(true);
    scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scroll.set_propagate_natural_height(false);
    panel.append(&scroll);
    (
        panel,
        controller_modules,
        exposure,
        rgb_denoise,
        raw_denoise,
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
    rgb_denoise: &RgbDenoisePanel,
    raw_denoise: &RawDenoisePanel,
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
    right_modules.append(rgb_denoise.widget());
    right_modules.append(raw_denoise.widget());
    let filtered_right = modules
        .right_modules()
        .filter(|module| module.id() != "exposure")
        .filter(|module| group.matches(module));
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
        ("loading", "calculating histogram…"),
        ("failure", "histogram unavailable for this preview"),
        ("stale", "preview changed; waiting for current histogram"),
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
            DarkroomModuleGroup::Basic,
            "group-basic",
            "B",
            "Basic modules",
        ),
        (DarkroomModuleGroup::Tone, "group-tone", "T", "Tone modules"),
        (
            DarkroomModuleGroup::Color,
            "group-color",
            "C",
            "Color modules",
        ),
        (
            DarkroomModuleGroup::Correct,
            "group-correct",
            "⌕",
            "Corrective modules",
        ),
        (
            DarkroomModuleGroup::Effects,
            "group-effects",
            "E",
            "Effect modules",
        ),
        (
            DarkroomModuleGroup::Grading,
            "group-grading",
            "◐",
            "Grading modules",
        ),
        (
            DarkroomModuleGroup::Technical,
            "group-technical",
            "⚙",
            "Technical modules",
        ),
        (
            DarkroomModuleGroup::Deprecated,
            "group-deprecated",
            "!",
            "Deprecated compatibility modules",
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

impl DarkroomRailStatus {
    pub(super) fn set_snapshots_projection(
        &self,
        projection: &DarkroomPanelProjection<DarkroomSnapshotsViewModel>,
        handler: Option<crate::presentation::DarkroomPanelActionHandler>,
    ) {
        replace_projection(
            &self.snapshots_body,
            &build_snapshots_panel(projection, handler),
        );
    }

    pub(super) fn set_history_projection(
        &self,
        projection: &DarkroomPanelProjection<DarkroomHistoryViewModel>,
        handler: Option<crate::presentation::DarkroomPanelActionHandler>,
    ) {
        replace_projection(
            &self.history_body,
            &build_history_panel(projection, handler),
        );
    }

    pub(super) fn set_detail(&self, detail: &PhotoDetailViewModel, target: DarkroomPanelTarget) {
        let information = DarkroomImageInformationViewModel::new(
            detail.title().as_str(),
            detail.facts().cloned().collect(),
            vec!["GPS coordinates".to_owned(), "copyright".to_owned()],
        )
        .expect("photo detail facts are already validated");
        let projection = DarkroomPanelProjection::ready(target, Revision::ZERO, information);
        replace_projection(
            &self.image_information_body,
            &build_image_information_panel(&projection),
        );
    }

    pub(super) fn clear_detail(&self) {
        let snapshots = DarkroomPanelProjection::<DarkroomSnapshotsViewModel>::empty();
        let history = DarkroomPanelProjection::<DarkroomHistoryViewModel>::empty();
        let image = DarkroomPanelProjection::<DarkroomImageInformationViewModel>::empty();
        replace_projection(
            &self.snapshots_body,
            &build_snapshots_panel(&snapshots, None),
        );
        replace_projection(&self.history_body, &build_history_panel(&history, None));
        replace_projection(
            &self.image_information_body,
            &build_image_information_panel(&image),
        );
    }
}

fn replace_projection(slot: &gtk4::Box, expander: &gtk4::Expander) {
    let body = projection_body(expander);
    while let Some(child) = slot.first_child() {
        child.unparent();
    }
    while let Some(child) = body.first_child() {
        child.unparent();
        slot.append(&child);
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
