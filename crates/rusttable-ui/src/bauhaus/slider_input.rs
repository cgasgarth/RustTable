//! GTK4 adapter for the typed-slider portion of Darktable's Bauhaus popup.
//!
//! This maps `_popup_show`, `_popup_reject`, `_slider_add_step`, and the
//! slider branch of `_popup_key_press` from `src/bauhaus/bauhaus.c`. The
//! retained file remains the oracle for the fine-tune drawing and range
//! behavior that is not yet ported.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::{gdk, glib, prelude::*};

use super::numeric_input::{NumericInputBuffer, resolve_raw_value};

const SECONDARY_BUTTON: u32 = 3;
const ENDPOINT_DELTA: f64 = 1_000_000.0;
const OPEN_CONTROLLER_NAME: &str = "dt-bauhaus-open";
const INPUT_CONTROLLER_NAME: &str = "dt-bauhaus-input";
const SECONDARY_CONTROLLER_NAME: &str = "dt-bauhaus-secondary";

/// The GTK4 composition that gives a scale a supported popover owner.
///
/// GTK4 scales do not own or allocate arbitrary popover children. A one-pixel
/// frameless `MenuButton` is therefore overlaid on the scale and owns the
/// popover through GTK's supported `MenuButton::set_popover` API.
#[derive(Clone, Debug)]
pub(crate) struct BauhausSlider {
    root: gtk4::Overlay,
    scale: gtk4::Scale,
}

impl BauhausSlider {
    pub(crate) fn widget(&self) -> &gtk4::Overlay {
        &self.root
    }

    pub(crate) fn scale(&self) -> &gtk4::Scale {
        &self.scale
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SliderInputSpec {
    pub(crate) factor: f64,
    pub(crate) offset: f64,
    pub(crate) step_multiplier: f64,
    pub(crate) suffix: &'static str,
}

impl SliderInputSpec {
    pub(crate) const IDENTITY: Self = Self {
        factor: 1.0,
        offset: 0.0,
        step_multiplier: 1.0,
        suffix: "",
    };

    pub(crate) const fn with_suffix(self, suffix: &'static str) -> Self {
        Self { suffix, ..self }
    }
}

impl Default for SliderInputSpec {
    fn default() -> Self {
        Self::IDENTITY
    }
}

pub(crate) fn attach(scale: gtk4::Scale, spec: SliderInputSpec) -> BauhausSlider {
    let input = Rc::new(RefCell::new(NumericInputBuffer::new()));
    let opening_value = Rc::new(Cell::new(scale.value()));
    let accepted = Rc::new(Cell::new(false));

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
    current_value.add_css_class("dt_bauhaus_current_value");

    let surface = gtk4::Overlay::new();
    surface.set_child(Some(&expression));
    surface.add_overlay(&current_value);

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
        Rc::clone(&input),
        Rc::clone(&opening_value),
        Rc::clone(&accepted),
        spec,
    );
    install_secondary_click(
        &scale,
        &anchor,
        &popup,
        &expression,
        &current_value,
        Rc::clone(&input),
        Rc::clone(&opening_value),
        Rc::clone(&accepted),
        spec,
    );
    install_popup_keyboard(
        &scale,
        &popup,
        &expression,
        &current_value,
        Rc::clone(&input),
        Rc::clone(&accepted),
        spec,
    );
    install_rejection(
        &scale,
        &expression,
        &current_value,
        input,
        opening_value,
        accepted,
        &popup,
        spec,
    );

    BauhausSlider { root, scale }
}

#[allow(clippy::too_many_arguments)]
fn install_scale_keyboard(
    scale: &gtk4::Scale,
    anchor: &gtk4::MenuButton,
    popup: &gtk4::Popover,
    expression: &gtk4::Label,
    current_value: &gtk4::Label,
    input: Rc<RefCell<NumericInputBuffer>>,
    opening_value: Rc<Cell<f64>>,
    accepted: Rc<Cell<bool>>,
    spec: SliderInputSpec,
) {
    let scale_widget = scale.clone();
    let anchor = anchor.downgrade();
    let popup = popup.downgrade();
    let expression = expression.downgrade();
    let current_value = current_value.downgrade();
    let scale = scale.downgrade();
    let key = gtk4::EventControllerKey::new();
    key.set_name(Some(OPEN_CONTROLLER_NAME));
    key.set_propagation_phase(gtk4::PropagationPhase::Capture);
    key.connect_key_pressed(move |_, key, _, _| {
        if !matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
            return glib::Propagation::Proceed;
        }
        let (Some(scale), Some(anchor), Some(popup), Some(expression), Some(current_value)) = (
            scale.upgrade(),
            anchor.upgrade(),
            popup.upgrade(),
            expression.upgrade(),
            current_value.upgrade(),
        ) else {
            return glib::Propagation::Proceed;
        };
        show_popup(
            &scale,
            &anchor,
            &popup,
            &expression,
            &current_value,
            &input,
            &opening_value,
            &accepted,
            spec,
        );
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
    input: Rc<RefCell<NumericInputBuffer>>,
    opening_value: Rc<Cell<f64>>,
    accepted: Rc<Cell<bool>>,
    spec: SliderInputSpec,
) {
    let scale_widget = scale.clone();
    let anchor = anchor.downgrade();
    let popup = popup.downgrade();
    let expression = expression.downgrade();
    let current_value = current_value.downgrade();
    let scale = scale.downgrade();
    let click = gtk4::GestureClick::new();
    click.set_name(Some(SECONDARY_CONTROLLER_NAME));
    click.set_button(SECONDARY_BUTTON);
    click.set_propagation_phase(gtk4::PropagationPhase::Capture);
    click.connect_pressed(move |gesture, _, _, _| {
        let (Some(scale), Some(anchor), Some(popup), Some(expression), Some(current_value)) = (
            scale.upgrade(),
            anchor.upgrade(),
            popup.upgrade(),
            expression.upgrade(),
            current_value.upgrade(),
        ) else {
            return;
        };
        show_popup(
            &scale,
            &anchor,
            &popup,
            &expression,
            &current_value,
            &input,
            &opening_value,
            &accepted,
            spec,
        );
        let _ = gesture.set_state(gtk4::EventSequenceState::Claimed);
    });
    scale_widget.add_controller(click);
}

fn install_popup_keyboard(
    scale: &gtk4::Scale,
    popup: &gtk4::Popover,
    expression: &gtk4::Label,
    current_value: &gtk4::Label,
    input: Rc<RefCell<NumericInputBuffer>>,
    accepted: Rc<Cell<bool>>,
    spec: SliderInputSpec,
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
                let replacement = resolve_raw_value(
                    scale.value(),
                    spec.factor,
                    spec.offset,
                    input.borrow().as_str(),
                );
                accepted.set(true);
                if let Some(replacement) = replacement {
                    source_set_value(&scale, replacement, spec.factor);
                }
                popup.popdown();
            }
            gdk::Key::Escape => popup.popdown(),
            gdk::Key::Home | gdk::Key::KP_Home => {
                source_add_step(&scale, -ENDPOINT_DELTA, modifiers, spec);
                update_current_value(&scale, &current_value, spec);
            }
            gdk::Key::End | gdk::Key::KP_End => {
                source_add_step(&scale, ENDPOINT_DELTA, modifiers, spec);
                update_current_value(&scale, &current_value, spec);
            }
            gdk::Key::Right
            | gdk::Key::KP_Right
            | gdk::Key::Up
            | gdk::Key::KP_Up
            | gdk::Key::Page_Up
            | gdk::Key::KP_Page_Up => {
                source_add_step(&scale, 1.0, modifiers, spec);
                update_current_value(&scale, &current_value, spec);
            }
            gdk::Key::Left
            | gdk::Key::KP_Left
            | gdk::Key::Down
            | gdk::Key::KP_Down
            | gdk::Key::Page_Down
            | gdk::Key::KP_Page_Down => {
                source_add_step(&scale, -1.0, modifiers, spec);
                update_current_value(&scale, &current_value, spec);
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
    opening_value: Rc<Cell<f64>>,
    accepted: Rc<Cell<bool>>,
    popup: &gtk4::Popover,
    spec: SliderInputSpec,
) {
    let scale = scale.downgrade();
    let expression = expression.downgrade();
    let current_value = current_value.downgrade();
    popup.connect_closed(move |_| {
        if !accepted.replace(false)
            && let Some(scale) = scale.upgrade()
        {
            source_set_value(&scale, opening_value.get(), spec.factor);
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
    input: &RefCell<NumericInputBuffer>,
    opening_value: &Cell<f64>,
    accepted: &Cell<bool>,
    spec: SliderInputSpec,
) {
    input.borrow_mut().clear();
    expression.set_text("");
    update_current_value(scale, current_value, spec);
    opening_value.set(scale.value());
    accepted.set(false);
    let width = scale.allocated_width().max(1);
    popup.set_size_request(width, width);
    popup.set_offset(0, -1);
    anchor.popup();
    popup.grab_focus();
}

fn update_current_value(scale: &gtk4::Scale, current_value: &gtk4::Label, spec: SliderInputSpec) {
    let adjustment = scale.adjustment();
    current_value.set_text(&displayed_value_text(
        scale.value(),
        adjustment.lower(),
        adjustment.upper(),
        spec.factor,
        spec.offset,
        scale.digits(),
        spec.suffix,
    ));
}

fn source_add_step(
    scale: &gtk4::Scale,
    delta: f64,
    modifiers: gdk::ModifierType,
    spec: SliderInputSpec,
) {
    let adjustment = scale.adjustment();
    let change = source_step_delta(
        delta,
        adjustment.step_increment(),
        spec.factor,
        scale.digits(),
        spec.step_multiplier,
        modifiers,
    );
    source_set_value(scale, scale.value() + change, spec.factor);
}

fn source_step_delta(
    delta: f64,
    step_increment: f64,
    factor: f64,
    digits: i32,
    step_multiplier: f64,
    modifiers: gdk::ModifierType,
) -> f64 {
    let signed_step = step_increment.abs().copysign(factor);
    let mut change = delta * signed_step * step_multiplier * modifier_speed_multiplier(modifiers);
    let min_visible = minimum_visible_increment(digits, factor);
    if change != 0.0 && change.abs() < min_visible {
        change = min_visible.copysign(change);
    }
    change
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

fn minimum_visible_increment(digits: i32, factor: f64) -> f64 {
    10.0_f64.powi(-digits) / factor.abs()
}

fn source_set_value(scale: &gtk4::Scale, raw_value: f64, factor: f64) {
    let adjustment = scale.adjustment();
    let replacement = rounded_and_clamped(
        raw_value,
        adjustment.lower(),
        adjustment.upper(),
        factor,
        scale.digits(),
    );
    let previous = scale.value();
    scale.set_value(replacement);
    if scale.value().to_bits() == previous.to_bits() {
        scale.emit_by_name::<()>("value-changed", &[]);
    }
}

fn rounded_and_clamped(raw_value: f64, lower: f64, upper: f64, factor: f64, digits: i32) -> f64 {
    let clamped = raw_value.clamp(lower, upper);
    let base = 10.0_f64.powi(digits) * factor;
    let rounded = if base == 0.0 || !base.is_finite() {
        clamped
    } else {
        (base * clamped).round() / base
    };
    rounded.clamp(lower, upper)
}

fn displayed_value_text(
    raw_value: f64,
    lower: f64,
    upper: f64,
    factor: f64,
    offset: f64,
    digits: i32,
    suffix: &str,
) -> String {
    let displayed_value = raw_value * factor + offset;
    let displayed_lower = lower * factor + offset;
    let displayed_upper = upper * factor + offset;
    let precision = usize::try_from(digits.max(0)).unwrap_or_default();
    let value = if displayed_lower * displayed_upper < 0.0 {
        format!("{displayed_value:+.precision$}")
    } else {
        format!("{displayed_value:.precision$}")
    };
    format!("{value}{suffix}")
}

#[cfg(test)]
mod tests {
    use gtk4::gdk;

    use super::{
        displayed_value_text, minimum_visible_increment, modifier_speed_multiplier,
        primary_accelerator_mask, rounded_and_clamped, source_step_delta,
    };

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
    fn source_step_uses_factor_sign_base_multiplier_and_minimum_visible_increment() {
        assert_close(
            source_step_delta(1.0, 0.5, -2.0, 2, 3.0, gdk::ModifierType::empty()),
            -1.5,
        );
        assert_close(minimum_visible_increment(2, 100.0), 0.0001);
        assert_close(
            source_step_delta(
                1.0,
                0.000_001,
                100.0,
                2,
                1.0,
                gdk::ModifierType::CONTROL_MASK,
            ),
            0.0001,
        );
        assert_close(
            source_step_delta(
                -1.0,
                0.000_001,
                100.0,
                2,
                1.0,
                gdk::ModifierType::CONTROL_MASK,
            ),
            -0.0001,
        );
    }

    #[test]
    fn source_setter_clamps_to_hard_bounds_and_rounds_in_display_units() {
        assert_close(rounded_and_clamped(1.236, -2.0, 2.0, 1.0, 2), 1.24);
        assert_close(rounded_and_clamped(0.012_36, -2.0, 2.0, 100.0, 2), 0.0124);
        assert_close(rounded_and_clamped(-3.0, -2.0, 2.0, 1.0, 2), -2.0);
        assert_close(rounded_and_clamped(3.0, -2.0, 2.0, 1.0, 2), 2.0);
    }

    #[test]
    fn displayed_value_uses_source_sign_rule_and_digits() {
        assert_eq!(
            displayed_value_text(0.5, -2.0, 2.0, 1.0, 0.0, 2, " EV"),
            "+0.50 EV"
        );
        assert_eq!(
            displayed_value_text(0.5, -2.0, 2.0, 1.0, 0.0, 2, ""),
            "+0.50"
        );
        assert_eq!(
            displayed_value_text(0.5, 0.0, 2.0, 100.0, 0.0, 1, ""),
            "50.0"
        );
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() <= f64::EPSILON);
    }
}
