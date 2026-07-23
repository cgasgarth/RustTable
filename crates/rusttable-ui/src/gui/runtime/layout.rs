//! GTK4 layout composition helpers for the Darktable-shaped shell.

use gtk4::prelude::*;
use rusttable_i18n::{I18n, MessageArgs, MessageId};

use crate::ai_batch::AiBatchPanel;
use crate::camera::CameraPanel;
use crate::external_editor::ExternalEditorPanel;
use crate::import::ImportSessionPanel;

use crate::gui::darkroom_modules::DarkroomModuleGroup;
use crate::gui::darktable_components::{
    dropdown, module_expander as shared_module_expander, module_row, module_title, slider, switch,
};
use crate::gui::darktable_spec::{FILMSTRIP_ITEM_GAP_PX, FILMSTRIP_MAX_CHILDREN_PER_LINE};
use crate::gui::display_profile::DisplayProfileBanner;
use crate::gui::{
    DARKROOM_PANEL_WIDTHS, DARKTABLE_DESKTOP_SPEC, ExportPanel, LIGHTTABLE_COMPOSITION,
    LIGHTTABLE_PANEL_WIDTHS, LIGHTTABLE_RIGHT_MODULES, LighttableLayoutControls, LighttableToolbar,
    ModuleControlKind, ModulePanelViewModel, PanelSlot, ShellRegion, ThemeRole,
    WorkspacePanelWidths, WorkspaceRole, apply_theme_role, darkroom_window_layout,
};
use crate::views::lighttable::empty_collection_state;

#[derive(Clone)]
pub(super) struct WorkspaceEdgeControls {
    pub(super) left: gtk4::Button,
    pub(super) right: gtk4::Button,
    pub(super) top: gtk4::Button,
    pub(super) bottom: gtk4::Button,
}

pub(super) fn right_panel() -> (
    gtk4::Box,
    ExportPanel,
    ExternalEditorPanel,
    AiBatchPanel,
    CameraPanel,
    ImportSessionPanel,
) {
    let panel = panel_column(
        ShellRegion::RightPanel,
        i32::from(LIGHTTABLE_PANEL_WIDTHS.right_px),
    );
    apply_theme_role(&panel, ThemeRole::Panel);
    let export_panel = ExportPanel::new();
    let external_editor_panel = ExternalEditorPanel::new();
    let ai_batch_panel = AiBatchPanel::new();
    let camera_panel = CameraPanel::new();
    let import_session_panel = ImportSessionPanel::new();
    let center = panel_slot(PanelSlot::RightCenter);
    for module in &LIGHTTABLE_RIGHT_MODULES[..LIGHTTABLE_RIGHT_MODULES.len() - 1] {
        center.append(&module_group(module.widget_name, module.title, false));
    }
    center.append(export_panel.widget());
    // External editors, AI batch, tethering, and import-session controls are
    // services, not lighttable modules. Keep their controllers alive without
    // placing their unrelated surfaces in the collection rail.
    append_panel_slots(
        &panel,
        &panel_slot(PanelSlot::RightTop),
        &center,
        &panel_slot(PanelSlot::RightBottom),
    );
    (
        panel,
        export_panel,
        external_editor_panel,
        ai_batch_panel,
        camera_panel,
        import_session_panel,
    )
}

pub(super) fn mode_panel_stack(
    id: &str,
    lighttable: &impl IsA<gtk4::Widget>,
    darkroom: &impl IsA<gtk4::Widget>,
    initial: WorkspaceRole,
) -> gtk4::Stack {
    let stack = gtk4::Stack::new();
    stack.set_widget_name(id);
    stack.set_transition_type(gtk4::StackTransitionType::None);
    // Size the rail from the active workspace. The wider inactive lighttable
    // child otherwise shifts the darkroom child left inside a narrow Paned,
    // clipping its disclosure arrows and labels while leaving trailing actions.
    stack.set_hhomogeneous(false);
    let minimum_width = i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.minimum_px);
    // The Paned position owns the active workspace's configured width. Keep
    // the child request at the configured resize minimum so GTK can move the
    // handle below a module's natural width without under-allocating the
    // opposite child during height-for-width measurement.
    stack.set_size_request(minimum_width, -1);
    stack.set_hexpand(false);
    stack.set_vexpand(true);
    stack.set_halign(gtk4::Align::Fill);
    stack.set_valign(gtk4::Align::Fill);
    stack.add_css_class("dt_rail_stack");
    stack.add_named(lighttable, Some(WorkspaceRole::Lighttable.stack_name()));
    stack.add_named(darkroom, Some(WorkspaceRole::Darkroom.stack_name()));
    stack.set_visible_child_name(initial.stack_name());
    stack
}

pub(super) fn synchronize_panel_stacks(
    workspace: &gtk4::Stack,
    left_panel: &gtk4::Stack,
    right_panel: &gtk4::Stack,
) {
    let left_panel = left_panel.clone();
    let right_panel = right_panel.clone();
    workspace.connect_visible_child_name_notify(move |workspace| {
        let Some(name) = workspace.visible_child_name() else {
            return;
        };
        left_panel.set_visible_child_name(&name);
        right_panel.set_visible_child_name(&name);
    });
}

pub(super) fn desktop_body(
    workspace: &gtk4::Stack,
    lighttable_toolbar: &LighttableToolbar,
    left_panel: &gtk4::Stack,
    right_panel: &gtk4::Stack,
    i18n: &I18n,
    geometry_changed: &std::rc::Rc<dyn Fn()>,
) -> (gtk4::Box, gtk4::FlowBox, gtk4::Box) {
    let layout = DARKTABLE_DESKTOP_SPEC.layout;
    let panel_widths = WorkspacePanelWidthState::new();
    let center = central_workspace(workspace, lighttable_toolbar);
    let (filmstrip_root, filmstrip) = filmstrip(i18n);
    let center_column = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    center_column.set_hexpand(true);
    center_column.set_vexpand(true);
    center_column.append(&center);
    center_column.append(&filmstrip_root);
    let split = gtk4::Paned::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .start_child(left_panel)
        .end_child(&center_column)
        .hexpand(true)
        .vexpand(true)
        .resize_start_child(false)
        .shrink_start_child(false)
        .shrink_end_child(true)
        .wide_handle(true)
        .position(i32::from(panel_widths.active(workspace).left_px))
        .build();
    split.set_widget_name("desktop-left-split");
    split.connect_map({
        let workspace = workspace.clone();
        let panel_widths = panel_widths.clone();
        move |paned| paned.set_position(i32::from(panel_widths.active(&workspace).left_px))
    });
    connect_left_rail_constraints(&split);
    connect_panel_width_tracking(&split, workspace, &panel_widths, PanelSide::Left);
    connect_geometry_refresh(&split, std::rc::Rc::clone(geometry_changed));
    let workspace_with_right_panel = gtk4::Paned::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .start_child(&split)
        .end_child(right_panel)
        .hexpand(true)
        .vexpand(true)
        .resize_end_child(false)
        .shrink_start_child(true)
        .shrink_end_child(false)
        .wide_handle(true)
        .build();
    workspace_with_right_panel.set_widget_name("desktop-right-split");
    // The scroller may have a wider natural size at 12pt. Permit the Paned to
    // allocate the explicit rail token, then clamp every drag to the readable
    // minimum instead of letting natural width consume the center workspace.
    workspace_with_right_panel.set_shrink_end_child(true);
    workspace_with_right_panel.set_position(
        i32::from(layout.content_width_px(layout.window_width_px))
            .saturating_sub(paned_handle_minimum_width(&workspace_with_right_panel))
            .saturating_sub(i32::from(panel_widths.active(workspace).right_px)),
    );
    workspace_with_right_panel.connect_map({
        let workspace = workspace.clone();
        let panel_widths = panel_widths.clone();
        move |paned| {
            let paned = paned.clone();
            let workspace = workspace.clone();
            let panel_widths = panel_widths.clone();
            gtk4::glib::idle_add_local_once(move || {
                set_right_rail_width(&paned, i32::from(panel_widths.active(&workspace).right_px));
                panel_widths.enable_tracking();
            });
        }
    });
    workspace.connect_visible_child_name_notify({
        let left_split = split.clone();
        let right_split = workspace_with_right_panel.clone();
        let panel_widths = panel_widths.clone();
        move |workspace| {
            let widths = panel_widths.active(workspace);
            left_split.set_position(i32::from(widths.left_px));
            set_right_rail_width(&right_split, i32::from(widths.right_px));
        }
    });
    connect_geometry_refresh(
        &workspace_with_right_panel,
        std::rc::Rc::clone(geometry_changed),
    );
    connect_right_rail_constraints(&workspace_with_right_panel);
    connect_panel_width_tracking(
        &workspace_with_right_panel,
        workspace,
        &panel_widths,
        PanelSide::Right,
    );
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let outer_border = i32::from(layout.outer_border_px);
    content.set_margin_top(outer_border);
    // Darktable's filmstrip reaches the bottom window edge; retaining the
    // horizontal and top shell inset while adding it below the strip steals
    // seven pixels from every image viewport.
    content.set_margin_bottom(0);
    content.set_margin_start(outer_border);
    content.set_margin_end(outer_border);
    content.append(&workspace_with_right_panel);
    (content, filmstrip, filmstrip_root)
}

pub(super) fn workspace_frame(
    workspace: &impl IsA<gtk4::Widget>,
) -> (gtk4::Overlay, WorkspaceEdgeControls) {
    let controls = WorkspaceEdgeControls {
        left: edge_toggle(WorkspaceEdge::Left),
        right: edge_toggle(WorkspaceEdge::Right),
        top: edge_toggle(WorkspaceEdge::Top),
        bottom: edge_toggle(WorkspaceEdge::Bottom),
    };
    let overlay = gtk4::Overlay::new();
    overlay.set_widget_name("workspace-frame");
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);
    overlay.set_child(Some(workspace));
    overlay.add_overlay(&controls.left);
    overlay.add_overlay(&controls.right);
    overlay.add_overlay(&controls.top);
    overlay.add_overlay(&controls.bottom);
    (overlay, controls)
}

#[derive(Clone, Copy)]
enum WorkspaceEdge {
    Left,
    Right,
    Top,
    Bottom,
}

fn edge_toggle(edge: WorkspaceEdge) -> gtk4::Button {
    let layout = DARKTABLE_DESKTOP_SPEC.layout;
    let button = gtk4::Button::new();
    let (id, accessible_name, horizontal_alignment, vertical_alignment) = match edge {
        WorkspaceEdge::Left => (
            "workspace-left-edge-toggle",
            "Show or hide the left panel",
            gtk4::Align::Start,
            gtk4::Align::Center,
        ),
        WorkspaceEdge::Right => (
            "workspace-right-edge-toggle",
            "Show or hide the right panel",
            gtk4::Align::End,
            gtk4::Align::Center,
        ),
        WorkspaceEdge::Top => (
            "workspace-top-edge-toggle",
            "Show or hide the header",
            gtk4::Align::Center,
            gtk4::Align::Start,
        ),
        WorkspaceEdge::Bottom => (
            "workspace-bottom-edge-toggle",
            "Show or hide the filmstrip",
            gtk4::Align::Center,
            gtk4::Align::End,
        ),
    };
    button.set_widget_name(id);
    button.set_halign(horizontal_alignment);
    button.set_valign(vertical_alignment);
    let outer_border = i32::from(layout.outer_border_px);
    match edge {
        WorkspaceEdge::Left | WorkspaceEdge::Right => {
            button.set_size_request(outer_border, 28);
            button.add_css_class("dt_edge_toggle_vertical");
        }
        WorkspaceEdge::Top | WorkspaceEdge::Bottom => {
            button.set_size_request(28, outer_border);
            button.add_css_class("dt_edge_toggle_horizontal");
        }
    }
    button.set_focus_on_click(false);
    button.set_tooltip_text(Some(accessible_name));
    button.update_property(&[gtk4::accessible::Property::Label(accessible_name)]);
    button.add_css_class("dt_edge_toggle");
    let triangle = gtk4::DrawingArea::new();
    match edge {
        WorkspaceEdge::Left | WorkspaceEdge::Right => {
            triangle.set_content_width(4);
            triangle.set_content_height(10);
        }
        WorkspaceEdge::Top | WorkspaceEdge::Bottom => {
            triangle.set_content_width(10);
            triangle.set_content_height(4);
        }
    }
    triangle.set_can_target(false);
    triangle.set_halign(gtk4::Align::Center);
    triangle.set_valign(gtk4::Align::Center);
    triangle.set_draw_func(move |_, context, width, height| {
        let width = f64::from(width);
        let height = f64::from(height);
        match edge {
            WorkspaceEdge::Left => {
                context.move_to(0.0, 0.0);
                context.line_to(width, height / 2.0);
                context.line_to(0.0, height);
            }
            WorkspaceEdge::Right => {
                context.move_to(width, 0.0);
                context.line_to(0.0, height / 2.0);
                context.line_to(width, height);
            }
            WorkspaceEdge::Top => {
                context.move_to(0.0, height);
                context.line_to(width / 2.0, 0.0);
                context.line_to(width, height);
            }
            WorkspaceEdge::Bottom => {
                context.move_to(0.0, 0.0);
                context.line_to(width / 2.0, height);
                context.line_to(width, 0.0);
            }
        }
        context.close_path();
        context.set_source_rgb(0.78, 0.78, 0.78);
        let _ = context.fill();
    });
    button.set_child(Some(&triangle));
    button
}

fn connect_geometry_refresh(paned: &gtk4::Paned, refresh: std::rc::Rc<dyn Fn()>) {
    let schedule = geometry_refresh_scheduler(refresh);
    if let Some(child) = paned.start_child() {
        connect_allocation_refresh(&child, std::rc::Rc::clone(&schedule));
    }
    if let Some(child) = paned.end_child() {
        connect_allocation_refresh(&child, std::rc::Rc::clone(&schedule));
    }
    paned.connect_position_notify(move |paned| {
        paned.queue_allocate();
        schedule();
    });
}

fn connect_right_rail_constraints(paned: &gtk4::Paned) {
    let clamp = |paned: &gtk4::Paned| {
        let width = paned.allocated_width();
        if width <= 0 {
            return;
        }
        let layout = DARKTABLE_DESKTOP_SPEC.layout;
        let minimum = i32::from(layout.side_panel_widths.minimum_px);
        let configured_maximum = i32::from(layout.side_panel_widths.maximum_px);
        let (opposite_rail_width, inner_handle_width, center_minimum_width) =
            paned.start_child().and_downcast::<gtk4::Paned>().map_or(
                (minimum, 0, i32::from(layout.center_minimum_width_px)),
                |left_split| {
                    (
                        left_split.position(),
                        paned_handle_width(&left_split),
                        configured_center_minimum_width(),
                    )
                },
            );
        let available_maximum = width
            .saturating_sub(paned_handle_width(paned))
            .saturating_sub(inner_handle_width)
            .saturating_sub(center_minimum_width)
            .saturating_sub(opposite_rail_width);
        let maximum = configured_maximum.min(available_maximum).max(minimum);
        let rail_width = right_rail_width(paned).clamp(minimum, maximum);
        let position = right_rail_position(paned, rail_width);
        if paned.position() != position {
            paned.set_position(position);
        }
    };
    paned.connect_position_notify(move |paned| clamp(paned));
    paned.connect_notify_local(Some("width"), move |paned, _| clamp(paned));
}

fn connect_left_rail_constraints(paned: &gtk4::Paned) {
    let clamp = |paned: &gtk4::Paned| {
        let width = paned.allocated_width();
        if width <= 0 {
            return;
        }
        let layout = DARKTABLE_DESKTOP_SPEC.layout;
        let minimum = i32::from(layout.side_panel_widths.minimum_px);
        let configured_maximum = i32::from(layout.side_panel_widths.maximum_px);
        let available_maximum = width
            .saturating_sub(paned_handle_width(paned))
            .saturating_sub(configured_center_minimum_width())
            .max(minimum);
        let maximum = configured_maximum.min(available_maximum);
        let position = paned.position().clamp(minimum, maximum);
        if paned.position() != position {
            paned.set_position(position);
        }
    };
    paned.connect_position_notify(move |paned| clamp(paned));
    paned.connect_notify_local(Some("width"), move |paned, _| clamp(paned));
}

fn geometry_refresh_scheduler(refresh: std::rc::Rc<dyn Fn()>) -> std::rc::Rc<dyn Fn()> {
    let pending = std::rc::Rc::new(std::cell::Cell::new(false));
    std::rc::Rc::new(move || {
        if pending.replace(true) {
            return;
        }
        let pending = std::rc::Rc::clone(&pending);
        let refresh = std::rc::Rc::clone(&refresh);
        gtk4::glib::idle_add_local_once(move || {
            pending.set(false);
            refresh();
        });
    })
}

fn connect_allocation_refresh(widget: &impl IsA<gtk4::Widget>, schedule: std::rc::Rc<dyn Fn()>) {
    widget.connect_notify_local(Some("width"), {
        let schedule = std::rc::Rc::clone(&schedule);
        move |_, _| schedule()
    });
    widget.connect_notify_local(Some("height"), move |_, _| schedule());
}

#[derive(Clone, Copy)]
enum PanelSide {
    Left,
    Right,
}

#[derive(Clone)]
struct WorkspacePanelWidthState {
    lighttable: std::rc::Rc<std::cell::Cell<WorkspacePanelWidths>>,
    darkroom: std::rc::Rc<std::cell::Cell<WorkspacePanelWidths>>,
    tracking_enabled: std::rc::Rc<std::cell::Cell<bool>>,
}

impl WorkspacePanelWidthState {
    fn new() -> Self {
        Self {
            lighttable: std::rc::Rc::new(std::cell::Cell::new(LIGHTTABLE_PANEL_WIDTHS)),
            darkroom: std::rc::Rc::new(std::cell::Cell::new(DARKROOM_PANEL_WIDTHS)),
            tracking_enabled: std::rc::Rc::new(std::cell::Cell::new(false)),
        }
    }

    fn active(&self, workspace: &gtk4::Stack) -> WorkspacePanelWidths {
        self.for_workspace(active_workspace(workspace)).get()
    }

    fn for_workspace(
        &self,
        workspace: WorkspaceRole,
    ) -> &std::rc::Rc<std::cell::Cell<WorkspacePanelWidths>> {
        match workspace {
            WorkspaceRole::Lighttable => &self.lighttable,
            WorkspaceRole::Darkroom => &self.darkroom,
        }
    }

    fn enable_tracking(&self) {
        self.tracking_enabled.set(true);
    }
}

fn connect_panel_width_tracking(
    paned: &gtk4::Paned,
    workspace: &gtk4::Stack,
    widths: &WorkspacePanelWidthState,
    side: PanelSide,
) {
    let workspace = workspace.clone();
    let widths = widths.clone();
    paned.connect_position_notify(move |paned| {
        if !widths.tracking_enabled.get() || paned.allocated_width() <= 0 {
            return;
        }
        let state = widths.for_workspace(active_workspace(&workspace));
        let mut active_widths = state.get();
        let width = match side {
            PanelSide::Left => paned.position(),
            PanelSide::Right => right_rail_width(paned),
        };
        let Ok(width) = u16::try_from(width) else {
            return;
        };
        match side {
            PanelSide::Left => active_widths.left_px = width,
            PanelSide::Right => active_widths.right_px = width,
        }
        state.set(active_widths);
    });
}

fn paned_handle_width(paned: &gtk4::Paned) -> i32 {
    let children_width = paned
        .start_child()
        .map_or(0, |child| child.allocated_width())
        .saturating_add(paned.end_child().map_or(0, |child| child.allocated_width()));
    let allocated_width = paned.allocated_width().saturating_sub(children_width);
    if allocated_width > 0 && allocated_width <= 64 {
        allocated_width
    } else {
        paned_handle_minimum_width(paned)
    }
}

fn paned_handle_minimum_width(paned: &gtk4::Paned) -> i32 {
    let start = paned.start_child();
    let end = paned.end_child();
    let mut child = paned.first_child();
    while let Some(widget) = child {
        let is_start = start.as_ref().is_some_and(|start| start == &widget);
        let is_end = end.as_ref().is_some_and(|end| end == &widget);
        if !is_start && !is_end {
            return widget.measure(gtk4::Orientation::Horizontal, -1).0;
        }
        child = widget.next_sibling();
    }
    0
}

fn configured_center_minimum_width() -> i32 {
    i32::from(DARKTABLE_DESKTOP_SPEC.layout.center_minimum_width_px)
}

fn right_rail_width(paned: &gtk4::Paned) -> i32 {
    paned
        .allocated_width()
        .saturating_sub(paned_handle_width(paned))
        .saturating_sub(paned.position())
}

fn right_rail_position(paned: &gtk4::Paned, rail_width: i32) -> i32 {
    paned
        .allocated_width()
        .saturating_sub(paned_handle_width(paned))
        .saturating_sub(rail_width)
}

fn set_right_rail_width(paned: &gtk4::Paned, rail_width: i32) {
    if paned.allocated_width() > 0 {
        paned.set_position(right_rail_position(paned, rail_width));
    }
}

fn active_workspace(workspace: &gtk4::Stack) -> WorkspaceRole {
    if workspace.visible_child_name().as_deref() == Some(WorkspaceRole::Darkroom.stack_name()) {
        WorkspaceRole::Darkroom
    } else {
        WorkspaceRole::Lighttable
    }
}

fn central_workspace(workspace: &gtk4::Stack, toolbar: &LighttableToolbar) -> gtk4::Box {
    let center = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    center.set_hexpand(true);
    center.set_vexpand(true);
    center.set_width_request(i32::from(
        darkroom_window_layout(DARKTABLE_DESKTOP_SPEC.layout.window_width_px)
            .center_minimum_width_px(),
    ));
    center.set_widget_name("workspace");
    apply_theme_role(&center, ThemeRole::Workspace);
    // SearchEntry and Dropdown carry platform-native natural heights. Bound
    // those internals behind Darktable's fixed 24 px top chrome so the center
    // viewport, not macOS widget padding, owns the remaining vertical space.
    let toolbar_height = i32::from(LIGHTTABLE_COMPOSITION.top_toolbar_height_px);
    let toolbar_clip = gtk4::ScrolledWindow::builder()
        .child(toolbar.widget())
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Never)
        .min_content_height(toolbar_height)
        .max_content_height(toolbar_height)
        .propagate_natural_height(false)
        .build();
    toolbar_clip.set_widget_name("lighttable-toolbar-clip");
    toolbar_clip.set_vexpand(false);
    center.append(&toolbar_clip);
    center.append(workspace);
    center
}

pub(super) fn workspace_stack(
    initial_workspace: WorkspaceRole,
    i18n: &I18n,
    darkroom_page: &gtk4::Box,
    display_profile: &DisplayProfileBanner,
) -> (
    gtk4::Stack,
    gtk4::GridView,
    gtk4::Stack,
    LighttableLayoutControls,
    LighttableToolbar,
) {
    let workspace = gtk4::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        // Only the active view participates in Paned resize constraints.
        .hhomogeneous(false)
        // Darktable switches workspaces immediately. Avoid allocating the
        // incoming Lighttable child at its tiny crossfade natural height.
        .transition_type(gtk4::StackTransitionType::None)
        .build();
    workspace.set_widget_name("center-workspace");
    apply_theme_role(&workspace, ThemeRole::Workspace);

    let lighttable = gtk4::GridView::builder()
        .halign(gtk4::Align::Fill)
        .valign(gtk4::Align::Fill)
        .hexpand(true)
        .vexpand(true)
        .build();
    lighttable.set_widget_name("lighttable-grid");
    apply_theme_role(&lighttable, ThemeRole::Lighttable);
    let lighttable_page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    lighttable_page.set_widget_name("lighttable-page");
    lighttable_page.set_hexpand(true);
    lighttable_page.set_vexpand(true);
    let lighttable_toolbar = LighttableToolbar::new();
    let lighttable_scroll = gtk4::ScrolledWindow::builder()
        .child(&lighttable)
        .hexpand(true)
        .vexpand(true)
        .build();
    let empty_state = empty_collection_state();
    empty_state.set_halign(gtk4::Align::Fill);
    empty_state.set_valign(gtk4::Align::Fill);
    empty_state.set_hexpand(true);
    empty_state.set_vexpand(true);
    empty_state.set_visible(true);
    let lighttable_canvas = gtk4::Stack::new();
    lighttable_canvas.set_hexpand(true);
    lighttable_canvas.set_vexpand(true);
    apply_theme_role(&lighttable_canvas, ThemeRole::Lighttable);
    lighttable_canvas.add_named(&lighttable_scroll, Some("grid"));
    lighttable_canvas.add_named(&empty_state, Some("empty"));
    lighttable_canvas.set_visible_child_name("empty");
    lighttable_page.append(&lighttable_canvas);
    let layout_controls = LighttableLayoutControls::new();
    lighttable_page.append(&lighttable_footer(
        i18n,
        &layout_controls,
        &lighttable_toolbar,
        display_profile,
    ));

    workspace.add_titled(
        &lighttable_page,
        Some(WorkspaceRole::Lighttable.stack_name()),
        &i18n.text(MessageId::WorkspaceLighttable, &MessageArgs::new()),
    );
    workspace.add_titled(
        darkroom_page,
        Some(WorkspaceRole::Darkroom.stack_name()),
        &i18n.text(MessageId::WorkspaceDarkroom, &MessageArgs::new()),
    );
    workspace.set_visible_child_name(initial_workspace.stack_name());
    (
        workspace,
        lighttable,
        lighttable_canvas,
        layout_controls,
        lighttable_toolbar,
    )
}

fn lighttable_footer(
    _i18n: &I18n,
    layout_controls: &LighttableLayoutControls,
    toolbar: &LighttableToolbar,
    display_profile: &DisplayProfileBanner,
) -> gtk4::CenterBox {
    let bottom_tools = gtk4::CenterBox::new();
    bottom_tools.set_widget_name(PanelSlot::CenterBottom.identifier());
    apply_theme_role(&bottom_tools, ThemeRole::Toolbar);
    bottom_tools.add_css_class("dt_lighttable_footer");
    bottom_tools.set_start_widget(Some(toolbar.footer_organization_widget()));
    bottom_tools.set_center_widget(Some(layout_controls.widget()));
    let display_controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
    display_controls.set_widget_name("lighttable-display-controls");
    display_controls.set_accessible_role(gtk4::AccessibleRole::Toolbar);
    display_controls.update_property(&[gtk4::accessible::Property::Label(
        "Lighttable display controls",
    )]);
    display_controls.append(display_profile.widget());
    bottom_tools.set_end_widget(Some(&display_controls));
    bottom_tools
}

#[cfg(all(test, target_os = "linux"))]
#[path = "layout/tests.rs"]
mod tests;

fn filmstrip(_i18n: &I18n) -> (gtk4::Box, gtk4::FlowBox) {
    let strip = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    strip.set_widget_name(ShellRegion::Filmstrip.identifier());
    apply_theme_role(&strip, ThemeRole::Filmstrip);
    let height = i32::from(DARKTABLE_DESKTOP_SPEC.layout.filmstrip_heights.preferred_px);
    strip.set_height_request(height);
    strip.set_hexpand(true);
    strip.set_vexpand(false);
    strip.set_valign(gtk4::Align::Start);
    let photos = gtk4::FlowBox::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .max_children_per_line(FILMSTRIP_MAX_CHILDREN_PER_LINE)
        .min_children_per_line(1)
        .column_spacing(u32::from(FILMSTRIP_ITEM_GAP_PX))
        .row_spacing(0)
        .selection_mode(gtk4::SelectionMode::None)
        .valign(gtk4::Align::Center)
        .build();
    photos.set_widget_name(PanelSlot::Bottom.identifier());
    // Darktable centers a short active strip while keeping one horizontal row
    // for larger collections. A full-width wrapper supplies the available
    // surface while the FlowBox keeps its natural item width.
    photos.set_halign(gtk4::Align::Start);
    photos.set_hexpand(false);
    photos.set_vexpand(false);

    let strip_surface = gtk4::Grid::new();
    strip_surface.set_halign(gtk4::Align::Fill);
    strip_surface.set_hexpand(true);
    strip_surface.attach(&photos, 0, 0, 1, 1);

    strip.append(&strip_surface);
    (strip, photos)
}

fn panel_column(region: ShellRegion, width: i32) -> gtk4::Box {
    let panel = gtk4::Box::new(
        gtk4::Orientation::Vertical,
        i32::from(DARKTABLE_DESKTOP_SPEC.layout.panel_module_spacing_px),
    );
    panel.set_widget_name(region.identifier());
    apply_theme_role(&panel, ThemeRole::Panel);
    panel.set_size_request(width, -1);
    panel.set_hexpand(false);
    panel.set_vexpand(true);
    panel.set_halign(gtk4::Align::Fill);
    panel.set_valign(gtk4::Align::Fill);
    panel
}

fn panel_slot(slot: PanelSlot) -> gtk4::Box {
    let slot_widget = gtk4::Box::new(
        gtk4::Orientation::Vertical,
        i32::from(DARKTABLE_DESKTOP_SPEC.layout.panel_module_spacing_px),
    );
    slot_widget.set_widget_name(slot.identifier());
    slot_widget.add_css_class("dt_panel_slot");
    slot_widget
}

fn append_panel_slots(panel: &gtk4::Box, top: &gtk4::Box, center: &gtk4::Box, bottom: &gtk4::Box) {
    let scrolling_center = center.clone();
    let scroll = gtk4::ScrolledWindow::builder()
        .child(&scrolling_center)
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        // Module labels and controls can be wider than a Darktable rail. Do not
        // let their natural width become the panel's requested width; the
        // enclosing Paned must retain a center viewport and the rail scrolls
        // vertically inside its allocated width.
        .propagate_natural_width(false)
        .hexpand(true)
        .vexpand(true)
        .build();
    scroll.set_placement(gtk4::CornerType::TopRight);
    panel.append(&top.clone());
    panel.append(&scroll);
    panel.append(&bottom.clone());
}

pub(super) fn render_modules<'a>(
    container: &gtk4::Box,
    modules: impl ExactSizeIterator<Item = &'a ModulePanelViewModel>,
    group: Option<DarkroomModuleGroup>,
) {
    clear_children(container);
    let mut rendered = 0;
    for (index, module) in modules.enumerate() {
        let _ = group;
        container.append(&module_expander(module, index));
        rendered += 1;
    }
    if rendered == 0 {
        let message = gtk4::Label::new(Some(match group {
            Some(_) => "No modules in this group",
            None => "No modules available",
        }));
        message.set_widget_name("darkroom-module-group-empty");
        message.set_halign(gtk4::Align::Start);
        message.add_css_class("dim-label");
        message.set_accessible_role(gtk4::AccessibleRole::Status);
        container.append(&message);
    }
}

fn module_group(id: &str, label: &str, expanded: bool) -> gtk4::Expander {
    let group_widget = shared_module_expander(id, label, expanded, None::<&gtk4::Widget>);
    group_widget.set_label_widget(Some(&module_title(id, label)));
    apply_theme_role(&group_widget, ThemeRole::ModuleGroup);
    group_widget
}

fn module_expander(module: &ModulePanelViewModel, index: usize) -> gtk4::Expander {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    content.set_widget_name(&format!("module-{index}-controls"));
    for control in module.controls() {
        let widget: gtk4::Widget = match control.kind() {
            ModuleControlKind::Slider => slider("module-slider", 0.0, 1.0, 0.01, false).upcast(),
            ModuleControlKind::Toggle => switch("module-switch").upcast(),
            ModuleControlKind::Choice => dropdown("module-dropdown", &["default"]).upcast(),
        };
        let row = module_row(control.label().as_str(), &widget);
        content.append(&row);
    }
    let expander = shared_module_expander(
        &format!("module-{index}"),
        module.title().as_str(),
        true,
        Some(&content),
    );
    expander.update_property(&[gtk4::accessible::Property::Label(module.title().as_str())]);
    apply_theme_role(&expander, ThemeRole::Module);
    expander
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}
