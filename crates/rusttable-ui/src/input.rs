//! GTK4 event-controller adapter for the display-independent input service.
//!
//! This is deliberately limited to translating GTK keyboard and stylus
//! events. MIDI and gamepad backends can feed [`rusttable_input::InputEvent`]
//! directly without taking a GTK dependency.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Instant;

use gtk4::gdk::{self, AxisUse, DeviceToolType, ModifierType};
use gtk4::prelude::*;
use rusttable_input::{
    ActionEvent, ActionInputService, ActionPhase, DeviceDescriptor, DeviceToken, InputEvent,
    InputSource, KeyCode, KeyboardEvent, Modifiers, TabletEvent,
};

/// The GTK-owned controllers and their tablet device token.
pub struct GtkInputAdapter {
    tablet: DeviceToken,
    key_controller: gtk4::EventControllerKey,
    stylus: gtk4::GestureStylus,
}

impl GtkInputAdapter {
    /// Attaches capture-phase keyboard and stylus translation to a widget.
    ///
    /// The service and callback are shared with both controllers; action
    /// handlers run synchronously on GTK's main thread in event order.
    #[must_use]
    pub fn attach<W, F>(widget: &W, service: &Rc<RefCell<ActionInputService>>, callback: F) -> Self
    where
        W: IsA<gtk4::Widget>,
        F: Fn(ActionEvent) + 'static,
    {
        let tablet = service.borrow_mut().connect_device(DeviceDescriptor::new(
            InputSource::Tablet,
            "gtk-stylus",
            "GTK stylus",
        ));
        let callback: Rc<dyn Fn(ActionEvent)> = Rc::new(callback);
        let started = Rc::new(Instant::now());
        let pressed = Rc::new(RefCell::new(BTreeSet::<(KeyCode, Modifiers)>::new()));

        let key_controller = gtk4::EventControllerKey::new();
        key_controller.set_propagation_phase(gtk4::PropagationPhase::Capture);
        {
            let service = Rc::clone(service);
            let callback = Rc::clone(&callback);
            let started = Rc::clone(&started);
            let pressed = Rc::clone(&pressed);
            key_controller.connect_key_pressed(move |_, key, _, state| {
                let key = key_code(key);
                let modifiers = modifiers(state);
                let repeat = !pressed.borrow_mut().insert((key.clone(), modifiers));
                let event = InputEvent::Keyboard(KeyboardEvent {
                    device: DeviceToken::new(InputSource::Keyboard, "keyboard", 1),
                    timestamp: elapsed_millis(&started),
                    key,
                    modifiers,
                    pressed: true,
                    repeat,
                });
                if dispatch(&service, &callback, &event) {
                    gtk4::glib::Propagation::Stop
                } else {
                    gtk4::glib::Propagation::Proceed
                }
            });
        }
        {
            let service = Rc::clone(service);
            let callback = Rc::clone(&callback);
            let started = Rc::clone(&started);
            let pressed = Rc::clone(&pressed);
            key_controller.connect_key_released(move |_, key, _, state| {
                let key = key_code(key);
                let modifiers = modifiers(state);
                pressed.borrow_mut().remove(&(key.clone(), modifiers));
                let event = InputEvent::Keyboard(KeyboardEvent {
                    device: DeviceToken::new(InputSource::Keyboard, "keyboard", 1),
                    timestamp: elapsed_millis(&started),
                    key,
                    modifiers,
                    pressed: false,
                    repeat: false,
                });
                let _ = dispatch(&service, &callback, &event);
            });
        }
        widget.add_controller(key_controller.clone());

        let stylus = gtk4::GestureStylus::new();
        stylus.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let stylus_dispatch = StylusDispatch {
            service: Rc::clone(service),
            callback: Rc::clone(&callback),
            started: Rc::clone(&started),
            tablet: tablet.clone(),
        };
        connect_stylus_down(&stylus, &stylus_dispatch);
        connect_stylus_motion(&stylus, &stylus_dispatch);
        connect_stylus_up(&stylus, &stylus_dispatch);
        widget.add_controller(stylus.clone());

        Self {
            tablet,
            key_controller,
            stylus,
        }
    }

    /// Returns the token used for events from the GTK stylus controller.
    #[must_use]
    pub const fn tablet_device(&self) -> &DeviceToken {
        &self.tablet
    }

    /// Keeps the controllers available for callers that need to adjust GTK
    /// propagation or inspect ownership in diagnostics.
    #[must_use]
    pub fn controllers(&self) -> (&gtk4::EventControllerKey, &gtk4::GestureStylus) {
        (&self.key_controller, &self.stylus)
    }
}

fn connect_stylus_down(stylus: &gtk4::GestureStylus, dispatch: &StylusDispatch) {
    let dispatch = dispatch.clone();
    stylus.connect_down(move |gesture, x, y| {
        dispatch.send(gesture, ActionPhase::Pressed, x, y);
    });
}

fn connect_stylus_motion(stylus: &gtk4::GestureStylus, dispatch: &StylusDispatch) {
    let dispatch = dispatch.clone();
    stylus.connect_motion(move |gesture, x, y| {
        dispatch.send(gesture, ActionPhase::Changed, x, y);
    });
}

fn connect_stylus_up(stylus: &gtk4::GestureStylus, dispatch: &StylusDispatch) {
    let dispatch = dispatch.clone();
    stylus.connect_up(move |gesture, x, y| {
        dispatch.send(gesture, ActionPhase::Released, x, y);
    });
}

#[derive(Clone)]
struct StylusDispatch {
    service: Rc<RefCell<ActionInputService>>,
    callback: Rc<dyn Fn(ActionEvent)>,
    started: Rc<Instant>,
    tablet: DeviceToken,
}

impl StylusDispatch {
    fn send(&self, gesture: &gtk4::GestureStylus, phase: ActionPhase, x: f64, y: f64) {
        let pressure = narrow_f64(gesture.axis(AxisUse::Pressure).unwrap_or(1.0));
        let tilt_x = narrow_f64(gesture.axis(AxisUse::Xtilt).unwrap_or(0.0));
        let tilt_y = narrow_f64(gesture.axis(AxisUse::Ytilt).unwrap_or(0.0));
        let eraser = gesture
            .device_tool()
            .is_some_and(|tool| tool.tool_type() == DeviceToolType::Eraser);
        let event = InputEvent::Tablet(TabletEvent {
            device: self.tablet.clone(),
            timestamp: elapsed_millis(&self.started),
            phase,
            x: narrow_f64(x),
            y: narrow_f64(y),
            pressure,
            tilt_x,
            tilt_y,
            eraser,
            button: None,
        });
        let _ = dispatch(&self.service, &self.callback, &event);
    }
}

fn dispatch(
    service: &Rc<RefCell<ActionInputService>>,
    callback: &Rc<dyn Fn(ActionEvent)>,
    event: &InputEvent,
) -> bool {
    let report = service.borrow_mut().ingest(event);
    let delivered = !report.events.is_empty();
    for action in report.events {
        callback(action);
    }
    delivered
}

fn elapsed_millis(started: &Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis().min(u128::from(u64::MAX))).unwrap_or(u64::MAX)
}

#[allow(clippy::cast_possible_truncation)]
fn narrow_f64(value: f64) -> f32 {
    value.clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32
}

fn key_code(key: gdk::Key) -> KeyCode {
    key.name().map_or_else(
        || KeyCode::Hardware(key.to_unicode().map_or(0, u32::from)),
        |name| KeyCode::from_name(name.as_str()),
    )
}

fn modifiers(state: ModifierType) -> Modifiers {
    let mut modifiers = Modifiers::empty();
    if state.contains(ModifierType::SHIFT_MASK) {
        modifiers = modifiers.union(Modifiers::SHIFT);
    }
    if state.contains(ModifierType::CONTROL_MASK) {
        modifiers = modifiers.union(Modifiers::CONTROL);
    }
    if state.contains(ModifierType::ALT_MASK) {
        modifiers = modifiers.union(Modifiers::ALT);
    }
    if state.contains(ModifierType::SUPER_MASK) {
        modifiers = modifiers.union(Modifiers::SUPER);
    }
    modifiers
}
