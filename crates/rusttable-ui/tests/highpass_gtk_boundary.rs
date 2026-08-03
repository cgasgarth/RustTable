#![forbid(unsafe_code)]

use std::{cell::RefCell, rc::Rc};

use gtk4::prelude::*;
use rusttable_core::OperationId;
use rusttable_ui::{
    DarkroomControlValue, DarkroomModuleAction, DarkroomModuleActionHandler,
    DarkroomModulesViewModel, GtkShell, WorkspaceRole, install_darktable_theme, reference_modules,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the Highpass callback regression");
    prohibit_macos_test_activation();
    let display = gtk4::gdk::Display::default().expect("GTK boundary needs the default display");
    install_darktable_theme(&display);
    highpass_uses_source_ordered_generic_sliders_and_control_actions();
    println!("Highpass GTK boundary passed");
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
    reason = "The Highpass GTK boundary fixture keeps source metadata, disabled defaults, and generic action routing together."
)]
fn highpass_uses_source_ordered_generic_sliders_and_control_actions() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.highpass-boundary"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("Highpass test application must register");
    let shell = GtkShell::new(&application);
    shell.window().set_focusable(false);
    shell.window().set_opacity(0.0);
    shell.window().set_default_size(1_200, 900);

    let operation_id = OperationId::new(1_307).expect("Highpass operation id");
    let template = reference_modules()
        .expect("registry descriptor module snapshot")
        .module("highpass")
        .expect("Highpass module")
        .clone()
        .with_operation_instance(operation_id, 0, 1);
    assert_eq!(template.title(), "highpass");
    assert_eq!(
        template.description(),
        Some("isolate high frequencies in the image")
    );
    assert_eq!(
        template.group_keys().collect::<Vec<_>>(),
        ["group.effect", "group.effects"]
    );
    assert!(!template.enabled(), "native Highpass starts disabled");
    assert!(!template.expanded(), "native Highpass starts collapsed");
    assert_eq!(
        template
            .controls()
            .controls()
            .map(|control| control.id().as_str())
            .collect::<Vec<_>>(),
        ["highpass-sharpness", "highpass-contrast"]
    );

    let modules =
        DarkroomModulesViewModel::new(vec![template.clone()]).expect("Highpass module snapshot");
    let state = Rc::new(RefCell::new(template));
    let emitted = Rc::new(RefCell::new(Vec::<DarkroomModuleAction>::new()));
    let state_for_handler = Rc::clone(&state);
    let emitted_for_handler = Rc::clone(&emitted);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        emitted_for_handler.borrow_mut().push(action.clone());
        state_for_handler.borrow_mut().apply(action)
    });
    shell.set_darkroom_module_stack(&modules, Some(Rc::clone(&handler)));
    shell.show_workspace(WorkspaceRole::Darkroom);
    shell.window().present();
    settle_gtk();
    assert!(!shell.window().is_active());

    let root: gtk4::Widget = shell.window().clone().upcast();
    let search = find_widget(&root, "darkroom-module-search")
        .expect("darkroom module search")
        .downcast::<gtk4::SearchEntry>()
        .expect("module search control type");
    search.set_text("highpass");
    search.emit_by_name::<()>("search-changed", &[]);
    settle_gtk();

    let highpass = find_widget(&root, "highpass")
        .expect("Highpass module panel")
        .downcast::<gtk4::Expander>()
        .expect("Highpass module expander type");
    assert!(!highpass.is_expanded());
    let title_root = highpass.label_widget().expect("Highpass title");
    let title = find_widget(&title_root, "highpass-label")
        .expect("Highpass title label")
        .downcast::<gtk4::Label>()
        .expect("Highpass title type");
    assert_eq!(title.text(), "highpass");
    let enabled = find_widget(&title_root, "highpass-enabled")
        .expect("Highpass enable control")
        .downcast::<gtk4::CheckButton>()
        .expect("Highpass enable control type");
    assert!(!enabled.is_active());

    let content = highpass.child().expect("Highpass module content");
    let sharpness = source_scale(&content, "highpass-sharpness-widget");
    let contrast = source_scale(&content, "highpass-contrast-widget");
    assert_slider(&sharpness, "the sharpness of highpass filter");
    assert_slider(&contrast, "the contrast of highpass filter");
    assert_eq!(
        row_label(&content, "highpass-sharpness").text(),
        "sharpness"
    );
    assert_eq!(
        row_label(&content, "highpass-contrast").text(),
        "contrast boost"
    );
    assert!(!sharpness.is_sensitive());
    assert!(!contrast.is_sensitive());
    assert!(find_widget(&content, "highpass-enabled").is_none());
    assert!(find_widget(&content, "highpass-reset").is_none());
    assert!(
        find_widget(&content, "highpass-third-control-widget").is_none(),
        "native gui_init creates exactly two controls"
    );

    enabled.set_active(true);
    settle_gtk();
    assert!(sharpness.is_sensitive());
    assert!(contrast.is_sensitive());
    assert!(matches!(
        emitted.borrow().last(),
        Some(DarkroomModuleAction::Enable {
            module_id,
            operation_id: Some(id),
            enabled: true,
            ..
        }) if module_id == "highpass" && *id == operation_id
    ));

    sharpness.set_value(77.0);
    settle_gtk();
    assert!(matches!(
        emitted.borrow().last(),
        Some(DarkroomModuleAction::Control {
            module_id,
            operation_id: Some(id),
            id: control_id,
            value: DarkroomControlValue::Slider(value),
            ..
        }) if module_id == "highpass"
            && *id == operation_id
            && control_id == "highpass-sharpness"
            && (*value - 77.0).abs() <= f64::EPSILON
    ));

    contrast.set_value(23.0);
    settle_gtk();
    assert!(matches!(
        emitted.borrow().last(),
        Some(DarkroomModuleAction::Control {
            module_id,
            operation_id: Some(id),
            id: control_id,
            value: DarkroomControlValue::Slider(value),
            ..
        }) if module_id == "highpass"
            && *id == operation_id
            && control_id == "highpass-contrast"
            && (*value - 23.0).abs() <= f64::EPSILON
    ));
    let state = state.borrow();
    assert!(state.enabled());
    assert_eq!(
        state
            .controls()
            .control("highpass-sharpness")
            .expect("sharpness state")
            .value(),
        DarkroomControlValue::Slider(77.0)
    );
    assert_eq!(
        state
            .controls()
            .control("highpass-contrast")
            .expect("contrast state")
            .value(),
        DarkroomControlValue::Slider(23.0)
    );
    drop(state);

    shell.window().close();
    settle_gtk();
}

fn source_scale(root: &gtk4::Widget, id: &str) -> gtk4::Scale {
    let scale = find_widget(root, id)
        .unwrap_or_else(|| panic!("{id} production Highpass control"))
        .downcast::<gtk4::Scale>()
        .unwrap_or_else(|_| panic!("{id} is a GTK scale"));
    let composite = scale.parent().expect("Highpass scale Bauhaus composite");
    assert!(composite.is::<gtk4::Overlay>());
    assert!(find_widget(&composite, "bauhaus-slider-anchor").is_some());
    scale
}

fn row_label(root: &gtk4::Widget, id: &str) -> gtk4::Label {
    find_widget(root, id)
        .unwrap_or_else(|| panic!("{id} Highpass control row"))
        .downcast::<gtk4::Box>()
        .unwrap_or_else(|_| panic!("{id} is a Highpass control row"))
        .first_child()
        .expect("Highpass control row label")
        .downcast::<gtk4::Label>()
        .expect("Highpass control row label type")
}

fn assert_slider(scale: &gtk4::Scale, tooltip: &str) {
    assert_eq!(
        (scale.adjustment().lower(), scale.adjustment().upper()),
        (0.0, 100.0)
    );
    assert_close(scale.adjustment().step_increment(), 1.0);
    assert_eq!(scale.digits(), 2);
    assert_eq!(scale.tooltip_text().as_deref(), Some(tooltip));
    assert_close(scale.value(), 50.0);
    scale.allocate(400, 40, -1, None);
    assert_eq!(
        scale.layout().expect("Highpass scale value layout").text(),
        "50.00%"
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
