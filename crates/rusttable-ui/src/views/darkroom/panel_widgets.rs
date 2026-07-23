//! Darkroom rail and panel widget composition.

#![allow(clippy::too_many_arguments)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::Revision;

use super::{
    DARKROOM_GEOMETRY, DarkroomModuleGroup, DarkroomModuleGroupHandler, DarkroomRailStatus,
    chrome_toggle,
};
use super::{ExposurePanel, RawDenoisePanel, RgbDenoisePanel, ThemeRole, apply_theme_role};
use crate::gui::DARKTABLE_UI_TOKENS;
use crate::gui::darktable_components::{
    RAIL_SCROLLBAR_RESERVE, module_expander as shared_module_expander, module_title,
    rail as shared_rail, rail_scroll as shared_rail_scroll,
};
use crate::iop::modules::{
    DarkroomModuleActionHandler, DarkroomModuleSide, DarkroomModulesViewModel,
    build_module_column_with_filter, build_module_column_without_empty,
};
use crate::presentation::{
    DarkroomHistoryViewModel, DarkroomImageInformationViewModel, DarkroomPanelProjection,
    DarkroomPanelTarget, DarkroomSnapshotsViewModel, PhotoDetailViewModel, Rgba8PreviewMetadata,
    build_history_panel, build_image_information_panel, build_snapshots_panel,
};
use crate::viewport_presentation::{DarkroomViewportState, NavigationCrop};
use crate::widgets::thumbnail::ThumbnailSurface;
use crate::{MaskManagerPanel, MultiscaleRetouchPanel};

const DENOISE_MODULE_GROUPS: &[DarkroomModuleGroup] =
    &[DarkroomModuleGroup::Correct, DarkroomModuleGroup::Technical];
const RETOUCH_MODULE_GROUPS: &[DarkroomModuleGroup] =
    &[DarkroomModuleGroup::Correct, DarkroomModuleGroup::Effects];

const NAVIGATION_MIN_HEIGHT: i32 = 100;
const NAVIGATION_MAX_HEIGHT: i32 = 300;
const NAVIGATION_ASPECT_WIDTH: i32 = 3;
const NAVIGATION_ASPECT_HEIGHT: i32 = 2;
const SNAPSHOTS_DEFAULT_HEIGHT: i32 = 200;
const HISTORY_DEFAULT_HEIGHT: i32 = 1_000;
const IMAGE_INFORMATION_DEFAULT_HEIGHT: i32 = 1_000;
const RESIZE_HANDLE_HEIGHT: i32 = 6;

#[derive(Clone)]
pub(super) struct ImplementedModulePanel {
    id: &'static str,
    title: &'static str,
    side: DarkroomModuleSide,
    groups: &'static [DarkroomModuleGroup],
    widget: gtk4::Widget,
}

impl ImplementedModulePanel {
    fn existing(
        id: &'static str,
        title: &'static str,
        side: DarkroomModuleSide,
        groups: &'static [DarkroomModuleGroup],
        widget: &impl IsA<gtk4::Widget>,
    ) -> Self {
        Self {
            id,
            title,
            side,
            groups,
            widget: widget.clone().upcast(),
        }
    }

    fn wrapped(
        id: &'static str,
        title: &'static str,
        side: DarkroomModuleSide,
        groups: &'static [DarkroomModuleGroup],
        child: &impl IsA<gtk4::Widget>,
    ) -> Self {
        let expander = shared_module_expander(id, title, false, Some(child));
        expander.set_label_widget(Some(&module_title(id, title)));
        Self::existing(id, title, side, groups, &expander)
    }

    fn is_visible(&self, group: DarkroomModuleGroup, query: &str) -> bool {
        implemented_module_matches(
            self.groups,
            group,
            query,
            crate::gtk_shell::darkroom_modules::search_matches(query, self.title, self.id, &[]),
        )
    }
}

fn implemented_module_matches(
    groups: &[DarkroomModuleGroup],
    selected: DarkroomModuleGroup,
    query: &str,
    search_matches: bool,
) -> bool {
    if !query.trim().is_empty() {
        return search_matches;
    }
    selected == DarkroomModuleGroup::Active || groups.contains(&selected)
}

pub(super) fn left_panel(width: i32) -> (gtk4::Box, gtk4::Box, DarkroomRailStatus) {
    let panel = rail("darkroom-left-panel", width, "Darkroom left module rail");
    let modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    modules.set_widget_name("darkroom-left-modules");
    let (navigation, navigation_state) = navigation_module(width);
    let snapshots_projection = DarkroomPanelProjection::<DarkroomSnapshotsViewModel>::empty();
    let history_projection = DarkroomPanelProjection::<DarkroomHistoryViewModel>::empty();
    let image_projection = DarkroomPanelProjection::<DarkroomImageInformationViewModel>::empty();
    let snapshots = build_snapshots_panel(&snapshots_projection, None);
    let history = build_history_panel(&history_projection, None);
    let image_information = build_image_information_panel(&image_projection);
    let snapshots_body = make_projection_resizable(
        &snapshots,
        "darkroom-snapshots-list",
        SNAPSHOTS_DEFAULT_HEIGHT,
    );
    let history_body =
        make_projection_resizable(&history, "darkroom-history-list", HISTORY_DEFAULT_HEIGHT);
    let image_information_body = make_projection_resizable(
        &image_information,
        "darkroom-image-information-list",
        IMAGE_INFORMATION_DEFAULT_HEIGHT,
    );
    modules.append(&navigation);
    modules.append(&image_information);
    modules.append(&history);
    modules.append(&snapshots);
    let controller_modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    controller_modules.set_widget_name("darkroom-left-controller-modules");
    modules.append(&controller_modules);
    let scroll = rail_scroll(&modules, width, "darkroom-left-module-scroll");
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

fn make_projection_resizable(
    expander: &gtk4::Expander,
    id: &str,
    default_height: i32,
) -> gtk4::Box {
    let source = projection_body(expander);
    let slot = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    while let Some(child) = source.first_child() {
        child.unparent();
        slot.append(&child);
    }
    let scroll = gtk4::ScrolledWindow::builder()
        .child(&slot)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .max_content_height(default_height)
        .propagate_natural_height(true)
        .overlay_scrolling(false)
        .build();
    scroll.set_widget_name(id);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.append(&scroll);
    let scroll_for_resize = scroll.clone();
    content.append(&resize_handle(default_height, 1, 1_000, move |height| {
        scroll_for_resize.set_max_content_height(height);
    }));
    expander.set_child(Some(&content));
    slot
}

#[allow(clippy::too_many_lines)]
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

    let search = gtk4::SearchEntry::new();
    search.set_widget_name("darkroom-module-search");
    search.set_placeholder_text(Some("search modules by name or tag"));
    search.set_accessible_role(gtk4::AccessibleRole::SearchBox);
    search.update_property(&[Property::Label("Search darkroom modules")]);
    search.set_width_request(0);
    search.set_hexpand(true);
    add_group_buttons(&groups, &search, &module_group, &module_group_handler);

    // Darktable places its live scope at the top of the processing rail, with
    // module grouping and search immediately below it.
    let histogram = histogram();
    panel.append(&histogram);
    let groups_scroll = gtk4::ScrolledWindow::builder()
        .child(&groups)
        .hscrollbar_policy(gtk4::PolicyType::Automatic)
        .vscrollbar_policy(gtk4::PolicyType::Never)
        .propagate_natural_width(false)
        .hexpand(true)
        .vexpand(false)
        .build();
    groups_scroll.set_widget_name("darkroom-module-groups-scroll");
    groups_scroll.set_height_request(DARKTABLE_UI_TOKENS.controls.toolbar_height);
    groups_scroll.set_min_content_width(width.saturating_sub(RAIL_SCROLLBAR_RESERVE));
    groups_scroll.set_propagate_natural_width(false);
    panel.append(&groups_scroll);

    panel.append(&search);

    let modules = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    modules.set_widget_name("darkroom-right-modules");
    modules.set_width_request(0);
    modules.set_hexpand(true);
    let exposure = ExposurePanel::new();
    let rgb_denoise = RgbDenoisePanel::new();
    let raw_denoise = RawDenoisePanel::new();
    let mask_manager = MaskManagerPanel::new();
    let multiscale_retouch = MultiscaleRetouchPanel::new();
    exposure.widget().set_expanded(false);
    let implemented_modules = vec![
        ImplementedModulePanel::existing(
            "exposure",
            "exposure",
            DarkroomModuleSide::Right,
            &[DarkroomModuleGroup::Basic],
            exposure.widget(),
        ),
        ImplementedModulePanel::wrapped(
            "rgb-denoise",
            "RGB AI denoise",
            DarkroomModuleSide::Right,
            DENOISE_MODULE_GROUPS,
            rgb_denoise.widget(),
        ),
        ImplementedModulePanel::wrapped(
            "raw-denoise",
            "RAW AI denoise",
            DarkroomModuleSide::Right,
            DENOISE_MODULE_GROUPS,
            raw_denoise.widget(),
        ),
        ImplementedModulePanel::wrapped(
            "mask-manager",
            "mask manager",
            DarkroomModuleSide::Left,
            &[],
            mask_manager.widget(),
        ),
        ImplementedModulePanel::wrapped(
            "multiscale-retouch",
            "multiscale retouch",
            DarkroomModuleSide::Right,
            RETOUCH_MODULE_GROUPS,
            multiscale_retouch.widget(),
        ),
    ];
    for module in implemented_modules
        .iter()
        .filter(|module| module.side == DarkroomModuleSide::Right)
    {
        modules.append(&module.widget);
    }
    let scroll = rail_scroll(&modules, width, "darkroom-right-module-scroll");
    panel.append(&scroll);
    (
        panel,
        modules,
        exposure,
        rgb_denoise,
        raw_denoise,
        mask_manager,
        multiscale_retouch,
        implemented_modules,
        histogram,
        search,
        module_group,
        module_group_handler,
    )
}

pub(super) fn render_typed_modules_into(
    left_modules: &gtk4::Box,
    right_modules: &gtk4::Box,
    implemented_modules: &[ImplementedModulePanel],
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
    for module in implemented_modules
        .iter()
        .filter(|module| module.side == DarkroomModuleSide::Left)
    {
        left_modules.append(&module.widget);
    }
    left_modules.append(&build_module_column_with_filter(
        modules.left_modules(),
        DarkroomModuleSide::Left,
        query,
        action_handler.as_ref(),
    ));

    clear_children(right_modules);
    let mut rendered = 0;
    for module in implemented_modules
        .iter()
        .filter(|module| module.side == DarkroomModuleSide::Right)
    {
        if module.is_visible(group, query) {
            right_modules.append(&module.widget);
            rendered += 1;
        }
    }
    // The active tab is the live controller-owned stack above. Registry descriptors describe
    // available operations, not active pipeline instances, and therefore belong only to their
    // truthful category tabs.
    let searching = !query.trim().is_empty();
    let typed = if group == DarkroomModuleGroup::Active && !searching {
        Vec::new()
    } else {
        modules
            .right_modules()
            .filter(|module| module.id() != "exposure")
            .filter(|module| searching || group.matches(module))
            .filter(|module| {
                crate::gtk_shell::darkroom_modules::module_matches_search(module, query)
            })
            .collect::<Vec<_>>()
    };
    rendered += typed.len();
    right_modules.append(&build_module_column_without_empty(
        typed.into_iter(),
        DarkroomModuleSide::Right,
        query,
        action_handler.as_ref(),
    ));
    if rendered == 0 {
        let empty = gtk4::Label::new(Some(if query.trim().is_empty() {
            "No modules available"
        } else {
            "No modules match this search"
        }));
        empty.set_widget_name("darkroom-module-search-empty");
        empty.set_halign(gtk4::Align::Start);
        empty.add_css_class("dim-label");
        empty.set_accessible_role(gtk4::AccessibleRole::Status);
        right_modules.append(&empty);
    }
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}

fn rail_scroll(child: &impl IsA<gtk4::Widget>, width: i32, id: &str) -> gtk4::ScrolledWindow {
    shared_rail_scroll(child, width, id)
}

fn histogram() -> gtk4::Stack {
    let histogram = gtk4::Stack::new();
    histogram.set_widget_name("darkroom-histogram");
    histogram.set_height_request(histogram_height_request());
    histogram.set_hexpand(true);
    histogram.set_halign(gtk4::Align::Fill);
    histogram.set_vexpand(false);
    histogram.set_valign(gtk4::Align::Start);
    histogram.set_accessible_role(gtk4::AccessibleRole::Img);
    histogram.update_property(&[Property::Label("Image histogram")]);
    histogram
}

fn histogram_height_request() -> i32 {
    i32::from(DARKROOM_GEOMETRY.histogram_min_height_px)
}

pub(super) fn add_group_buttons(
    groups: &gtk4::Box,
    search: &gtk4::SearchEntry,
    state: &Rc<Cell<DarkroomModuleGroup>>,
    handler: &Rc<RefCell<Option<DarkroomModuleGroupHandler>>>,
) {
    let buttons = [
        (
            DarkroomModuleGroup::Active,
            "group-active",
            "system-shutdown-symbolic",
            "Active modules",
        ),
        (
            DarkroomModuleGroup::Favorites,
            "group-favorites",
            "starred-symbolic",
            "Favorite modules",
        ),
        (
            DarkroomModuleGroup::Basic,
            "group-basic",
            "preferences-system-symbolic",
            "Basic modules",
        ),
        (
            DarkroomModuleGroup::Tone,
            "group-tone",
            "weather-clear-night-symbolic",
            "Tone modules",
        ),
        (
            DarkroomModuleGroup::Color,
            "group-color",
            "applications-graphics-symbolic",
            "Color modules",
        ),
        (
            DarkroomModuleGroup::Correct,
            "group-correct",
            "find-location-symbolic",
            "Corrective modules",
        ),
        (
            DarkroomModuleGroup::Effects,
            "group-effects",
            "applications-multimedia-symbolic",
            "Effect modules",
        ),
        (
            DarkroomModuleGroup::Grading,
            "group-grading",
            "color-select-symbolic",
            "Grading modules",
        ),
        (
            DarkroomModuleGroup::Technical,
            "group-technical",
            "applications-engineering-symbolic",
            "Technical modules",
        ),
        (
            DarkroomModuleGroup::Deprecated,
            "group-deprecated",
            "dialog-warning-symbolic",
            "Deprecated compatibility modules",
        ),
    ]
    .into_iter()
    .map(|(group, id, icon_name, label)| {
        let button = chrome_toggle(id, "", label);
        button.set_child(Some(&gtk4::Image::from_icon_name(icon_name)));
        button.set_active(group == DarkroomModuleGroup::Active);
        let state = Rc::clone(state);
        let handler = Rc::clone(handler);
        let search = search.clone();
        button.connect_toggled(move |button| {
            if !button.is_active() {
                return;
            }
            search.set_text("");
            state.set(group);
            if let Some(handler) = handler.borrow().as_ref() {
                handler(group);
            }
        });
        button
    })
    .collect::<Vec<_>>();
    if let Some(first) = buttons.first() {
        for button in buttons.iter().skip(1) {
            button.set_group(Some(first));
        }
    }
    for button in buttons {
        groups.append(&button);
    }
}

#[cfg(test)]
mod parity_tests {
    use super::{
        DENOISE_MODULE_GROUPS, DarkroomModuleGroup, RETOUCH_MODULE_GROUPS,
        histogram_height_request, implemented_module_matches,
    };

    #[test]
    fn implemented_module_groups_match_darktable_defaults() {
        assert_eq!(
            DENOISE_MODULE_GROUPS,
            &[DarkroomModuleGroup::Correct, DarkroomModuleGroup::Technical]
        );
        assert_eq!(
            RETOUCH_MODULE_GROUPS,
            &[DarkroomModuleGroup::Correct, DarkroomModuleGroup::Effects]
        );
        assert!(implemented_module_matches(
            DENOISE_MODULE_GROUPS,
            DarkroomModuleGroup::Correct,
            "",
            true
        ));
        assert!(implemented_module_matches(
            DENOISE_MODULE_GROUPS,
            DarkroomModuleGroup::Technical,
            "",
            true
        ));
        assert!(!implemented_module_matches(
            DENOISE_MODULE_GROUPS,
            DarkroomModuleGroup::Color,
            "",
            true
        ));
    }

    #[test]
    fn nonempty_search_overrides_the_selected_module_group() {
        assert!(implemented_module_matches(
            &[DarkroomModuleGroup::Correct],
            DarkroomModuleGroup::Color,
            "denoise",
            true
        ));
        assert!(!implemented_module_matches(
            &[DarkroomModuleGroup::Correct],
            DarkroomModuleGroup::Color,
            "exposure",
            false
        ));
    }

    #[test]
    fn histogram_uses_compact_darktable_height() {
        assert_eq!(histogram_height_request(), 120);
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
    shared_rail(id, width, accessible_name)
}

#[derive(Clone)]
pub(super) struct NavigationPreview {
    thumbnail: ThumbnailSurface,
    crop: Rc<Cell<NavigationCrop>>,
    indicator: gtk4::DrawingArea,
    zoom: gtk4::Label,
}

fn navigation_module(width: i32) -> (gtk4::Box, NavigationPreview) {
    let preview_width = width.saturating_sub(RAIL_SCROLLBAR_RESERVE).max(1);
    let preview_height = navigation_default_height(width);
    let thumbnail = ThumbnailSurface::new(
        "darkroom-navigation-preview",
        "Navigation preview for the selected photo",
        preview_width,
        preview_height,
    );
    thumbnail.widget().set_width_request(0);
    thumbnail.widget().set_height_request(preview_height);
    thumbnail.widget().set_hexpand(true);
    thumbnail.set_unavailable();
    let crop = Rc::new(Cell::new(
        DarkroomViewportState::default().navigation_crop(),
    ));
    let indicator = gtk4::DrawingArea::new();
    indicator.set_widget_name("darkroom-navigation-crop");
    indicator.set_can_target(false);
    indicator.set_visible(false);
    let crop_for_draw = Rc::clone(&crop);
    indicator.set_draw_func(move |_, context, width, height| {
        draw_navigation_crop(context, width, height, crop_for_draw.get());
    });
    let surface = gtk4::Overlay::new();
    surface.set_child(Some(thumbnail.widget()));
    surface.add_overlay(&indicator);
    let zoom = gtk4::Label::new(Some("fit"));
    zoom.set_widget_name("darkroom-navigation-zoom");
    zoom.set_halign(gtk4::Align::End);
    zoom.set_valign(gtk4::Align::End);
    zoom.add_css_class("dim-label");
    let thumbnail_for_resize = thumbnail.widget().clone();
    let resize = resize_handle(
        preview_height,
        NAVIGATION_MIN_HEIGHT,
        NAVIGATION_MAX_HEIGHT,
        move |height| thumbnail_for_resize.set_height_request(height),
    );
    resize.set_valign(gtk4::Align::End);
    surface.add_overlay(&resize);
    surface.add_overlay(&zoom);
    let module = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    module.set_widget_name("darkroom-navigation");
    module.set_focusable(true);
    module.add_css_class("dt_module_expander");
    module.append(&surface);
    module.update_property(&[Property::Label("navigation")]);
    apply_theme_role(&module, ThemeRole::ModuleGroup);
    (
        module,
        NavigationPreview {
            thumbnail,
            crop,
            indicator,
            zoom,
        },
    )
}

fn navigation_default_height(width: i32) -> i32 {
    let preview_width = width.saturating_sub(RAIL_SCROLLBAR_RESERVE).max(1);
    clamp_resize_height(
        preview_width.saturating_mul(NAVIGATION_ASPECT_HEIGHT) / NAVIGATION_ASPECT_WIDTH,
        NAVIGATION_MIN_HEIGHT,
        NAVIGATION_MAX_HEIGHT,
    )
}

fn resize_handle(
    initial_height: i32,
    minimum_height: i32,
    maximum_height: i32,
    apply_height: impl Fn(i32) + 'static,
) -> gtk4::DrawingArea {
    let handle = gtk4::DrawingArea::new();
    handle.set_widget_name("darkroom-module-resize-handle");
    handle.set_height_request(RESIZE_HANDLE_HEIGHT);
    handle.set_hexpand(true);
    handle.set_cursor_from_name(Some("ns-resize"));
    handle.set_draw_func(|_, context, width, height| {
        context.set_source_rgba(1.0, 1.0, 1.0, 0.35);
        context.set_line_width(1.0);
        context.move_to(f64::from(width) * 0.375, f64::from(height) / 2.0);
        context.line_to(f64::from(width) * 0.625, f64::from(height) / 2.0);
        let _ = context.stroke();
    });
    let current_height = Rc::new(Cell::new(initial_height));
    let start_height = Rc::new(Cell::new(initial_height));
    let drag = gtk4::GestureDrag::new();
    let start_for_begin = Rc::clone(&start_height);
    let current_for_begin = Rc::clone(&current_height);
    drag.connect_drag_begin(move |_, _, _| start_for_begin.set(current_for_begin.get()));
    let current_for_update = Rc::clone(&current_height);
    drag.connect_drag_update(move |_, _, offset_y| {
        #[allow(clippy::cast_possible_truncation)]
        let rounded_offset = offset_y.round() as i32;
        let requested = start_height.get().saturating_add(rounded_offset);
        let height = clamp_resize_height(requested, minimum_height, maximum_height);
        current_for_update.set(height);
        apply_height(height);
    });
    handle.add_controller(drag);
    handle
}

const fn clamp_resize_height(requested: i32, minimum: i32, maximum: i32) -> i32 {
    if requested < minimum {
        minimum
    } else if requested > maximum {
        maximum
    } else {
        requested
    }
}

fn draw_navigation_crop(
    context: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    crop: NavigationCrop,
) {
    let width = f64::from(width.max(1));
    let height = f64::from(height.max(1));
    let x = width * f64::from(crop.x_milli()) / 1_000.0;
    let y = height * f64::from(crop.y_milli()) / 1_000.0;
    let crop_width = width * f64::from(crop.width_milli()) / 1_000.0;
    let crop_height = height * f64::from(crop.height_milli()) / 1_000.0;
    context.set_source_rgba(0.0, 0.0, 0.0, 0.48);
    context.rectangle(0.0, 0.0, width, y);
    context.rectangle(0.0, y + crop_height, width, height - y - crop_height);
    context.rectangle(0.0, y, x, crop_height);
    context.rectangle(x + crop_width, y, width - x - crop_width, crop_height);
    let _ = context.fill();
    context.set_line_width(3.0);
    context.set_source_rgb(0.0, 0.0, 0.0);
    context.rectangle(
        x + 1.5,
        y + 1.5,
        (crop_width - 3.0).max(1.0),
        (crop_height - 3.0).max(1.0),
    );
    let _ = context.stroke();
    context.set_line_width(1.0);
    context.set_source_rgb(1.0, 1.0, 1.0);
    context.rectangle(
        x + 1.5,
        y + 1.5,
        (crop_width - 3.0).max(1.0),
        (crop_height - 3.0).max(1.0),
    );
    let _ = context.stroke();
}

impl NavigationPreview {
    pub(super) fn set_rgba8(
        &self,
        metadata: &Rgba8PreviewMetadata,
    ) -> Result<(), crate::widgets::preview::PhotoPreviewTextureError> {
        self.thumbnail.set_rgba8(metadata)?;
        self.indicator.set_visible(true);
        self.indicator.queue_draw();
        Ok(())
    }

    pub(super) fn set_loading(&self) {
        self.thumbnail.set_loading();
        self.indicator.set_visible(false);
    }

    pub(super) fn set_unavailable(&self) {
        self.thumbnail.set_unavailable();
        self.indicator.set_visible(false);
    }

    pub(super) fn set_failed(&self) {
        self.thumbnail.set_failed();
        self.indicator.set_visible(false);
    }

    pub(super) fn sync_viewport(&self, state: DarkroomViewportState) {
        self.crop.set(state.navigation_crop());
        self.zoom.set_text(state.zoom().label());
        self.indicator.queue_draw();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HISTORY_DEFAULT_HEIGHT, IMAGE_INFORMATION_DEFAULT_HEIGHT, NAVIGATION_MAX_HEIGHT,
        NAVIGATION_MIN_HEIGHT, SNAPSHOTS_DEFAULT_HEIGHT, clamp_resize_height,
        navigation_default_height,
    };
    use crate::presentation::DARKROOM_LEFT_PANEL_ORDER;

    #[test]
    fn left_rail_follows_darktable_container_positions() {
        assert_eq!(
            DARKROOM_LEFT_PANEL_ORDER,
            [
                "darkroom-navigation",
                "darkroom-image-information",
                "darkroom-history",
                "darkroom-snapshots",
            ]
        );
    }

    #[test]
    fn navigation_height_matches_darktable_config_and_clamps_resizing() {
        assert_eq!(navigation_default_height(180), 114);
        assert_eq!(NAVIGATION_MIN_HEIGHT, 100);
        assert_eq!(NAVIGATION_MAX_HEIGHT, 300);
        assert_eq!(clamp_resize_height(40, 100, 300), 100);
        assert_eq!(clamp_resize_height(240, 100, 300), 240);
        assert_eq!(clamp_resize_height(900, 100, 300), 300);
    }

    #[test]
    fn resizable_list_defaults_match_darktable_config() {
        assert_eq!(SNAPSHOTS_DEFAULT_HEIGHT, 200);
        assert_eq!(HISTORY_DEFAULT_HEIGHT, 1_000);
        assert_eq!(IMAGE_INFORMATION_DEFAULT_HEIGHT, 1_000);
    }
}
