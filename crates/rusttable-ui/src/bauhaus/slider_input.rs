//! GTK4 adapter for Darktable's linear Bauhaus slider and fine-tune popup.
//!
//! This maps `_popup_show`, `_popup_reject`, `_slider_add_step`, and the
//! slider branches of `_popup_draw`, `_popup_scroll`, `_popup_button_press`,
//! `_popup_button_release`, `_window_motion_notify`, and `_popup_key_press`
//! from `src/bauhaus/bauhaus.c`, plus the closed-widget controller path.
//! Circular/color-wheel painting and its matching pointer mapping remain
//! gated until they can be enabled together with source gradients/config.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

use gtk4::{gdk, glib, prelude::*};

use super::{
    numeric_input::{NumericInputBuffer, resolve_raw_value},
    slider::{AutomaticStepPolicy, BauhausSliderModel},
    slider_popup::{
        SliderRange, SliderRanges, SmoothScrollAccumulator, ZoomRangeChange,
        linear_pointer_position, loupe_scale, should_activate_change, zoom_range,
    },
};
use crate::gui::darktable_spec::DARKTABLE_COLORS;

const SECONDARY_BUTTON: u32 = 3;
const PRIMARY_BUTTON: u32 = 1;
const MIDDLE_BUTTON: u32 = 2;
const ENDPOINT_DELTA: f64 = 1_000_000.0;
// `INNER_PADDING` is selected by Darktable's non-condensed Bauhaus default.
const INNER_PADDING: i32 = 4;
const GUIDELINE_STEPS: u32 = 64;
const SOURCE_BORDER_WIDTH: f64 = 2.0;
const OPEN_CONTROLLER_NAME: &str = "dt-bauhaus-open";
const INPUT_CONTROLLER_NAME: &str = "dt-bauhaus-input";
const MAIN_CLICK_CONTROLLER_NAME: &str = "dt-bauhaus-main-click";
const MAIN_MIDDLE_CONTROLLER_NAME: &str = "dt-bauhaus-middle";
const MAIN_MOTION_CONTROLLER_NAME: &str = "dt-bauhaus-main-motion";
const MAIN_SCROLL_CONTROLLER_NAME: &str = "dt-bauhaus-main-scroll";
const SECONDARY_CONTROLLER_NAME: &str = "dt-bauhaus-secondary";
const POPUP_MOTION_CONTROLLER_NAME: &str = "dt-bauhaus-popup-motion";
const POPUP_CLICK_CONTROLLER_NAME: &str = "dt-bauhaus-popup-click";
const POPUP_SCROLL_CONTROLLER_NAME: &str = "dt-bauhaus-popup-scroll";

// Darktable re-reads the global `bauhaus/zoom_step` preference for every
// automatic-step query.
static SOURCE_ZOOM_STEP: AtomicBool = AtomicBool::new(true);

thread_local! {
    // `dt_gui_get_scroll_unit_deltas` deliberately shares these remainders
    // across every widget on GTK's main thread (`src/gui/gtk.c:522-523`).
    static SOURCE_SCROLL_UNITS: RefCell<SmoothScrollAccumulator> =
        RefCell::new(SmoothScrollAccumulator::default());
}

pub(super) fn set_zoom_step(enabled: bool) {
    SOURCE_ZOOM_STEP.store(enabled, Ordering::Relaxed);
}

fn source_automatic_step_policy() -> AutomaticStepPolicy {
    if SOURCE_ZOOM_STEP.load(Ordering::Relaxed) {
        AutomaticStepPolicy::VisibleRange
    } else {
        AutomaticStepPolicy::SoftRange
    }
}

/// Coalesces drag notifications at the same GLib-main-context boundary used
/// by Darktable's `_slider_value_change_dragging`.
#[derive(Debug, Default)]
struct ActionCoalescer {
    deferred: Cell<bool>,
    pending: Cell<bool>,
    idle_scheduled: Cell<bool>,
    flushing: Cell<bool>,
}

impl ActionCoalescer {
    fn begin(&self) {
        self.deferred.set(true);
    }

    fn defer_change(self: &Rc<Self>, scale: &gtk4::Scale) {
        self.pending.set(true);
        if self.idle_scheduled.replace(true) {
            return;
        }

        let scale = scale.downgrade();
        let coalescer = Rc::clone(self);
        glib::idle_add_local_once(move || {
            coalescer.idle_scheduled.set(false);
            if let Some(scale) = scale.upgrade() {
                coalescer.emit_pending(&scale);
            } else {
                coalescer.pending.set(false);
            }
        });
    }

    fn finish(&self, scale: &gtk4::Scale) {
        self.deferred.set(false);
        self.emit_pending(scale);
    }

    fn flush(&self, scale: &gtk4::Scale) {
        self.emit_pending(scale);
    }

    fn emit_pending(&self, scale: &gtk4::Scale) {
        if !self.pending.replace(false) {
            return;
        }
        self.flushing.set(true);
        scale.emit_by_name::<()>("value-changed", &[]);
        self.flushing.set(false);
    }
}

/// The GTK4 composition that gives a scale a supported popover owner.
///
/// GTK4 scales do not own or allocate arbitrary popover children. A one-pixel
/// frameless `MenuButton` is therefore overlaid on the scale and owns the
/// popover through GTK's supported `MenuButton::set_popover` API.
#[derive(Clone, Debug)]
pub(crate) struct BauhausSlider {
    root: gtk4::Overlay,
    scale: gtk4::Scale,
    model: Rc<RefCell<BauhausSliderModel>>,
    sync_guard: Rc<Cell<bool>>,
}

impl BauhausSlider {
    pub(crate) fn widget(&self) -> &gtk4::Overlay {
        &self.root
    }

    pub(crate) fn scale(&self) -> &gtk4::Scale {
        &self.scale
    }

    pub(crate) fn set_value(&self, value: f64) {
        source_set_value(&self.scale, &self.model, &self.sync_guard, value);
        debug_assert_eq!(self.value().to_bits(), self.scale.value().to_bits());
    }

    #[must_use]
    pub(crate) fn value(&self) -> f64 {
        self.model.borrow().value()
    }

    pub(crate) fn set_digits(&self, digits: i32) {
        self.model.borrow_mut().set_digits(digits);
        sync_scale_from_model(&self.scale, &self.model, &self.sync_guard);
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SliderInputSpec {
    pub(crate) factor: f64,
    pub(crate) offset: f64,
    pub(crate) suffix: &'static str,
    soft_range: Option<(f64, f64)>,
    default_value: Option<f64>,
    digits: Option<i32>,
    automatic_step: bool,
}

impl SliderInputSpec {
    pub(crate) const IDENTITY: Self = Self {
        factor: 1.0,
        offset: 0.0,
        suffix: "",
        soft_range: None,
        default_value: None,
        digits: None,
        automatic_step: false,
    };

    pub(crate) const fn with_suffix(self, suffix: &'static str) -> Self {
        Self { suffix, ..self }
    }

    pub(crate) const fn with_soft_range(self, minimum: f64, maximum: f64) -> Self {
        Self {
            soft_range: Some((minimum, maximum)),
            ..self
        }
    }

    pub(crate) const fn with_default_value(self, default_value: f64) -> Self {
        Self {
            default_value: Some(default_value),
            ..self
        }
    }

    pub(crate) const fn with_digits(self, digits: i32) -> Self {
        Self {
            digits: Some(digits),
            ..self
        }
    }

    pub(crate) const fn with_automatic_step(self) -> Self {
        Self {
            automatic_step: true,
            ..self
        }
    }
}

impl Default for SliderInputSpec {
    fn default() -> Self {
        Self::IDENTITY
    }
}

fn model_from_scale(scale: &gtk4::Scale, spec: SliderInputSpec) -> BauhausSliderModel {
    let adjustment = scale.adjustment();
    let step = if spec.automatic_step {
        0.0
    } else {
        adjustment.step_increment()
    };
    let default_value = spec.default_value.unwrap_or_else(|| scale.value());
    let digits = spec.digits.unwrap_or_else(|| scale.digits());
    let mut model = BauhausSliderModel::new(
        adjustment.lower(),
        adjustment.upper(),
        step,
        default_value,
        digits,
        true,
    )
    .expect("GTK Scale and SliderInputSpec must define a valid Bauhaus slider");
    model.set_factor(spec.factor);
    model.set_offset(spec.offset);
    model.set_format(spec.suffix);
    if let Some((minimum, maximum)) = spec.soft_range {
        model.set_soft_range(minimum, maximum);
    }
    model
}

impl BauhausSliderModel {
    /// Builds the shared GTK adapter around this exact model for black-box
    /// controller boundary tests.
    #[doc(hidden)]
    #[must_use]
    pub fn into_gtk_input_test_fixture(self, widget_name: &str) -> gtk4::Overlay {
        let (minimum, maximum) = self.visible_range();
        let configured_step = self.configured_step().abs();
        let gtk_step = if configured_step > 0.0 {
            configured_step
        } else {
            1.0
        };
        let scale =
            gtk4::Scale::with_range(gtk4::Orientation::Horizontal, minimum, maximum, gtk_step);
        scale.set_widget_name(widget_name);
        attach_model(scale, self).widget().clone()
    }
}

fn install_scale_value_format(scale: &gtk4::Scale, model: Rc<RefCell<BauhausSliderModel>>) {
    scale.set_format_value_func(move |_, value| source_closed_value_text(&model.borrow(), value));
}

fn source_closed_value_text(model: &BauhausSliderModel, value: f64) -> String {
    model.value_text(value)
}

fn source_scale_inverted(model: &BauhausSliderModel) -> bool {
    model.factor() < 0.0
}

fn sync_scale_from_model(
    scale: &gtk4::Scale,
    model: &RefCell<BauhausSliderModel>,
    sync_guard: &Cell<bool>,
) {
    let (value, (minimum, maximum), digits, step) = {
        let model = model.borrow();
        (
            model.value(),
            model.visible_range(),
            model.digits(),
            model.effective_step(source_automatic_step_policy()).abs(),
        )
    };
    let adjustment = scale.adjustment();
    let previous_guard = sync_guard.replace(true);
    scale.set_inverted(source_scale_inverted(&model.borrow()));
    scale.set_digits(digits);
    adjustment.configure(
        value,
        minimum,
        maximum,
        step,
        adjustment.page_increment(),
        adjustment.page_size(),
    );
    sync_guard.set(previous_guard);
}

fn install_scale_model_sync(
    scale: &gtk4::Scale,
    model: Rc<RefCell<BauhausSliderModel>>,
    sync_guard: Rc<Cell<bool>>,
    action_coalescer: Rc<ActionCoalescer>,
) {
    scale.connect_value_changed(move |scale| {
        if action_coalescer.flushing.get() {
            return;
        }
        if sync_guard.get() {
            if action_coalescer.deferred.get() {
                scale.stop_signal_emission_by_name("value-changed");
                action_coalescer.defer_change(scale);
            }
            return;
        }
        let gtk_value = scale.value();
        model.borrow_mut().set_value(gtk_value);
        let model_value = model.borrow().value();
        if action_coalescer.deferred.get() {
            scale.stop_signal_emission_by_name("value-changed");
            action_coalescer.defer_change(scale);
        } else if model_value.to_bits() != gtk_value.to_bits() {
            scale.stop_signal_emission_by_name("value-changed");
        }
        sync_scale_from_model(scale, &model, &sync_guard);
    });
}

pub(crate) fn attach(scale: gtk4::Scale, spec: SliderInputSpec) -> BauhausSlider {
    let model = model_from_scale(&scale, spec);
    attach_model(scale, model)
}

fn attach_model(scale: gtk4::Scale, model: BauhausSliderModel) -> BauhausSlider {
    scale.set_focusable(true);
    let input = Rc::new(RefCell::new(NumericInputBuffer::new()));
    let sync_guard = Rc::new(Cell::new(false));
    let action_coalescer = Rc::new(ActionCoalescer::default());
    let model = Rc::new(RefCell::new(model));
    install_scale_value_format(&scale, Rc::clone(&model));
    install_scale_model_sync(
        &scale,
        Rc::clone(&model),
        Rc::clone(&sync_guard),
        Rc::clone(&action_coalescer),
    );
    let opening_position = Rc::new(Cell::new(model.borrow().normalized_position()));
    let accepted = Rc::new(Cell::new(false));
    let change_active = Rc::new(Cell::new(false));
    let mouse_line_distance = Rc::new(Cell::new(0.0));
    let primary_button_down = Rc::new(Cell::new(false));

    let expression = gtk4::Label::new(None);
    expression.set_widget_name("bauhaus-slider-expression");
    expression.set_halign(gtk4::Align::End);
    expression.set_valign(gtk4::Align::Center);
    expression.set_hexpand(true);
    expression.set_vexpand(true);
    expression.set_wrap(true);
    expression.add_css_class("dt_bauhaus_numeric_text");

    let current_value = gtk4::Label::new(None);
    current_value.set_widget_name("bauhaus-slider-current-value");
    current_value.set_halign(gtk4::Align::End);
    current_value.set_valign(gtk4::Align::Start);
    current_value.set_can_target(false);
    current_value.set_focusable(false);
    current_value.add_css_class("dt_bauhaus_current_value");

    let minimum_value = popup_range_label("bauhaus-slider-minimum-value", gtk4::Align::Start);
    let maximum_value = popup_range_label("bauhaus-slider-maximum-value", gtk4::Align::End);

    let fine_tune_surface = gtk4::DrawingArea::new();
    fine_tune_surface.set_widget_name("bauhaus-slider-fine-tune-surface");
    fine_tune_surface.set_hexpand(true);
    fine_tune_surface.set_vexpand(true);
    fine_tune_surface.set_halign(gtk4::Align::Fill);
    fine_tune_surface.set_valign(gtk4::Align::Fill);
    fine_tune_surface.set_can_target(false);
    fine_tune_surface.set_focusable(false);
    fine_tune_surface.set_accessible_role(gtk4::AccessibleRole::Presentation);
    fine_tune_surface.add_css_class("dt_bauhaus_fine_tune");
    install_fine_tune_drawing(
        &fine_tune_surface,
        Rc::clone(&model),
        Rc::clone(&opening_position),
    );

    let surface = gtk4::Overlay::new();
    surface.set_widget_name("bauhaus-slider-fine-tune-content");
    surface.set_child(Some(&fine_tune_surface));
    surface.add_overlay(&expression);
    surface.add_overlay(&current_value);
    surface.add_overlay(&minimum_value);
    surface.add_overlay(&maximum_value);

    let popup = gtk4::Popover::builder()
        .autohide(true)
        .has_arrow(false)
        .child(&surface)
        .build();
    popup.set_widget_name("bauhaus-slider");
    popup.set_position(gtk4::PositionType::Bottom);
    popup.set_halign(gtk4::Align::Start);
    popup.set_focusable(true);
    popup.add_css_class("dt_bauhaus_popup");
    install_popup_presentation_sync(
        &scale,
        &popup,
        &fine_tune_surface,
        &current_value,
        &minimum_value,
        &maximum_value,
        Rc::clone(&model),
    );
    install_popup_pointer_input(
        &surface,
        &popup,
        &scale,
        &fine_tune_surface,
        &current_value,
        &minimum_value,
        &maximum_value,
        &opening_position,
        &accepted,
        &change_active,
        &mouse_line_distance,
        &primary_button_down,
        &model,
        &sync_guard,
    );

    let anchor = gtk4::MenuButton::new();
    let anchor_content = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    anchor_content.set_size_request(1, 1);
    anchor_content.add_css_class("dt_bauhaus_anchor_content");
    anchor.set_widget_name("bauhaus-slider-anchor");
    anchor.set_halign(gtk4::Align::Start);
    anchor.set_valign(gtk4::Align::Start);
    anchor.set_size_request(1, 1);
    anchor.set_focusable(false);
    anchor.set_can_target(false);
    anchor.set_accessible_role(gtk4::AccessibleRole::Presentation);
    anchor.set_has_frame(false);
    anchor.set_always_show_arrow(false);
    anchor.set_child(Some(&anchor_content));
    anchor.add_css_class("dt_bauhaus_anchor");
    anchor.set_popover(Some(&popup));

    let root = gtk4::Overlay::new();
    root.add_css_class("dt_bauhaus");
    root.set_child(Some(&scale));
    root.add_overlay(&anchor);

    install_scale_keyboard(
        &scale,
        &anchor,
        &popup,
        &expression,
        &current_value,
        &minimum_value,
        &maximum_value,
        &fine_tune_surface,
        Rc::clone(&input),
        Rc::clone(&opening_position),
        Rc::clone(&accepted),
        Rc::clone(&model),
        Rc::clone(&sync_guard),
    );
    install_closed_slider_input(
        &scale,
        &popup,
        &fine_tune_surface,
        &current_value,
        &minimum_value,
        &maximum_value,
        &opening_position,
        &model,
        &sync_guard,
        &action_coalescer,
    );
    install_secondary_click(
        &scale,
        &anchor,
        &popup,
        &expression,
        &current_value,
        &minimum_value,
        &maximum_value,
        &fine_tune_surface,
        Rc::clone(&input),
        Rc::clone(&opening_position),
        Rc::clone(&accepted),
        Rc::clone(&model),
    );
    install_popup_keyboard(
        &scale,
        &popup,
        &expression,
        &current_value,
        Rc::clone(&input),
        Rc::clone(&accepted),
        Rc::clone(&model),
        Rc::clone(&sync_guard),
    );
    install_rejection(
        &scale,
        &expression,
        &current_value,
        input,
        opening_position,
        accepted,
        &popup,
        Rc::clone(&model),
        Rc::clone(&sync_guard),
    );

    let digits = model.borrow().digits();
    let slider = BauhausSlider {
        root,
        scale,
        model,
        sync_guard,
    };
    slider.set_digits(digits);
    slider
}

fn popup_range_label(name: &str, horizontal_alignment: gtk4::Align) -> gtk4::Label {
    let label = gtk4::Label::new(None);
    label.set_widget_name(name);
    label.set_halign(horizontal_alignment);
    label.set_valign(gtk4::Align::Start);
    label.set_can_target(false);
    label.set_focusable(false);
    label.set_sensitive(false);
    label.add_css_class("dt_bauhaus_range_value");
    label
}

fn install_popup_presentation_sync(
    scale: &gtk4::Scale,
    popup: &gtk4::Popover,
    fine_tune_surface: &gtk4::DrawingArea,
    current_value: &gtk4::Label,
    minimum_value: &gtk4::Label,
    maximum_value: &gtk4::Label,
    model: Rc<RefCell<BauhausSliderModel>>,
) {
    let popup = popup.downgrade();
    let fine_tune_surface = fine_tune_surface.downgrade();
    let current_value = current_value.downgrade();
    let minimum_value = minimum_value.downgrade();
    let maximum_value = maximum_value.downgrade();
    scale.connect_value_changed(move |_| {
        let (
            Some(popup),
            Some(fine_tune_surface),
            Some(current_value),
            Some(minimum_value),
            Some(maximum_value),
        ) = (
            popup.upgrade(),
            fine_tune_surface.upgrade(),
            current_value.upgrade(),
            minimum_value.upgrade(),
            maximum_value.upgrade(),
        )
        else {
            return;
        };
        if popup.is_visible() {
            update_popup_values(&model, &current_value, &minimum_value, &maximum_value);
            position_range_labels(&fine_tune_surface, &minimum_value, &maximum_value);
            fine_tune_surface.queue_draw();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn install_popup_pointer_input(
    content: &gtk4::Overlay,
    popup: &gtk4::Popover,
    scale: &gtk4::Scale,
    fine_tune_surface: &gtk4::DrawingArea,
    current_value: &gtk4::Label,
    minimum_value: &gtk4::Label,
    maximum_value: &gtk4::Label,
    opening_position: &Rc<Cell<f64>>,
    accepted: &Rc<Cell<bool>>,
    change_active: &Rc<Cell<bool>>,
    mouse_line_distance: &Rc<Cell<f64>>,
    primary_button_down: &Rc<Cell<bool>>,
    model: &Rc<RefCell<BauhausSliderModel>>,
    sync_guard: &Rc<Cell<bool>>,
) {
    {
        let change_active = Rc::clone(change_active);
        let mouse_line_distance = Rc::clone(mouse_line_distance);
        let primary_button_down = Rc::clone(primary_button_down);
        popup.connect_show(move |_| {
            change_active.set(false);
            mouse_line_distance.set(0.0);
            primary_button_down.set(false);
        });
    }

    let motion = gtk4::EventControllerMotion::new();
    motion.set_name(Some(POPUP_MOTION_CONTROLLER_NAME));
    motion.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let scale = scale.downgrade();
        let fine_tune_surface = fine_tune_surface.downgrade();
        let current_value = current_value.downgrade();
        let minimum_value = minimum_value.downgrade();
        let maximum_value = maximum_value.downgrade();
        let opening_position = Rc::clone(opening_position);
        let change_active = Rc::clone(change_active);
        let mouse_line_distance = Rc::clone(mouse_line_distance);
        let primary_button_down = Rc::clone(primary_button_down);
        let model = Rc::clone(model);
        let sync_guard = Rc::clone(sync_guard);
        motion.connect_motion(move |controller, x, y| {
            let (
                Some(scale),
                Some(fine_tune_surface),
                Some(current_value),
                Some(minimum_value),
                Some(maximum_value),
            ) = (
                scale.upgrade(),
                fine_tune_surface.upgrade(),
                current_value.upgrade(),
                minimum_value.upgrade(),
                maximum_value.upgrade(),
            )
            else {
                return;
            };
            let button_is_down = primary_button_down.get()
                || controller
                    .current_event_state()
                    .contains(gdk::ModifierType::BUTTON1_MASK);
            source_popup_motion(
                &scale,
                &fine_tune_surface,
                &current_value,
                &minimum_value,
                &maximum_value,
                &opening_position,
                &change_active,
                &mouse_line_distance,
                button_is_down,
                &model,
                &sync_guard,
                x,
                y,
            );
        });
    }
    content.add_controller(motion);

    let click = gtk4::GestureClick::new();
    click.set_name(Some(POPUP_CLICK_CONTROLLER_NAME));
    click.set_button(0);
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let popup = popup.downgrade();
        let scale = scale.downgrade();
        let fine_tune_surface = fine_tune_surface.downgrade();
        let current_value = current_value.downgrade();
        let minimum_value = minimum_value.downgrade();
        let maximum_value = maximum_value.downgrade();
        let opening_position = Rc::clone(opening_position);
        let change_active = Rc::clone(change_active);
        let mouse_line_distance = Rc::clone(mouse_line_distance);
        let primary_button_down = Rc::clone(primary_button_down);
        let model = Rc::clone(model);
        let sync_guard = Rc::clone(sync_guard);
        click.connect_pressed(move |gesture, _, x, y| {
            let (
                Some(popup),
                Some(scale),
                Some(fine_tune_surface),
                Some(current_value),
                Some(minimum_value),
                Some(maximum_value),
            ) = (
                popup.upgrade(),
                scale.upgrade(),
                fine_tune_surface.upgrade(),
                current_value.upgrade(),
                minimum_value.upgrade(),
                maximum_value.upgrade(),
            )
            else {
                return;
            };
            match source_gesture_button(gesture) {
                PRIMARY_BUTTON => {
                    primary_button_down.set(true);
                    change_active.set(true);
                    source_popup_motion(
                        &scale,
                        &fine_tune_surface,
                        &current_value,
                        &minimum_value,
                        &maximum_value,
                        &opening_position,
                        &change_active,
                        &mouse_line_distance,
                        true,
                        &model,
                        &sync_guard,
                        x,
                        y,
                    );
                }
                MIDDLE_BUTTON => {
                    source_zoom_range(
                        &scale,
                        &fine_tune_surface,
                        &current_value,
                        &minimum_value,
                        &maximum_value,
                        &opening_position,
                        &change_active,
                        &mouse_line_distance,
                        &model,
                        &sync_guard,
                        0.0,
                    );
                }
                _ => popup.popdown(),
            }
            let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
    }
    {
        let popup = popup.downgrade();
        let accepted = Rc::clone(accepted);
        let change_active = Rc::clone(change_active);
        let primary_button_down = Rc::clone(primary_button_down);
        click.connect_released(move |gesture, _, _, _| {
            let button = source_gesture_button(gesture);
            if button == PRIMARY_BUTTON {
                primary_button_down.set(false);
            }
            if change_active.get()
                && button != MIDDLE_BUTTON
                && let Some(popup) = popup.upgrade()
            {
                accepted.set(true);
                popup.popdown();
            }
            let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
    }
    content.add_controller(click);

    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::BOTH_AXES);
    scroll.set_name(Some(POPUP_SCROLL_CONTROLLER_NAME));
    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let scale = scale.downgrade();
        let fine_tune_surface = fine_tune_surface.downgrade();
        let current_value = current_value.downgrade();
        let minimum_value = minimum_value.downgrade();
        let maximum_value = maximum_value.downgrade();
        let opening_position = Rc::clone(opening_position);
        let change_active = Rc::clone(change_active);
        let mouse_line_distance = Rc::clone(mouse_line_distance);
        let model = Rc::clone(model);
        let sync_guard = Rc::clone(sync_guard);
        scroll.connect_scroll(move |controller, delta_x, delta_y| {
            if controller
                .current_event()
                .is_some_and(|event| event.is_pointer_emulated())
            {
                return glib::Propagation::Stop;
            }
            if let Some(delta) = source_scroll_unit_delta(controller, delta_x, delta_y)
                && let (
                    Some(scale),
                    Some(fine_tune_surface),
                    Some(current_value),
                    Some(minimum_value),
                    Some(maximum_value),
                ) = (
                    scale.upgrade(),
                    fine_tune_surface.upgrade(),
                    current_value.upgrade(),
                    minimum_value.upgrade(),
                    maximum_value.upgrade(),
                )
            {
                source_zoom_range(
                    &scale,
                    &fine_tune_surface,
                    &current_value,
                    &minimum_value,
                    &maximum_value,
                    &opening_position,
                    &change_active,
                    &mouse_line_distance,
                    &model,
                    &sync_guard,
                    f64::from(delta),
                );
            }
            glib::Propagation::Stop
        });
    }
    scroll.connect_scroll_end(|_| {
        reset_source_scroll_units();
    });
    content.add_controller(scroll);
}

fn source_scroll_unit_delta(
    controller: &gtk4::EventControllerScroll,
    delta_x: f64,
    delta_y: f64,
) -> Option<i32> {
    if controller.unit() == gdk::ScrollUnit::Wheel {
        let x = scroll_direction(delta_x);
        let y = scroll_direction(delta_y);
        let delta = x.saturating_add(y);
        return (delta != 0).then_some(delta);
    }

    source_smooth_scroll_unit_delta(delta_x, delta_y)
}

fn source_smooth_scroll_unit_delta(delta_x: f64, delta_y: f64) -> Option<i32> {
    #[cfg(target_os = "macos")]
    let (delta_x, delta_y) = (delta_x / 50.0, delta_y / 50.0);
    SOURCE_SCROLL_UNITS.with(|units| units.borrow_mut().push_sum(delta_x, delta_y))
}

fn reset_source_scroll_units() {
    SOURCE_SCROLL_UNITS.with(|units| units.borrow_mut().stop());
}

fn source_gesture_button(gesture: &gtk4::GestureClick) -> u32 {
    let current = gesture.current_button();
    if current == 0 {
        gesture.button()
    } else {
        current
    }
}

fn request_source_focus(scale: &gtk4::Scale) {
    scale.grab_focus();
}

fn scroll_direction(delta: f64) -> i32 {
    match delta.total_cmp(&0.0) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

#[allow(clippy::too_many_arguments)]
fn source_popup_motion(
    scale: &gtk4::Scale,
    fine_tune_surface: &gtk4::DrawingArea,
    current_value: &gtk4::Label,
    minimum_value: &gtk4::Label,
    maximum_value: &gtk4::Label,
    opening_position: &Cell<f64>,
    change_active: &Cell<bool>,
    mouse_line_distance: &Cell<f64>,
    primary_button_down: bool,
    model: &RefCell<BauhausSliderModel>,
    sync_guard: &Cell<bool>,
    x: f64,
    y: f64,
) {
    let width = f64::from(fine_tune_surface.allocated_width().max(1));
    let old_position = opening_position.get();
    let pointer_position = {
        let model = model.borrow();
        let (minimum, maximum) = model.visible_range();
        linear_pointer_position(
            old_position,
            loupe_scale(
                model.digits(),
                SliderRange::new(minimum, maximum),
                model.factor(),
            ),
            x / width,
            y / width,
            FineTuneMetrics::for_surface(fine_tune_surface).header_height / width,
        )
    };
    let offset = pointer_position - old_position;
    if should_activate_change(primary_button_down, mouse_line_distance.get(), offset) {
        change_active.set(true);
    }
    mouse_line_distance.set(offset);
    if change_active.get() {
        let previous = scale.value();
        model.borrow_mut().set_normalized_position(pointer_position);
        sync_source_change(scale, model, sync_guard, previous);
        update_popup_values(model, current_value, minimum_value, maximum_value);
        fine_tune_surface.queue_draw();
    }
}

#[allow(clippy::too_many_arguments)]
fn source_zoom_range(
    scale: &gtk4::Scale,
    fine_tune_surface: &gtk4::DrawingArea,
    current_value: &gtk4::Label,
    minimum_value: &gtk4::Label,
    maximum_value: &gtk4::Label,
    opening_position: &Cell<f64>,
    change_active: &Cell<bool>,
    mouse_line_distance: &Cell<f64>,
    model: &RefCell<BauhausSliderModel>,
    sync_guard: &Cell<bool>,
    zoom: f64,
) {
    change_active.set(false);
    mouse_line_distance.set(0.0);
    let change = {
        let model = model.borrow();
        let (minimum, maximum) = model.visible_range();
        let (soft_minimum, soft_maximum) = model.soft_range();
        let (hard_minimum, hard_maximum) = model.hard_range();
        zoom_range(
            SliderRanges::new(
                SliderRange::new(minimum, maximum),
                SliderRange::new(soft_minimum, soft_maximum),
                SliderRange::new(hard_minimum, hard_maximum),
            ),
            model.value(),
            model.digits(),
            model.factor(),
            zoom,
        )
    };
    let toggled = matches!(change, ZoomRangeChange::Toggled(_));
    let changed = match change {
        ZoomRangeChange::Zoomed(range) => model
            .borrow_mut()
            .set_visible_range_preserving_position(range.minimum(), range.maximum()),
        ZoomRangeChange::Toggled(range) => {
            let changed = model
                .borrow_mut()
                .set_visible_range_preserving_value(range.minimum(), range.maximum());
            opening_position.set(model.borrow().normalized_position());
            changed
        }
        ZoomRangeChange::Rejected => false,
    };
    if changed {
        if toggled {
            let previous = scale.value();
            sync_source_change(scale, model, sync_guard, previous);
        } else {
            sync_scale_from_model(scale, model, sync_guard);
        }
        update_popup_values(model, current_value, minimum_value, maximum_value);
    }
    fine_tune_surface.queue_draw();
}

fn update_popup_values(
    model: &RefCell<BauhausSliderModel>,
    current_value: &gtk4::Label,
    minimum_value: &gtk4::Label,
    maximum_value: &gtk4::Label,
) {
    let model = model.borrow();
    let (minimum, maximum) = model.visible_range();
    current_value.set_text(&model.value_text(model.value()));
    if model.factor() > 0.0 {
        minimum_value.set_text(&model.value_text(minimum));
        maximum_value.set_text(&model.value_text(maximum));
    } else {
        minimum_value.set_text(&model.value_text(maximum));
        maximum_value.set_text(&model.value_text(minimum));
    }
}

fn position_range_labels(
    fine_tune_surface: &gtk4::DrawingArea,
    minimum_value: &gtk4::Label,
    maximum_value: &gtk4::Label,
) {
    let line_height = source_line_height(fine_tune_surface);
    let top = line_height.saturating_add(INNER_PADDING.saturating_mul(3));
    minimum_value.set_margin_top(top);
    maximum_value.set_margin_top(top);
}

fn source_line_height(surface: &gtk4::DrawingArea) -> i32 {
    let layout = surface.create_pango_layout(Some("m"));
    layout.pixel_size().1.max(1)
}

#[derive(Debug, Clone, Copy)]
struct FineTuneMetrics {
    baseline_top: f64,
    baseline_thickness: f64,
    marker_size: f64,
    header_height: f64,
}

impl FineTuneMetrics {
    fn for_surface(surface: &gtk4::DrawingArea) -> Self {
        let line_height = f64::from(source_line_height(surface));
        let inner_padding = f64::from(INNER_PADDING);
        let baseline_size = line_height / 3.0;
        Self {
            baseline_top: line_height + inner_padding,
            baseline_thickness: (baseline_size - SOURCE_BORDER_WIDTH).max(0.0),
            marker_size: (baseline_size + SOURCE_BORDER_WIDTH) * 0.95,
            header_height: line_height + inner_padding * 2.0,
        }
    }
}

fn install_fine_tune_drawing(
    surface: &gtk4::DrawingArea,
    model: Rc<RefCell<BauhausSliderModel>>,
    opening_position: Rc<Cell<f64>>,
) {
    surface.set_draw_func(move |surface, context, width, height| {
        if width <= 0 || height <= 0 {
            return;
        }
        draw_fine_tune_surface(
            surface,
            context,
            width,
            height,
            &model.borrow(),
            opening_position.get(),
        );
    });
}

#[allow(clippy::float_cmp)]
fn draw_fine_tune_surface(
    surface: &gtk4::DrawingArea,
    context: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    model: &BauhausSliderModel,
    opening_position: f64,
) {
    let metrics = FineTuneMetrics::for_surface(surface);
    let width = f64::from(width);
    let height = f64::from(height);
    let (minimum, maximum) = model.visible_range();
    let visible = SliderRange::new(minimum, maximum);

    draw_source_baseline(context, width, model, metrics);

    let scale = loupe_scale(model.digits(), visible, model.factor());
    if scale.is_finite() && scale > 0.0 {
        context
            .save()
            .expect("fine-tune guideline context can be saved");
        context.rectangle(
            0.0,
            metrics.header_height,
            width,
            (height - metrics.header_height).max(0.0),
        );
        context.clip();
        context.set_line_width(0.5);

        let mut index = 0.0;
        let guideline_count = (1.0 / scale).floor();
        while index < guideline_count {
            let offset = index * scale - opening_position;
            let alpha = if offset == 0.0 {
                1.0
            } else {
                (scale / offset.abs()).min(1.0)
            };
            set_source_shaded_token(context, DARKTABLE_COLORS.foreground.rgba(), 0.95, alpha);
            draw_source_guideline(
                context,
                opening_position,
                offset,
                scale,
                width,
                height,
                metrics.header_height,
            );
            index += 1.0;
        }
        context
            .restore()
            .expect("fine-tune guideline context can be restored");

        context
            .save()
            .expect("active guideline context can be saved");
        context.set_line_width(2.0);
        set_source_shaded_token(context, DARKTABLE_COLORS.foreground.rgba(), 0.95, 1.0);
        draw_source_guideline(
            context,
            opening_position,
            model.normalized_position() - opening_position,
            scale,
            width,
            height,
            metrics.header_height,
        );
        context
            .restore()
            .expect("active guideline context can be restored");
    }

    draw_source_indicator(context, width, model, metrics);
}

#[allow(clippy::float_cmp)]
fn draw_source_baseline(
    context: &gtk4::cairo::Context,
    width: f64,
    model: &BauhausSliderModel,
    metrics: FineTuneMetrics,
) {
    context
        .save()
        .expect("Bauhaus baseline context can be saved");
    context.rectangle(0.0, metrics.baseline_top, width, metrics.baseline_thickness);
    set_source_shaded_token(
        context,
        DARKTABLE_COLORS.module_background.rgba(),
        0.75,
        1.0,
    );
    let _ = context.fill();

    let (minimum, maximum) = model.visible_range();
    let (soft_minimum, soft_maximum) = model.soft_range();
    let range_width = maximum - minimum;
    if range_width > 0.0 && (minimum != soft_minimum || maximum != soft_maximum) {
        let scale = width / range_width;
        set_source_shaded_token(context, DARKTABLE_COLORS.foreground.rgba(), 0.95, 0.5);
        context.rectangle(
            0.0,
            metrics.baseline_top,
            scale * (soft_minimum - minimum).max(0.0),
            metrics.baseline_thickness,
        );
        let _ = context.fill();
        let upper = scale * (soft_maximum.min(maximum) - minimum);
        context.rectangle(
            upper,
            metrics.baseline_top,
            width - upper,
            metrics.baseline_thickness,
        );
        let _ = context.fill();
    }

    if model.fill_feedback() && range_width > 0.0 {
        let origin_value = if model.factor() > 0.0 {
            -minimum - model.offset() / model.factor()
        } else {
            maximum + model.offset() / model.factor()
        };
        let origin = (origin_value / range_width).clamp(0.0, 1.0) * width;
        let position = model.normalized_position() * width;
        context.set_operator(gtk4::cairo::Operator::Screen);
        set_source_token(context, DARKTABLE_COLORS.thumbnail_background.rgba(), 1.0);
        context.rectangle(
            origin,
            metrics.baseline_top,
            position - origin,
            metrics.baseline_thickness,
        );
        let _ = context.fill();
    }
    context
        .restore()
        .expect("Bauhaus baseline context can be restored");
}

fn draw_source_guideline(
    context: &gtk4::cairo::Context,
    position: f64,
    offset: f64,
    scale: f64,
    width: f64,
    height: f64,
    header_height: f64,
) {
    context.move_to(width * (position + offset), header_height * 0.7);
    context.line_to(width * (position + offset), header_height);
    for step in 1..GUIDELINE_STEPS {
        let y = f64::from(step) / f64::from(GUIDELINE_STEPS - 1);
        let y_squared = y * y;
        let x = y_squared * 0.5 * (1.0 + offset / scale) + (1.0 - y_squared) * (position + offset);
        context.line_to(x * width, header_height + y * (height - header_height));
    }
    let _ = context.stroke();
}

fn draw_source_indicator(
    context: &gtk4::cairo::Context,
    width: f64,
    model: &BauhausSliderModel,
    metrics: FineTuneMetrics,
) {
    context
        .save()
        .expect("Bauhaus indicator context can be saved");
    context.translate(
        model.normalized_position() * width,
        metrics.baseline_top + metrics.baseline_thickness / 2.0,
    );
    context.scale(1.0, -1.0);
    context.set_line_cap(gtk4::cairo::LineCap::Round);

    draw_triangle_indicator(context, metrics.marker_size);
    context.set_line_width(SOURCE_BORDER_WIDTH);
    set_source_token(
        context,
        DARKTABLE_COLORS.active_field_background.rgba(),
        1.0,
    );
    let _ = context.stroke();

    let inner_size = (metrics.marker_size - SOURCE_BORDER_WIDTH).max(0.0);
    draw_triangle_indicator(context, inner_size);
    context.set_line_width(SOURCE_BORDER_WIDTH);
    set_source_shaded_token(context, DARKTABLE_COLORS.foreground.rgba(), 0.95, 1.0);
    if model.fill_feedback() || model.stops().is_empty() {
        let _ = context.fill();
    } else {
        let _ = context.stroke();
    }
    context
        .restore()
        .expect("Bauhaus indicator context can be restored");
}

fn draw_triangle_indicator(context: &gtk4::cairo::Context, radius: f64) {
    let side = 0.866_025_404 * radius;
    let vertical = 0.5 * radius;
    context.move_to(0.0, radius);
    context.line_to(-side, -vertical);
    context.line_to(side, -vertical);
    context.close_path();
}

fn set_source_token(context: &gtk4::cairo::Context, rgba: [u8; 4], alpha: f64) {
    context.set_source_rgba(
        f64::from(rgba[0]) / 255.0,
        f64::from(rgba[1]) / 255.0,
        f64::from(rgba[2]) / 255.0,
        f64::from(rgba[3]) / 255.0 * alpha,
    );
}

fn set_source_shaded_token(context: &gtk4::cairo::Context, rgba: [u8; 4], shade: f64, alpha: f64) {
    context.set_source_rgba(
        f64::from(rgba[0]) / 255.0 * shade,
        f64::from(rgba[1]) / 255.0 * shade,
        f64::from(rgba[2]) / 255.0 * shade,
        f64::from(rgba[3]) / 255.0 * alpha,
    );
}

#[allow(clippy::cast_possible_truncation, clippy::too_many_arguments)]
fn install_closed_slider_input(
    scale: &gtk4::Scale,
    popup: &gtk4::Popover,
    fine_tune_surface: &gtk4::DrawingArea,
    current_value: &gtk4::Label,
    minimum_value: &gtk4::Label,
    maximum_value: &gtk4::Label,
    opening_position: &Rc<Cell<f64>>,
    model: &Rc<RefCell<BauhausSliderModel>>,
    sync_guard: &Rc<Cell<bool>>,
    action_coalescer: &Rc<ActionCoalescer>,
) {
    let dragging = Rc::new(Cell::new(false));
    let mouse_x = Rc::new(Cell::new(f64::NAN));
    let force_element = Rc::new(Cell::new(false));

    let motion = gtk4::EventControllerMotion::new();
    motion.set_name(Some(MAIN_MOTION_CONTROLLER_NAME));
    motion.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let scale = scale.downgrade();
        let dragging = Rc::clone(&dragging);
        let mouse_x = Rc::clone(&mouse_x);
        let force_element = Rc::clone(&force_element);
        let model = Rc::clone(model);
        let sync_guard = Rc::clone(sync_guard);
        motion.connect_motion(move |controller, x, _| {
            let Some(scale) = scale.upgrade() else {
                return;
            };
            let width = f64::from(scale.allocated_width().max(1));
            let position = x / width;
            force_element.set(!(0.1..=0.9).contains(&position));
            if !dragging.get() {
                scale.queue_draw();
                return;
            }

            let modifiers = controller.current_event_state();
            let reference = mouse_x.get();
            if reference.is_nan() {
                if source_has_drag_modifier(modifiers) {
                    mouse_x.set(x);
                } else {
                    source_set_normalized(&scale, &model, &sync_guard, position);
                }
            } else {
                let (minimum, maximum, factor, step) = {
                    let model = model.borrow();
                    let (minimum, maximum) = model.visible_range();
                    (
                        minimum,
                        maximum,
                        model.factor(),
                        model.effective_step(source_automatic_step_policy()),
                    )
                };
                if let Some((delta, updated_reference)) = source_relative_drag_transition(
                    width,
                    maximum - minimum,
                    step,
                    factor,
                    x,
                    reference,
                ) {
                    source_add_step(&scale, &model, &sync_guard, delta, modifiers);
                    mouse_x.set(updated_reference);
                }
            }
            scale.queue_draw();
        });
    }
    {
        let scale = scale.downgrade();
        motion.connect_enter(move |_, _, _| {
            if let Some(scale) = scale.upgrade() {
                scale.queue_draw();
            }
        });
    }
    {
        let scale = scale.downgrade();
        let force_element = Rc::clone(&force_element);
        motion.connect_leave(move |_| {
            force_element.set(false);
            if let Some(scale) = scale.upgrade() {
                scale.queue_draw();
            }
        });
    }
    scale.add_controller(motion);

    let primary = gtk4::GestureClick::new();
    primary.set_name(Some(MAIN_CLICK_CONTROLLER_NAME));
    primary.set_button(PRIMARY_BUTTON);
    primary.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let scale = scale.downgrade();
        let popup = popup.downgrade();
        let dragging = Rc::clone(&dragging);
        let mouse_x = Rc::clone(&mouse_x);
        let model = Rc::clone(model);
        let sync_guard = Rc::clone(sync_guard);
        let action_coalescer = Rc::clone(action_coalescer);
        primary.connect_pressed(move |gesture, press_count, x, y| {
            let (Some(scale), Some(popup)) = (scale.upgrade(), popup.upgrade()) else {
                return;
            };
            request_source_focus(&scale);
            scale.queue_draw();
            if press_count == 2 {
                dragging.set(false);
                action_coalescer.finish(&scale);
                let previous = scale.value();
                model.borrow_mut().reset();
                sync_source_change(&scale, &model, &sync_guard, previous);
                popup.popdown();
            } else {
                dragging.set(true);
                action_coalescer.begin();
                let width = f64::from(scale.allocated_width().max(1));
                let line_height = f64::from(scale.create_pango_layout(Some("m")).pixel_size().1);
                let modifiers = gesture.current_event_state();
                if !source_has_drag_modifier(modifiers) && x <= width && y > line_height * 0.5 {
                    mouse_x.set(f64::NAN);
                    source_set_normalized(&scale, &model, &sync_guard, x / width);
                } else {
                    mouse_x.set(x);
                }
            }
            let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
    }
    {
        let scale = scale.downgrade();
        let dragging = Rc::clone(&dragging);
        let action_coalescer = Rc::clone(action_coalescer);
        primary.connect_released(move |gesture, _, _, _| {
            dragging.set(false);
            if let Some(scale) = scale.upgrade() {
                action_coalescer.finish(&scale);
                scale.queue_draw();
            }
            let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
    }
    {
        let scale = scale.downgrade();
        let action_coalescer = Rc::clone(action_coalescer);
        primary.connect_stopped(move |_| {
            if let Some(scale) = scale.upgrade() {
                action_coalescer.flush(&scale);
            }
        });
    }
    scale.add_controller(primary);

    let middle = gtk4::GestureClick::new();
    middle.set_name(Some(MAIN_MIDDLE_CONTROLLER_NAME));
    middle.set_button(MIDDLE_BUTTON);
    middle.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let scale = scale.downgrade();
        let fine_tune_surface = fine_tune_surface.downgrade();
        let current_value = current_value.downgrade();
        let minimum_value = minimum_value.downgrade();
        let maximum_value = maximum_value.downgrade();
        let opening_position = Rc::clone(opening_position);
        let model = Rc::clone(model);
        let sync_guard = Rc::clone(sync_guard);
        middle.connect_pressed(move |gesture, _, _, _| {
            let (
                Some(scale),
                Some(fine_tune_surface),
                Some(current_value),
                Some(minimum_value),
                Some(maximum_value),
            ) = (
                scale.upgrade(),
                fine_tune_surface.upgrade(),
                current_value.upgrade(),
                minimum_value.upgrade(),
                maximum_value.upgrade(),
            )
            else {
                return;
            };
            request_source_focus(&scale);
            source_zoom_range(
                &scale,
                &fine_tune_surface,
                &current_value,
                &minimum_value,
                &maximum_value,
                &opening_position,
                &Cell::new(false),
                &Cell::new(0.0),
                &model,
                &sync_guard,
                0.0,
            );
            let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
        });
    }
    scale.add_controller(middle);

    // Darktable installs the closed scroll controller in bubble phase. A GTK4
    // Scale already owns a native target-phase scroll handler, so this adapter
    // must capture and stop first to avoid a second, differently stepped edit.
    let scroll = gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::BOTH_AXES);
    scroll.set_name(Some(MAIN_SCROLL_CONTROLLER_NAME));
    scroll.set_propagation_phase(gtk4::PropagationPhase::Capture);
    {
        let scale = scale.downgrade();
        let fine_tune_surface = fine_tune_surface.downgrade();
        let current_value = current_value.downgrade();
        let minimum_value = minimum_value.downgrade();
        let maximum_value = maximum_value.downgrade();
        let opening_position = Rc::clone(opening_position);
        let force_element = Rc::clone(&force_element);
        let model = Rc::clone(model);
        let sync_guard = Rc::clone(sync_guard);
        scroll.connect_scroll(move |controller, delta_x, delta_y| {
            if controller
                .current_event()
                .is_some_and(|event| event.is_pointer_emulated())
            {
                return glib::Propagation::Stop;
            }
            let Some(delta) = source_scroll_unit_delta(controller, delta_x, delta_y) else {
                return glib::Propagation::Stop;
            };
            let Some(scale) = scale.upgrade() else {
                return glib::Propagation::Stop;
            };
            request_source_focus(&scale);
            let modifiers = controller.current_event_state();
            if force_element.get() && source_force_range_step(modifiers) {
                if let (
                    Some(fine_tune_surface),
                    Some(current_value),
                    Some(minimum_value),
                    Some(maximum_value),
                ) = (
                    fine_tune_surface.upgrade(),
                    current_value.upgrade(),
                    minimum_value.upgrade(),
                    maximum_value.upgrade(),
                ) {
                    source_zoom_range(
                        &scale,
                        &fine_tune_surface,
                        &current_value,
                        &minimum_value,
                        &maximum_value,
                        &opening_position,
                        &Cell::new(false),
                        &Cell::new(0.0),
                        &model,
                        &sync_guard,
                        f64::from(delta),
                    );
                }
            } else {
                source_add_step_with_force(
                    &scale,
                    &model,
                    &sync_guard,
                    -f64::from(delta),
                    modifiers,
                    force_element.get(),
                );
            }
            scale.queue_draw();
            glib::Propagation::Stop
        });
    }
    scroll.connect_scroll_end(|_| {
        reset_source_scroll_units();
    });
    scale.add_controller(scroll);
}

#[allow(clippy::too_many_arguments)]
fn install_scale_keyboard(
    scale: &gtk4::Scale,
    anchor: &gtk4::MenuButton,
    popup: &gtk4::Popover,
    expression: &gtk4::Label,
    current_value: &gtk4::Label,
    minimum_value: &gtk4::Label,
    maximum_value: &gtk4::Label,
    fine_tune_surface: &gtk4::DrawingArea,
    input: Rc<RefCell<NumericInputBuffer>>,
    opening_position: Rc<Cell<f64>>,
    accepted: Rc<Cell<bool>>,
    model: Rc<RefCell<BauhausSliderModel>>,
    sync_guard: Rc<Cell<bool>>,
) {
    let scale_widget = scale.clone();
    let anchor = anchor.downgrade();
    let popup = popup.downgrade();
    let expression = expression.downgrade();
    let current_value = current_value.downgrade();
    let minimum_value = minimum_value.downgrade();
    let maximum_value = maximum_value.downgrade();
    let fine_tune_surface = fine_tune_surface.downgrade();
    let scale = scale.downgrade();
    let key = gtk4::EventControllerKey::new();
    key.set_name(Some(OPEN_CONTROLLER_NAME));
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key.connect_key_pressed(move |_, key, _, modifiers| {
        let (
            Some(scale),
            Some(anchor),
            Some(popup),
            Some(expression),
            Some(current_value),
            Some(minimum_value),
            Some(maximum_value),
            Some(fine_tune_surface),
        ) = (
            scale.upgrade(),
            anchor.upgrade(),
            popup.upgrade(),
            expression.upgrade(),
            current_value.upgrade(),
            minimum_value.upgrade(),
            maximum_value.upgrade(),
            fine_tune_surface.upgrade(),
        )
        else {
            return glib::Propagation::Proceed;
        };
        match key {
            gdk::Key::Return | gdk::Key::KP_Enter => {
                request_source_focus(&scale);
                show_popup(
                    &scale,
                    &anchor,
                    &popup,
                    &expression,
                    &current_value,
                    &minimum_value,
                    &maximum_value,
                    &fine_tune_surface,
                    &input,
                    &opening_position,
                    &accepted,
                    &model,
                );
            }
            gdk::Key::Right | gdk::Key::KP_Right | gdk::Key::Up | gdk::Key::KP_Up => {
                request_source_focus(&scale);
                source_add_step(&scale, &model, &sync_guard, 1.0, modifiers);
            }
            gdk::Key::Left | gdk::Key::KP_Left | gdk::Key::Down | gdk::Key::KP_Down => {
                request_source_focus(&scale);
                source_add_step(&scale, &model, &sync_guard, -1.0, modifiers);
            }
            // The retained widget is a DrawingArea and therefore has no native
            // Scale bindings for these range/arithmetic keys. Stop only those
            // GTK4 substitute mutations; unrelated and traversal keys must
            // continue to focus or parent handlers.
            gdk::Key::Home
            | gdk::Key::KP_Home
            | gdk::Key::End
            | gdk::Key::KP_End
            | gdk::Key::Page_Up
            | gdk::Key::KP_Page_Up
            | gdk::Key::Page_Down
            | gdk::Key::KP_Page_Down
            | gdk::Key::plus
            | gdk::Key::KP_Add
            | gdk::Key::minus
            | gdk::Key::KP_Subtract => return glib::Propagation::Stop,
            _ => return glib::Propagation::Proceed,
        }
        scale.queue_draw();
        glib::Propagation::Stop
    });
    scale_widget.add_controller(key);
}

#[allow(clippy::too_many_arguments)]
fn install_secondary_click(
    scale: &gtk4::Scale,
    anchor: &gtk4::MenuButton,
    popup: &gtk4::Popover,
    expression: &gtk4::Label,
    current_value: &gtk4::Label,
    minimum_value: &gtk4::Label,
    maximum_value: &gtk4::Label,
    fine_tune_surface: &gtk4::DrawingArea,
    input: Rc<RefCell<NumericInputBuffer>>,
    opening_position: Rc<Cell<f64>>,
    accepted: Rc<Cell<bool>>,
    model: Rc<RefCell<BauhausSliderModel>>,
) {
    let scale_widget = scale.clone();
    let anchor = anchor.downgrade();
    let popup = popup.downgrade();
    let expression = expression.downgrade();
    let current_value = current_value.downgrade();
    let minimum_value = minimum_value.downgrade();
    let maximum_value = maximum_value.downgrade();
    let fine_tune_surface = fine_tune_surface.downgrade();
    let scale = scale.downgrade();
    let click = gtk4::GestureClick::new();
    click.set_name(Some(SECONDARY_CONTROLLER_NAME));
    click.set_button(SECONDARY_BUTTON);
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    click.connect_pressed(move |gesture, _, _, _| {
        let (
            Some(scale),
            Some(anchor),
            Some(popup),
            Some(expression),
            Some(current_value),
            Some(minimum_value),
            Some(maximum_value),
            Some(fine_tune_surface),
        ) = (
            scale.upgrade(),
            anchor.upgrade(),
            popup.upgrade(),
            expression.upgrade(),
            current_value.upgrade(),
            minimum_value.upgrade(),
            maximum_value.upgrade(),
            fine_tune_surface.upgrade(),
        )
        else {
            return;
        };
        request_source_focus(&scale);
        show_popup(
            &scale,
            &anchor,
            &popup,
            &expression,
            &current_value,
            &minimum_value,
            &maximum_value,
            &fine_tune_surface,
            &input,
            &opening_position,
            &accepted,
            &model,
        );
        let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
    });
    scale_widget.add_controller(click);
}

#[allow(clippy::too_many_arguments)]
fn install_popup_keyboard(
    scale: &gtk4::Scale,
    popup: &gtk4::Popover,
    expression: &gtk4::Label,
    current_value: &gtk4::Label,
    input: Rc<RefCell<NumericInputBuffer>>,
    accepted: Rc<Cell<bool>>,
    model: Rc<RefCell<BauhausSliderModel>>,
    sync_guard: Rc<Cell<bool>>,
) {
    let popup_widget = popup.clone();
    let popup = popup.downgrade();
    let expression = expression.downgrade();
    let current_value = current_value.downgrade();
    let scale = scale.downgrade();
    let key = gtk4::EventControllerKey::new();
    key.set_name(Some(INPUT_CONTROLLER_NAME));
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key.connect_key_pressed(move |_, key, _, modifiers| {
        let (Some(scale), Some(popup), Some(expression), Some(current_value)) = (
            scale.upgrade(),
            popup.upgrade(),
            expression.upgrade(),
            current_value.upgrade(),
        ) else {
            return glib::Propagation::Proceed;
        };

        match key {
            gdk::Key::BackSpace | gdk::Key::Delete | gdk::Key::KP_Delete => {
                input.borrow_mut().erase_last();
                expression.set_text(input.borrow().as_str());
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                let replacement = {
                    let model = model.borrow();
                    resolve_raw_value(
                        model.value(),
                        model.factor(),
                        model.offset(),
                        input.borrow().as_str(),
                    )
                };
                accepted.set(true);
                if let Some(replacement) = replacement {
                    source_set_value(&scale, &model, &sync_guard, replacement);
                }
                popup.popdown();
            }
            gdk::Key::Escape => popup.popdown(),
            gdk::Key::Home | gdk::Key::KP_Home => {
                source_add_step(&scale, &model, &sync_guard, -ENDPOINT_DELTA, modifiers);
                update_current_value(&model, &current_value);
            }
            gdk::Key::End | gdk::Key::KP_End => {
                source_add_step(&scale, &model, &sync_guard, ENDPOINT_DELTA, modifiers);
                update_current_value(&model, &current_value);
            }
            gdk::Key::Right
            | gdk::Key::KP_Right
            | gdk::Key::Up
            | gdk::Key::KP_Up
            | gdk::Key::Page_Up
            | gdk::Key::KP_Page_Up => {
                source_add_step(&scale, &model, &sync_guard, 1.0, modifiers);
                update_current_value(&model, &current_value);
            }
            gdk::Key::Left
            | gdk::Key::KP_Left
            | gdk::Key::Down
            | gdk::Key::KP_Down
            | gdk::Key::Page_Down
            | gdk::Key::KP_Page_Down => {
                source_add_step(&scale, &model, &sync_guard, -1.0, modifiers);
                update_current_value(&model, &current_value);
            }
            _ => {
                if has_shortcut_modifier(modifiers) {
                    return glib::Propagation::Proceed;
                }
                let Some(character) = key.to_unicode() else {
                    return glib::Propagation::Proceed;
                };
                if character.is_control() {
                    return glib::Propagation::Proceed;
                }
                input.borrow_mut().push(character);
                expression.set_text(input.borrow().as_str());
            }
        }

        glib::Propagation::Stop
    });
    popup_widget.add_controller(key);
}

#[allow(clippy::too_many_arguments)]
fn install_rejection(
    scale: &gtk4::Scale,
    expression: &gtk4::Label,
    current_value: &gtk4::Label,
    input: Rc<RefCell<NumericInputBuffer>>,
    opening_position: Rc<Cell<f64>>,
    accepted: Rc<Cell<bool>>,
    popup: &gtk4::Popover,
    model: Rc<RefCell<BauhausSliderModel>>,
    sync_guard: Rc<Cell<bool>>,
) {
    let scale = scale.downgrade();
    let expression = expression.downgrade();
    let current_value = current_value.downgrade();
    popup.connect_closed(move |_| {
        if !accepted.replace(false)
            && let Some(scale) = scale.upgrade()
        {
            let previous = scale.value();
            model
                .borrow_mut()
                .set_normalized_position(opening_position.get());
            sync_source_change(&scale, &model, &sync_guard, previous);
        }
        input.borrow_mut().clear();
        if let Some(expression) = expression.upgrade() {
            expression.set_text("");
        }
        if let Some(current_value) = current_value.upgrade() {
            current_value.set_text("");
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn show_popup(
    scale: &gtk4::Scale,
    anchor: &gtk4::MenuButton,
    popup: &gtk4::Popover,
    expression: &gtk4::Label,
    current_value: &gtk4::Label,
    minimum_value: &gtk4::Label,
    maximum_value: &gtk4::Label,
    fine_tune_surface: &gtk4::DrawingArea,
    input: &RefCell<NumericInputBuffer>,
    opening_position: &Cell<f64>,
    accepted: &Cell<bool>,
    model: &RefCell<BauhausSliderModel>,
) {
    input.borrow_mut().clear();
    expression.set_text("");
    update_popup_values(model, current_value, minimum_value, maximum_value);
    opening_position.set(model.borrow().normalized_position());
    accepted.set(false);
    let width = scale.allocated_width().max(1);
    popup.set_size_request(width, width);
    popup.set_offset(0, -1);
    position_range_labels(fine_tune_surface, minimum_value, maximum_value);
    fine_tune_surface.queue_draw();
    anchor.popup();
    popup.grab_focus();
}

fn update_current_value(model: &RefCell<BauhausSliderModel>, current_value: &gtk4::Label) {
    let model = model.borrow();
    current_value.set_text(&model.value_text(model.value()));
}

#[allow(clippy::cast_possible_truncation)]
fn source_relative_drag_transition(
    width: f64,
    visible_width: f64,
    signed_step: f64,
    factor: f64,
    x: f64,
    reference: f64,
) -> Option<(f64, f64)> {
    let scaled_step = width as f32 * signed_step as f32 / visible_width as f32;
    if !scaled_step.is_finite() || scaled_step == 0.0 {
        return None;
    }
    let steps = ((x - reference) as f32 / scaled_step).floor();
    if steps == 0.0 {
        return None;
    }
    let direction = 1.0_f32.copysign(factor as f32);
    Some((
        f64::from(direction * steps),
        reference + f64::from(steps * scaled_step),
    ))
}

fn source_add_step(
    scale: &gtk4::Scale,
    model: &RefCell<BauhausSliderModel>,
    sync_guard: &Cell<bool>,
    delta: f64,
    modifiers: gdk::ModifierType,
) {
    source_add_step_with_force(scale, model, sync_guard, delta, modifiers, false);
}

fn source_add_step_with_force(
    scale: &gtk4::Scale,
    model: &RefCell<BauhausSliderModel>,
    sync_guard: &Cell<bool>,
    delta: f64,
    modifiers: gdk::ModifierType,
    force: bool,
) {
    let previous = scale.value();
    model.borrow_mut().add_step(
        delta,
        modifier_speed_multiplier(modifiers),
        force || source_force_range_step(modifiers),
        source_automatic_step_policy(),
    );
    sync_source_change(scale, model, sync_guard, previous);
}

fn source_set_normalized(
    scale: &gtk4::Scale,
    model: &RefCell<BauhausSliderModel>,
    sync_guard: &Cell<bool>,
    position: f64,
) {
    let previous = scale.value();
    model.borrow_mut().set_normalized_position(position);
    sync_source_change(scale, model, sync_guard, previous);
}

fn modifier_speed_multiplier(modifiers: gdk::ModifierType) -> f64 {
    let primary = primary_accelerator_mask();
    let relevant = primary
        | gdk::ModifierType::SHIFT_MASK
        | gdk::ModifierType::CONTROL_MASK
        | gdk::ModifierType::ALT_MASK;
    let cleaned = modifiers & relevant;
    if cleaned == primary {
        0.1
    } else if cleaned == gdk::ModifierType::SHIFT_MASK
        || cleaned == primary | gdk::ModifierType::SHIFT_MASK
    {
        10.0
    } else {
        1.0
    }
}

fn primary_accelerator_mask() -> gdk::ModifierType {
    #[cfg(target_os = "macos")]
    {
        gdk::ModifierType::META_MASK
    }
    #[cfg(not(target_os = "macos"))]
    {
        gdk::ModifierType::CONTROL_MASK
    }
}

fn has_shortcut_modifier(modifiers: gdk::ModifierType) -> bool {
    modifiers.intersects(
        gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK
            | gdk::ModifierType::HYPER_MASK
            | gdk::ModifierType::META_MASK,
    )
}

fn source_has_drag_modifier(modifiers: gdk::ModifierType) -> bool {
    modifiers.intersects(
        gdk::ModifierType::SHIFT_MASK
            | gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::SUPER_MASK
            | gdk::ModifierType::HYPER_MASK
            | gdk::ModifierType::META_MASK,
    )
}

fn source_force_range_step(modifiers: gdk::ModifierType) -> bool {
    let relevant = gdk::ModifierType::SHIFT_MASK
        | gdk::ModifierType::CONTROL_MASK
        | gdk::ModifierType::ALT_MASK
        | gdk::ModifierType::SUPER_MASK
        | gdk::ModifierType::HYPER_MASK
        | gdk::ModifierType::META_MASK;
    (modifiers & relevant) == (gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK)
}

fn source_set_value(
    scale: &gtk4::Scale,
    model: &RefCell<BauhausSliderModel>,
    sync_guard: &Cell<bool>,
    raw_value: f64,
) {
    let previous = scale.value();
    model.borrow_mut().set_value(raw_value);
    sync_source_change(scale, model, sync_guard, previous);
}

fn sync_source_change(
    scale: &gtk4::Scale,
    model: &RefCell<BauhausSliderModel>,
    sync_guard: &Cell<bool>,
    previous: f64,
) {
    sync_scale_from_model(scale, model, sync_guard);
    if scale.value().to_bits() == previous.to_bits() {
        scale.emit_by_name::<()>("value-changed", &[]);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use gtk4::gdk;

    use super::{
        AutomaticStepPolicy, BauhausSliderModel, SliderInputSpec, modifier_speed_multiplier,
        primary_accelerator_mask, reset_source_scroll_units, set_zoom_step,
        source_automatic_step_policy, source_closed_value_text, source_force_range_step,
        source_relative_drag_transition, source_scale_inverted, source_smooth_scroll_unit_delta,
    };

    static ZOOM_STEP_TEST_LOCK: Mutex<()> = Mutex::new(());

    struct RestoreZoomStep;

    impl Drop for RestoreZoomStep {
        fn drop(&mut self) {
            set_zoom_step(true);
        }
    }

    #[test]
    fn source_speed_modifiers_match_darktable_defaults() {
        assert_close(modifier_speed_multiplier(gdk::ModifierType::empty()), 1.0);
        assert_close(modifier_speed_multiplier(primary_accelerator_mask()), 0.1);
        assert_close(
            modifier_speed_multiplier(gdk::ModifierType::SHIFT_MASK),
            10.0,
        );
        assert_close(
            modifier_speed_multiplier(primary_accelerator_mask() | gdk::ModifierType::SHIFT_MASK),
            10.0,
        );
        assert_close(
            modifier_speed_multiplier(primary_accelerator_mask() | gdk::ModifierType::ALT_MASK),
            1.0,
        );
        assert_close(
            modifier_speed_multiplier(gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::ALT_MASK),
            1.0,
        );
        #[cfg(target_os = "macos")]
        {
            assert_close(
                modifier_speed_multiplier(gdk::ModifierType::CONTROL_MASK),
                1.0,
            );
            assert_close(
                modifier_speed_multiplier(
                    gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK,
                ),
                1.0,
            );
        }
    }

    #[test]
    fn source_force_requires_exact_shift_control_shortcut_modifiers() {
        let force = gdk::ModifierType::SHIFT_MASK | gdk::ModifierType::CONTROL_MASK;
        assert!(source_force_range_step(force));
        assert!(source_force_range_step(
            force | gdk::ModifierType::LOCK_MASK
        ));
        assert!(!source_force_range_step(gdk::ModifierType::SHIFT_MASK));
        assert!(!source_force_range_step(
            force | gdk::ModifierType::ALT_MASK
        ));
    }

    #[test]
    fn input_spec_const_builders_preserve_source_metadata() {
        const SPEC: SliderInputSpec = SliderInputSpec::IDENTITY
            .with_suffix(" EV")
            .with_soft_range(-3.0, 4.0)
            .with_default_value(0.0)
            .with_digits(3)
            .with_automatic_step();

        assert_eq!(SPEC.suffix, " EV");
        assert_eq!(SPEC.soft_range, Some((-3.0, 4.0)));
        assert_eq!(SPEC.default_value, Some(0.0));
        assert_eq!(SPEC.digits, Some(3));
        const {
            assert!(SPEC.automatic_step);
        }
    }

    #[test]
    fn disabled_live_zoom_step_configuration_selects_soft_range_policy() {
        let _lock = ZOOM_STEP_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        set_zoom_step(true);
        let _restore = RestoreZoomStep;

        set_zoom_step(false);

        assert_eq!(
            source_automatic_step_policy(),
            AutomaticStepPolicy::SoftRange
        );
    }

    #[test]
    fn smooth_scroll_accumulates_shared_fractions_and_stop_resets_remainder() {
        reset_source_scroll_units();
        let fraction = source_scroll_fraction(0.4);
        assert_eq!(source_smooth_scroll_unit_delta(0.0, fraction), None);
        assert_eq!(source_smooth_scroll_unit_delta(0.0, fraction), None);
        assert_eq!(source_smooth_scroll_unit_delta(0.0, fraction), Some(1));

        reset_source_scroll_units();
        assert_eq!(
            source_smooth_scroll_unit_delta(0.0, source_scroll_fraction(0.75)),
            None
        );
        reset_source_scroll_units();
        assert_eq!(
            source_smooth_scroll_unit_delta(0.0, source_scroll_fraction(0.5)),
            None,
            "scroll-end reset must discard the preceding 0.75 remainder"
        );
    }

    #[test]
    fn reverse_factor_uses_reverse_scale_position_and_source_drag_direction() {
        let mut model = BauhausSliderModel::new(0.0, 10.0, 1.0, 5.0, 1, true)
            .expect("valid reverse-factor slider");
        model.set_factor(-1.0);
        model.set_value(5.0);
        assert!(source_scale_inverted(&model));

        let (delta, reference) =
            source_relative_drag_transition(100.0, 10.0, -1.0, -1.0, 60.0, 50.0)
                .expect("one negative-factor source step");
        assert_close(delta, 1.0);
        assert_close(reference, 60.0);
        let before = model.value();
        model.add_step(delta, 1.0, false, AutomaticStepPolicy::VisibleRange);
        assert!(
            model.value() < before,
            "dragging a reverse-factor source slider right decreases its raw value"
        );
    }

    #[test]
    fn closed_value_text_uses_bauhaus_factor_offset_suffix_and_percent_rules() {
        let mut display = BauhausSliderModel::new(-1.0, 1.0, 0.1, 0.25, 2, true)
            .expect("valid display-format slider");
        display.set_factor(2.0);
        display.set_offset(1.0);
        display.set_format(" EV");
        assert_eq!(source_closed_value_text(&display, 0.25), "+1.50 EV");

        let mut percent =
            BauhausSliderModel::new(0.0, 1.0, 0.1, 1.0, 3, true).expect("valid percent slider");
        percent.set_format("%");
        assert_eq!(source_closed_value_text(&percent, 1.0), "100.0%");
    }

    fn source_scroll_fraction(value: f64) -> f64 {
        #[cfg(target_os = "macos")]
        {
            value * 50.0
        }
        #[cfg(not(target_os = "macos"))]
        {
            value
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() <= f64::EPSILON);
    }
}
