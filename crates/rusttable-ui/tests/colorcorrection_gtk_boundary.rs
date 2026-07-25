#![forbid(unsafe_code)]

use std::{cell::RefCell, rc::Rc};

use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};
use rusttable_ui::iop::colorcorrection::{COLORCORRECTION_GRID_TOOLTIP, ColorCorrectionGridState};
use rusttable_ui::{
    DarkroomControlValue, DarkroomModuleAction, DarkroomModuleActionHandler,
    DarkroomModulesViewModel, GtkShell, WorkspaceRole, install_darktable_theme, reference_modules,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the Color Correction boundary");
    prohibit_macos_test_activation();
    let display = gtk4::gdk::Display::default().expect("GTK boundary needs the default display");
    install_darktable_theme(&display);
    colorcorrection_uses_atomic_grid_and_gates_unpersistable_presets_without_showing_a_window();
    targetless_colorcorrection_grid_and_reset_callbacks_remain_available_without_showing_a_window();
    println!("Color Correction GTK boundary passed");
}

#[cfg(target_os = "macos")]
fn prohibit_macos_test_activation() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let marker = MainThreadMarker::new().expect("custom GTK smoke must start on the main thread");
    let application = NSApplication::sharedApplication(marker);
    application.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    assert_eq!(
        application.activationPolicy(),
        NSApplicationActivationPolicy::Prohibited,
        "automated GTK smoke must not activate or steal focus"
    );
}

#[cfg(not(target_os = "macos"))]
fn prohibit_macos_test_activation() {}

#[allow(clippy::too_many_lines)]
fn colorcorrection_uses_atomic_grid_and_gates_unpersistable_presets_without_showing_a_window() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.colorcorrection-boundary"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("Color Correction test application must register");
    let shell = GtkShell::new(&application);
    shell.window().set_focusable(false);
    shell.window().set_opacity(0.0);
    shell.window().set_default_size(1_200, 900);

    let operation_id = OperationId::new(919).expect("Color Correction operation id");
    let mut module = reference_modules()
        .expect("registry descriptor module snapshot")
        .module("colorcorrection")
        .expect("Color Correction module")
        .clone()
        .with_operation_instance(operation_id, 0, 1);
    module
        .reconcile_operation(
            Revision::from_u64(7),
            false,
            [(
                "colorcorrection-saturation".to_owned(),
                DarkroomControlValue::Slider(4.25),
            )],
        )
        .expect("persisted saturation");
    module
        .reconcile_color_correction_grid(
            Revision::from_u64(7),
            ColorCorrectionGridState::new(4.0, 5.0, -6.0, -7.0).expect("persisted endpoint state"),
        )
        .expect("persisted grid");
    let modules =
        DarkroomModulesViewModel::new(vec![module.clone()]).expect("Color Correction snapshot");
    let state = Rc::new(RefCell::new(module));
    let emitted = Rc::new(RefCell::new(Vec::<DarkroomModuleAction>::new()));
    let state_for_handler = Rc::clone(&state);
    let emitted_for_handler = Rc::clone(&emitted);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        assert_eq!(
            action.operation_id(),
            Some(operation_id),
            "every production callback must carry the exact persisted operation"
        );
        emitted_for_handler.borrow_mut().push(action.clone());
        state_for_handler.borrow_mut().apply(action)
    });
    shell.set_darkroom_module_stack(&modules, Some(handler));
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
    let search = find_widget(&root, "darkroom-module-search")
        .expect("darkroom module search")
        .downcast::<gtk4::SearchEntry>()
        .expect("module search type");
    search.set_text("color correction");
    search.emit_by_name::<()>("search-changed", &[]);
    settle_gtk();

    let panel = find_widget(&root, "colorcorrection")
        .expect("Color Correction search result")
        .downcast::<gtk4::Expander>()
        .expect("Color Correction panel type");
    let title_root = panel.label_widget().expect("Color Correction title");
    let title = find_widget(&title_root, "colorcorrection-label")
        .expect("Color Correction title label")
        .downcast::<gtk4::Label>()
        .expect("Color Correction title type");
    assert_eq!(title.text(), "color correction");
    assert!(
        find_widget(&title_root, "colorcorrection-icon").is_none(),
        "native module has no invented standalone icon"
    );
    let content = panel.child().expect("Color Correction content");
    let grid = find_widget(&content, "colorcorrection-grid")
        .expect("atomic endpoint grid")
        .downcast::<gtk4::DrawingArea>()
        .expect("endpoint grid drawing area");
    assert_eq!(
        grid.tooltip_text().as_deref(),
        Some(COLORCORRECTION_GRID_TOOLTIP)
    );
    assert!(grid.hexpands());
    assert_eq!(grid.halign(), gtk4::Align::Fill);
    assert!(
        grid.is_sensitive(),
        "native grid remains interactive while the module is disabled"
    );
    for controller in [
        "dt-colorcorrection-motion",
        "dt-colorcorrection-click",
        "dt-colorcorrection-scroll",
        "dt-colorcorrection-key",
    ] {
        assert!(
            named_controller(&grid, controller).is_some(),
            "production grid owns {controller}"
        );
    }
    let scroll = named_controller(&grid, "dt-colorcorrection-scroll")
        .expect("grid scroll controller")
        .downcast::<gtk4::EventControllerScroll>()
        .expect("grid scroll type");
    assert!(
        scroll
            .flags()
            .contains(gtk4::EventControllerScrollFlags::BOTH_AXES)
    );

    let saturation = source_scale(&content, "colorcorrection-saturation-widget");
    assert_eq!(
        (
            saturation.adjustment().lower(),
            saturation.adjustment().upper()
        ),
        (-3.0, 3.0)
    );
    assert_close(saturation.value(), 3.0);
    assert_eq!(
        state
            .borrow()
            .controls()
            .control("colorcorrection-saturation")
            .expect("raw saturation state")
            .value(),
        DarkroomControlValue::Slider(4.25),
        "the projected module retains the finite native outlier used by the initial grid draw"
    );
    assert_close(saturation.adjustment().step_increment(), 0.01);
    assert_eq!(saturation.digits(), 2);
    assert_eq!(
        saturation.tooltip_text().as_deref(),
        Some("set the global saturation")
    );
    assert!(
        saturation.is_sensitive(),
        "native saturation remains interactive while the module is disabled"
    );
    let enabled = find_widget(&content, "colorcorrection-enabled")
        .expect("Color Correction enabled toggle")
        .downcast::<gtk4::CheckButton>()
        .expect("enabled toggle type");
    assert!(!enabled.is_active());
    #[cfg(target_os = "macos")]
    let one_surface_scroll_unit = 50.0_f64;
    #[cfg(not(target_os = "macos"))]
    let one_surface_scroll_unit = 1.0_f64;
    assert!(
        scroll.emit_by_name::<bool>("scroll", &[&0.0_f64, &one_surface_scroll_unit]),
        "the native grid consumes vertical saturation scroll"
    );
    settle_gtk();
    assert!(matches!(
        emitted.borrow().last(),
        Some(DarkroomModuleAction::Control {
            operation_id: Some(id),
            expected_revision,
            id: control_id,
            value: DarkroomControlValue::Slider(value),
            ..
        }) if *id == operation_id
            && *expected_revision == Revision::from_u64(7)
            && control_id == "colorcorrection-saturation"
            && (*value - 3.0).abs() < 1.0e-9
    ));
    assert_eq!(
        state
            .borrow()
            .controls()
            .control("colorcorrection-saturation")
            .expect("first-scroll saturation state")
            .value(),
        DarkroomControlValue::Slider(3.0),
        "the first wheel edit starts from raw 4.25 and persists the native 3.0 clamp"
    );
    assert_eq!(state.borrow().revision(), Revision::from_u64(8));
    assert!(state.borrow().enabled());
    assert!(
        enabled.is_active(),
        "the first accepted edit synchronizes the mounted enable toggle"
    );
    assert_eq!(
        emitted.borrow().len(),
        1,
        "automatic enabling must not emit a duplicate Enable action"
    );
    emitted.borrow_mut().clear();
    for invented in ["hia", "hib", "loa", "lob"] {
        assert!(
            find_widget(&content, &format!("colorcorrection-{invented}-widget")).is_none(),
            "{invented} must remain part of the one atomic grid"
        );
    }

    let presets = find_widget(&content, "colorcorrection-presets")
        .expect("Color Correction presets")
        .downcast::<gtk4::Button>()
        .expect("unavailable Color Correction presets use the inert affordance");
    assert!(!presets.is_sensitive());
    assert!(!presets.is_focusable());
    assert_eq!(
        presets.tooltip_text().as_deref(),
        Some(
            "Color Correction presets require RGB-display blend state, which the current edit model cannot persist"
        ),
        "production explains why source-derived presets are gated"
    );

    assert!(enabled.is_active());
    assert!(grid.is_sensitive());
    assert!(saturation.is_sensitive());
    assert!(state.borrow().enabled());
    assert!(emitted.borrow().is_empty());

    grid.allocate(110, 110, -1, None);
    let width = f64::from(grid.allocated_width());
    let height = f64::from(grid.allocated_height());
    assert!(
        width > 2.0 * 5.0 && height > 2.0 * 5.0,
        "the nonactivating boundary allocates usable grid geometry"
    );
    let motion = named_controller(&grid, "dt-colorcorrection-motion")
        .expect("grid motion controller")
        .downcast::<gtk4::EventControllerMotion>()
        .expect("grid motion type");
    let click = named_controller(&grid, "dt-colorcorrection-click")
        .expect("grid click controller")
        .downcast::<gtk4::GestureClick>()
        .expect("grid click type");
    let inner_width = width - 10.0;
    let inner_height = height - 10.0;
    let shadow_x = 5.0 + 0.5 * inner_width * (1.0 - 6.0 / 40.0);
    let shadow_y = 5.0 + 0.5 * inner_height * (1.0 + 7.0 / 40.0);
    let canceled_start = emitted.borrow().len();
    motion.emit_by_name::<()>("motion", &[&shadow_x, &shadow_y]);
    click.emit_by_name::<()>("pressed", &[&1_i32, &shadow_x, &shadow_y]);
    motion.emit_by_name::<()>(
        "motion",
        &[&(5.0 + inner_width * 0.30), &(5.0 + inner_height * 0.70)],
    );
    motion.emit_by_name::<()>(
        "motion",
        &[&(5.0 + inner_width * 0.20), &(5.0 + inner_height * 0.80)],
    );
    click.emit_by_name::<()>("cancel", &[&None::<gtk4::gdk::EventSequence>]);
    click.emit_by_name::<()>(
        "released",
        &[
            &1_i32,
            &(5.0 + inner_width * 0.20),
            &(5.0 + inner_height * 0.80),
        ],
    );
    assert_eq!(
        emitted.borrow().len(),
        canceled_start,
        "cancel restores the origin and a later release emits nothing"
    );

    // Hovering the original shadow endpoint again proves cancellation restored
    // canonical interaction state before this second gesture begins.
    motion.emit_by_name::<()>("motion", &[&shadow_x, &shadow_y]);
    click.emit_by_name::<()>("pressed", &[&1_i32, &shadow_x, &shadow_y]);
    let drag_start = emitted.borrow().len();
    motion.emit_by_name::<()>(
        "motion",
        &[&(5.0 + inner_width * 0.25), &(5.0 + inner_height * 0.75)],
    );
    motion.emit_by_name::<()>(
        "motion",
        &[&(5.0 + inner_width * 0.15), &(5.0 + inner_height * 0.85)],
    );
    assert_eq!(
        emitted.borrow().len(),
        drag_start,
        "motion stays live so the first update cannot destroy the active grid"
    );
    let same_grid = find_widget(&content, "colorcorrection-grid")
        .expect("grid remains in the live panel")
        .downcast::<gtk4::DrawingArea>()
        .expect("live grid type");
    assert_eq!(
        same_grid, grid,
        "two motions stay on the same DrawingArea before persistence"
    );
    assert_eq!(
        named_controller(&same_grid, "dt-colorcorrection-motion")
            .expect("live motion controller")
            .downcast::<gtk4::EventControllerMotion>()
            .expect("live motion type"),
        motion,
        "both motions stay on the same controller callback"
    );
    click.emit_by_name::<()>(
        "released",
        &[
            &1_i32,
            &(5.0 + inner_width * 0.15),
            &(5.0 + inner_height * 0.85),
        ],
    );
    let drag_actions = emitted.borrow();
    assert_eq!(
        drag_actions.len() - drag_start,
        1,
        "release coalesces the live gesture into one persisted grid action"
    );
    let DarkroomModuleAction::ColorCorrectionGrid {
        operation_id: Some(id),
        expected_revision,
        grid: persisted_grid,
        ..
    } = &drag_actions[drag_start]
    else {
        panic!("drag release emits one atomic Color Correction grid action");
    };
    assert_eq!(*id, operation_id);
    assert_eq!(*expected_revision, Revision::from_u64(8));
    assert!(
        persisted_grid.loa() < -20.0 && persisted_grid.lob() < -20.0,
        "the persisted action carries the final, not first, motion"
    );
    drop(drag_actions);
    assert_eq!(state.borrow().revision(), Revision::from_u64(9));

    let reset_start = emitted.borrow().len();
    motion.emit_by_name::<()>("motion", &[&(width - 5.0), &5.0_f64]);
    click.emit_by_name::<()>("pressed", &[&2_i32, &(width - 5.0), &5.0_f64]);
    assert!(matches!(
        emitted.borrow().get(reset_start),
        Some(DarkroomModuleAction::ColorCorrectionResetParameters {
            operation_id: Some(id),
            expected_revision,
            ..
        }) if *id == operation_id && *expected_revision == Revision::from_u64(9)
    ));
    assert_eq!(state.borrow().revision(), Revision::from_u64(10));
    assert!(state.borrow().enabled());
    assert_eq!(
        state.borrow().color_correction_grid(),
        Some(ColorCorrectionGridState::DEFAULT)
    );
    assert_eq!(
        state
            .borrow()
            .controls()
            .control("colorcorrection-saturation")
            .expect("reset saturation")
            .value(),
        DarkroomControlValue::Slider(1.0)
    );

    saturation.set_value(0.5);
    settle_gtk();
    assert!(matches!(
        emitted.borrow().last(),
        Some(DarkroomModuleAction::Control {
            operation_id: Some(id),
            expected_revision,
            id: control_id,
            value: DarkroomControlValue::Slider(value),
            ..
        }) if *id == operation_id
            && *expected_revision == Revision::from_u64(10)
            && control_id == "colorcorrection-saturation"
            && (*value - 0.5).abs() < 1.0e-9
    ));
    assert_eq!(state.borrow().revision(), Revision::from_u64(11));
    assert!(!shell.window().is_visible());
    assert!(!shell.window().is_active());
    shell.window().close();
    settle_gtk();
}

#[allow(clippy::too_many_lines)]
fn targetless_colorcorrection_grid_and_reset_callbacks_remain_available_without_showing_a_window() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.colorcorrection-template-boundary"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("Color Correction template application must register");
    let shell = GtkShell::new(&application);
    shell.window().set_focusable(false);
    shell.window().set_opacity(0.0);
    shell.window().set_default_size(1_200, 900);

    let module = reference_modules()
        .expect("registry descriptor module snapshot")
        .module("colorcorrection")
        .expect("Color Correction template")
        .clone();
    assert_eq!(module.operation_id(), None);
    let modules =
        DarkroomModulesViewModel::new(vec![module.clone()]).expect("template-only snapshot");
    let state = Rc::new(RefCell::new(module));
    let emitted = Rc::new(RefCell::new(Vec::<DarkroomModuleAction>::new()));
    let state_for_handler = Rc::clone(&state);
    let emitted_for_handler = Rc::clone(&emitted);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        assert_eq!(
            action.operation_id(),
            None,
            "the first template callback remains targetless for controller materialization"
        );
        emitted_for_handler.borrow_mut().push(action.clone());
        state_for_handler.borrow_mut().apply(action)
    });
    shell.set_darkroom_module_stack(&modules, Some(handler));
    shell.show_workspace(WorkspaceRole::Darkroom);
    settle_gtk();
    assert!(!shell.window().is_visible());
    assert!(!shell.window().is_active());

    let root: gtk4::Widget = shell.window().clone().upcast();
    let search = find_widget(&root, "darkroom-module-search")
        .expect("darkroom module search")
        .downcast::<gtk4::SearchEntry>()
        .expect("module search type");
    search.set_text("color correction");
    search.emit_by_name::<()>("search-changed", &[]);
    settle_gtk();

    let panel = find_widget(&root, "colorcorrection")
        .expect("Color Correction template panel")
        .downcast::<gtk4::Expander>()
        .expect("Color Correction panel type");
    let title = panel.label_widget().expect("Color Correction title");
    let actions = find_widget(&title, "colorcorrection-actions")
        .expect("targetless title action placeholder");
    assert!(
        !actions.is::<gtk4::MenuButton>(),
        "the targetless template cannot expose an exact-instance MenuButton"
    );
    let actions = actions
        .downcast::<gtk4::Button>()
        .expect("targetless title retains the inert geometry placeholder");
    assert!(!actions.is_sensitive());
    assert!(!actions.is_focusable());
    assert_eq!(
        actions.tooltip_text().as_deref(),
        Some("Presets and module menu unavailable")
    );
    assert!(
        find_widget(&root, "colorcorrection-instance-menu").is_none(),
        "the exact-instance popover is not built before materialization"
    );
    let content = panel.child().expect("Color Correction template content");
    let grid = find_widget(&content, "colorcorrection-grid")
        .expect("targetless atomic endpoint grid")
        .downcast::<gtk4::DrawingArea>()
        .expect("endpoint grid drawing area");
    grid.allocate(110, 110, -1, None);
    let width = f64::from(grid.allocated_width());
    let height = f64::from(grid.allocated_height());
    let inner_width = width - 10.0;
    let inner_height = height - 10.0;
    let center_x = 5.0 + 0.5 * inner_width;
    let center_y = 4.0 + 0.5 * inner_height;
    let next_x = 5.0 + 0.75 * inner_width;
    let next_y = 5.0 + 0.75 * inner_height;
    let motion = named_controller(&grid, "dt-colorcorrection-motion")
        .expect("template grid motion controller")
        .downcast::<gtk4::EventControllerMotion>()
        .expect("grid motion type");
    let click = named_controller(&grid, "dt-colorcorrection-click")
        .expect("template grid click controller")
        .downcast::<gtk4::GestureClick>()
        .expect("grid click type");

    motion.emit_by_name::<()>("motion", &[&center_x, &center_y]);
    click.emit_by_name::<()>("pressed", &[&1_i32, &center_x, &center_y]);
    motion.emit_by_name::<()>("motion", &[&next_x, &next_y]);
    assert!(
        emitted.borrow().is_empty(),
        "targetless drag motion remains live and release-coalesced"
    );
    click.emit_by_name::<()>("released", &[&1_i32, &next_x, &next_y]);
    let actions = emitted.borrow();
    let Some(DarkroomModuleAction::ColorCorrectionGrid {
        operation_id: None,
        expected_revision,
        grid: persisted_grid,
        ..
    }) = actions.first()
    else {
        panic!("targetless drag release emits one materializable grid action");
    };
    assert_eq!(*expected_revision, Revision::ZERO);
    assert!(
        persisted_grid.hia() > 15.0 && persisted_grid.hib() < -15.0,
        "the released action carries the final dragged highlight endpoint"
    );
    assert_eq!(
        actions.len(),
        1,
        "release persists exactly one targetless grid action"
    );
    drop(actions);
    assert_eq!(state.borrow().revision(), Revision::from_u64(1));
    assert!(state.borrow().enabled());

    let empty_x = width - 5.0;
    let empty_y = 5.0;
    motion.emit_by_name::<()>("motion", &[&empty_x, &empty_y]);
    click.emit_by_name::<()>("pressed", &[&2_i32, &empty_x, &empty_y]);
    assert!(matches!(
        emitted.borrow().get(1),
        Some(DarkroomModuleAction::ColorCorrectionResetParameters {
            operation_id: None,
            expected_revision,
            ..
        }) if *expected_revision == Revision::from_u64(1)
    ));
    assert_eq!(emitted.borrow().len(), 2);
    assert_eq!(state.borrow().revision(), Revision::from_u64(2));
    assert_eq!(
        state.borrow().color_correction_grid(),
        Some(ColorCorrectionGridState::DEFAULT)
    );
    assert_eq!(
        state
            .borrow()
            .controls()
            .control("colorcorrection-saturation")
            .expect("reset saturation")
            .value(),
        DarkroomControlValue::Slider(1.0)
    );
    assert!(!shell.window().is_visible());
    assert!(!shell.window().is_active());
    shell.window().close();
    settle_gtk();
}

fn source_scale(root: &gtk4::Widget, id: &str) -> gtk4::Scale {
    let scale = find_widget(root, id)
        .unwrap_or_else(|| panic!("{id} production control"))
        .downcast::<gtk4::Scale>()
        .unwrap_or_else(|_| panic!("{id} is a GTK scale"));
    let composite = scale.parent().expect("Bauhaus slider composite");
    assert!(composite.is::<gtk4::Overlay>());
    assert!(find_widget(&composite, "bauhaus-slider-anchor").is_some());
    scale
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
        (actual - expected).abs() <= 1.0e-6,
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
