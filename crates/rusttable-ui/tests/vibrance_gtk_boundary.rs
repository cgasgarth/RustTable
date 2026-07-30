#![forbid(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};
use rusttable_ui::{
    DarkroomControlValue, DarkroomModuleAction, DarkroomModuleActionHandler, DarkroomModuleError,
    DarkroomModuleViewModel, DarkroomModulesViewModel, GtkShell, WorkspaceRole,
    install_darktable_theme, reference_modules,
};

const DEPRECATION_MESSAGE: &str = "this module is deprecated. please use the vibrance slider in the color balance rgb module instead.";

fn main() {
    gtk4::init().expect("GTK must initialize for the Vibrance callback regression");
    prohibit_macos_test_activation();
    let display = gtk4::gdk::Display::default().expect("GTK boundary needs the default display");
    install_darktable_theme(&display);
    vibrance_uses_one_deprecated_bauhaus_slider_and_exact_operation_target();
    slider_snapshot_bridge_preserves_live_controllers_and_revision();
    println!("Vibrance GTK boundary passed");
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
fn vibrance_uses_one_deprecated_bauhaus_slider_and_exact_operation_target() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.vibrance-boundary"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("Vibrance test application must register");
    let shell = GtkShell::new(&application);
    shell.window().set_focusable(false);
    shell.window().set_opacity(0.0);
    shell.window().set_default_size(1_200, 900);

    let operation_id = OperationId::new(719).expect("Vibrance operation id");
    let template = reference_modules()
        .expect("registry descriptor module snapshot")
        .module("vibrance")
        .expect("Vibrance module")
        .clone()
        .with_operation_instance(operation_id, 0, 1);
    let modules =
        DarkroomModulesViewModel::new(vec![template.clone()]).expect("Vibrance module snapshot");
    let state = Rc::new(RefCell::new(template));
    let action_count = Rc::new(Cell::new(0_usize));
    let state_for_handler = Rc::clone(&state);
    let action_count_for_handler = Rc::clone(&action_count);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        assert_eq!(
            action.operation_id(),
            Some(operation_id),
            "every GTK action must target the persisted Vibrance operation"
        );
        action_count_for_handler.set(action_count_for_handler.get() + 1);
        state_for_handler.borrow_mut().apply(action)
    });
    shell.set_darkroom_module_stack(&modules, Some(Rc::clone(&handler)));
    shell.show_workspace(WorkspaceRole::Darkroom);
    settle_gtk();
    assert!(
        !shell.window().is_visible(),
        "Vibrance boundary window must remain unshown"
    );
    assert!(
        !shell.window().is_active(),
        "unshown Vibrance boundary window must not activate"
    );

    let root: gtk4::Widget = shell.window().clone().upcast();
    assert!(
        find_widget(&root, "vibrance").is_none(),
        "deprecated Vibrance must not appear in the default Active group"
    );
    let deprecated = find_widget(&root, "group-deprecated")
        .expect("Deprecated module group")
        .downcast::<gtk4::ToggleButton>()
        .expect("Deprecated group control type");
    deprecated.set_active(true);
    settle_gtk();

    let vibrance = find_widget(&root, "vibrance")
        .expect("Vibrance in Deprecated group")
        .downcast::<gtk4::Expander>()
        .expect("Vibrance module expander type");
    assert!(!vibrance.is_expanded());
    let title_root = vibrance.label_widget().expect("Vibrance title");
    let title = find_widget(&title_root, "vibrance-label")
        .expect("Vibrance label")
        .downcast::<gtk4::Label>()
        .expect("Vibrance label type");
    assert_eq!(title.text(), "vibrance");
    assert_eq!(
        title.tooltip_text().as_deref(),
        Some(DEPRECATION_MESSAGE),
        "the native module title exposes the exact deprecation message"
    );
    assert!(
        find_widget(&title_root, "vibrance-icon").is_none(),
        "native Vibrance has no standalone module icon"
    );
    let instance_actions = find_widget(&title_root, "vibrance-actions")
        .expect("Vibrance instance menu")
        .downcast::<gtk4::MenuButton>()
        .expect("Vibrance instance menu type");
    assert!(instance_actions.is_sensitive());
    assert_eq!(
        instance_actions.tooltip_text().as_deref(),
        Some("multiple instance actions\nright-click creates new instance")
    );
    let instance_menu: gtk4::Widget = instance_actions
        .popover()
        .expect("Vibrance instance popover")
        .upcast();
    assert!(instance_button(&instance_menu, "vibrance-instance-new").is_sensitive());
    for suppressed in [
        "vibrance-instance-duplicate",
        "vibrance-instance-move-up",
        "vibrance-instance-move-down",
    ] {
        assert!(
            find_widget(&instance_menu, suppressed).is_none(),
            "{suppressed} stays hidden until its native state can be reproduced"
        );
    }
    assert!(!instance_button(&instance_menu, "vibrance-instance-delete").is_sensitive());

    let content = vibrance.child().expect("Vibrance content");
    let warning = find_widget(&content, "vibrance-deprecation-warning")
        .expect("Vibrance deprecation warning")
        .downcast::<gtk4::Label>()
        .expect("Vibrance deprecation warning type");
    assert_eq!(warning.text(), DEPRECATION_MESSAGE);
    assert!(
        warning.has_css_class("dt_warning"),
        "the source warning uses Darktable's deprecation warning role"
    );
    assert!(
        find_widget(&content, "vibrance-status").is_none(),
        "the persistent source warning replaces RustTable's transient status row"
    );
    assert!(
        find_widget(&content, "vibrance-recover").is_none(),
        "available deprecated modules do not expose an irrelevant refresh control"
    );
    let enabled = find_widget(&title_root, "vibrance-enabled")
        .expect("Vibrance header enable control")
        .downcast::<gtk4::CheckButton>()
        .expect("Vibrance enable type");
    assert!(!enabled.is_active());
    let reset = find_widget(&title_root, "vibrance-reset")
        .expect("Vibrance header reset")
        .downcast::<gtk4::Button>()
        .expect("Vibrance reset type");
    assert!(reset.is_sensitive());
    assert!(find_widget(&content, "vibrance-enabled").is_none());
    assert!(find_widget(&content, "vibrance-reset").is_none());
    assert!(
        find_widget(&root, "vibrance-presets").is_none(),
        "an unavailable preset must not be replaced by an inert placeholder"
    );

    let amount = source_scale(&content, "vibrance-amount-widget");
    assert_eq!(
        (amount.adjustment().lower(), amount.adjustment().upper()),
        (0.0, 100.0)
    );
    assert_close(amount.value(), 25.0);
    assert_close(amount.adjustment().step_increment(), 1.0);
    assert_eq!(amount.digits(), 2);
    assert_eq!(
        amount.tooltip_text().as_deref(),
        Some("the amount of vibrance")
    );
    assert!(!amount.is_sensitive());
    assert!(
        find_widget(&content, "vibrance-strength-widget").is_none(),
        "native gui_init creates no second slider"
    );

    enabled.set_active(true);
    vibrance.set_expanded(true);
    settle_gtk();
    assert!(amount.is_sensitive());
    assert_value_text(&amount, "25.00%");
    amount.set_value(80.0);
    settle_gtk();
    assert_eq!(
        state
            .borrow()
            .controls()
            .control("vibrance-amount")
            .expect("amount state")
            .value(),
        DarkroomControlValue::Slider(80.0)
    );
    assert_eq!(
        warning.text(),
        DEPRECATION_MESSAGE,
        "successful actions must not overwrite the persistent source warning"
    );

    enabled.set_active(false);
    settle_gtk();
    let before_reset = action_count.get();
    reset.emit_clicked();
    settle_gtk();
    assert_eq!(
        action_count.get(),
        before_reset + 1,
        "reset checkbox synchronization must not dispatch a duplicate enable"
    );
    assert!(enabled.is_active());
    assert!(amount.is_sensitive());
    assert!(state.borrow().enabled());
    assert_eq!(
        state
            .borrow()
            .controls()
            .control("vibrance-amount")
            .expect("reset amount state")
            .value(),
        DarkroomControlValue::Slider(25.0)
    );

    let reset_modules =
        DarkroomModulesViewModel::new(vec![state.borrow().clone()]).expect("reset module snapshot");
    shell.set_darkroom_module_stack(&reset_modules, Some(handler));
    settle_gtk();
    let reprojected_content = find_widget(&root, "vibrance")
        .expect("reprojected Vibrance")
        .downcast::<gtk4::Expander>()
        .expect("reprojected Vibrance type")
        .child()
        .expect("reprojected Vibrance content");
    let reprojected_amount = source_scale(&reprojected_content, "vibrance-amount-widget");
    assert_close(reprojected_amount.value(), 25.0);
    assert_value_text(&reprojected_amount, "25.00%");

    assert_multi_instance_menu_routes_exact_targets(&shell, &root);
    shell.window().close();
    settle_gtk();
}

#[allow(clippy::too_many_lines)]
fn slider_snapshot_bridge_preserves_live_controllers_and_revision() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.vibrance-slider-lifecycle"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("Vibrance lifecycle application must register");
    let shell = GtkShell::new(&application);
    shell.window().set_focusable(false);
    shell.window().set_opacity(0.0);
    shell.window().set_default_size(1_200, 900);

    let first_id = OperationId::new(921).expect("first lifecycle Vibrance id");
    let second_id = OperationId::new(922).expect("second lifecycle Vibrance id");
    let initial_revision = Revision::from_u64(17);
    let first_value = Rc::new(Cell::new(25.0));
    let second_value = Rc::new(Cell::new(25.0));
    let current_revision = Rc::new(Cell::new(initial_revision));
    let template = reference_modules()
        .expect("registry descriptor module snapshot")
        .module("vibrance")
        .expect("Vibrance module")
        .clone();
    let modules = vibrance_instance_stack(
        &template,
        first_id,
        second_id,
        initial_revision,
        first_value.get(),
        second_value.get(),
    );
    let emitted = Rc::new(RefCell::new(Vec::<DarkroomModuleAction>::new()));
    let emitted_for_handler = Rc::clone(&emitted);
    let first_value_for_handler = Rc::clone(&first_value);
    let second_value_for_handler = Rc::clone(&second_value);
    let revision_for_handler = Rc::clone(&current_revision);
    let template_for_handler = template.clone();
    let shell_for_handler = shell.clone();
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        let DarkroomModuleAction::Control {
            module_id,
            operation_id: Some(operation_id),
            expected_revision,
            id,
            value: DarkroomControlValue::Slider(value),
        } = &action
        else {
            panic!("lifecycle fixture accepts only exact Vibrance slider controls");
        };
        assert_eq!(module_id, "vibrance");
        assert_eq!(id, "vibrance-amount");
        assert_eq!(*expected_revision, revision_for_handler.get());
        if *operation_id == first_id {
            first_value_for_handler.set(*value);
        } else if *operation_id == second_id {
            second_value_for_handler.set(*value);
        } else {
            panic!("unexpected Vibrance lifecycle operation {operation_id}");
        }
        emitted_for_handler.borrow_mut().push(action);
        let revision = revision_for_handler
            .get()
            .checked_increment()
            .map_err(|_| DarkroomModuleError::RevisionOverflow)?;
        revision_for_handler.set(revision);
        let snapshot = vibrance_instance_stack(
            &template_for_handler,
            first_id,
            second_id,
            revision,
            first_value_for_handler.get(),
            second_value_for_handler.get(),
        );
        shell_for_handler.update_darkroom_module_stack_snapshot(&snapshot, revision);
        Ok(revision)
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
    search.set_text("vibrance");
    search.emit_by_name::<()>("search-changed", &[]);
    settle_gtk();

    let first_widget_id = format!("vibrance-instance-{first_id}");
    let second_widget_id = format!("vibrance-instance-{second_id}");
    let first_scale_id = format!("{first_widget_id}-amount-widget");
    let second_scale_id = format!("{second_widget_id}-amount-widget");
    let first_scale = source_scale(&root, &first_scale_id);
    let second_scale = source_scale(&root, &second_scale_id);
    first_scale.allocate(400, 40, -1, None);
    let width = f64::from(first_scale.allocated_width().max(1));
    let height = f64::from(first_scale.allocated_height().max(1));
    let primary = named_controller(&first_scale, "dt-bauhaus-main-click")
        .expect("Vibrance primary controller")
        .downcast::<gtk4::GestureClick>()
        .expect("Vibrance primary controller type");
    let motion = named_controller(&first_scale, "dt-bauhaus-main-motion")
        .expect("Vibrance motion controller")
        .downcast::<gtk4::EventControllerMotion>()
        .expect("Vibrance motion controller type");
    let scroll = named_controller(&first_scale, "dt-bauhaus-main-scroll")
        .expect("Vibrance scroll controller")
        .downcast::<gtk4::EventControllerScroll>()
        .expect("Vibrance scroll controller type");

    primary.emit_by_name::<()>("pressed", &[&1_i32, &(width * 0.4), &(height * 0.75)]);
    primary.emit_by_name::<()>("stopped", &[]);
    assert_eq!(emitted.borrow().len(), 1);
    assert_same_live_scale_and_controller(&root, &first_scale_id, &first_scale, &primary);

    motion.emit_by_name::<()>("motion", &[&(width * 0.6), &(height * 0.75)]);
    settle_gtk();
    assert_eq!(emitted.borrow().len(), 2);
    assert_same_live_scale_and_controller(&root, &first_scale_id, &first_scale, &primary);

    motion.emit_by_name::<()>("motion", &[&(width * 0.8), &(height * 0.75)]);
    primary.emit_by_name::<()>("released", &[&1_i32, &(width * 0.8), &(height * 0.75)]);
    settle_gtk();
    assert_eq!(emitted.borrow().len(), 3);
    assert_close(first_value.get(), 80.0);
    assert_same_live_scale_and_controller(&root, &first_scale_id, &first_scale, &primary);

    #[cfg(target_os = "macos")]
    let one_surface_scroll_unit = 50.0_f64;
    #[cfg(not(target_os = "macos"))]
    let one_surface_scroll_unit = 1.0_f64;
    for _ in 0..2 {
        assert!(
            scroll.emit_by_name::<bool>("scroll", &[&0.0_f64, &one_surface_scroll_unit]),
            "Vibrance consumes smooth vertical scroll"
        );
        settle_gtk();
        assert_same_live_scale_and_controller(&root, &first_scale_id, &first_scale, &primary);
    }
    assert_eq!(emitted.borrow().len(), 5);

    let cross_panel_revision = current_revision.get();
    second_scale.set_value(61.0);
    settle_gtk();
    assert!(matches!(
        emitted.borrow().last(),
        Some(DarkroomModuleAction::Control {
            operation_id: Some(id),
            expected_revision,
            value: DarkroomControlValue::Slider(value),
            ..
        }) if *id == second_id
            && *expected_revision == cross_panel_revision
            && (*value - 61.0).abs() <= 1.0e-9
    ));
    assert_same_live_scale_and_controller(&root, &first_scale_id, &first_scale, &primary);

    let persisted_first = first_value.get();
    let rerender_revision = current_revision.get();
    search.emit_by_name::<()>("search-changed", &[]);
    settle_gtk();
    let rerendered_first = source_scale(&root, &first_scale_id);
    assert_ne!(
        rerendered_first, first_scale,
        "explicit search rerender replaces the old presentation"
    );
    assert_close(rerendered_first.value(), persisted_first);
    rerendered_first.set_value(37.0);
    settle_gtk();
    assert!(matches!(
        emitted.borrow().last(),
        Some(DarkroomModuleAction::Control {
            operation_id: Some(id),
            expected_revision,
            value: DarkroomControlValue::Slider(value),
            ..
        }) if *id == first_id
            && *expected_revision == rerender_revision
            && (*value - 37.0).abs() <= 1.0e-9
    ));
    assert!(!shell.window().is_visible());
    assert!(!shell.window().is_active());
    shell.window().close();
    settle_gtk();
}

fn vibrance_instance_stack(
    template: &DarkroomModuleViewModel,
    first_id: OperationId,
    second_id: OperationId,
    revision: Revision,
    first_value: f64,
    second_value: f64,
) -> DarkroomModulesViewModel {
    DarkroomModulesViewModel::new(vec![
        vibrance_instance(template, first_id, 0, revision, first_value),
        vibrance_instance(template, second_id, 1, revision, second_value),
    ])
    .expect("two-instance lifecycle Vibrance snapshot")
}

fn vibrance_instance(
    template: &DarkroomModuleViewModel,
    operation_id: OperationId,
    sequence: usize,
    revision: Revision,
    value: f64,
) -> DarkroomModuleViewModel {
    let mut module = template
        .clone()
        .with_operation_instance(operation_id, sequence, 2);
    module
        .reconcile_operation(
            revision,
            true,
            [(
                "vibrance-amount".to_owned(),
                DarkroomControlValue::Slider(value),
            )],
        )
        .expect("persisted lifecycle Vibrance state");
    module.restore_expanded_presentation(true);
    module
}

fn assert_multi_instance_menu_routes_exact_targets(shell: &GtkShell, root: &gtk4::Widget) {
    let first_id = OperationId::new(811).expect("first Vibrance operation id");
    let second_id = OperationId::new(812).expect("second Vibrance operation id");
    let template = reference_modules()
        .expect("registry descriptor module snapshot")
        .module("vibrance")
        .expect("Vibrance module")
        .clone();
    let modules = DarkroomModulesViewModel::new(vec![
        template.clone().with_operation_instance(first_id, 0, 2),
        template.with_operation_instance(second_id, 1, 2),
    ])
    .expect("two-instance Vibrance snapshot");
    let emitted = Rc::new(RefCell::new(Vec::<DarkroomModuleAction>::new()));
    let emitted_for_handler = Rc::clone(&emitted);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        emitted_for_handler.borrow_mut().push(action.clone());
        Ok(action.expected_revision())
    });
    shell.set_darkroom_module_stack(&modules, Some(handler));
    settle_gtk();

    let first_widget_id = format!("vibrance-instance-{first_id}");
    let second_widget_id = format!("vibrance-instance-{second_id}");
    let first_menu = instance_menu(root, &first_widget_id);
    let second_menu = instance_menu(root, &second_widget_id);
    for suppressed in [
        "instance-duplicate",
        "instance-move-up",
        "instance-move-down",
    ] {
        assert!(
            find_widget(&first_menu, &format!("{first_widget_id}-{suppressed}")).is_none(),
            "{suppressed} stays hidden until its native state can be reproduced"
        );
        assert!(
            find_widget(&second_menu, &format!("{second_widget_id}-{suppressed}")).is_none(),
            "{suppressed} stays hidden until its native state can be reproduced"
        );
    }
    assert!(
        instance_button(&first_menu, &format!("{first_widget_id}-instance-new")).is_sensitive()
    );
    assert!(
        instance_button(&first_menu, &format!("{first_widget_id}-instance-delete")).is_sensitive()
    );
    assert!(
        instance_button(&second_menu, &format!("{second_widget_id}-instance-new")).is_sensitive()
    );
    assert!(
        instance_button(&second_menu, &format!("{second_widget_id}-instance-delete"))
            .is_sensitive()
    );

    instance_button(&first_menu, &format!("{first_widget_id}-instance-new")).emit_clicked();
    instance_button(&first_menu, &format!("{first_widget_id}-instance-delete")).emit_clicked();
    instance_button(&second_menu, &format!("{second_widget_id}-instance-delete")).emit_clicked();
    settle_gtk();

    let emitted = emitted.borrow();
    assert_eq!(emitted.len(), 3);
    assert!(matches!(
        emitted[0],
        DarkroomModuleAction::NewInstance {
            operation_id: Some(id),
            ..
        } if id == first_id
    ));
    assert!(matches!(
        emitted[1],
        DarkroomModuleAction::DeleteInstance {
            operation_id: Some(id),
            ..
        } if id == first_id
    ));
    assert!(matches!(
        emitted[2],
        DarkroomModuleAction::DeleteInstance {
            operation_id: Some(id),
            ..
        } if id == second_id
    ));
}

fn instance_menu(root: &gtk4::Widget, widget_id: &str) -> gtk4::Widget {
    find_widget(root, &format!("{widget_id}-actions"))
        .unwrap_or_else(|| panic!("{widget_id} instance action menu"))
        .downcast::<gtk4::MenuButton>()
        .unwrap_or_else(|_| panic!("{widget_id} instance action menu type"))
        .popover()
        .unwrap_or_else(|| panic!("{widget_id} instance popover"))
        .upcast()
}

fn instance_button(root: &gtk4::Widget, id: &str) -> gtk4::Button {
    find_widget(root, id)
        .unwrap_or_else(|| panic!("{id} instance action"))
        .downcast::<gtk4::Button>()
        .unwrap_or_else(|_| panic!("{id} instance action type"))
}

fn source_scale(root: &gtk4::Widget, id: &str) -> gtk4::Scale {
    let scale = find_widget(root, id)
        .unwrap_or_else(|| {
            panic!(
                "{id} production control; Vibrance widget names: {:?}",
                widget_names_with_prefix(root, "vibrance")
            )
        })
        .downcast::<gtk4::Scale>()
        .unwrap_or_else(|_| panic!("{id} is a GTK scale"));
    let composite = scale.parent().expect("Vibrance scale Bauhaus composite");
    assert!(composite.is::<gtk4::Overlay>());
    assert!(find_widget(&composite, "bauhaus-slider-anchor").is_some());
    scale
}

fn widget_names_with_prefix(root: &gtk4::Widget, prefix: &str) -> Vec<String> {
    let mut names = Vec::new();
    let name = root.widget_name();
    if name.starts_with(prefix) {
        names.push(name.to_string());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        names.extend(widget_names_with_prefix(&current, prefix));
        child = current.next_sibling();
    }
    names
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

fn assert_same_live_scale_and_controller(
    root: &gtk4::Widget,
    scale_id: &str,
    expected_scale: &gtk4::Scale,
    expected_primary: &gtk4::GestureClick,
) {
    let live_scale = source_scale(root, scale_id);
    assert_eq!(
        &live_scale, expected_scale,
        "snapshot-only reconciliation must retain the active scale"
    );
    let live_primary = named_controller(&live_scale, "dt-bauhaus-main-click")
        .expect("live Vibrance primary controller")
        .downcast::<gtk4::GestureClick>()
        .expect("live Vibrance primary controller type");
    assert_eq!(
        &live_primary, expected_primary,
        "snapshot-only reconciliation must retain the active controller"
    );
}

fn assert_value_text(scale: &gtk4::Scale, expected: &str) {
    assert_eq!(
        scale
            .layout()
            .expect("draw-value Vibrance scale owns a text layout")
            .text(),
        expected
    );
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
