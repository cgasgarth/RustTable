//! GTK4 layout composition helpers for the Darktable-shaped shell.

use gtk4::prelude::*;
use rusttable_i18n::{I18n, MessageArgs, MessageId};

use crate::ai_batch::AiBatchPanel;
use crate::camera::CameraPanel;
use crate::external_editor::ExternalEditorPanel;
use crate::import::ImportSessionPanel;

use crate::gui::darkroom_modules::DarkroomModuleGroup;
use crate::gui::darktable_spec::{FILMSTRIP_ITEM_GAP_PX, FILMSTRIP_MAX_CHILDREN_PER_LINE};
use crate::gui::{
    DARKTABLE_DESKTOP_SPEC, ExportPanel, LIGHTTABLE_RIGHT_MODULES, LighttableLayoutControls,
    ModuleControlKind, ModulePanelViewModel, PanelSlot, ShellRegion, ThemeRole, WorkspaceRole,
    apply_theme_role, darkroom_window_layout,
};
use crate::views::lighttable::empty_collection_state;

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
        i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.preferred_px),
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
    center.append(external_editor_panel.widget());
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
    let preferred_width = i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.preferred_px);
    // Paned allocates a child from its minimum/natural width.  The module
    // contents are intentionally wider than Darktable's rail in places, so
    // make the stack's initial request explicit and let its inner scroller
    // handle overflow instead of allowing natural width to consume the
    // center workspace.
    stack.set_width_request(preferred_width);
    stack.set_hexpand(false);
    stack.set_vexpand(true);
    stack.set_halign(gtk4::Align::Fill);
    stack.set_valign(gtk4::Align::Fill);
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
) -> (gtk4::Box, gtk4::FlowBox, gtk4::Box) {
    let layout = DARKTABLE_DESKTOP_SPEC.layout;
    let center = central_workspace(workspace);
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
        .shrink_end_child(false)
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
        .hexpand(true)
        .vexpand(true)
        .resize_end_child(false)
        .shrink_end_child(false)
        .position(i32::from(
            layout.preferred_right_panel_position_px(layout.window_width_px),
        ))
        .build();
    // The right rail is vertically scrolled and must be allowed to shrink to
    // its preferred Darktable width.  Without this, the natural width of an
    // expanded module wins the first allocation and the rail covers the
    // center of the window.
    workspace_with_right_panel.set_shrink_end_child(true);
    workspace_with_right_panel.connect_map(move |paned| {
        let paned = paned.clone();
        gtk4::glib::idle_add_local_once(move || {
            let content_width = u16::try_from(paned.allocated_width()).unwrap_or(u16::MAX);
            if content_width == 0 {
                return;
            }
            paned.set_position(i32::from(
                layout.preferred_right_panel_position_for_content_width(content_width),
            ));
        });
    });
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    let outer_border = i32::from(layout.outer_border_px);
    content.set_margin_top(outer_border);
    content.set_margin_bottom(outer_border);
    content.set_margin_start(outer_border);
    content.set_margin_end(outer_border);
    content.append(&workspace_with_right_panel);
    (content, filmstrip, filmstrip_root)
}

fn central_workspace(workspace: &gtk4::Stack) -> gtk4::Box {
    let center = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    center.set_hexpand(true);
    center.set_vexpand(true);
    center.set_width_request(i32::from(
        darkroom_window_layout(DARKTABLE_DESKTOP_SPEC.layout.window_width_px)
            .center_minimum_width_px(),
    ));
    center.set_widget_name("workspace");
    apply_theme_role(&center, ThemeRole::Workspace);
    center.append(workspace);
    center
}

pub(super) fn workspace_stack(
    initial_workspace: WorkspaceRole,
    i18n: &I18n,
    darkroom_page: &gtk4::Box,
) -> (
    gtk4::Stack,
    gtk4::FlowBox,
    gtk4::Stack,
    LighttableLayoutControls,
) {
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
    let layout_controls = LighttableLayoutControls::new();
    lighttable_page.append(&lighttable_footer(i18n, &layout_controls));

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
    (workspace, lighttable, lighttable_canvas, layout_controls)
}

fn lighttable_footer(i18n: &I18n, layout_controls: &LighttableLayoutControls) -> gtk4::Box {
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
    bottom_tools.append(layout_controls.widget());
    bottom_tools.append(&gtk4::Button::with_label("100%"));
    bottom_tools
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use gtk4::prelude::*;

    use super::lighttable_footer;
    use rusttable_i18n::I18n;

    use crate::gtk_shell::LighttableLayoutControls;

    #[test]
    fn lighttable_footer_attaches_every_control_to_one_parent() {
        if gtk4::init().is_err() {
            return;
        }
        let controls = LighttableLayoutControls::new();
        let footer = lighttable_footer(&I18n::default(), &controls);
        let mut child = footer.first_child();
        let mut count = 0;
        while let Some(widget) = child {
            assert!(widget.parent().is_some(), "footer child must be attached");
            count += 1;
            child = widget.next_sibling();
        }
        assert_eq!(count, 5);
    }
}

fn filmstrip(_i18n: &I18n) -> (gtk4::Box, gtk4::FlowBox) {
    let strip = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    strip.set_widget_name(ShellRegion::Filmstrip.identifier());
    apply_theme_role(&strip, ThemeRole::Filmstrip);
    let height = i32::from(DARKTABLE_DESKTOP_SPEC.layout.filmstrip_heights.preferred_px);
    strip.set_height_request(height);
    strip.set_hexpand(true);
    strip.set_vexpand(false);
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
    photos.set_halign(gtk4::Align::Start);
    photos.set_vexpand(false);

    let scroll = gtk4::ScrolledWindow::new();
    scroll.set_widget_name("filmstrip-scroll");
    scroll.set_policy(gtk4::PolicyType::Automatic, gtk4::PolicyType::Never);
    scroll.set_hexpand(true);
    scroll.set_vexpand(false);
    scroll.set_height_request(height);
    scroll.set_child(Some(&photos));
    strip.append(&scroll);
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
    content.set_widget_name(&format!("module-{index}-controls"));
    for control in module.controls() {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.add_css_class("dt_module_row");
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
    expander.set_focusable(true);
    expander.update_property(&[gtk4::accessible::Property::Label(module.title().as_str())]);
    apply_theme_role(&expander, ThemeRole::Module);
    expander
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}
