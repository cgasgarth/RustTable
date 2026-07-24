#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use rusttable_ui::{
    RawDenoiseAction, RawDenoisePanel, RawDenoiseViewModel, RgbDenoiseAction, RgbDenoisePanel,
    RgbDenoiseViewModel,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the neural-restore strength regression");
    prohibit_macos_test_activation();
    source_strength_controls_use_bauhaus();
    println!("Neural-restore strength GTK boundary passed");
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

fn source_strength_controls_use_bauhaus() {
    // `src/libs/neural_restore.c:4227-4229,4244-4246`: each persisted key
    // falls back independently to full model strength.
    assert_eq!(RawDenoiseViewModel::default().strength(), 100);
    assert_eq!(RgbDenoiseViewModel::default().strength(), 100);

    let raw = RawDenoisePanel::new();
    let rgb = RgbDenoisePanel::new();
    let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    content.append(raw.widget());
    content.append(rgb.widget());
    let window = gtk4::Window::new();
    window.set_decorated(false);
    window.set_opacity(0.0);
    window.set_default_size(1_200, 900);
    window.set_child(Some(&content));
    window.present();
    settle_gtk();
    assert!(
        !window.is_active(),
        "transparent GTK boundary window must not activate or steal focus"
    );

    let raw_root: gtk4::Widget = raw.widget().clone().upcast();
    let rgb_root: gtk4::Widget = rgb.widget().clone().upcast();
    let raw_strength = strength_scale(&raw_root, "raw-denoise-strength");
    let rgb_strength = strength_scale(&rgb_root, "rgb-denoise-strength");

    // `neural_restore.c:4229-4233,4246-4250`: hard/soft 0..100,
    // explicit step 1, digits 0, and the literal percent suffix.
    for scale in [&raw_strength, &rgb_strength] {
        assert_eq!(
            (scale.adjustment().lower(), scale.adjustment().upper()),
            (0.0, 100.0)
        );
        assert_close(scale.adjustment().step_increment(), 1.0);
        assert_eq!(scale.digits(), 0);
        assert_close(scale.value(), 100.0);
        assert_percent_popup(scale);
    }

    let raw_actions = Rc::new(RefCell::new(Vec::<RawDenoiseAction>::new()));
    let raw_actions_for_handler = Rc::clone(&raw_actions);
    raw.connect_action(move |action| raw_actions_for_handler.borrow_mut().push(action));
    let rgb_actions = Rc::new(RefCell::new(Vec::<RgbDenoiseAction>::new()));
    let rgb_actions_for_handler = Rc::clone(&rgb_actions);
    rgb.connect_action(move |action| rgb_actions_for_handler.borrow_mut().push(action));

    raw_strength.set_value(42.6);
    assert_close(raw_strength.value(), 43.0);
    assert_close_with_context(
        rgb_strength.value(),
        100.0,
        "RAW and RGB saved/current strength state must remain independent",
    );
    rgb_strength.set_value(7.4);
    assert_close(rgb_strength.value(), 7.0);
    assert_close_with_context(
        raw_strength.value(),
        43.0,
        "RGB changes must not overwrite RAW strength",
    );
    assert!(
        raw_actions
            .borrow()
            .iter()
            .any(|action| matches!(action, RawDenoiseAction::SetStrength(43)))
    );
    assert!(
        rgb_actions
            .borrow()
            .iter()
            .any(|action| matches!(action, RgbDenoiseAction::SetStrength(7)))
    );

    window.close();
    settle_gtk();
}

fn strength_scale(root: &gtk4::Widget, name: &str) -> gtk4::Scale {
    let scale = find_widget(root, name)
        .unwrap_or_else(|| panic!("{name} production control"))
        .downcast::<gtk4::Scale>()
        .unwrap_or_else(|_| panic!("{name} is a GTK scale"));
    assert!(
        scale
            .parent()
            .is_some_and(|parent| parent.is::<gtk4::Overlay>()),
        "{name} must be routed through the shared Bauhaus composite"
    );
    scale
}

fn assert_percent_popup(scale: &gtk4::Scale) {
    let slider_root = scale.parent().expect("scale belongs to Bauhaus composite");
    let anchor = find_widget(&slider_root, "bauhaus-slider-anchor")
        .expect("Bauhaus popup anchor")
        .downcast::<gtk4::MenuButton>()
        .expect("Bauhaus anchor type");
    let popup = anchor.popover().expect("Bauhaus popup");
    let popup_root: gtk4::Widget = popup.clone().upcast();
    let current_value = find_widget(&popup_root, "bauhaus-slider-current-value")
        .expect("Bauhaus current-value label")
        .downcast::<gtk4::Label>()
        .expect("Bauhaus current-value label type");
    let open_controller = scale
        .observe_controllers()
        .into_iter()
        .flatten()
        .filter_map(|controller| controller.downcast::<gtk4::EventController>().ok())
        .find(|controller| controller.name().as_deref() == Some("dt-bauhaus-open"))
        .expect("Bauhaus popup opener")
        .downcast::<gtk4::EventControllerKey>()
        .expect("Bauhaus popup opener type");
    let propagation = open_controller.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk4::gdk::Key::Return,
            &0_u32,
            &gtk4::gdk::ModifierType::empty(),
        ],
    );
    assert!(propagation);
    assert!(popup.is_visible());
    assert_eq!(
        current_value.text(),
        "100%",
        "source literal percent format must reach the production Bauhaus popup"
    );
    popup.popdown();
}

fn settle_gtk() {
    let context = gtk4::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

fn assert_close(actual: f64, expected: f64) {
    assert_close_with_context(actual, expected, "source numeric contract");
}

fn assert_close_with_context(actual: f64, expected: f64, context: &str) {
    assert!(
        (actual - expected).abs() <= f64::EPSILON,
        "{context}: expected {expected:?}, got {actual:?}"
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
