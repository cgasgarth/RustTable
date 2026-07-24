#![forbid(unsafe_code)]

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::prelude::*;
use rusttable_core::Revision;
use rusttable_processing::ExposureAction;
use rusttable_ui::bauhaus::slider::BauhausSliderModel;
use rusttable_ui::gui::DARKTABLE_UI_TOKENS;
use rusttable_ui::presentation::darkroom_controls::DarkroomControlValue;
use rusttable_ui::{
    DarkroomModuleAction, DarkroomModuleActionHandler, DarkroomModuleError, ExposurePanel,
    GtkShell, WorkspaceRole, install_darktable_theme, reference_modules,
};

fn main() {
    gtk4::init().expect("GTK must initialize for the Bauhaus slider regression");
    prohibit_macos_test_activation();
    let display = gtk4::gdk::Display::default().expect("GTK boundary needs the default display");
    install_darktable_theme(&display);
    shared_production_slider_has_source_input_boundary();
    negative_factor_uses_mapped_production_controller_route();
    generic_descriptor_slider_defers_source_popup();
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
    let actions = Rc::new(RefCell::new(Vec::<ExposureAction>::new()));
    let actions_for_handler = Rc::clone(&actions);
    panel.set_action_handler(move |action| actions_for_handler.borrow_mut().push(action));
    let window = gtk4::Window::new();
    window.set_decorated(false);
    window.set_opacity(0.0);
    window.set_default_size(1_200, 800);
    let focus_probe = gtk4::Button::with_label("focus probe");
    focus_probe.set_widget_name("bauhaus-focus-probe");
    let test_surface = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    test_surface.append(&focus_probe);
    test_surface.append(panel.widget());
    window.set_child(Some(&test_surface));
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
    assert_eq!(
        (scale.adjustment().lower(), scale.adjustment().upper()),
        (-3.0, 4.0),
        "production Exposure must initially expose Darktable's soft range"
    );
    let exposure_step = scale.adjustment().step_increment();
    assert!(
        source_float_close(exposure_step, 0.05_f32),
        "automatic Exposure step must derive to Darktable's 0.05 EV; got {exposure_step:?}"
    );
    let black_scale = find_widget(&root, "exposure-black")
        .expect("production black-level scale")
        .downcast::<gtk4::Scale>()
        .expect("black-level control is a scale");
    let black_step = black_scale.adjustment().step_increment();
    assert!(
        source_float_close(black_step, 0.001_f32),
        "automatic black-level step must derive to Darktable's 0.001; got {black_step:?}"
    );

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
    let fine_tune_surface = find_widget(&popup_root, "bauhaus-slider-fine-tune-surface")
        .expect("popup source-derived fine-tune surface")
        .downcast::<gtk4::DrawingArea>()
        .expect("fine-tune presentation uses a GTK drawing area");
    assert!(!fine_tune_surface.is_focusable());
    assert!(!fine_tune_surface.can_target());
    assert_eq!(
        fine_tune_surface.accessible_role(),
        gtk4::AccessibleRole::Presentation,
        "drawn popup chrome must not create a phantom accessibility control"
    );
    assert!(
        fine_tune_surface.has_css_class("dt_bauhaus_fine_tune"),
        "fine-tune surface must retain the source popup styling boundary"
    );
    assert_eq!(
        fine_tune_surface.observe_controllers().n_items(),
        0,
        "presentation-only drawing must not own popup input controllers"
    );
    let fine_tune_content = fine_tune_surface
        .parent()
        .expect("fine-tune surface belongs to popup content")
        .downcast::<gtk4::Overlay>()
        .expect("fine-tune presentation and labels share an overlay");
    assert_eq!(
        fine_tune_content.widget_name(),
        "bauhaus-slider-fine-tune-content"
    );
    assert_popup_pointer_controllers(&fine_tune_content);
    let minimum_value = find_widget(&popup_root, "bauhaus-slider-minimum-value")
        .expect("popup visible-range minimum")
        .downcast::<gtk4::Label>()
        .expect("visible minimum uses an overlay label");
    let maximum_value = find_widget(&popup_root, "bauhaus-slider-maximum-value")
        .expect("popup visible-range maximum")
        .downcast::<gtk4::Label>()
        .expect("visible maximum uses an overlay label");
    assert_eq!(minimum_value.halign(), gtk4::Align::Start);
    assert_eq!(maximum_value.halign(), gtk4::Align::End);
    assert!(!minimum_value.is_focusable());
    assert!(!maximum_value.is_focusable());
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
    let mut primary_click = None;
    let mut middle_click = None;
    let mut main_motion = None;
    let mut main_scroll = None;
    let mut secondary_click = None;
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
        match controller.name().as_deref() {
            Some("dt-bauhaus-main-click") => {
                primary_click = controller.clone().downcast::<gtk4::GestureClick>().ok();
            }
            Some("dt-bauhaus-middle") => {
                middle_click = controller.clone().downcast::<gtk4::GestureClick>().ok();
            }
            Some("dt-bauhaus-main-motion") => {
                main_motion = controller
                    .clone()
                    .downcast::<gtk4::EventControllerMotion>()
                    .ok();
            }
            Some("dt-bauhaus-main-scroll") => {
                main_scroll = controller
                    .clone()
                    .downcast::<gtk4::EventControllerScroll>()
                    .ok();
            }
            Some("dt-bauhaus-secondary")
                if controller
                    .clone()
                    .downcast::<gtk4::GestureClick>()
                    .is_ok_and(|click| click.button() == 3) =>
            {
                secondary_clicks += 1;
                secondary_click = controller.clone().downcast::<gtk4::GestureClick>().ok();
            }
            _ => {}
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
    assert_secondary_opening_focus(
        &window,
        &focus_probe,
        &scale,
        &popup,
        &secondary_click.expect("source secondary click controller"),
    );
    refresh_macos_popup_surface(&anchor, &popup);
    assert_closed_source_controllers(
        &scale,
        &actions,
        open_controller.as_ref().expect("source key controller"),
        &primary_click.expect("source primary click controller"),
        &middle_click.expect("source middle click controller"),
        &main_motion.expect("source closed-widget motion controller"),
        &main_scroll.expect("source closed-widget scroll controller"),
    );

    // Exposure projects the value into its adjacent label, while other
    // production Bauhaus scales ask GtkScale to draw it. Enable that same
    // GtkScale presentation here to exercise the formatter installed by the
    // shared adapter rather than GTK's native numeric formatter.
    scale.set_draw_value(true);
    scale.set_value(1.0);
    assert!(
        scale.grab_focus(),
        "mapped production scale must accept GTK-local focus"
    );
    settle_gtk();
    assert_eq!(
        scale
            .layout()
            .expect("draw-value scale owns its closed value layout")
            .text(),
        "+1.000 EV",
        "closed GtkScale text must use the Bauhaus factor/offset/suffix formatter"
    );
    let propagation = open_controller
        .as_ref()
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
    let fine_tune_parent = fine_tune_surface
        .parent()
        .expect("fine-tune surface belongs to the popup overlay");
    assert_eq!(
        fine_tune_surface.allocated_width(),
        fine_tune_parent.allocated_width(),
        "presentation drawing must span the popup content width"
    );
    assert_eq!(
        fine_tune_surface.allocated_height(),
        fine_tune_parent.allocated_height(),
        "presentation drawing must span the popup content height"
    );
    assert!(
        !window.is_active(),
        "mapped popup must not activate or raise the transparent test window"
    );
    assert_eq!(
        current_value.text(),
        "+1.000 EV",
        "opening must snapshot the source-formatted Exposure value"
    );
    assert_eq!(
        minimum_value.text(),
        "-3.000 EV",
        "popup minimum must project the current visible-range lower bound"
    );
    assert_eq!(
        maximum_value.text(),
        "+4.000 EV",
        "popup maximum must project the current visible-range upper bound"
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
        gtk4::gdk::Key::_9,
        gtk4::gdk::Key::Return,
    ] {
        let propagation = input_controller.emit_by_name::<bool>(
            "key-pressed",
            &[&key, &0_u32, &gtk4::gdk::ModifierType::empty()],
        );
        assert!(propagation);
    }
    assert!(
        (scale.value() - 10.0).abs() <= f64::EPSILON,
        "x + 9 must route through the hard range to the real scale"
    );
    assert_eq!(
        (scale.adjustment().lower(), scale.adjustment().upper()),
        (-3.0, 10.0),
        "a hard-range value must expand only the currently visible range"
    );
    settle_gtk();
    refresh_macos_popup_surface(&anchor, &popup);

    assert_popup_rejection_after_range_toggle(
        &scale,
        &popup,
        &fine_tune_content,
        &input_controller,
        &actions,
        open_controller
            .as_ref()
            .expect("source popup opening controller"),
    );
    settle_gtk();
    window.close();
    settle_gtk();
}

fn negative_factor_uses_mapped_production_controller_route() {
    let mut model = BauhausSliderModel::new(0.0, 10.0, 1.0, 5.0, 1, true)
        .expect("valid reverse-factor fixture model");
    model.set_factor(-1.0);
    model.set_value(5.0);
    let fixture = model.into_gtk_input_test_fixture("reverse-factor-scale");

    let window = gtk4::Window::new();
    window.set_decorated(false);
    window.set_opacity(0.0);
    window.set_default_size(600, 120);
    window.set_child(Some(&fixture));
    window.present();
    settle_gtk();
    assert!(
        !window.is_active(),
        "reverse-factor fixture must not activate or steal focus"
    );

    let fixture_root: gtk4::Widget = fixture.clone().upcast();
    let scale = find_widget(&fixture_root, "reverse-factor-scale")
        .expect("mapped reverse-factor scale")
        .downcast::<gtk4::Scale>()
        .expect("reverse-factor fixture uses the shared GTK scale adapter");
    assert!(
        scale.is_inverted(),
        "negative display factor must reverse the production GtkScale position"
    );
    let primary = named_controller(&scale, "dt-bauhaus-main-click")
        .downcast::<gtk4::GestureClick>()
        .expect("reverse-factor source primary controller");
    let motion = named_controller(&scale, "dt-bauhaus-main-motion")
        .downcast::<gtk4::EventControllerMotion>()
        .expect("reverse-factor source motion controller");
    let values = Rc::new(RefCell::new(Vec::<f64>::new()));
    let values_for_signal = Rc::clone(&values);
    scale.connect_value_changed(move |scale| values_for_signal.borrow_mut().push(scale.value()));

    let before = scale.value();
    let width = f64::from(scale.allocated_width().max(1));
    primary.emit_by_name::<()>("pressed", &[&1_i32, &(width * 0.5), &0.0_f64]);
    motion.emit_by_name::<()>("motion", &[&(width * 0.75), &0.0_f64]);
    primary.emit_by_name::<()>("released", &[&1_i32, &(width * 0.75), &0.0_f64]);

    assert!(
        scale.value() < before,
        "source relative drag to the right must decrease a negative-factor raw value"
    );
    assert_eq!(
        values.borrow().last().map(|value| value.to_bits()),
        Some(scale.value().to_bits()),
        "mapped production route must publish the reverse-factor drag value"
    );

    window.close();
    settle_gtk();
}

fn generic_descriptor_slider_defers_source_popup() {
    let application = gtk4::Application::new(
        Some("com.cgasgarth.rusttable.test.generic-slider-boundary"),
        gtk4::gio::ApplicationFlags::default(),
    );
    application
        .register(None::<&gtk4::gio::Cancellable>)
        .expect("generic slider test application must register");
    let shell = GtkShell::new(&application);
    let actions = Rc::new(RefCell::new(Vec::<DarkroomModuleAction>::new()));
    let actions_for_handler = Rc::clone(&actions);
    let handler: DarkroomModuleActionHandler = Rc::new(move |action| {
        let next_revision = action
            .expected_revision()
            .checked_increment()
            .map_err(|_| DarkroomModuleError::RevisionOverflow)?;
        actions_for_handler.borrow_mut().push(action);
        Ok(next_revision)
    });
    let modules = reference_modules().expect("registry descriptor module snapshot");
    shell.set_darkroom_module_stack(&modules, Some(handler));
    shell.show_workspace(WorkspaceRole::Darkroom);

    let root: gtk4::Widget = shell.window().clone().upcast();
    let search = find_widget(&root, "darkroom-module-search")
        .expect("darkroom module search")
        .downcast::<gtk4::SearchEntry>()
        .expect("module search control type");
    search.set_text("temperature");
    search.emit_by_name::<()>("search-changed", &[]);
    settle_gtk();
    let temperature = find_widget(&root, "temperature")
        .expect("generic temperature descriptor module")
        .downcast::<gtk4::Expander>()
        .expect("temperature module expander type");
    let temperature_content = temperature
        .child()
        .expect("temperature descriptor module content");

    let scale = find_widget(&temperature_content, "temperature-temperature-widget")
        .expect("generic temperature descriptor scale")
        .downcast::<gtk4::Scale>()
        .expect("temperature descriptor uses a GTK scale");
    assert_eq!(
        (scale.adjustment().lower(), scale.adjustment().upper()),
        (1901.0, 25_000.0)
    );
    assert!(
        (scale.adjustment().step_increment() - 0.001).abs() <= f64::EPSILON,
        "generic descriptor step must remain unchanged"
    );
    assert!((scale.value() - 4_000.0).abs() <= f64::EPSILON);
    assert_eq!(scale.digits(), 3);
    assert!(scale.draws_value());
    assert_eq!(scale.value_pos(), gtk4::PositionType::Right);

    let scale_root = scale
        .parent()
        .expect("generic scale belongs directly to its module row");
    assert!(
        scale_root.is::<gtk4::Box>(),
        "generic metadata must not wrap the scale in a Bauhaus overlay"
    );
    assert!(
        find_widget(&scale_root, "bauhaus-slider-anchor").is_none(),
        "generic metadata must not create a Bauhaus popup anchor"
    );
    assert!(
        find_widget(&scale_root, "bauhaus-slider").is_none(),
        "generic metadata must not create a Bauhaus fine-tune popup"
    );
    assert_eq!(
        named_controller_count(&scale, "dt-bauhaus-"),
        0,
        "generic scale must not install source-specific Bauhaus input controllers"
    );

    assert!(scale.has_css_class("dt_slider"));
    assert_eq!(
        scale.height_request(),
        DARKTABLE_UI_TOKENS.controls.control_height
    );
    assert_eq!(
        scale.width_request(),
        DARKTABLE_UI_TOKENS.controls.module_control_min_width
    );
    assert!(scale.hexpands());
    assert!(scale.is_focusable());
    assert_eq!(scale.accessible_role(), gtk4::AccessibleRole::Slider);
    assert_eq!(
        scale.tooltip_text().as_deref(),
        Some("Temperature; range 1901.000 to 25000.000")
    );

    scale.set_value(5_000.125);
    settle_gtk();
    assert!((scale.value() - 5_000.125).abs() <= f64::EPSILON);
    let dispatched = actions
        .borrow()
        .last()
        .cloned()
        .expect("generic scale value change dispatches a module action");
    assert!(matches!(
        dispatched,
        DarkroomModuleAction::Control {
            module_id,
            expected_revision: Revision::ZERO,
            id,
            value: DarkroomControlValue::Slider(value),
        } if module_id == "temperature"
            && id == "temperature-temperature"
            && (value - 5_000.125).abs() <= f64::EPSILON
    ));
    let status = find_widget(&temperature_content, "temperature-status")
        .expect("temperature module status")
        .downcast::<gtk4::Label>()
        .expect("temperature status label type");
    assert_eq!(status.text(), "Ready · revision 1");

    shell.window().close();
    settle_gtk();
}

fn named_controller_count(scale: &gtk4::Scale, prefix: &str) -> usize {
    scale
        .observe_controllers()
        .into_iter()
        .flatten()
        .filter_map(|controller| controller.downcast::<gtk4::EventController>().ok())
        .filter(|controller| {
            controller
                .name()
                .as_deref()
                .is_some_and(|name| name.starts_with(prefix))
        })
        .count()
}

fn assert_secondary_opening_focus(
    window: &gtk4::Window,
    focus_probe: &gtk4::Button,
    scale: &gtk4::Scale,
    popup: &gtk4::Popover,
    secondary_click: &gtk4::GestureClick,
) {
    assert!(
        focus_probe.grab_focus(),
        "focus probe must own GTK-local focus before the secondary press"
    );
    assert_eq!(
        gtk4::prelude::RootExt::focus(window).as_ref(),
        Some(focus_probe.upcast_ref()),
        "precondition: another widget owns focus"
    );
    let acquisitions = Rc::new(Cell::new(0_u32));
    let acquisitions_for_signal = Rc::clone(&acquisitions);
    let focus_observer = gtk4::EventControllerFocus::new();
    focus_observer.connect_enter(move |_| {
        acquisitions_for_signal.set(acquisitions_for_signal.get() + 1);
    });
    scale.add_controller(focus_observer);
    secondary_click.emit_by_name::<()>("pressed", &[&1_i32, &1.0_f64, &1.0_f64]);
    assert_eq!(
        acquisitions.get(),
        1,
        "secondary opening must acquire the source slider before popup focus"
    );
    assert!(
        popup.is_visible(),
        "secondary press must open the source popup"
    );
    popup.popdown();
    settle_gtk();
}

fn assert_closed_source_controllers(
    scale: &gtk4::Scale,
    actions: &Rc<RefCell<Vec<ExposureAction>>>,
    key: &gtk4::EventControllerKey,
    primary_click: &gtk4::GestureClick,
    middle_click: &gtk4::GestureClick,
    motion: &gtk4::EventControllerMotion,
    scroll: &gtk4::EventControllerScroll,
) {
    assert_eq!(primary_click.button(), 1);
    assert_eq!(middle_click.button(), 2);
    assert_eq!(
        primary_click.propagation_phase(),
        gtk4::PropagationPhase::Capture,
        "source primary gesture must preempt GtkScale's substitute gesture"
    );
    assert_eq!(
        scroll.propagation_phase(),
        gtk4::PropagationPhase::Capture,
        "source scrolling must preempt GtkScale's substitute scroll path"
    );
    assert!(
        scroll
            .flags()
            .contains(gtk4::EventControllerScrollFlags::BOTH_AXES),
        "source widget scroll combines horizontal and vertical units"
    );
    assert!(
        !scroll
            .flags()
            .contains(gtk4::EventControllerScrollFlags::DISCRETE),
        "closed source scrolling must preserve smooth fractional deltas"
    );

    scale.set_value(0.0);
    settle_gtk();
    actions.borrow_mut().clear();
    let handled = key.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk4::gdk::Key::Up,
            &0_u32,
            &gtk4::gdk::ModifierType::empty(),
        ],
    );
    assert!(
        handled,
        "source Up arrow must preempt GtkScale keyboard input"
    );
    assert_one_exposure_action(actions, 0.05_f32, -3.0_f32, 4.0_f32);

    actions.borrow_mut().clear();
    let handled = key.emit_by_name::<bool>(
        "key-pressed",
        &[&gtk4::gdk::Key::Up, &0_u32, &primary_accelerator_mask()],
    );
    assert!(
        handled,
        "fine source arrow must remain in the custom controller"
    );
    assert_one_exposure_action(actions, 0.055_f32, -3.0_f32, 4.0_f32);

    for key_value in [
        gtk4::gdk::Key::Home,
        gtk4::gdk::Key::KP_Home,
        gtk4::gdk::Key::End,
        gtk4::gdk::Key::KP_End,
        gtk4::gdk::Key::Page_Up,
        gtk4::gdk::Key::KP_Page_Up,
        gtk4::gdk::Key::Page_Down,
        gtk4::gdk::Key::KP_Page_Down,
        gtk4::gdk::Key::plus,
        gtk4::gdk::Key::minus,
        gtk4::gdk::Key::KP_Add,
        gtk4::gdk::Key::KP_Subtract,
    ] {
        let value_before = scale.value();
        actions.borrow_mut().clear();
        let handled = key.emit_by_name::<bool>(
            "key-pressed",
            &[&key_value, &0_u32, &gtk4::gdk::ModifierType::empty()],
        );
        assert!(
            handled,
            "{key_value:?} must not reach GtkScale's native key bindings"
        );
        assert_eq!(
            scale.value().to_bits(),
            value_before.to_bits(),
            "{key_value:?} is not a retained closed Bauhaus command"
        );
        assert!(
            exposure_values(actions).is_empty(),
            "{key_value:?} must not emit a production slider action"
        );
    }
    for (key_value, modifiers) in [
        (gtk4::gdk::Key::Tab, gtk4::gdk::ModifierType::empty()),
        (
            gtk4::gdk::Key::ISO_Left_Tab,
            gtk4::gdk::ModifierType::SHIFT_MASK,
        ),
        (gtk4::gdk::Key::a, gtk4::gdk::ModifierType::empty()),
    ] {
        let value_before = scale.value();
        actions.borrow_mut().clear();
        let handled = key.emit_by_name::<bool>("key-pressed", &[&key_value, &0_u32, &modifiers]);
        assert!(
            !handled,
            "{key_value:?} must proceed to focus or parent handlers"
        );
        assert_eq!(
            scale.value().to_bits(),
            value_before.to_bits(),
            "{key_value:?} propagation must not mutate the slider"
        );
        assert!(
            exposure_values(actions).is_empty(),
            "{key_value:?} propagation must not emit a slider action"
        );
    }

    scale.set_value(0.0);
    settle_gtk();
    actions.borrow_mut().clear();
    let width = f64::from(scale.allocated_width().max(1));
    let height = f64::from(scale.allocated_height().max(1));
    primary_click.emit_by_name::<()>("pressed", &[&1_i32, &(width * 0.25), &(height * 0.75)]);
    primary_click.emit_by_name::<()>("stopped", &[]);
    assert_eq!(
        exposure_values(actions).len(),
        1,
        "GestureClick::stopped flushes the initial coalesced press once"
    );
    motion.emit_by_name::<()>("motion", &[&(width * 0.5), &(height * 0.75)]);
    motion.emit_by_name::<()>("motion", &[&(width * 0.75), &(height * 0.75)]);
    assert_eq!(
        exposure_values(actions).len(),
        1,
        "post-stopped same-turn motion must remain coalesced until release"
    );
    primary_click.emit_by_name::<()>("released", &[&1_i32, &(width * 0.75), &(height * 0.75)]);
    assert_eq!(
        exposure_values(actions).len(),
        2,
        "release must flush one post-stopped drag value"
    );
    assert_last_exposure_action(actions, 2.25_f32, -3.0_f32, 4.0_f32);
    assert!(
        gtk4::prelude::RootExt::focus(&scale.root().expect("mapped scale root"))
            .is_some_and(|focused| focused == scale.clone().upcast::<gtk4::Widget>()),
        "source primary press must leave GTK focus on the closed slider"
    );

    scale.set_value(1.0);
    settle_gtk();
    actions.borrow_mut().clear();
    primary_click.emit_by_name::<()>("pressed", &[&2_i32, &(width * 0.5), &(height * 0.75)]);
    assert_one_exposure_action(actions, 0.0_f32, -3.0_f32, 4.0_f32);
    assert_eq!(
        (scale.adjustment().lower(), scale.adjustment().upper()),
        (-3.0, 4.0),
        "double-primary reset must restore Darktable's soft range"
    );

    scale.set_value(10.0);
    settle_gtk();
    actions.borrow_mut().clear();
    middle_click.emit_by_name::<()>("pressed", &[&1_i32, &(width * 0.5), &(height * 0.75)]);
    assert_eq!(
        (scale.adjustment().lower(), scale.adjustment().upper()),
        (-18.0, 18.0),
        "middle press must toggle the current range to the hard bounds"
    );

    primary_click.emit_by_name::<()>("pressed", &[&2_i32, &(width * 0.5), &(height * 0.75)]);
    settle_gtk();
    actions.borrow_mut().clear();
    let handled = scroll.emit_by_name::<bool>("scroll", &[&0.0_f64, &source_scroll_delta()]);
    assert!(
        handled,
        "source closed-widget scroll must stop GtkScale handling"
    );
    assert_one_exposure_action(actions, -0.05_f32, -3.0_f32, 4.0_f32);
}

fn assert_popup_rejection_after_range_toggle(
    scale: &gtk4::Scale,
    popup: &gtk4::Popover,
    content: &gtk4::Overlay,
    input: &gtk4::EventControllerKey,
    actions: &Rc<RefCell<Vec<ExposureAction>>>,
    open: &gtk4::EventControllerKey,
) {
    scale.set_value(1.0);
    settle_gtk();
    actions.borrow_mut().clear();
    let handled = open.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk4::gdk::Key::Return,
            &0_u32,
            &gtk4::gdk::ModifierType::empty(),
        ],
    );
    assert!(handled);
    assert!(popup.is_visible());

    let click = named_controller(content, "dt-bauhaus-popup-click")
        .downcast::<gtk4::GestureClick>()
        .expect("popup click controller type");
    let motion = named_controller(content, "dt-bauhaus-popup-motion")
        .downcast::<gtk4::EventControllerMotion>()
        .expect("popup motion controller type");
    let width = f64::from(content.allocated_width().max(1));
    let height = f64::from(content.allocated_height().max(1));
    click.set_button(1);
    click.emit_by_name::<()>("pressed", &[&1_i32, &(width * 0.25), &(height * 0.75)]);
    motion.emit_by_name::<()>("motion", &[&(width * 0.75), &(height * 0.75)]);
    click.set_button(2);
    click.emit_by_name::<()>("pressed", &[&1_i32, &(width * 0.5), &(height * 0.75)]);
    click.set_button(0);

    let value_after_toggle = scale.value();
    let range_after_toggle = (scale.adjustment().lower(), scale.adjustment().upper());
    let position_after_toggle = normalized_scale_position(scale);
    let actions_before_escape = exposure_values(actions);
    assert_eq!(
        actions_before_escape.len(),
        3,
        "primary press, motion, and middle range toggle each emit one source change"
    );
    assert_eq!(
        range_after_toggle,
        (-18.0, 18.0),
        "popup middle press must recapture the value in the hard range"
    );

    let handled = input.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk4::gdk::Key::Escape,
            &0_u32,
            &gtk4::gdk::ModifierType::empty(),
        ],
    );
    assert!(handled);
    settle_gtk();
    assert!(
        !popup.is_visible(),
        "Escape must reject and close the popup"
    );
    assert_eq!(
        (scale.adjustment().lower(), scale.adjustment().upper()),
        range_after_toggle,
        "rejection must preserve the range selected by the middle toggle"
    );
    assert_eq!(
        scale.value().to_bits(),
        value_after_toggle.to_bits(),
        "rejection restores the source's recaptured normalized position, not the pre-popup raw value"
    );
    assert_eq!(
        normalized_scale_position(scale).to_bits(),
        position_after_toggle.to_bits(),
        "the recaptured normalized popup position must survive rejection exactly"
    );
    assert_eq!(
        exposure_values(actions).len(),
        actions_before_escape.len() + 1,
        "normalized rejection must emit exactly one final source change"
    );
}

fn normalized_scale_position(scale: &gtk4::Scale) -> f64 {
    let adjustment = scale.adjustment();
    (scale.value() - adjustment.lower()) / (adjustment.upper() - adjustment.lower())
}

fn named_controller(widget: &impl IsA<gtk4::Widget>, name: &str) -> gtk4::EventController {
    widget
        .observe_controllers()
        .into_iter()
        .flatten()
        .filter_map(|controller| controller.downcast::<gtk4::EventController>().ok())
        .find(|controller| controller.name().as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing GTK controller {name}"))
}

fn assert_one_exposure_action(
    actions: &Rc<RefCell<Vec<ExposureAction>>>,
    rounded_source_value: f32,
    minimum: f32,
    maximum: f32,
) {
    let values = exposure_values(actions);
    let position = (rounded_source_value - minimum) / (maximum - minimum);
    let expected_source_value = f64::from(minimum + position * (maximum - minimum));
    assert!(
        values.len() == 1 && values[0].to_bits() == expected_source_value.to_bits(),
        "one source-rounded value must cross the production action boundary; got {values:?}"
    );
}

fn assert_last_exposure_action(
    actions: &Rc<RefCell<Vec<ExposureAction>>>,
    rounded_source_value: f32,
    minimum: f32,
    maximum: f32,
) {
    let values = exposure_values(actions);
    let position = (rounded_source_value - minimum) / (maximum - minimum);
    let expected_source_value = f64::from(minimum + position * (maximum - minimum));
    assert_eq!(
        values.last().map(|value| value.to_bits()),
        Some(expected_source_value.to_bits()),
        "latest source-rounded production value mismatch; got {values:?}"
    );
}

fn exposure_values(actions: &Rc<RefCell<Vec<ExposureAction>>>) -> Vec<f64> {
    actions
        .borrow()
        .iter()
        .filter_map(|action| match action {
            ExposureAction::SetExposureEv(value) => Some(*value),
            _ => None,
        })
        .collect()
}

fn primary_accelerator_mask() -> gtk4::gdk::ModifierType {
    #[cfg(target_os = "macos")]
    {
        gtk4::gdk::ModifierType::META_MASK
    }
    #[cfg(not(target_os = "macos"))]
    {
        gtk4::gdk::ModifierType::CONTROL_MASK
    }
}

fn source_scroll_delta() -> f64 {
    #[cfg(target_os = "macos")]
    {
        50.0
    }
    #[cfg(not(target_os = "macos"))]
    {
        1.0
    }
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

fn assert_popup_pointer_controllers(content: &gtk4::Overlay) {
    let controllers = content.observe_controllers();
    let mut motion = 0;
    let mut click = 0;
    let mut scroll = 0;
    for index in 0..controllers.n_items() {
        let controller = controllers
            .item(index)
            .expect("popup controller item")
            .downcast::<gtk4::EventController>()
            .expect("GTK popup controller type");
        match controller.name().as_deref() {
            Some("dt-bauhaus-popup-motion") if controller.is::<gtk4::EventControllerMotion>() => {
                motion += 1;
            }
            Some("dt-bauhaus-popup-click") if controller.is::<gtk4::GestureClick>() => {
                click += 1;
            }
            Some("dt-bauhaus-popup-scroll") if controller.is::<gtk4::EventControllerScroll>() => {
                let scroll_controller = controller
                    .downcast::<gtk4::EventControllerScroll>()
                    .expect("popup scroll controller");
                assert!(
                    scroll_controller
                        .flags()
                        .contains(gtk4::EventControllerScrollFlags::BOTH_AXES),
                    "popup zoom must accept source horizontal and vertical scroll units"
                );
                scroll += 1;
            }
            _ => {}
        }
    }
    assert_eq!(motion, 1, "popup owns one fine-tune motion controller");
    assert_eq!(click, 1, "popup owns one accepted/rejected click lifecycle");
    assert_eq!(scroll, 1, "popup owns one source-range zoom controller");
}

fn settle_gtk() {
    let context = gtk4::glib::MainContext::default();
    while context.pending() {
        context.iteration(false);
    }
}

#[cfg(target_os = "macos")]
fn refresh_macos_popup_surface(anchor: &gtk4::MenuButton, popup: &gtk4::Popover) {
    // GTK 4.22.4's macOS GdkPopup REMAP path reuses an already-thawed native
    // surface. Reassign through the supported owner API after each settled
    // synthetic lifecycle so the next mapping receives a fresh surface.
    anchor.set_popover(None::<&gtk4::Popover>);
    anchor.set_popover(Some(popup));
}

#[cfg(not(target_os = "macos"))]
fn refresh_macos_popup_surface(_anchor: &gtk4::MenuButton, _popup: &gtk4::Popover) {}

fn source_float_close(actual: f64, expected: f32) -> bool {
    let tolerance = f64::from(f32::EPSILON * expected.abs() * 2.0);
    (actual - f64::from(expected)).abs() <= tolerance
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
