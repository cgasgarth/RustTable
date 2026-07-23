use gtk4::prelude::*;
use rusttable_i18n::I18n;

use super::{
    DARKROOM_PANEL_WIDTHS, DARKTABLE_DESKTOP_SPEC, DisplayProfileBanner, LIGHTTABLE_PANEL_WIDTHS,
    LighttableToolbar, WorkspaceRole, desktop_body, lighttable_footer, mode_panel_stack,
    paned_handle_width, right_rail_width,
};
use crate::gtk_shell::LighttableLayoutControls;

#[test]
fn lighttable_footer_attaches_every_control_to_one_parent() {
    if gtk4::init().is_err() {
        return;
    }
    let controls = LighttableLayoutControls::new();
    let toolbar = LighttableToolbar::new();
    let display_profile = DisplayProfileBanner::new();
    let footer = lighttable_footer(&I18n::default(), &controls, &toolbar, &display_profile);
    assert_eq!(
        footer
            .start_widget()
            .expect("footer organization controls")
            .widget_name(),
        "lighttable-footer-organization"
    );
    assert_eq!(
        footer
            .center_widget()
            .expect("footer layout selector")
            .widget_name(),
        "lighttable-layout-controls"
    );
    assert_eq!(
        footer
            .end_widget()
            .expect("footer display controls")
            .widget_name(),
        "lighttable-display-controls"
    );
}

#[gtk4::test]
fn side_dividers_resize_in_place_and_refresh_after_child_allocation() {
    let fixture = PanedFixture::new(1_280);
    let left_split = &fixture.left_split;
    let right_split = &fixture.right_split;
    assert!(
        left_split.is_wide_handle(),
        "left divider has a usable hit target"
    );
    assert!(
        right_split.is_wide_handle(),
        "right divider has a usable hit target"
    );
    assert_eq!(
        left_split
            .start_child()
            .expect("left rail")
            .allocated_width(),
        i32::from(LIGHTTABLE_PANEL_WIDTHS.left_px)
    );
    assert_eq!(
        right_split
            .end_child()
            .expect("right rail")
            .allocated_width(),
        i32::from(LIGHTTABLE_PANEL_WIDTHS.right_px)
    );

    let initial_left_width = left_split.position();
    let initial_right_width = right_rail_width(right_split);
    let refresh_before_drag = fixture.refresh_count.get();
    right_split.set_position(right_split.position().saturating_sub(80));
    settle_gtk();

    assert_eq!(left_split.position(), initial_left_width);
    assert_eq!(right_rail_width(right_split), initial_right_width + 80);
    assert_eq!(
        right_split
            .end_child()
            .expect("resized right rail")
            .allocated_width(),
        initial_right_width + 80
    );
    assert!(
        left_split
            .end_child()
            .expect("center workspace")
            .allocated_width()
            >= i32::from(DARKTABLE_DESKTOP_SPEC.layout.center_minimum_width_px),
        "right drag must preserve the configured center minimum"
    );
    assert!(
        fixture.refresh_count.get() > refresh_before_drag,
        "divider movement and the resulting child allocation must refresh geometry"
    );

    let refresh_before_left_drag = fixture.refresh_count.get();
    left_split.set_position(left_split.position().saturating_add(40));
    settle_gtk();
    assert_eq!(left_split.position(), initial_left_width + 40);
    assert_eq!(right_rail_width(right_split), initial_right_width + 80);
    assert!(
        fixture.refresh_count.get() > refresh_before_left_drag,
        "left-rail allocation must refresh redraw-dependent geometry"
    );
}

#[gtk4::test]
fn divider_widths_are_clamped_and_retained_per_workspace() {
    let fixture = PanedFixture::new(1_224);
    let workspace = &fixture.workspace;
    let left_split = &fixture.left_split;
    let right_split = &fixture.right_split;
    let minimum = i32::from(DARKTABLE_DESKTOP_SPEC.layout.side_panel_widths.minimum_px);

    left_split.set_position(0);
    right_split.set_position(right_split.allocated_width());
    settle_gtk();
    assert_eq!(left_split.position(), minimum);
    assert_eq!(right_rail_width(right_split), minimum);

    left_split.set_position(210);
    let lighttable_right_width = 260;
    right_split.set_position(
        right_split
            .allocated_width()
            .saturating_sub(paned_handle_width(right_split))
            .saturating_sub(lighttable_right_width),
    );
    settle_gtk();
    workspace.set_visible_child_name(WorkspaceRole::Darkroom.stack_name());
    settle_gtk();
    assert_eq!(
        left_split.position(),
        i32::from(DARKROOM_PANEL_WIDTHS.left_px)
    );
    assert_eq!(
        right_rail_width(right_split),
        i32::from(DARKROOM_PANEL_WIDTHS.right_px)
    );
    assert_eq!(
        right_split
            .end_child()
            .expect("1224 px Darkroom right rail")
            .allocated_width(),
        i32::from(DARKROOM_PANEL_WIDTHS.right_px)
    );

    left_split.set_position(230);
    right_split.set_position(
        right_split
            .allocated_width()
            .saturating_sub(paned_handle_width(right_split))
            .saturating_sub(280),
    );
    settle_gtk();
    workspace.set_visible_child_name(WorkspaceRole::Lighttable.stack_name());
    settle_gtk();
    assert_eq!(left_split.position(), 210);
    assert_eq!(right_rail_width(right_split), lighttable_right_width);
}

struct PanedFixture {
    _window: gtk4::Window,
    workspace: gtk4::Stack,
    left_split: gtk4::Paned,
    right_split: gtk4::Paned,
    refresh_count: std::rc::Rc<std::cell::Cell<u32>>,
}

impl PanedFixture {
    fn new(window_width: i32) -> Self {
        let workspace = test_workspace();
        let left_panel = test_panel_stack("left-panel-stack", WorkspaceRole::Lighttable);
        let right_panel = test_panel_stack("right-panel-stack", WorkspaceRole::Lighttable);
        let refresh_count = std::rc::Rc::new(std::cell::Cell::new(0_u32));
        let geometry_changed: std::rc::Rc<dyn Fn()> = std::rc::Rc::new({
            let refresh_count = std::rc::Rc::clone(&refresh_count);
            move || refresh_count.set(refresh_count.get().saturating_add(1))
        });
        let (content, _, _) = desktop_body(
            &workspace,
            &LighttableToolbar::new(),
            &left_panel,
            &right_panel,
            &I18n::default(),
            &geometry_changed,
        );
        let window = gtk4::Window::builder()
            .default_width(window_width)
            .default_height(768)
            .child(&content)
            .build();
        window.present();
        settle_gtk();
        let root: gtk4::Widget = content.upcast();
        Self {
            _window: window,
            workspace,
            left_split: find_paned(&root, "desktop-left-split"),
            right_split: find_paned(&root, "desktop-right-split"),
            refresh_count,
        }
    }
}

fn find_paned(root: &gtk4::Widget, name: &str) -> gtk4::Paned {
    find_widget(root, name)
        .unwrap_or_else(|| panic!("missing {name}"))
        .downcast::<gtk4::Paned>()
        .unwrap_or_else(|_| panic!("{name} is not a Paned"))
}

fn test_workspace() -> gtk4::Stack {
    let workspace = gtk4::Stack::new();
    workspace.set_hexpand(true);
    workspace.set_vexpand(true);
    workspace.add_named(
        &gtk4::Box::new(gtk4::Orientation::Vertical, 0),
        Some(WorkspaceRole::Lighttable.stack_name()),
    );
    workspace.add_named(
        &gtk4::Box::new(gtk4::Orientation::Vertical, 0),
        Some(WorkspaceRole::Darkroom.stack_name()),
    );
    workspace.set_visible_child_name(WorkspaceRole::Lighttable.stack_name());
    workspace
}

fn test_panel_stack(id: &str, initial: WorkspaceRole) -> gtk4::Stack {
    mode_panel_stack(
        id,
        &gtk4::Box::new(gtk4::Orientation::Vertical, 0),
        &gtk4::Box::new(gtk4::Orientation::Vertical, 0),
        initial,
    )
}

fn settle_gtk() {
    let context = gtk4::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn find_widget(root: &gtk4::Widget, name: &str) -> Option<gtk4::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = find_widget(&widget, name) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}
