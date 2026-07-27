#![forbid(unsafe_code)]

use std::{cell::RefCell, rc::Rc};

use gtk4::prelude::*;
use rusttable_ui::{
    DarkroomControlValue, DarkroomModuleActionHandler, GtkShell, WorkspaceRole,
    install_darktable_theme, reference_modules,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the Velvia callback regression");
    prohibit_macos_test_activation();
    let display = gtk4::gdk::Display::default().expect("GTK boundary needs the default display");
    install_darktable_theme(&display);
    velvia_source_projection_uses_bauhaus_and_routes_state();
    println!("Velvia GTK boundary passed");
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
fn velvia_source_projection_uses_bauhaus_and_routes_state() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.velvia-boundary"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("Velvia test application must register");
    let shell = GtkShell::new(&application);
    let modules = reference_modules().expect("registry descriptor module snapshot");
    let state = Rc::new(RefCell::new(
        modules.module("velvia").expect("Velvia module").clone(),
    ));
    let state_for_handler = Rc::clone(&state);
    let handler: DarkroomModuleActionHandler =
        Rc::new(move |action| state_for_handler.borrow_mut().apply(action));
    shell.set_darkroom_module_stack(&modules, Some(handler));
    shell.show_workspace(WorkspaceRole::Darkroom);
    shell.window().set_opacity(0.0);
    shell.window().set_default_size(1_200, 900);
    shell.window().present();
    settle_gtk();
    assert!(
        !shell.window().is_active(),
        "transparent GTK boundary window must not activate or steal focus"
    );

    let root: gtk4::Widget = shell.window().clone().upcast();
    let search = find_widget(&root, "darkroom-module-search")
        .expect("darkroom module search")
        .downcast::<gtk4::SearchEntry>()
        .expect("module search control type");
    search.set_text("saturation");
    search.emit_by_name::<()>("search-changed", &[]);
    settle_gtk();

    let velvia = find_widget(&root, "velvia")
        .expect("Velvia alias search result")
        .downcast::<gtk4::Expander>()
        .expect("Velvia module expander type");
    assert!(!velvia.is_expanded());
    let title_root = velvia.label_widget().expect("Velvia source title");
    let title = find_widget(&title_root, "velvia-label")
        .expect("Velvia module title")
        .downcast::<gtk4::Label>()
        .expect("Velvia title type");
    assert_eq!(title.text(), "velvia");
    let icon = find_widget(&title_root, "velvia-icon")
        .expect("bundled Velvia source icon")
        .downcast::<gtk4::Image>()
        .expect("Velvia icon type");
    assert!(
        icon.paintable().is_some(),
        "data/pixmaps/plugins/darkroom/velvia.svg must back the production title"
    );

    let velvia_content = velvia.child().expect("Velvia module content");
    let enabled = find_widget(&title_root, "velvia-enabled")
        .expect("Velvia header enable control")
        .downcast::<gtk4::CheckButton>()
        .expect("Velvia enable control type");
    assert!(!enabled.is_active());
    for body_duplicate in [
        "velvia-enabled",
        "velvia-reset",
        "velvia-status",
        "velvia-recover",
    ] {
        assert!(
            find_widget(&velvia_content, body_duplicate).is_none(),
            "{body_duplicate} belongs only to the shared source header"
        );
    }
    assert!(
        find_widget(&root, "velvia-presets").is_none(),
        "an unavailable preset must not be replaced by an inert placeholder"
    );

    let strength = velvia_scale(&velvia_content, "velvia-strength-widget");
    let bias = velvia_scale(&velvia_content, "velvia-bias-widget");
    assert_slider(
        &strength,
        (0.0, 100.0),
        25.0,
        1.0,
        2,
        "the strength of saturation boost",
    );
    assert_slider(
        &bias,
        (0.0, 1.0),
        1.0,
        0.01,
        2,
        "how much to spare highlights and shadows",
    );
    assert!(!strength.is_sensitive());
    assert!(!bias.is_sensitive());

    enabled.set_active(true);
    settle_gtk();
    assert!(strength.is_sensitive());
    assert!(bias.is_sensitive());
    velvia.set_expanded(true);
    settle_gtk();
    assert_value_text(&strength, "25.00%");
    assert_value_text(&bias, "1.00");
    strength.set_value(60.0);
    bias.set_value(0.4);
    settle_gtk();

    let state = state.borrow();
    assert!(state.enabled());
    assert!(state.expanded());
    assert!(matches!(
        state
            .controls()
            .control("velvia-strength")
            .expect("strength state")
            .value(),
        DarkroomControlValue::Slider(value) if (value - 60.0).abs() < 0.000_01
    ));
    assert!(matches!(
        state
            .controls()
            .control("velvia-bias")
            .expect("bias state")
            .value(),
        DarkroomControlValue::Slider(value) if (value - 0.4).abs() < 0.000_01
    ));

    shell.window().close();
    settle_gtk();
}

fn velvia_scale(root: &gtk4::Widget, id: &str) -> gtk4::Scale {
    let scale = find_widget(root, id)
        .unwrap_or_else(|| panic!("{id} production control"))
        .downcast::<gtk4::Scale>()
        .unwrap_or_else(|_| panic!("{id} is a GTK scale"));
    let composite = scale.parent().expect("Velvia scale Bauhaus composite");
    assert!(
        composite.is::<gtk4::Overlay>(),
        "{id} must leave the provisional generic-control route"
    );
    assert!(
        find_widget(&composite, "bauhaus-slider-anchor").is_some(),
        "{id} must expose the source-qualified Bauhaus popup"
    );
    scale
}

fn assert_slider(
    scale: &gtk4::Scale,
    range: (f64, f64),
    value: f32,
    step: f32,
    digits: i32,
    tooltip: &str,
) {
    assert_eq!(
        (scale.adjustment().lower(), scale.adjustment().upper()),
        range
    );
    assert_close(scale.value(), value);
    assert_close(scale.adjustment().step_increment(), step);
    assert_eq!(scale.digits(), digits);
    assert_eq!(scale.tooltip_text().as_deref(), Some(tooltip));
}

fn assert_value_text(scale: &gtk4::Scale, value_text: &str) {
    assert_eq!(
        scale
            .layout()
            .expect("draw-value Velvia scale owns a text layout")
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

fn assert_close(actual: f64, expected: f32) {
    let expected = f64::from(expected);
    assert!(
        (actual - expected).abs() <= f64::EPSILON,
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
