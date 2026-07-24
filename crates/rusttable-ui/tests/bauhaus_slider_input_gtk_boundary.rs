#![forbid(unsafe_code)]

use gtk4::prelude::*;
use rusttable_ui::{ExposurePanel, install_darktable_theme};

fn main() {
    gtk4::init().expect("GTK must initialize for the Bauhaus slider regression");
    prohibit_macos_test_activation();
    let display = gtk4::gdk::Display::default().expect("GTK boundary needs the default display");
    install_darktable_theme(&display);
    shared_production_slider_has_source_input_boundary();
    println!("Bauhaus slider input GTK boundary passed");
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
        "automated GTK smoke must use the non-activating macOS policy"
    );
}

#[cfg(not(target_os = "macos"))]
fn prohibit_macos_test_activation() {}

fn shared_production_slider_has_source_input_boundary() {
    let panel = ExposurePanel::new();
    let window = gtk4::Window::new();
    window.set_decorated(false);
    window.set_opacity(0.0);
    window.set_default_size(1_200, 800);
    window.set_child(Some(panel.widget()));
    window.present();
    settle_gtk();
    assert!(
        !window.is_active(),
        "transparent GTK boundary window must not activate or steal focus"
    );

    let root: gtk4::Widget = panel.widget().clone().upcast();
    let scale = find_widget(&root, "exposure-ev")
        .expect("production Exposure scale")
        .downcast::<gtk4::Scale>()
        .expect("Exposure control is a scale");

    let slider_root = scale
        .parent()
        .expect("scale belongs to the Bauhaus composite");
    assert!(
        slider_root.is::<gtk4::Overlay>(),
        "Bauhaus composite uses a supported GTK overlay"
    );
    let anchor = find_widget(&slider_root, "bauhaus-slider-anchor")
        .expect("production Bauhaus popover anchor")
        .downcast::<gtk4::MenuButton>()
        .expect("Bauhaus anchor is a supported GTK menu button");
    assert_eq!(anchor.width_request(), 1);
    assert_eq!(anchor.height_request(), 1);
    assert_eq!(anchor.allocated_width(), 1);
    assert_eq!(anchor.allocated_height(), 1);
    assert!(anchor.has_css_class("dt_bauhaus_anchor"));
    assert_eq!(
        anchor.accessible_role(),
        gtk4::AccessibleRole::Presentation,
        "invisible popup anchor must not create a phantom accessibility control"
    );
    let popup = anchor
        .popover()
        .expect("menu button owns the Bauhaus popup");
    assert_eq!(popup.widget_name(), "bauhaus-slider");
    assert!(popup.has_css_class("dt_bauhaus_popup"));
    assert!(!popup.has_arrow());
    assert_eq!(
        popup.parent().as_ref(),
        Some(anchor.upcast_ref()),
        "the supported MenuButton API must own the popover"
    );

    let popup_root: gtk4::Widget = popup.clone().upcast();
    let current_value = find_widget(&popup_root, "bauhaus-slider-current-value")
        .expect("popup current-value label")
        .downcast::<gtk4::Label>()
        .expect("current value uses a label");
    assert!(current_value.has_css_class("dt_bauhaus_current_value"));
    let expression = find_widget(&popup_root, "bauhaus-slider-expression")
        .expect("popup expression label")
        .downcast::<gtk4::Label>()
        .expect("source append-only input uses a label, not an editable entry");
    assert!(expression.has_css_class("dt_bauhaus_numeric_text"));

    let controllers = scale.observe_controllers();
    let mut open_controller = None;
    let mut secondary_clicks = 0;
    for index in 0..controllers.n_items() {
        let controller = controllers
            .item(index)
            .expect("controller item")
            .downcast::<gtk4::EventController>()
            .expect("GTK controller type");
        if controller.name().as_deref() == Some("dt-bauhaus-open") {
            open_controller = controller
                .clone()
                .downcast::<gtk4::EventControllerKey>()
                .ok();
        }
        if let Ok(click) = controller.downcast::<gtk4::GestureClick>()
            && click.name().as_deref() == Some("dt-bauhaus-secondary")
            && click.button() == 3
        {
            secondary_clicks += 1;
        }
    }
    assert!(
        open_controller.is_some(),
        "Return must open the source popup"
    );
    assert_eq!(
        secondary_clicks, 1,
        "secondary click must open exactly one source popup"
    );

    scale.set_value(1.0);
    assert!(
        scale.grab_focus(),
        "mapped production scale must accept GTK-local focus"
    );
    settle_gtk();
    let propagation = open_controller
        .expect("source popup opening controller")
        .emit_by_name::<bool>(
            "key-pressed",
            &[
                &gtk4::gdk::Key::Return,
                &0_u32,
                &gtk4::gdk::ModifierType::empty(),
            ],
        );
    assert!(
        propagation,
        "Return must route through the source popup opener"
    );
    // The controller signal is synthetic, so under prohibited macOS activation
    // it cannot retain a real GDK grab across a main-context iteration. Assert
    // the synchronously established production mapping before that test-only
    // autohide artifact runs, then close it through the production input route.
    assert!(popup.is_visible(), "Return must make the popup visible");
    assert!(popup.is_mapped(), "Return must map the popup surface");
    let focused_widget =
        gtk4::prelude::RootExt::focus(&window).expect("mapped popup must own GTK focus");
    assert!(
        popup.has_focus() || popup.is_focus() || focus_is_within_popup(&popup, &focused_widget),
        "GTK focus must remain within the mapped Bauhaus popup"
    );
    assert_popup_geometry(&window, &scale, &popup);
    assert!(
        !window.is_active(),
        "mapped popup must not activate or raise the transparent test window"
    );
    assert_eq!(
        current_value.text(),
        "+1.000 EV",
        "opening must snapshot the source-formatted Exposure value"
    );
    assert!(
        expression.text().is_empty(),
        "opening must clear the append-only expression"
    );

    let input_controller = popup
        .observe_controllers()
        .into_iter()
        .flatten()
        .filter_map(|controller| controller.downcast::<gtk4::EventController>().ok())
        .find(|controller| controller.name().as_deref() == Some("dt-bauhaus-input"))
        .expect("source popup key controller")
        .downcast::<gtk4::EventControllerKey>()
        .expect("source popup controller type");
    for key in [
        gtk4::gdk::Key::x,
        gtk4::gdk::Key::plus,
        gtk4::gdk::Key::_1,
        gtk4::gdk::Key::Return,
    ] {
        let propagation = input_controller.emit_by_name::<bool>(
            "key-pressed",
            &[&key, &0_u32, &gtk4::gdk::ModifierType::empty()],
        );
        assert!(propagation);
    }
    assert!(
        (scale.value() - 2.0).abs() <= f64::EPSILON,
        "x + 1 must route to the real scale"
    );

    popup.popdown();
    settle_gtk();
    window.close();
    settle_gtk();
}

fn assert_popup_geometry(window: &gtk4::Window, scale: &gtk4::Scale, popup: &gtk4::Popover) {
    let surface = popup.surface().expect("mapped popup owns a GDK surface");
    let popup_surface = surface
        .downcast::<gtk4::gdk::Popup>()
        .expect("Bauhaus popover surface implements GDK popup geometry");
    let scale_origin = scale
        .compute_point(window, &gtk4::graphene::Point::new(0.0, 0.0))
        .expect("scale coordinates resolve within the transparent test window");
    let scale_width = scale.allocated_width();
    assert_eq!(popup.width_request(), scale_width);
    assert_eq!(popup.height_request(), scale_width);
    assert!(
        (f64::from(popup_surface.position_x()) - f64::from(scale_origin.x())).abs() <= 2.0,
        "popup northwest x must match the scale within the GTK CSS-border tolerance"
    );
    assert!(
        (f64::from(popup_surface.position_y()) - f64::from(scale_origin.y())).abs() <= 2.0,
        "popup northwest y must match the scale within the GTK CSS-border tolerance"
    );
    assert!(
        popup_surface.width() >= scale_width,
        "mapped popup surface must span the production scale width"
    );
    assert!(
        popup_surface.height() >= scale.allocated_height(),
        "mapped popup surface must cover at least the production scale allocation"
    );
}

fn focus_is_within_popup(popup: &gtk4::Popover, focused_widget: &gtk4::Widget) -> bool {
    let popup_widget: gtk4::Widget = popup.clone().upcast();
    let mut candidate = Some(focused_widget.clone());
    while let Some(widget) = candidate {
        if widget == popup_widget {
            return true;
        }
        candidate = widget.parent();
    }
    false
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
    while let Some(current) = child {
        if let Some(found) = find_widget(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
}
