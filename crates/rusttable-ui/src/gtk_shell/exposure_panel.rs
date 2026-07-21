//! GTK4 Exposure IOP panel matching Darktable's manual controls.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_processing::{
    BLACK_LEVEL_MAXIMUM, BLACK_LEVEL_MINIMUM, BLACK_LEVEL_SOFT_MAXIMUM, BLACK_LEVEL_SOFT_MINIMUM,
    EXPOSURE_EV_MAXIMUM, EXPOSURE_EV_MINIMUM, EXPOSURE_EV_SOFT_MAXIMUM, EXPOSURE_EV_SOFT_MINIMUM,
    ExposureAction, ExposureActionError, ExposureMode, ExposureModuleState,
};

use super::{ThemeRole, apply_theme_role};

type ExposureActionHandler = Box<dyn Fn(ExposureAction)>;

/// Native GTK4 realization of one Darktable Exposure module panel.
#[derive(Clone)]
pub struct ExposurePanel {
    expander: gtk4::Expander,
    state: Rc<RefCell<ExposureModuleState>>,
    enabled: gtk4::Switch,
    mode: gtk4::DropDown,
    exposure: gtk4::Scale,
    exposure_value: gtk4::Label,
    black: gtk4::Scale,
    black_value: gtk4::Label,
    compensate_exposure_bias: gtk4::Switch,
    compensate_highlight_preservation: gtk4::Switch,
    actions: Rc<RefCell<Option<ExposureActionHandler>>>,
}

impl ExposurePanel {
    /// Builds an Exposure panel using Darktable's parameter defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::from_state(ExposureModuleState::default())
    }

    /// Builds an Exposure panel from an existing typed module state.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn from_state(initial_state: ExposureModuleState) -> Self {
        let state = Rc::new(RefCell::new(initial_state));
        let actions = Rc::new(RefCell::new(None));
        let enabled = gtk4::Switch::new();
        let mode = gtk4::DropDown::from_strings(&["manual", "automatic"]);
        let exposure = gtk4::Scale::with_range(
            gtk4::Orientation::Horizontal,
            EXPOSURE_EV_MINIMUM,
            EXPOSURE_EV_MAXIMUM,
            0.001,
        );
        let black = gtk4::Scale::with_range(
            gtk4::Orientation::Horizontal,
            BLACK_LEVEL_MINIMUM,
            BLACK_LEVEL_MAXIMUM,
            0.0001,
        );
        exposure.set_digits(3);
        exposure.set_hexpand(true);
        exposure.set_draw_value(false);
        exposure.set_tooltip_text(Some(&format!(
            "adjust exposure correction; soft range {EXPOSURE_EV_SOFT_MINIMUM:.0} to \
             {EXPOSURE_EV_SOFT_MAXIMUM:.0} EV"
        )));
        black.set_digits(4);
        black.set_hexpand(true);
        black.set_draw_value(false);
        black.set_tooltip_text(Some(&format!(
            "adjust black level; soft range {BLACK_LEVEL_SOFT_MINIMUM:.1} to \
             {BLACK_LEVEL_SOFT_MAXIMUM:.1}"
        )));
        let compensate_exposure_bias = gtk4::Switch::new();
        let compensate_highlight_preservation = gtk4::Switch::new();
        let exposure_value = value_label("exposure-value", "Exposure value");
        let black_value = value_label("black-value", "Black-level value");
        let presets = gtk4::Button::with_label("presets");
        presets.set_widget_name("exposure-presets");
        presets.set_sensitive(false);
        presets.set_focusable(false);
        presets.set_tooltip_text(Some("Exposure presets are unavailable"));
        presets.update_property(&[Property::Label("Exposure presets unavailable")]);
        let reset = gtk4::Button::with_label("reset");
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);

        append_switch_row(&content, "enabled", &enabled);
        append_dropdown_row(&content, "mode", &mode);
        append_scale_row(&content, "exposure", &exposure, &exposure_value, "EV");
        append_scale_row(&content, "black", &black, &black_value, "");
        append_switch_row(
            &content,
            "compensate exposure bias",
            &compensate_exposure_bias,
        );
        append_switch_row(
            &content,
            "compensate highlight preservation",
            &compensate_highlight_preservation,
        );
        content.append(&presets);
        content.append(&reset);

        let expander = gtk4::Expander::builder()
            .label("exposure")
            .expanded(initial_state.expanded())
            .child(&content)
            .build();
        expander.set_widget_name("exposure");
        apply_theme_role(&expander, ThemeRole::Module);
        expander.set_accessible_role(gtk4::AccessibleRole::Group);
        expander.update_property(&[Property::Label("Exposure processing module")]);
        identify(&enabled, "exposure-enabled", "Enable exposure module");
        identify(&mode, "exposure-mode", "Exposure mode");
        identify(&exposure, "exposure-ev", "Exposure correction in EV");
        identify(
            &exposure_value,
            "exposure-value",
            "Current exposure correction in EV",
        );
        identify(&black, "exposure-black", "Exposure black level");
        identify(&black_value, "black-value", "Current exposure black level");
        identify(
            &compensate_exposure_bias,
            "exposure-bias-compensation",
            "Compensate camera exposure bias",
        );
        identify(
            &compensate_highlight_preservation,
            "exposure-highlight-compensation",
            "Compensate highlight preservation",
        );
        identify(&reset, "exposure-reset", "Reset exposure module");

        let panel = Self {
            expander,
            state,
            enabled,
            mode,
            exposure,
            exposure_value,
            black,
            black_value,
            compensate_exposure_bias,
            compensate_highlight_preservation,
            actions,
        };
        panel.sync_widgets();
        panel.connect_actions(&reset);
        panel
    }

    /// Returns the root GTK expander.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Expander {
        &self.expander
    }

    /// Returns a snapshot of the current typed state.
    #[must_use]
    pub fn state(&self) -> ExposureModuleState {
        *self.state.borrow()
    }

    /// Installs a callback receiving each accepted explicit user action.
    pub fn set_action_handler<F>(&self, handler: F)
    where
        F: Fn(ExposureAction) + 'static,
    {
        self.actions.replace(Some(Box::new(handler)));
    }

    /// Applies an explicit action and updates all native controls.
    ///
    /// # Errors
    ///
    /// Returns the domain error when a numeric value is outside Darktable's
    /// persisted parameter bounds.
    pub fn apply(&self, action: ExposureAction) -> Result<(), ExposureActionError> {
        self.state.borrow_mut().apply(action)?;
        self.sync_widgets();
        Ok(())
    }

    fn connect_actions(&self, reset: &gtk4::Button) {
        connect_switch_action(
            &self.enabled,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            ExposureAction::SetEnabled,
        );
        connect_expander_action(
            &self.expander,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
        );
        connect_mode_action(&self.mode, Rc::clone(&self.state), Rc::clone(&self.actions));
        connect_scale_action(
            &self.exposure,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            ExposureAction::SetExposureEv,
        );
        connect_scale_action(
            &self.black,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            ExposureAction::SetBlackLevel,
        );
        connect_switch_action(
            &self.compensate_exposure_bias,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            ExposureAction::SetCompensateExposureBias,
        );
        connect_switch_action(
            &self.compensate_highlight_preservation,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            ExposureAction::SetCompensateHighlightPreservation,
        );

        let state = Rc::clone(&self.state);
        let actions = Rc::clone(&self.actions);
        let controls = ControlSet {
            expander: self.expander.clone(),
            enabled: self.enabled.clone(),
            mode: self.mode.clone(),
            exposure: self.exposure.clone(),
            exposure_value: self.exposure_value.clone(),
            black: self.black.clone(),
            black_value: self.black_value.clone(),
            compensate_exposure_bias: self.compensate_exposure_bias.clone(),
            compensate_highlight_preservation: self.compensate_highlight_preservation.clone(),
        };
        reset.connect_clicked(move |_| {
            dispatch(&state, &actions, ExposureAction::Reset);
            sync_controls(&state, &controls);
        });
    }

    fn sync_widgets(&self) {
        let controls = ControlSet {
            expander: self.expander.clone(),
            enabled: self.enabled.clone(),
            mode: self.mode.clone(),
            exposure: self.exposure.clone(),
            exposure_value: self.exposure_value.clone(),
            black: self.black.clone(),
            black_value: self.black_value.clone(),
            compensate_exposure_bias: self.compensate_exposure_bias.clone(),
            compensate_highlight_preservation: self.compensate_highlight_preservation.clone(),
        };
        sync_controls(&self.state, &controls);
    }
}

impl Default for ExposurePanel {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
struct ControlSet {
    expander: gtk4::Expander,
    enabled: gtk4::Switch,
    mode: gtk4::DropDown,
    exposure: gtk4::Scale,
    exposure_value: gtk4::Label,
    black: gtk4::Scale,
    black_value: gtk4::Label,
    compensate_exposure_bias: gtk4::Switch,
    compensate_highlight_preservation: gtk4::Switch,
}

fn sync_controls(state: &Rc<RefCell<ExposureModuleState>>, controls: &ControlSet) {
    let state = *state.borrow();
    controls.enabled.set_active(state.enabled());
    controls.expander.set_expanded(state.expanded());
    controls.mode.set_selected(mode_index(state.mode()));
    controls.exposure.set_value(state.exposure_ev());
    controls
        .exposure_value
        .set_text(&format!("{:.3} EV", state.exposure_ev()));
    controls.black.set_value(state.black_level());
    controls
        .black_value
        .set_text(&format!("{:.4}", state.black_level()));
    controls
        .compensate_exposure_bias
        .set_active(state.compensate_exposure_bias());
    controls
        .compensate_highlight_preservation
        .set_active(state.compensate_highlight_preservation());
}

fn append_switch_row(container: &gtk4::Box, label: &str, control: &gtk4::Switch) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.add_css_class("dt_module_row");
    let text = gtk4::Label::new(Some(label));
    text.set_halign(gtk4::Align::Start);
    text.set_hexpand(true);
    row.append(&text);
    row.append(control);
    container.append(&row);
}

fn append_dropdown_row(container: &gtk4::Box, label: &str, control: &gtk4::DropDown) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.add_css_class("dt_module_row");
    let text = gtk4::Label::new(Some(label));
    text.set_halign(gtk4::Align::Start);
    text.set_hexpand(true);
    row.append(&text);
    row.append(control);
    container.append(&row);
}

fn append_scale_row(
    container: &gtk4::Box,
    label: &str,
    control: &gtk4::Scale,
    value: &gtk4::Label,
    unit: &str,
) {
    let row = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    row.add_css_class("dt_module_row");
    let heading_text = if unit.is_empty() {
        label.to_owned()
    } else {
        format!("{label} ({unit})")
    };
    let heading_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let heading = gtk4::Label::new(Some(&heading_text));
    heading.set_halign(gtk4::Align::Start);
    heading.set_hexpand(true);
    heading_row.append(&heading);
    heading_row.append(value);
    row.append(&heading_row);
    row.append(control);
    container.append(&row);
}

fn value_label(id: &str, accessible_name: &str) -> gtk4::Label {
    let label = gtk4::Label::new(None);
    label.set_widget_name(id);
    label.add_css_class("dt_module_value");
    label.set_halign(gtk4::Align::End);
    label.update_property(&[Property::Label(accessible_name)]);
    label
}

fn identify(
    widget: &(impl IsA<gtk4::Widget> + IsA<gtk4::Accessible>),
    id: &str,
    accessible_name: &str,
) {
    widget.set_widget_name(id);
    widget.update_property(&[Property::Label(accessible_name)]);
}

fn connect_switch_action<F>(
    control: &gtk4::Switch,
    state: Rc<RefCell<ExposureModuleState>>,
    actions: Rc<RefCell<Option<ExposureActionHandler>>>,
    action: F,
) where
    F: Fn(bool) -> ExposureAction + 'static,
{
    control.connect_active_notify(move |control| {
        dispatch(&state, &actions, action(control.is_active()));
    });
}

fn connect_expander_action(
    control: &gtk4::Expander,
    state: Rc<RefCell<ExposureModuleState>>,
    actions: Rc<RefCell<Option<ExposureActionHandler>>>,
) {
    control.connect_expanded_notify(move |control| {
        dispatch(
            &state,
            &actions,
            ExposureAction::SetExpanded(control.is_expanded()),
        );
    });
}

fn connect_mode_action(
    control: &gtk4::DropDown,
    state: Rc<RefCell<ExposureModuleState>>,
    actions: Rc<RefCell<Option<ExposureActionHandler>>>,
) {
    control.connect_selected_notify(move |control| {
        let mode = if control.selected() == mode_index(ExposureMode::Manual) {
            ExposureMode::Manual
        } else {
            ExposureMode::Automatic
        };
        dispatch(&state, &actions, ExposureAction::SetMode(mode));
    });
}

fn connect_scale_action<F>(
    control: &gtk4::Scale,
    state: Rc<RefCell<ExposureModuleState>>,
    actions: Rc<RefCell<Option<ExposureActionHandler>>>,
    action: F,
) where
    F: Fn(f64) -> ExposureAction + 'static,
{
    control.connect_value_changed(move |control| {
        dispatch(&state, &actions, action(control.value()));
    });
}

fn dispatch(
    state: &Rc<RefCell<ExposureModuleState>>,
    actions: &Rc<RefCell<Option<ExposureActionHandler>>>,
    action: ExposureAction,
) {
    let accepted = state.borrow_mut().apply(action).is_ok();
    if accepted && let Some(handler) = actions.borrow().as_ref() {
        handler(action);
    }
}

const fn mode_index(mode: ExposureMode) -> u32 {
    match mode {
        ExposureMode::Manual => 0,
        ExposureMode::Automatic => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusttable_processing::{DEFAULT_BLACK_LEVEL, DEFAULT_EXPOSURE_EV};

    #[test]
    fn constants_match_darktable_manual_control_contract() {
        assert_close(EXPOSURE_EV_MINIMUM, -18.0);
        assert_close(EXPOSURE_EV_MAXIMUM, 18.0);
        assert_close(DEFAULT_EXPOSURE_EV, 0.0);
        assert_close(BLACK_LEVEL_MINIMUM, -1.0);
        assert_close(DEFAULT_BLACK_LEVEL, 0.0);
    }

    #[test]
    fn mode_index_matches_dropdown_order() {
        assert_eq!(mode_index(ExposureMode::Manual), 0);
        assert_eq!(mode_index(ExposureMode::Automatic), 1);
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < f64::EPSILON);
    }
}
