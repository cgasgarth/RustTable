//! GTK4 layout composition helpers for the Darktable-shaped shell.

use gtk4::prelude::*;
use rusttable_i18n::{I18n, MessageArgs, MessageId};

use crate::ai_batch::AiBatchPanel;
use crate::camera::CameraPanel;
use crate::external_editor::ExternalEditorPanel;
use crate::import::ImportSessionPanel;
use crate::neural_restore::NeuralRestorePanel;

use super::lighttable::empty_collection_state;
use super::{
    DARKTABLE_DESKTOP_SPEC, ExportPanel, LIGHTTABLE_RIGHT_MODULES, ModuleControlKind,
    ModulePanelViewModel, PanelSlot, ShellRegion, ThemeRole, WorkspaceRole, apply_theme_role,
};

pub(super) fn right_panel() -> (
    gtk4::Box,
    ExportPanel,
    ExternalEditorPanel,
    NeuralRestorePanel,
    AiBatchPanel,
    CameraPanel,
    ImportSessionPanel,
) {
    let panel = panel_column(
        ShellRegion::RightPanel,
        i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.preferred_px),
    );
    apply_theme_role(&panel, ThemeRole::Panel);
    let export_panel = ExportPanel::new();
    let external_editor_panel = ExternalEditorPanel::new();
    let neural_restore_panel = NeuralRestorePanel::new();
    let ai_batch_panel = AiBatchPanel::new();
    let camera_panel = CameraPanel::new();
    let import_session_panel = ImportSessionPanel::new();
    let center = panel_slot(PanelSlot::RightCenter);
    for module in &LIGHTTABLE_RIGHT_MODULES[..LIGHTTABLE_RIGHT_MODULES.len() - 1] {
        center.append(&module_group(module.widget_name, module.title, false));
    }
    center.append(export_panel.widget());
    center.append(external_editor_panel.widget());
    center.append(neural_restore_panel.widget());
    center.append(ai_batch_panel.widget());
    center.append(camera_panel.widget());
    center.append(import_session_panel.widget());
    let bottom = panel_slot(PanelSlot::RightBottom);
    let search = gtk4::SearchEntry::new();
    search.set_widget_name("right-module-search");
    bottom.append(&search);
    append_panel_slots(&panel, &panel_slot(PanelSlot::RightTop), &center, &bottom);
    (
        panel,
        export_panel,
        external_editor_panel,
        neural_restore_panel,
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
    left_panel: &gtk4::Stack,
    right_panel: &gtk4::Stack,
    i18n: &I18n,
) -> (gtk4::Box, gtk4::FlowBox) {
    let layout = DARKTABLE_DESKTOP_SPEC.layout;
    let center = central_workspace(workspace);
    let split = gtk4::Paned::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .start_child(left_panel)
        .end_child(&center)
        .resize_start_child(false)
        .shrink_start_child(true)
        .position(i32::from(layout.side_panel_widths.preferred_px))
        .build();
    split.connect_map({
        let preferred_width = i32::from(layout.side_panel_widths.preferred_px);
        move |paned| paned.set_position(preferred_width)
    });
    let workspace_with_right_panel = gtk4::Paned::builder()
        .orientation(gtk4::Orientation::Horizontal)
        .start_child(&split)
        .end_child(right_panel)
        .resize_end_child(false)
        .shrink_end_child(true)
        .position(i32::from(
            layout.preferred_right_panel_position_px(layout.window_width_px),
        ))
        .build();
    let (filmstrip_root, filmstrip) = filmstrip(i18n);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let outer_border = i32::from(layout.outer_border_px);
    content.set_margin_top(outer_border);
    content.set_margin_bottom(outer_border);
    content.set_margin_start(outer_border);
    content.set_margin_end(outer_border);
    content.append(&workspace_with_right_panel);
    content.append(&filmstrip_root);
    (content, filmstrip)
}

fn central_workspace(workspace: &gtk4::Stack) -> gtk4::Box {
    let center = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    center.set_hexpand(true);
    center.set_vexpand(true);
    center.set_widget_name("workspace");
    apply_theme_role(&center, ThemeRole::Workspace);
    center.append(workspace);
    center
}

pub(super) fn workspace_stack(
    initial_workspace: WorkspaceRole,
    i18n: &I18n,
    darkroom_page: &gtk4::Box,
) -> (gtk4::Stack, gtk4::FlowBox, gtk4::Stack) {
    let workspace = gtk4::Stack::builder()
        .hexpand(true)
        .vexpand(true)
        .transition_type(gtk4::StackTransitionType::Crossfade)
        .build();
    workspace.set_widget_name("center-workspace");
    apply_theme_role(&workspace, ThemeRole::Workspace);

    let lighttable = gtk4::FlowBox::builder()
        .max_children_per_line(6)
        .selection_mode(gtk4::SelectionMode::None)
        .valign(gtk4::Align::Start)
        .build();
    lighttable.set_widget_name("lighttable-grid");
    apply_theme_role(&lighttable, ThemeRole::Lighttable);
    let lighttable_page = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    lighttable_page.set_widget_name("lighttable-page");
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
    lighttable_page.append(&lighttable_footer(i18n));

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
    (workspace, lighttable, lighttable_canvas)
}

fn lighttable_footer(i18n: &I18n) -> gtk4::Box {
    let bottom_tools = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    bottom_tools.set_widget_name(PanelSlot::CenterBottom.identifier());
    apply_theme_role(&bottom_tools, ThemeRole::Toolbar);
    bottom_tools.add_css_class("dt_lighttable_footer");
    for message_id in [
        MessageId::WorkspaceFit,
        MessageId::WorkspaceBeforeAfter,
        MessageId::WorkspaceSoftProof,
    ] {
        bottom_tools.append(&gtk4::Button::with_label(
            &i18n.text(message_id, &MessageArgs::new()),
        ));
    }
    bottom_tools.insert_child_after(&gtk4::Button::with_label("100%"), None::<&gtk4::Widget>);
    bottom_tools
}

fn filmstrip(_i18n: &I18n) -> (gtk4::Box, gtk4::FlowBox) {
    let strip = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    strip.set_widget_name(ShellRegion::Filmstrip.identifier());
    apply_theme_role(&strip, ThemeRole::Filmstrip);
    strip.set_height_request(i32::from(
        DARKTABLE_DESKTOP_SPEC.layout.filmstrip_heights.preferred_px,
    ));
    let photos = gtk4::FlowBox::builder()
        .max_children_per_line(12)
        .selection_mode(gtk4::SelectionMode::None)
        .build();
    photos.set_widget_name(PanelSlot::Bottom.identifier());
    strip.append(&photos);
    (strip, photos)
}

fn panel_column(region: ShellRegion, width: i32) -> gtk4::Box {
    let panel = gtk4::Box::new(
        gtk4::Orientation::Vertical,
        i32::from(DARKTABLE_DESKTOP_SPEC.layout.panel_module_spacing_px),
    );
    panel.set_widget_name(region.identifier());
    apply_theme_role(&panel, ThemeRole::Panel);
    panel.set_width_request(width);
    panel
}

fn panel_slot(slot: PanelSlot) -> gtk4::Box {
    let slot_widget = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    slot_widget.set_widget_name(slot.identifier());
    slot_widget.add_css_class("dt_panel_slot");
    slot_widget
}

fn append_panel_slots(panel: &gtk4::Box, top: &gtk4::Box, center: &gtk4::Box, bottom: &gtk4::Box) {
    let scrolling_center = center.clone();
    let scroll = gtk4::ScrolledWindow::builder()
        .child(&scrolling_center)
        .hexpand(true)
        .vexpand(true)
        .build();
    panel.append(&top.clone());
    panel.append(&scroll);
    panel.append(&bottom.clone());
}

pub(super) fn render_modules<'a>(
    container: &gtk4::Box,
    modules: impl ExactSizeIterator<Item = &'a ModulePanelViewModel>,
) {
    clear_children(container);
    for (index, module) in modules.enumerate() {
        container.append(&module_expander(module, index));
    }
}

fn module_group(id: &str, label: &str, expanded: bool) -> gtk4::Expander {
    let group_widget = gtk4::Expander::builder()
        .label(label)
        .expanded(expanded)
        .build();
    group_widget.set_widget_name(id);
    apply_theme_role(&group_widget, ThemeRole::ModuleGroup);
    group_widget
}

fn module_expander(module: &ModulePanelViewModel, index: usize) -> gtk4::Expander {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    for control in module.controls() {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        let label = gtk4::Label::new(Some(control.label().as_str()));
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        row.append(&label);
        let widget: gtk4::Widget = match control.kind() {
            ModuleControlKind::Slider => {
                gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 1.0, 0.01).upcast()
            }
            ModuleControlKind::Toggle => gtk4::Switch::new().upcast(),
            ModuleControlKind::Choice => gtk4::DropDown::from_strings(&["default"]).upcast(),
        };
        row.append(&widget);
        content.append(&row);
    }
    let expander = gtk4::Expander::builder()
        .label(module.title().as_str())
        .expanded(true)
        .child(&content)
        .build();
    expander.set_widget_name(&format!("module-{index}"));
    apply_theme_role(&expander, ThemeRole::Module);
    expander
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}
