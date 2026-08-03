#![forbid(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::prelude::*;
use rusttable_core::OperationId;
use rusttable_ui::{
    DarkroomControlValue, DarkroomModuleActionHandler, DarkroomModulesViewModel, GtkShell,
    WorkspaceRole, install_darktable_theme, reference_modules,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the Color Contrast callback regression");
    prohibit_macos_test_activation();
    let display = gtk4::gdk::Display::default().expect("GTK boundary needs the default display");
    install_darktable_theme(&display);
    colorcontrast_uses_two_bauhaus_sliders_without_an_invented_icon();
    println!("Color Contrast GTK boundary passed");
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

#[expect(
    clippy::too_many_lines,
    reason = "The GTK boundary fixture keeps source Color Contrast controls, hierarchy, and allocation checks together."
)]
fn colorcontrast_uses_two_bauhaus_sliders_without_an_invented_icon() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.colorcontrast-boundary"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("Color Contrast test application must register");
    let shell = GtkShell::new(&application);
    let modules = reference_modules().expect("registry descriptor module snapshot");
    shell.set_darkroom_module_stack(&modules, None);
    shell.show_workspace(WorkspaceRole::Darkroom);
    shell.window().set_focusable(false);
    shell.window().set_opacity(0.0);
    shell.window().set_default_size(1_200, 900);
    shell.window().present();
    settle_gtk();
    assert!(
        !shell.window().is_active(),
        "transparent GTK boundary window must not activate or steal focus"
    );

    let root: gtk4::Widget = shell.window().clone().upcast();
    assert_default_active_filters_controller_owned_colorcontrast(&shell, &root, &modules);

    let state = Rc::new(RefCell::new(
        modules
            .module("colorcontrast")
            .expect("Color Contrast module")
            .clone(),
    ));
    let action_count = Rc::new(Cell::new(0_usize));
    let state_for_handler = Rc::clone(&state);
    let action_count_for_handler = Rc::clone(&action_count);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        action_count_for_handler.set(action_count_for_handler.get() + 1);
        state_for_handler.borrow_mut().apply(action)
    });
    shell.set_darkroom_module_stack(&modules, Some(Rc::clone(&handler)));
    settle_gtk();

    let search = find_widget(&root, "darkroom-module-search")
        .expect("darkroom module search")
        .downcast::<gtk4::SearchEntry>()
        .expect("module search control type");
    search.set_text("saturation");
    search.emit_by_name::<()>("search-changed", &[]);
    settle_gtk();

    let colorcontrast = find_widget(&root, "colorcontrast")
        .expect("Color Contrast alias search result")
        .downcast::<gtk4::Expander>()
        .expect("Color Contrast module expander type");
    assert!(!colorcontrast.is_expanded());
    let title_root = colorcontrast
        .label_widget()
        .expect("generic Color Contrast title");
    let title = find_widget(&title_root, "colorcontrast-label")
        .expect("Color Contrast module title")
        .downcast::<gtk4::Label>()
        .expect("Color Contrast title type");
    assert_eq!(title.text(), "color contrast");
    assert!(
        find_widget(&title_root, "colorcontrast-icon").is_none(),
        "native Color Contrast has no standalone darkroom icon asset"
    );

    let content = colorcontrast
        .child()
        .expect("Color Contrast module content");
    let enabled = find_widget(&title_root, "colorcontrast-enabled")
        .expect("Color Contrast header enable control")
        .downcast::<gtk4::CheckButton>()
        .expect("Color Contrast enable control type");
    assert!(!enabled.is_active());
    let reset = find_widget(&title_root, "colorcontrast-reset")
        .expect("Color Contrast header reset control")
        .downcast::<gtk4::Button>()
        .expect("Color Contrast reset control type");
    assert!(
        reset.is_sensitive(),
        "an available resettable module can reset while disabled"
    );
    for body_duplicate in [
        "colorcontrast-enabled",
        "colorcontrast-reset",
        "colorcontrast-status",
        "colorcontrast-recover",
    ] {
        assert!(
            find_widget(&content, body_duplicate).is_none(),
            "{body_duplicate} belongs only to the shared source header"
        );
    }
    assert!(
        find_widget(&root, "colorcontrast-presets").is_none(),
        "an unavailable preset must not be replaced by an inert placeholder"
    );

    let a_steepness = source_scale(&content, "colorcontrast-a-steepness-widget");
    let b_steepness = source_scale(&content, "colorcontrast-b-steepness-widget");
    assert_slider(
        &a_steepness,
        "steepness of the a* curve in Lab\nlower values desaturate greens and magenta while higher saturate them",
    );
    assert_slider(
        &b_steepness,
        "steepness of the b* curve in Lab\nlower values desaturate blues and yellows while higher saturate them",
    );
    for hidden in [
        "colorcontrast-a-offset-widget",
        "colorcontrast-b-offset-widget",
        "colorcontrast-unbound-widget",
    ] {
        assert!(
            find_widget(&content, hidden).is_none(),
            "native gui_init does not construct {hidden}"
        );
    }
    assert!(!a_steepness.is_sensitive());
    assert!(!b_steepness.is_sensitive());

    enabled.set_active(true);
    settle_gtk();
    assert!(a_steepness.is_sensitive());
    assert!(b_steepness.is_sensitive());
    colorcontrast.set_expanded(true);
    settle_gtk();
    assert_value_text(&a_steepness, "1.00");
    assert_value_text(&b_steepness, "1.00");
    a_steepness.set_value(2.5);
    b_steepness.set_value(3.5);
    settle_gtk();

    let borrowed_state = state.borrow();
    assert!(borrowed_state.enabled());
    assert!(borrowed_state.expanded());
    assert!(matches!(
        borrowed_state
            .controls()
            .control("colorcontrast-a-steepness")
            .expect("a* steepness state")
            .value(),
        DarkroomControlValue::Slider(value) if (value - 2.5).abs() < 0.000_01
    ));
    assert!(matches!(
        borrowed_state
            .controls()
            .control("colorcontrast-b-steepness")
            .expect("b* steepness state")
            .value(),
        DarkroomControlValue::Slider(value) if (value - 3.5).abs() < 0.000_01
    ));
    drop(borrowed_state);

    enabled.set_active(false);
    settle_gtk();
    assert!(!a_steepness.is_sensitive());
    assert!(!b_steepness.is_sensitive());
    assert!(
        reset.is_sensitive(),
        "disabling an available module must not disable its reset action"
    );
    let action_count_before_reset = action_count.get();
    reset.emit_clicked();
    settle_gtk();

    assert_eq!(
        action_count.get(),
        action_count_before_reset + 1,
        "reset-induced checkbox synchronization must not route a duplicate enable action"
    );
    assert!(
        enabled.is_active(),
        "native reset must visibly enable the module"
    );
    assert!(a_steepness.is_sensitive());
    assert!(b_steepness.is_sensitive());
    let borrowed_state = state.borrow();
    assert!(
        borrowed_state.enabled(),
        "native reset must enable the module model"
    );
    for id in ["colorcontrast-a-steepness", "colorcontrast-b-steepness"] {
        assert!(
            matches!(
                borrowed_state
                    .controls()
                    .control(id)
                    .expect("reset Color Contrast control")
                    .value(),
                DarkroomControlValue::Slider(value) if (value - 1.0).abs() < 0.000_01
            ),
            "{id} must reset to its source default"
        );
    }
    drop(borrowed_state);

    let reset_modules =
        DarkroomModulesViewModel::new(vec![state.borrow().clone()]).expect("reset module snapshot");
    shell.set_darkroom_module_stack(&reset_modules, Some(handler));
    settle_gtk();
    let reprojected = find_widget(&root, "colorcontrast")
        .expect("reprojected Color Contrast panel")
        .downcast::<gtk4::Expander>()
        .expect("reprojected Color Contrast panel type");
    let reprojected_content = reprojected
        .child()
        .expect("reprojected Color Contrast content");
    for id in [
        "colorcontrast-a-steepness-widget",
        "colorcontrast-b-steepness-widget",
    ] {
        let scale = source_scale(&reprojected_content, id);
        assert_close(scale.value(), 1.0);
        assert_value_text(&scale, "1.00");
    }
    assert_eq!(
        action_count.get(),
        action_count_before_reset + 1,
        "reprojecting controller state must not route another module action"
    );

    assert_multi_instance_descendant_identity_and_targets(&shell, &root);

    shell.window().close();
    settle_gtk();
}

fn assert_default_active_filters_controller_owned_colorcontrast(
    shell: &GtkShell,
    root: &gtk4::Widget,
    modules: &DarkroomModulesViewModel,
) {
    let search = find_widget(root, "darkroom-module-search")
        .expect("darkroom module search")
        .downcast::<gtk4::SearchEntry>()
        .expect("module search control type");
    let active = find_widget(root, "group-active")
        .expect("default Active module group")
        .downcast::<gtk4::ToggleButton>()
        .expect("Active module group control type");
    assert!(
        active.is_active() && search.text().is_empty(),
        "production shell must begin in default Active without a search override"
    );
    assert!(
        find_widget(root, "colorcontrast").is_none(),
        "disabled Color Contrast must stay out of the default Active group"
    );

    let mut enabled_modules = modules.clone();
    let colorcontrast = enabled_modules
        .module_mut("colorcontrast")
        .expect("Color Contrast module");
    let revision = colorcontrast.revision();
    colorcontrast
        .set_enabled(revision, true)
        .expect("controller enables Color Contrast");
    shell.set_darkroom_module_stack(&enabled_modules, None);
    settle_gtk();
    assert!(
        active.is_active()
            && search.text().is_empty()
            && find_widget(root, "colorcontrast").is_some(),
        "enabled Color Contrast must appear in the default Active group"
    );
    assert!(
        !shell.window().is_active(),
        "Active-group projection must not activate or steal focus"
    );

    shell.set_darkroom_module_stack(modules, None);
    settle_gtk();
    assert!(
        find_widget(root, "colorcontrast").is_none(),
        "restored disabled Color Contrast must leave the default Active group"
    );
}

#[expect(
    clippy::too_many_lines,
    reason = "The GTK boundary fixture keeps multi-instance descendant identity and action-target checks together."
)]
fn assert_multi_instance_descendant_identity_and_targets(shell: &GtkShell, root: &gtk4::Widget) {
    let first_id = OperationId::new(41).expect("first operation id");
    let second_id = OperationId::new(73).expect("second operation id");
    let templates = reference_modules().expect("registry descriptor module snapshot");
    let template = templates
        .module("colorcontrast")
        .expect("Color Contrast module")
        .clone();
    let modules = DarkroomModulesViewModel::new(vec![
        template.clone().with_operation_instance(first_id, 0, 2),
        template.with_operation_instance(second_id, 1, 2),
    ])
    .expect("two source-derived Color Contrast instances");
    let state = Rc::new(RefCell::new(modules.clone()));
    let state_for_handler = Rc::clone(&state);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        let module_id = action.module_id().to_owned();
        let operation_id = action.operation_id();
        let mut modules = state_for_handler.borrow_mut();
        let revision = modules
            .module_target_mut(module_id.as_str(), operation_id)
            .expect("GTK action carries an exact operation target")
            .apply(action)?;
        let snapshots = modules
            .left_modules()
            .chain(modules.right_modules())
            .map(|module| {
                (
                    module.id().to_owned(),
                    module.operation_id(),
                    module.expanded(),
                    module.enabled(),
                    module.controls().controls().cloned().collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        for (module_id, operation_id, expanded, enabled, controls) in snapshots {
            modules
                .module_target_mut(&module_id, operation_id)
                .expect("controller projection retains every exact module target")
                .reconcile_snapshot(revision, expanded, enabled, controls)?;
        }
        Ok(revision)
    });
    shell.set_darkroom_module_stack(&modules, Some(handler));
    settle_gtk();

    let first_widget_id = format!("colorcontrast-instance-{first_id}");
    let second_widget_id = format!("colorcontrast-instance-{second_id}");
    let first_content = module_content(root, &first_widget_id);
    let second_content = module_content(root, &second_widget_id);
    let first_row_id = format!("{first_widget_id}-a-steepness");
    let second_row_id = format!("{second_widget_id}-a-steepness");
    assert!(
        find_widget(&first_content, &first_row_id).is_some_and(|widget| widget.is::<gtk4::Box>()),
        "first instance row uses its panel identity"
    );
    assert!(
        find_widget(&second_content, &second_row_id).is_some_and(|widget| widget.is::<gtk4::Box>()),
        "second instance row uses its panel identity"
    );
    let first_a = source_scale(&first_content, &format!("{first_row_id}-widget"));
    let second_a = source_scale(&second_content, &format!("{second_row_id}-widget"));
    assert!(
        find_widget(root, "colorcontrast-a-steepness-widget").is_none(),
        "a duplicated stack must not expose the ambiguous logical control id as a GTK name"
    );

    let first_enabled = find_widget(root, &format!("{first_widget_id}-enabled"))
        .expect("first instance header enable control")
        .downcast::<gtk4::CheckButton>()
        .expect("first instance enable type");
    let second_enabled = find_widget(root, &format!("{second_widget_id}-enabled"))
        .expect("second instance header enable control")
        .downcast::<gtk4::CheckButton>()
        .expect("second instance enable type");
    assert!(
        find_widget(&first_content, &format!("{first_widget_id}-enabled")).is_none()
            && find_widget(&second_content, &format!("{second_widget_id}-enabled")).is_none(),
        "duplicate instance enables stay in their exact source headers"
    );
    first_enabled.set_active(true);
    second_enabled.set_active(true);
    settle_gtk();
    first_a.set_value(2.25);
    second_a.set_value(3.75);
    settle_gtk();

    let state = state.borrow();
    for (operation_id, expected) in [(first_id, 2.25), (second_id, 3.75)] {
        let instance = state
            .module_target("colorcontrast", Some(operation_id))
            .expect("exact Color Contrast instance");
        assert!(instance.enabled());
        assert!(matches!(
            instance
                .controls()
                .control("colorcontrast-a-steepness")
                .expect("raw persisted control id")
                .value(),
            DarkroomControlValue::Slider(value) if (value - expected).abs() < 0.000_01
        ));
        assert!(matches!(
            instance
                .controls()
                .control("colorcontrast-b-steepness")
                .expect("sibling source control")
                .value(),
            DarkroomControlValue::Slider(value) if (value - 1.0).abs() < 0.000_01
        ));
    }
}

fn module_content(root: &gtk4::Widget, id: &str) -> gtk4::Widget {
    find_widget(root, id)
        .unwrap_or_else(|| panic!("{id} module panel"))
        .downcast::<gtk4::Expander>()
        .unwrap_or_else(|_| panic!("{id} module panel type"))
        .child()
        .unwrap_or_else(|| panic!("{id} module content"))
}

fn source_scale(root: &gtk4::Widget, id: &str) -> gtk4::Scale {
    let scale = find_widget(root, id)
        .unwrap_or_else(|| panic!("{id} production control"))
        .downcast::<gtk4::Scale>()
        .unwrap_or_else(|_| panic!("{id} is a GTK scale"));
    let composite = scale
        .parent()
        .expect("Color Contrast scale Bauhaus composite");
    assert!(
        composite.is::<gtk4::Overlay>(),
        "{id} must use source-qualified Bauhaus presentation"
    );
    assert!(
        find_widget(&composite, "bauhaus-slider-anchor").is_some(),
        "{id} must expose the source-qualified Bauhaus popup"
    );
    scale
}

fn assert_slider(scale: &gtk4::Scale, tooltip: &str) {
    assert_eq!(
        (scale.adjustment().lower(), scale.adjustment().upper()),
        (0.0, 5.0)
    );
    assert_close(scale.value(), 1.0);
    assert_close(scale.adjustment().step_increment(), 0.05);
    assert_eq!(scale.digits(), 2);
    assert_eq!(scale.tooltip_text().as_deref(), Some(tooltip));
}

fn assert_value_text(scale: &gtk4::Scale, value_text: &str) {
    assert_eq!(
        scale
            .layout()
            .expect("draw-value Color Contrast scale owns a text layout")
            .text(),
        value_text
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
