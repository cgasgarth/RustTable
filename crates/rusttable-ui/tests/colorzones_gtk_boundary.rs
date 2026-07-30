#![forbid(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::prelude::*;
use rusttable_core::{OperationId, OperationOpacity, Revision};
use rusttable_ui::iop::colorzones::{
    COLORZONES_DESCRIPTION, COLORZONES_EDIT_BY_AREA_LABEL, COLORZONES_EDIT_BY_AREA_TOOLTIP,
    COLORZONES_GRAPH_HEIGHT_DEFAULT, COLORZONES_GRAPH_HEIGHT_MAX, COLORZONES_GRAPH_HEIGHT_MIN,
    COLORZONES_GRAPH_INSET, COLORZONES_INTERPOLATION_LABEL, COLORZONES_INTERPOLATION_OPTIONS,
    COLORZONES_INTERPOLATION_TOOLTIP, COLORZONES_MODE_LABEL, COLORZONES_MODE_OPTIONS,
    COLORZONES_MODE_TOOLTIP, COLORZONES_OUTPUT_LABELS, COLORZONES_SELECTION_LABEL,
    COLORZONES_SELECTION_OPTIONS, COLORZONES_SELECTION_TOOLTIP, COLORZONES_STRENGTH_LABEL,
    COLORZONES_STRENGTH_TOOLTIP, COLORZONES_TITLE, ColorZonesEditorState, ColorZonesGraphHeight,
    ColorZonesGtkActionHandler, ColorZonesGtkHandlerOutcome, ColorZonesGtkPreferences,
    ColorZonesGtkPreferencesHandler, ColorZonesGtkState, ColorZonesScrollOutcome,
    ColorZonesSettledAction, build_colorzones_gtk,
};
use rusttable_ui::{
    DarkroomModuleSide, DarkroomModuleViewModel, DarkroomModulesViewModel, GtkShell, WorkspaceRole,
    install_darktable_theme,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the Color Zones boundary");
    prohibit_macos_test_activation();
    let display = gtk4::gdk::Display::default().expect("GTK boundary needs the default display");
    install_darktable_theme(&display);
    fractional_smooth_scroll_units_are_shared_across_distinct_graphs();
    normalized_smooth_scroll_coalesces_at_the_production_leaf_boundary();
    modifier_routing_preserves_tabs_edits_and_durable_graph_height();
    production_rail_mount_preserves_source_hierarchy_and_reconciles_in_place();
    production_rail_routes_settled_graph_actions_without_showing_a_window();
    println!("Color Zones GTK boundary passed");
}

#[cfg(target_os = "macos")]
fn prohibit_macos_test_activation() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let marker =
        MainThreadMarker::new().expect("custom GTK boundary must start on the main thread");
    let application = NSApplication::sharedApplication(marker);
    application.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    assert_eq!(
        application.activationPolicy(),
        NSApplicationActivationPolicy::Prohibited,
        "automated GTK boundary must not activate or steal focus"
    );
}

#[cfg(not(target_os = "macos"))]
fn prohibit_macos_test_activation() {}

#[allow(clippy::too_many_lines)]
fn production_rail_mount_preserves_source_hierarchy_and_reconciles_in_place() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.colorzones-boundary"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("Color Zones test application must register");
    let shell = GtkShell::new(&application);
    shell.window().set_focusable(false);
    shell.window().set_opacity(0.0);
    shell.window().set_default_size(1_200, 900);

    let operation_id = OperationId::new(7_331).expect("Color Zones operation ID");
    let revision = Revision::from_u64(17);
    let state = ColorZonesGtkState::new(
        operation_id,
        revision,
        ColorZonesEditorState::default(),
        false,
        OperationOpacity::ONE,
        true,
        true,
    );
    let module = module_snapshot(state.clone());
    assert_eq!(module.controls().controls().count(), 0);
    let modules = DarkroomModulesViewModel::new(vec![module]).expect("Color Zones module snapshot");
    shell.set_darkroom_module_stack(&modules, None);
    shell.show_workspace(WorkspaceRole::Darkroom);
    settle_gtk();
    assert!(
        !shell.window().is_visible(),
        "boundary window stays unshown"
    );
    assert!(
        !shell.window().is_active(),
        "unshown boundary window must not activate or steal focus"
    );

    let root: gtk4::Widget = shell.window().clone().upcast();
    let color_group = find_widget(&root, "group-color")
        .expect("Color module group")
        .downcast::<gtk4::ToggleButton>()
        .expect("Color module group type");
    color_group.set_active(true);
    settle_gtk();
    let panel = find_widget(&root, "colorzones")
        .expect("production rail mounts Color Zones")
        .downcast::<gtk4::Expander>()
        .expect("Color Zones panel type");
    assert_eq!(panel.accessible_role(), gtk4::AccessibleRole::Group);
    let title_root = panel.label_widget().expect("Color Zones title widget");
    assert_eq!(title_root.widget_name(), "colorzones-title");
    assert!(title_root.has_css_class("dt_module_header"));
    assert!(title_root.has_css_class("dt_darkroom_section_title"));
    let title = find_widget(&title_root, "colorzones-label")
        .expect("Color Zones title label")
        .downcast::<gtk4::Label>()
        .expect("Color Zones title type");
    assert_eq!(title.text(), COLORZONES_TITLE);
    assert_eq!(
        title_root.tooltip_text().as_deref(),
        Some(COLORZONES_DESCRIPTION)
    );
    let enabled = widget::<gtk4::CheckButton>(&title_root, "colorzones-enabled");
    assert!(!enabled.is_active());
    let reset = widget::<gtk4::Button>(&title_root, "colorzones-reset");
    assert!(reset.is_sensitive());
    let icon_slot = widget::<gtk4::Box>(&title_root, "iop-panel-icon-colorzones");
    assert!(icon_slot.first_child().is_none());

    let content = panel.child().expect("Color Zones module content");
    for omitted in [
        "colorzones-header",
        "colorzones-enabled",
        "colorzones-reset",
        "colorzones-status-row",
        "colorzones-status",
        "colorzones-recover",
        "colorzones-partial-warning",
        "colorzones-picker",
        "colorzones-color-picker",
        "colorzones-display-selection",
        "colorzones-show-selection",
        "colorzones-mask-display",
    ] {
        assert!(
            find_widget(&content, omitted).is_none(),
            "module-body placeholder or duplicate {omitted} must remain omitted"
        );
    }
    assert!(
        find_widget(&root, "colorzones-presets").is_none(),
        "unimplemented presets must not appear anywhere as a placeholder"
    );

    let editor = find_widget(&content, "colorzones-colorzones-editor")
        .expect("source-specific Color Zones editor")
        .downcast::<gtk4::Box>()
        .expect("Color Zones editor root type");
    assert_eq!(editor.orientation(), gtk4::Orientation::Vertical);
    assert_eq!(editor.spacing(), 0);
    assert_direct_child_order(
        &editor,
        [
            "colorzones-channel-tabs",
            "colorzones-graph",
            "colorzones-bottom-strip",
            "colorzones-edit-by-area",
            "colorzones-select-by",
            "colorzones-mode",
            "colorzones-strength",
            "colorzones-interpolator",
        ],
    );

    let notebook = widget::<gtk4::Notebook>(&editor, "colorzones-channel-tabs");
    assert!(!notebook.is_scrollable());
    assert!(!notebook.enables_popup());
    assert!(notebook.hexpands());
    assert!(!notebook.vexpands());
    assert_eq!(notebook.n_pages(), 3);
    for (page, expected) in COLORZONES_OUTPUT_LABELS.into_iter().enumerate() {
        let page = notebook
            .nth_page(Some(u32::try_from(page).expect("page index")))
            .expect("Color Zones output page");
        assert!(!page.vexpands());
        let notebook_page = notebook.page(&page);
        assert!(notebook_page.is_tab_expand());
        assert!(notebook_page.is_tab_fill());
        let label = notebook
            .tab_label(&page)
            .expect("Color Zones output tab label")
            .downcast::<gtk4::Label>()
            .expect("output tab label type");
        assert_eq!(label.text(), expected);
        assert_eq!(label.ellipsize(), gtk4::pango::EllipsizeMode::End);
        assert_eq!(label.tooltip_text().as_deref(), Some(expected));
    }

    let graph = widget::<gtk4::DrawingArea>(&editor, "colorzones-graph");
    assert_eq!(graph.accessible_role(), gtk4::AccessibleRole::Slider);
    assert!(graph.is_focusable());
    assert!(graph.hexpands());
    assert_eq!(
        graph.content_height(),
        i32::from(COLORZONES_GRAPH_HEIGHT_DEFAULT)
    );
    assert_eq!(
        graph.height_request(),
        i32::from(COLORZONES_GRAPH_HEIGHT_DEFAULT)
    );
    graph.allocate(310, i32::from(COLORZONES_GRAPH_HEIGHT_DEFAULT), -1, None);
    assert_eq!(graph.allocated_width(), 310);
    assert_eq!(
        graph.allocated_height(),
        i32::from(COLORZONES_GRAPH_HEIGHT_DEFAULT)
    );
    assert_eq!(COLORZONES_GRAPH_INSET.to_bits(), 5.0_f32.to_bits());
    assert_eq!(
        graph.allocated_width() - 10,
        300,
        "graph paint and pointer routing share the exact five-pixel interior"
    );
    for controller in [
        "dt-colorzones-motion",
        "dt-colorzones-click",
        "dt-colorzones-secondary-click",
        "dt-colorzones-scroll",
        "dt-colorzones-key",
    ] {
        assert!(
            named_controller(&graph, controller).is_some(),
            "production graph owns stable controller {controller}"
        );
    }
    let scroll = named_controller(&graph, "dt-colorzones-scroll")
        .expect("Color Zones scroll controller")
        .downcast::<gtk4::EventControllerScroll>()
        .expect("Color Zones scroll type");
    assert_eq!(
        scroll.flags(),
        gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
        "native graph consumes both scroll axes through one kinetic sequence boundary"
    );

    let bottom_bar = widget::<gtk4::Box>(&editor, "iop-bottom-bar");
    assert_eq!(bottom_bar.spacing(), 0);
    assert!(!bottom_bar.vexpands());
    let bottom = widget::<gtk4::DrawingArea>(&bottom_bar, "colorzones-bottom-strip");
    assert!(bottom.hexpands());
    assert!(bottom.vexpands());
    assert_eq!(bottom.height_request(), -1);
    let bottom_click = named_controller(&bottom, "dt-colorzones-bottom-click")
        .expect("bottom-strip reset controller")
        .downcast::<gtk4::GestureClick>()
        .expect("bottom-strip reset controller type");
    assert_eq!(bottom_click.button(), 1);

    let edit_by_area = widget::<gtk4::CheckButton>(&editor, "colorzones-edit-by-area");
    assert_eq!(
        edit_by_area.label().as_deref(),
        Some(COLORZONES_EDIT_BY_AREA_LABEL)
    );
    assert_eq!(edit_by_area.halign(), gtk4::Align::Start);
    assert!(!edit_by_area.hexpands());
    assert_eq!(
        edit_by_area.tooltip_text().as_deref(),
        Some(COLORZONES_EDIT_BY_AREA_TOOLTIP)
    );

    let source_controls = direct_children(&editor)
        .into_iter()
        .skip(4)
        .collect::<Vec<_>>();
    assert_eq!(source_controls.len(), 4);
    for control in &source_controls {
        assert!(control.has_css_class("dt_bauhaus"));
        assert!(control.hexpands());
        assert_eq!(control.halign(), gtk4::Align::Fill);
    }

    let select_by = widget::<gtk4::DropDown>(&editor, "colorzones-select-by-selection");
    assert_bauhaus_label(&select_by, COLORZONES_SELECTION_LABEL);
    assert_dropdown(&select_by, &COLORZONES_SELECTION_OPTIONS);
    assert_eq!(
        select_by.tooltip_text().as_deref(),
        Some(COLORZONES_SELECTION_TOOLTIP)
    );
    let mode = widget::<gtk4::DropDown>(&editor, "colorzones-mode-selection");
    assert_bauhaus_label(&mode, COLORZONES_MODE_LABEL);
    assert_dropdown(&mode, &COLORZONES_MODE_OPTIONS);
    assert_eq!(
        mode.tooltip_text().as_deref(),
        Some(COLORZONES_MODE_TOOLTIP)
    );
    let strength = widget::<gtk4::Scale>(&editor, "colorzones-strength");
    assert_bauhaus_label(&strength, COLORZONES_STRENGTH_LABEL);
    assert_eq!(
        (
            strength.adjustment().lower().to_bits(),
            strength.adjustment().upper().to_bits(),
        ),
        ((-200.0_f64).to_bits(), 200.0_f64.to_bits())
    );
    assert_eq!(
        strength.adjustment().step_increment().to_bits(),
        1.0_f64.to_bits()
    );
    assert_eq!(strength.digits(), 2);
    assert_eq!(
        strength.tooltip_text().as_deref(),
        Some(COLORZONES_STRENGTH_TOOLTIP)
    );
    let strength_value = widget::<gtk4::Label>(&source_controls[2], "bauhaus-slider-value");
    assert_eq!(strength_value.text(), "+0.00%");
    let strength_click = named_controller(&strength, "dt-bauhaus-main-click")
        .expect("Bauhaus strength settled-gesture controller")
        .downcast::<gtk4::GestureClick>()
        .expect("Bauhaus strength settled-gesture controller type");
    assert_eq!(strength_click.button(), 1);
    let interpolator = widget::<gtk4::DropDown>(&editor, "colorzones-interpolator-selection");
    assert_bauhaus_label(&interpolator, COLORZONES_INTERPOLATION_LABEL);
    assert_dropdown(&interpolator, &COLORZONES_INTERPOLATION_OPTIONS);
    assert_eq!(
        interpolator.tooltip_text().as_deref(),
        Some(COLORZONES_INTERPOLATION_TOOLTIP)
    );

    notebook.set_current_page(Some(2));
    settle_gtk();
    assert_eq!(notebook.current_page(), Some(2));

    let original_motion =
        named_controller(&graph, "dt-colorzones-motion").expect("original graph motion controller");
    let mut reconciled_editor = ColorZonesEditorState::default();
    reconciled_editor
        .set_strength(35.0)
        .expect("reconciled source strength");
    let reconciled_height = ColorZonesGraphHeight::new(237).expect("reconciled graph height");
    let reconciled_state = gtk_state(operation_id, Revision::from_u64(18), reconciled_editor)
        .with_graph_height(reconciled_height);
    let reconciled_modules = DarkroomModulesViewModel::new(vec![module_snapshot(reconciled_state)])
        .expect("reconciled Color Zones snapshot");
    shell.set_darkroom_module_stack(&reconciled_modules, None);
    settle_gtk();
    let same_graph = find_widget(&root, "colorzones-graph")
        .expect("reconciled graph")
        .downcast::<gtk4::DrawingArea>()
        .expect("reconciled graph type");
    assert_eq!(
        same_graph, graph,
        "snapshot reconciliation retains DrawingArea identity"
    );
    assert_eq!(
        notebook.current_page(),
        Some(2),
        "processing reconciliation retains the UI-owned output tab"
    );
    assert_eq!(same_graph.content_height(), 237);
    assert_eq!(same_graph.height_request(), 237);
    assert_eq!(
        named_controller(&same_graph, "dt-colorzones-motion")
            .expect("reconciled motion controller"),
        original_motion,
        "snapshot reconciliation retains controller identity"
    );
    let same_strength = find_widget(&root, "colorzones-strength")
        .expect("reconciled strength")
        .downcast::<gtk4::Scale>()
        .expect("reconciled strength type");
    let normalized_strength = (35.0_f32 - -200.0_f32) / (200.0_f32 - -200.0_f32);
    let source_strength = -200.0_f32 + normalized_strength * (200.0_f32 - -200.0_f32);
    assert_eq!(
        same_strength.value().to_bits(),
        f64::from(source_strength).to_bits(),
        "GTK projection preserves Darktable's float slider round trip"
    );
    assert!(!shell.window().is_visible());
    assert!(!shell.window().is_active());
    shell.window().close();
    settle_gtk();
}

#[allow(clippy::too_many_lines)]
fn production_rail_routes_settled_graph_actions_without_showing_a_window() {
    let operation_id = OperationId::new(9_119).expect("Color Zones action operation ID");
    let initial_revision = Revision::from_u64(41);
    let actions = Rc::new(RefCell::new(Vec::<ColorZonesSettledAction>::new()));
    let actions_for_handler = Rc::clone(&actions);
    let handler: ColorZonesGtkActionHandler = Rc::new(move |action| {
        assert_eq!(action.target(), operation_id);
        assert_eq!(
            action.output_channel(),
            rusttable_processing::ColorZonesChannel::Lightness
        );
        assert!(!action.materialization_required());
        actions_for_handler.borrow_mut().push(action);
        ColorZonesGtkHandlerOutcome::Commit {
            revision: action
                .expected_revision()
                .checked_increment()
                .expect("test revisions do not overflow"),
        }
    });
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.colorzones-actions-boundary"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("Color Zones action application must register");
    let shell = GtkShell::new(&application);
    shell.window().set_focusable(false);
    shell.window().set_opacity(0.0);
    shell.window().set_default_size(1_200, 900);
    shell.set_colorzones_action_handler(Some(handler));
    let modules = DarkroomModulesViewModel::new(vec![module_snapshot(gtk_state(
        operation_id,
        initial_revision,
        ColorZonesEditorState::default(),
    ))])
    .expect("Color Zones action module snapshot");
    shell.set_darkroom_module_stack(&modules, None);
    shell.show_workspace(WorkspaceRole::Darkroom);
    settle_gtk();
    let root: gtk4::Widget = shell.window().clone().upcast();
    let graph = widget::<gtk4::DrawingArea>(&root, "colorzones-graph");
    graph.allocate(310, 200, -1, None);
    let motion = named_controller(&graph, "dt-colorzones-motion")
        .expect("action motion controller")
        .downcast::<gtk4::EventControllerMotion>()
        .expect("action motion type");
    let click = named_controller(&graph, "dt-colorzones-click")
        .expect("action primary controller")
        .downcast::<gtk4::GestureClick>()
        .expect("action primary type");
    let secondary = named_controller(&graph, "dt-colorzones-secondary-click")
        .expect("action secondary controller")
        .downcast::<gtk4::GestureClick>()
        .expect("action secondary type");
    let key = named_controller(&graph, "dt-colorzones-key")
        .expect("action key controller")
        .downcast::<gtk4::EventControllerKey>()
        .expect("action key type");

    let point = |x: f64, y: f64| (5.0 + 300.0 * x, 5.0 + 190.0 * (1.0 - y));
    let (node_x, node_y) = point(0.25, 0.5);
    let (blank_x, blank_y) = point(0.5, 0.75);

    motion.emit_by_name::<()>("motion", &[&blank_x, &blank_y]);
    click.emit_by_name::<()>("pressed", &[&2_i32, &blank_x, &blank_y]);
    assert!(
        actions.borrow().is_empty(),
        "resetting a default curve is a no-op"
    );

    motion.emit_by_name::<()>("motion", &[&node_x, &node_y]);
    click.emit_by_name::<()>("pressed", &[&1_i32, &node_x, &node_y]);
    let (rejected_x, rejected_y) = point(0.748, 0.65);
    motion.emit_by_name::<()>("motion", &[&rejected_x, &rejected_y]);
    click.emit_by_name::<()>("released", &[&1_i32, &rejected_x, &rejected_y]);
    assert!(
        actions.borrow().is_empty(),
        "too-close drag rejection must not author an action"
    );

    motion.emit_by_name::<()>("motion", &[&node_x, &node_y]);
    click.emit_by_name::<()>("pressed", &[&1_i32, &node_x, &node_y]);
    let (canceled_x, canceled_y) = point(0.2, 0.6);
    motion.emit_by_name::<()>("motion", &[&canceled_x, &canceled_y]);
    click.emit_by_name::<()>("cancel", &[&None::<gtk4::gdk::EventSequence>]);
    click.emit_by_name::<()>("released", &[&1_i32, &canceled_x, &canceled_y]);
    assert!(
        actions.borrow().is_empty(),
        "canceled drag must not author an action"
    );

    motion.emit_by_name::<()>("motion", &[&node_x, &node_y]);
    click.emit_by_name::<()>("pressed", &[&1_i32, &node_x, &node_y]);
    let (drag_x, drag_y) = point(0.2, 0.6);
    motion.emit_by_name::<()>("motion", &[&drag_x, &drag_y]);
    assert!(
        actions.borrow().is_empty(),
        "live drag motion is release-coalesced"
    );
    click.emit_by_name::<()>("released", &[&1_i32, &drag_x, &drag_y]);
    assert_eq!(
        actions.borrow().len(),
        1,
        "one release authors exactly one action"
    );
    let drag = actions.borrow()[0];
    assert_eq!(drag.target(), operation_id);
    assert_eq!(drag.expected_revision(), initial_revision);
    assert_close(f64::from(drag.parameters().curves[0][0].x), 0.2);
    assert_close(f64::from(drag.parameters().curves[0][0].y), 0.6);

    assert!(key.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk4::gdk::Key::Up,
            &0_u32,
            &gtk4::gdk::ModifierType::empty(),
        ],
    ));
    assert_eq!(
        actions.borrow().len(),
        2,
        "arrow key movement authors one action"
    );
    assert_eq!(
        actions.borrow()[1].expected_revision(),
        Revision::from_u64(42)
    );

    let current = actions.borrow()[1].parameters().curves[0][0];
    let (current_x, current_y) = point(f64::from(current.x), f64::from(current.y));
    motion.emit_by_name::<()>("motion", &[&current_x, &current_y]);
    secondary.emit_by_name::<()>("pressed", &[&1_i32, &current_x, &current_y]);
    assert_eq!(
        actions.borrow().len(),
        3,
        "secondary node deletion authors one action"
    );
    assert_eq!(actions.borrow()[2].parameters().curve_num_nodes[0], 1);

    let edit_by_area = widget::<gtk4::CheckButton>(&root, "colorzones-edit-by-area");
    edit_by_area.set_active(true);
    let remaining = actions.borrow()[2].parameters().curves[0][0];
    let (remaining_x, remaining_y) = point(f64::from(remaining.x), f64::from(remaining.y));
    secondary.emit_by_name::<()>("pressed", &[&1_i32, &remaining_x, &remaining_y]);
    assert_eq!(
        actions.borrow().len(),
        3,
        "edit-by-area secondary press must not hit-test a node at the event position"
    );

    let (area_x, area_y) = point(0.5, 0.8);
    motion.emit_by_name::<()>("motion", &[&area_x, &area_y]);
    click.emit_by_name::<()>("pressed", &[&1_i32, &area_x, &area_y]);
    let (area_drag_x, area_drag_y) = point(0.55, 0.9);
    motion.emit_by_name::<()>("motion", &[&area_drag_x, &area_drag_y]);
    assert_eq!(
        actions.borrow().len(),
        3,
        "area edit stays release-coalesced"
    );
    click.emit_by_name::<()>("released", &[&1_i32, &area_drag_x, &area_drag_y]);
    assert_eq!(
        actions.borrow().len(),
        4,
        "area edit release authors one action"
    );

    assert_eq!(
        actions.borrow()[3].expected_revision(),
        Revision::from_u64(44)
    );

    let strength = widget::<gtk4::Scale>(&root, "colorzones-strength");
    let strength_click = named_controller(&strength, "dt-bauhaus-main-click")
        .expect("Bauhaus strength settled-gesture controller")
        .downcast::<gtk4::GestureClick>()
        .expect("strength settled-gesture controller type");
    strength_click.emit_by_name::<()>("pressed", &[&1_i32, &0.0_f64, &0.0_f64]);
    strength.set_value(10.0);
    strength.set_value(20.0);
    assert_eq!(
        actions.borrow().len(),
        4,
        "live strength changes stay release-coalesced"
    );
    strength_click.emit_by_name::<()>("released", &[&1_i32, &0.0_f64, &0.0_f64]);
    assert_eq!(
        actions.borrow().len(),
        5,
        "one strength release authors exactly one action"
    );
    assert_eq!(
        actions.borrow()[4].expected_revision(),
        Revision::from_u64(45)
    );
    assert_close(f64::from(actions.borrow()[4].parameters().strength), 20.0);

    let bottom = widget::<gtk4::DrawingArea>(&root, "colorzones-bottom-strip");
    let bottom_click = named_controller(&bottom, "dt-colorzones-bottom-click")
        .expect("bottom reset controller")
        .downcast::<gtk4::GestureClick>()
        .expect("bottom reset type");
    bottom_click.emit_by_name::<()>("pressed", &[&2_i32, &0.0_f64, &0.0_f64]);
    assert_eq!(
        actions.borrow().len(),
        5,
        "bottom-strip double-click resets only transient graph view"
    );

    assert!(!shell.window().is_visible());
    assert!(!shell.window().is_active());
    shell.window().close();
    settle_gtk();
}

fn fractional_smooth_scroll_units_are_shared_across_distinct_graphs() {
    let first = build_colorzones_gtk(
        "colorzones-shared-scroll-first",
        gtk_state(
            OperationId::new(9_122).expect("first shared-scroll operation ID"),
            Revision::from_u64(70),
            ColorZonesEditorState::default(),
        ),
        None,
    );
    let second = build_colorzones_gtk(
        "colorzones-shared-scroll-second",
        gtk_state(
            OperationId::new(9_123).expect("second shared-scroll operation ID"),
            Revision::from_u64(80),
            ColorZonesEditorState::default(),
        ),
        None,
    );
    let first_root: gtk4::Widget = first.widget().clone().upcast();
    let second_root: gtk4::Widget = second.widget().clone().upcast();
    let first_graph =
        widget::<gtk4::DrawingArea>(&first_root, "colorzones-shared-scroll-first-graph");
    let second_graph =
        widget::<gtk4::DrawingArea>(&second_root, "colorzones-shared-scroll-second-graph");
    let first_controller = named_controller(&first_graph, "dt-colorzones-scroll")
        .expect("first shared-scroll controller");
    let second_controller = named_controller(&second_graph, "dt-colorzones-scroll")
        .expect("second shared-scroll controller");
    assert_ne!(
        first_controller, second_controller,
        "the regression must cross distinct production controller instances"
    );

    #[cfg(target_os = "macos")]
    let half_unit = 25.0;
    #[cfg(not(target_os = "macos"))]
    let half_unit = 0.5;
    let first_notebook =
        widget::<gtk4::Notebook>(&first_root, "colorzones-shared-scroll-first-channel-tabs");
    let second_notebook =
        widget::<gtk4::Notebook>(&second_root, "colorzones-shared-scroll-second-channel-tabs");

    first.begin_graph_scroll_sequence();
    assert_eq!(
        first
            .route_raw_graph_scroll(
                gtk4::gdk::ScrollUnit::Surface,
                0.0,
                -half_unit,
                gtk4::gdk::ModifierType::ALT_MASK,
            )
            .expect("first fractional smooth delta"),
        None
    );
    assert_eq!(first_notebook.current_page(), Some(0));
    second.begin_graph_scroll_sequence();
    assert!(matches!(
        second
            .route_raw_graph_scroll(
                gtk4::gdk::ScrollUnit::Surface,
                0.0,
                -half_unit,
                gtk4::gdk::ModifierType::ALT_MASK,
            )
            .expect("second fractional smooth delta"),
        Some(ColorZonesScrollOutcome::ForwardToChannelTabs { delta_y })
            if delta_y.to_bits() == (-1.0_f32).to_bits()
    ));
    assert_eq!(
        second_notebook.current_page(),
        Some(1),
        "fractions from distinct graphs emit one combined source unit"
    );
    second.end_graph_scroll_sequence();
    first.end_graph_scroll_sequence();

    first.begin_graph_scroll_sequence();
    assert_eq!(
        first
            .route_raw_graph_scroll(
                gtk4::gdk::ScrollUnit::Surface,
                0.0,
                half_unit,
                gtk4::gdk::ModifierType::ALT_MASK,
            )
            .expect("fraction before stop"),
        None
    );
    first.end_graph_scroll_sequence();
    second.begin_graph_scroll_sequence();
    assert_eq!(
        second
            .route_raw_graph_scroll(
                gtk4::gdk::ScrollUnit::Surface,
                0.0,
                half_unit,
                gtk4::gdk::ModifierType::ALT_MASK,
            )
            .expect("fraction after stop"),
        None,
        "scroll-end resets the shared source remainder"
    );
    second.end_graph_scroll_sequence();
}

fn normalized_smooth_scroll_coalesces_at_the_production_leaf_boundary() {
    let operation_id = OperationId::new(9_121).expect("smooth-scroll operation ID");
    let initial_revision = Revision::from_u64(60);
    let actions = Rc::new(RefCell::new(Vec::<ColorZonesSettledAction>::new()));
    let actions_for_handler = Rc::clone(&actions);
    let handler: ColorZonesGtkActionHandler = Rc::new(move |action| {
        actions_for_handler.borrow_mut().push(action);
        ColorZonesGtkHandlerOutcome::Commit {
            revision: action
                .expected_revision()
                .checked_increment()
                .expect("test revisions do not overflow"),
        }
    });
    let leaf = build_colorzones_gtk(
        "colorzones-smooth-scroll",
        gtk_state(
            operation_id,
            initial_revision,
            ColorZonesEditorState::default(),
        ),
        Some(handler),
    );
    let root: gtk4::Widget = leaf.widget().clone().upcast();
    let graph = widget::<gtk4::DrawingArea>(&root, "colorzones-smooth-scroll-graph");
    graph.allocate(310, 200, -1, None);
    let motion = named_controller(&graph, "dt-colorzones-motion")
        .expect("smooth-scroll motion controller")
        .downcast::<gtk4::EventControllerMotion>()
        .expect("smooth-scroll motion type");
    let click = named_controller(&graph, "dt-colorzones-click")
        .expect("smooth-scroll primary controller")
        .downcast::<gtk4::GestureClick>()
        .expect("smooth-scroll primary type");
    let node_x = 5.0 + 300.0 * 0.25;
    let node_y = 5.0 + 190.0 * 0.5;
    motion.emit_by_name::<()>("motion", &[&node_x, &node_y]);
    click.emit_by_name::<()>("pressed", &[&1_i32, &node_x, &node_y]);
    click.emit_by_name::<()>("released", &[&1_i32, &node_x, &node_y]);
    assert!(actions.borrow().is_empty());

    leaf.begin_graph_scroll_sequence();
    assert_eq!(
        leaf.route_graph_scroll_sequence(-1.0, gtk4::gdk::ModifierType::empty())
            .expect("first normalized smooth unit"),
        ColorZonesScrollOutcome::NodeMoved
    );
    assert_eq!(
        leaf.route_graph_scroll_sequence(-1.0, gtk4::gdk::ModifierType::empty())
            .expect("second normalized smooth unit"),
        ColorZonesScrollOutcome::NodeMoved
    );
    assert!(
        actions.borrow().is_empty(),
        "live smooth units remain inside one production scroll sequence"
    );
    leaf.end_graph_scroll_sequence();
    assert_eq!(actions.borrow().len(), 1);
    assert_eq!(actions.borrow()[0].expected_revision(), initial_revision);
    assert_close(
        f64::from(actions.borrow()[0].parameters().curves[0][0].y),
        0.502,
    );
    leaf.end_graph_scroll_sequence();
    assert_eq!(
        actions.borrow().len(),
        1,
        "a duplicate scroll-end boundary cannot author another action"
    );

    assert_eq!(
        leaf.route_graph_scroll(-1.0, gtk4::gdk::ModifierType::empty())
            .expect("normalized discrete wheel unit"),
        ColorZonesScrollOutcome::NodeMoved
    );
    assert_eq!(
        actions.borrow().len(),
        2,
        "one discrete wheel unit remains one logical commit"
    );

    let edit_by_area = widget::<gtk4::CheckButton>(&root, "colorzones-smooth-scroll-edit-by-area");
    edit_by_area.set_active(true);
    leaf.begin_graph_scroll_sequence();
    assert_eq!(
        leaf.route_graph_scroll_sequence(1.0, gtk4::gdk::ModifierType::empty())
            .expect("normalized radius unit"),
        ColorZonesScrollOutcome::RadiusChanged
    );
    leaf.end_graph_scroll_sequence();
    assert_eq!(
        actions.borrow().len(),
        2,
        "area-radius scrolling changes only transient editor state"
    );
}

#[allow(clippy::too_many_lines)]
fn modifier_routing_preserves_tabs_edits_and_durable_graph_height() {
    let operation_id = OperationId::new(9_120).expect("modifier-routing operation ID");
    let actions = Rc::new(Cell::new(0_usize));
    let actions_for_handler = Rc::clone(&actions);
    let handler: ColorZonesGtkActionHandler = Rc::new(move |_| {
        actions_for_handler.set(actions_for_handler.get() + 1);
        ColorZonesGtkHandlerOutcome::Rollback
    });
    let leaf = build_colorzones_gtk(
        "colorzones-modifier-routing",
        gtk_state(
            operation_id,
            Revision::from_u64(3),
            ColorZonesEditorState::default(),
        ),
        Some(handler),
    );
    let root: gtk4::Widget = leaf.widget().clone().upcast();
    let notebook = widget::<gtk4::Notebook>(&root, "colorzones-modifier-routing-channel-tabs");
    let graph = widget::<gtk4::DrawingArea>(&root, "colorzones-modifier-routing-graph");
    let persisted = Rc::new(RefCell::new(Vec::<ColorZonesGtkPreferences>::new()));
    let persisted_for_handler = Rc::clone(&persisted);
    let preferences_handler: ColorZonesGtkPreferencesHandler = Rc::new(move |preferences| {
        persisted_for_handler.borrow_mut().push(preferences);
        true
    });
    leaf.set_preferences_handler(Some(preferences_handler));

    assert_eq!(notebook.current_page(), Some(0));
    let default_parameters = leaf.state().editor().parameters_value();
    let ordinary = leaf
        .route_graph_scroll(-1.0, gtk4::gdk::ModifierType::empty())
        .expect("finite ordinary wheel unit");
    assert_eq!(ordinary, ColorZonesScrollOutcome::Consumed);
    assert_eq!(notebook.current_page(), Some(0));
    assert_eq!(
        leaf.state().graph_height().logical_pixels(),
        COLORZONES_GRAPH_HEIGHT_DEFAULT
    );
    assert_eq!(leaf.state().editor().parameters_value(), default_parameters);
    assert!(persisted.borrow().is_empty());
    assert_eq!(actions.get(), 0);

    let alt = leaf
        .route_graph_scroll(-1.0, gtk4::gdk::ModifierType::ALT_MASK)
        .expect("finite Alt wheel unit");
    assert!(matches!(
        alt,
        ColorZonesScrollOutcome::ForwardToChannelTabs { delta_y }
            if delta_y.to_bits() == (-1.0_f32).to_bits()
    ));
    assert_eq!(notebook.current_page(), Some(1));
    assert_eq!(
        leaf.state().graph_height().logical_pixels(),
        COLORZONES_GRAPH_HEIGHT_DEFAULT
    );
    assert_eq!(leaf.state().editor().parameters_value(), default_parameters);
    assert_eq!(actions.get(), 0);
    assert_eq!(
        persisted.borrow().as_slice(),
        &[ColorZonesGtkPreferences::new(
            rusttable_processing::ColorZonesChannel::Chroma,
            ColorZonesGraphHeight::default(),
        )]
    );

    let resize = gtk4::gdk::ModifierType::SHIFT_MASK | gtk4::gdk::ModifierType::ALT_MASK;
    assert_eq!(
        leaf.route_graph_scroll(1.0, resize)
            .expect("finite Shift+Alt wheel unit"),
        ColorZonesScrollOutcome::Consumed
    );
    assert_eq!(notebook.current_page(), Some(1));
    assert_eq!(
        leaf.state().graph_height().logical_pixels(),
        COLORZONES_GRAPH_HEIGHT_DEFAULT + 1
    );
    assert_eq!(
        graph.content_height(),
        i32::from(COLORZONES_GRAPH_HEIGHT_DEFAULT + 1)
    );
    assert_eq!(leaf.state().editor().parameters_value(), default_parameters);
    assert_eq!(actions.get(), 0);
    assert_eq!(
        persisted.borrow().last().copied(),
        Some(ColorZonesGtkPreferences::new(
            rusttable_processing::ColorZonesChannel::Chroma,
            ColorZonesGraphHeight::new(COLORZONES_GRAPH_HEIGHT_DEFAULT + 1)
                .expect("incremented graph height"),
        )),
        "Shift+Alt persists only the graph height"
    );

    assert_eq!(
        leaf.route_graph_scroll(f64::from(u16::MAX), resize)
            .expect("finite maximum height request"),
        ColorZonesScrollOutcome::Consumed
    );
    assert_eq!(
        leaf.state().graph_height().logical_pixels(),
        COLORZONES_GRAPH_HEIGHT_MAX
    );
    assert_eq!(
        graph.content_height(),
        i32::from(COLORZONES_GRAPH_HEIGHT_MAX)
    );
    assert_eq!(notebook.current_page(), Some(1));
    assert_eq!(
        persisted.borrow().last().copied(),
        Some(ColorZonesGtkPreferences::new(
            rusttable_processing::ColorZonesChannel::Chroma,
            ColorZonesGraphHeight::new(COLORZONES_GRAPH_HEIGHT_MAX).expect("maximum graph height"),
        ))
    );

    assert_eq!(
        leaf.route_graph_scroll(-f64::from(u16::MAX), resize)
            .expect("finite minimum height request"),
        ColorZonesScrollOutcome::Consumed
    );
    assert_eq!(
        leaf.state().graph_height().logical_pixels(),
        COLORZONES_GRAPH_HEIGHT_MIN
    );
    assert_eq!(
        graph.content_height(),
        i32::from(COLORZONES_GRAPH_HEIGHT_MIN)
    );
    assert_eq!(notebook.current_page(), Some(1));
    assert_eq!(leaf.state().editor().parameters_value(), default_parameters);
    assert_eq!(actions.get(), 0);
    assert_eq!(
        persisted.borrow().last().copied(),
        Some(ColorZonesGtkPreferences::new(
            rusttable_processing::ColorZonesChannel::Chroma,
            ColorZonesGraphHeight::new(COLORZONES_GRAPH_HEIGHT_MIN).expect("minimum graph height"),
        ))
    );

    let durable = leaf.state().graph_height();
    leaf.set_preferences_handler(Some(Rc::new(|_| false)));
    assert_eq!(
        leaf.route_graph_scroll(1.0, resize)
            .expect("rejected height preference"),
        ColorZonesScrollOutcome::Consumed
    );
    assert_eq!(leaf.state().graph_height(), durable);
    assert_eq!(graph.content_height(), i32::from(durable.logical_pixels()));
}

fn module_snapshot(state: ColorZonesGtkState) -> DarkroomModuleViewModel {
    let module = DarkroomModuleViewModel::new(
        "colorzones",
        COLORZONES_TITLE,
        DarkroomModuleSide::Right,
        true,
        state.enabled(),
        true,
        state.revision(),
        Vec::new(),
    )
    .expect("Color Zones module")
    .with_description(COLORZONES_DESCRIPTION)
    .with_group_keys(["group.color", "group.grading"]);
    let module = if state.materialization_required() {
        module
    } else {
        module.with_operation_instance(state.operation_id(), 0, 1)
    };
    module.with_colorzones_editor_state(state)
}

fn gtk_state(
    operation_id: OperationId,
    revision: Revision,
    editor: ColorZonesEditorState,
) -> ColorZonesGtkState {
    ColorZonesGtkState::new(
        operation_id,
        revision,
        editor,
        true,
        OperationOpacity::ONE,
        true,
        false,
    )
}

fn direct_children(root: &gtk4::Box) -> Vec<gtk4::Widget> {
    let mut children = Vec::new();
    let mut child = root.first_child();
    while let Some(current) = child {
        children.push(current.clone());
        child = current.next_sibling();
    }
    children
}

fn assert_direct_child_order<const N: usize>(root: &gtk4::Box, expected: [&str; N]) {
    let children = direct_children(root);
    assert_eq!(children.len(), expected.len());
    for (child, expected_id) in children.iter().zip(expected) {
        assert!(
            find_widget(child, expected_id).is_some(),
            "direct child {} must own {expected_id}",
            child.widget_name()
        );
    }
}

fn assert_bauhaus_label<W: IsA<gtk4::Widget>>(control: &W, expected: &str) {
    let mut ancestor = control.as_ref().parent();
    let bauhaus = loop {
        let candidate = ancestor.expect("control is inside a full-width Bauhaus composite");
        if candidate.has_css_class("dt_bauhaus") {
            break candidate;
        }
        ancestor = candidate.parent();
    };
    let label = find_widget(&bauhaus, "bauhaus-combobox-label")
        .or_else(|| find_widget(&bauhaus, "bauhaus-slider-label"))
        .expect("Bauhaus internal label")
        .downcast::<gtk4::Label>()
        .expect("Bauhaus internal label type");
    assert_eq!(label.text(), expected);
}

fn assert_dropdown(dropdown: &gtk4::DropDown, expected: &[&str]) {
    let model = dropdown.model().expect("dropdown string model");
    let actual = (0..model.n_items())
        .map(|index| {
            model
                .item(index)
                .expect("dropdown item")
                .downcast::<gtk4::StringObject>()
                .expect("dropdown string item")
                .string()
                .to_string()
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn widget<T: IsA<gtk4::Widget>>(root: &impl IsA<gtk4::Widget>, id: &str) -> T {
    find_widget(root.as_ref(), id)
        .unwrap_or_else(|| panic!("{id} production widget"))
        .downcast::<T>()
        .unwrap_or_else(|_| panic!("{id} production widget type"))
}

fn named_controller<W: IsA<gtk4::Widget>>(widget: &W, name: &str) -> Option<gtk4::EventController> {
    let controllers = widget.observe_controllers();
    (0..controllers.n_items()).find_map(|index| {
        let controller = controllers
            .item(index)?
            .downcast::<gtk4::EventController>()
            .ok()?;
        (controller.name().as_deref() == Some(name)).then_some(controller)
    })
}

fn settle_gtk() {
    let context = gtk4::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn assert_close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() <= 1.0e-5,
        "expected {expected:?}, got {actual:?}"
    );
}

fn find_widget(root: &gtk4::Widget, name: &str) -> Option<gtk4::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        if let Some(found) = find_widget(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
