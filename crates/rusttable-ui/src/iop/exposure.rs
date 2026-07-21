//! GTK4 Exposure IOP panel matching Darktable's manual controls.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::Revision;
use rusttable_processing::{
    BLACK_LEVEL_MAXIMUM, BLACK_LEVEL_MINIMUM, BLACK_LEVEL_SOFT_MAXIMUM, BLACK_LEVEL_SOFT_MINIMUM,
    EXPOSURE_EV_MAXIMUM, EXPOSURE_EV_MINIMUM, EXPOSURE_EV_SOFT_MAXIMUM, EXPOSURE_EV_SOFT_MINIMUM,
    ExposureAction, ExposureActionError, ExposureMode, ExposureModuleState,
};

use super::modules::{DarkroomModuleAction, DarkroomModuleActionHandler};
use super::{ThemeRole, apply_theme_role};

type ExposureActionHandler = Box<dyn Fn(ExposureAction)>;

/// Native GTK4 realization of one Darktable Exposure module panel.
#[derive(Clone)]
pub struct ExposurePanel {
    expander: gtk4::Expander,
    state: Rc<RefCell<ExposureModuleState>>,
    mode_stack: gtk4::Stack,
    enabled: gtk4::Switch,
    mode: gtk4::DropDown,
    exposure: gtk4::Scale,
    exposure_value: gtk4::Label,
    black: gtk4::Scale,
    black_value: gtk4::Label,
    compensate_exposure_bias: gtk4::Switch,
    compensate_highlight_preservation: gtk4::Switch,
    actions: Rc<RefCell<Option<ExposureActionHandler>>>,
    module_actions: Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    module_revision: Rc<RefCell<Revision>>,
    sync_guard: Rc<Cell<bool>>,
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
        let module_actions = Rc::new(RefCell::new(None));
        let module_revision = Rc::new(RefCell::new(Revision::ZERO));
        let sync_guard = Rc::new(Cell::new(false));
        let enabled = gtk4::Switch::new();
        let mode = gtk4::DropDown::from_strings(&["manual", "automatic"]);
        let mode_stack = gtk4::Stack::new();
        mode_stack.set_widget_name("exposure-mode-stack");
        mode_stack.set_hhomogeneous(false);
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
        let manual = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        append_switch_row(
            &manual,
            "compensate camera exposure bias",
            &compensate_exposure_bias,
        );
        append_switch_row(
            &manual,
            "compensate highlight preservation",
            &compensate_highlight_preservation,
        );
        append_scale_row(&manual, "exposure", &exposure, &exposure_value, "EV");
        mode_stack.add_named(&manual, Some("manual"));
        let automatic = gtk4::Label::new(Some("automatic exposure uses the source histogram"));
        automatic.set_widget_name("exposure-automatic-status");
        automatic.set_halign(gtk4::Align::Start);
        automatic.add_css_class("dim-label");
        automatic.set_accessible_role(gtk4::AccessibleRole::Status);
        mode_stack.add_named(&automatic, Some("automatic"));
        let presets = gtk4::Button::with_label("presets");
        presets.set_widget_name("exposure-presets");
        presets.set_sensitive(false);
        presets.set_focusable(false);
        presets.set_tooltip_text(Some("Exposure presets are unavailable"));
        presets.update_property(&[Property::Label("Exposure presets unavailable")]);
        let reset = gtk4::Button::with_label("reset");
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        append_dropdown_row(&content, "mode", &mode);
        content.append(&mode_stack);
        append_scale_row(&content, "black", &black, &black_value, "");

        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let title = gtk4::Label::new(Some("exposure"));
        title.set_halign(gtk4::Align::Start);
        title.set_hexpand(true);
        header.append(&title);
        header.append(&enabled);
        header.append(&presets);
        header.append(&reset);

        let expander = gtk4::Expander::builder()
            .label("exposure")
            .expanded(initial_state.expanded())
            .child(&content)
            .build();
        expander.set_label_widget(Some(&header));
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
            mode_stack,
            enabled,
            mode,
            exposure,
            exposure_value,
            black,
            black_value,
            compensate_exposure_bias,
            compensate_highlight_preservation,
            actions,
            module_actions,
            module_revision,
            sync_guard,
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

    /// Connects the legacy Exposure widget to the controller-owned generic
    /// darkroom operation path.
    pub fn set_module_action_handler(
        &self,
        handler: Option<DarkroomModuleActionHandler>,
        revision: Revision,
    ) {
        self.module_actions.replace(handler);
        self.module_revision.replace(revision);
    }

    /// Projects persisted exposure parameters into the native panel.
    ///
    /// # Errors
    ///
    /// Returns the processing-domain validation error when persisted values are
    /// outside the Exposure module's accepted bounds.
    pub fn set_module_projection(
        &self,
        revision: Revision,
        enabled: bool,
        expanded: bool,
        exposure_ev: f64,
        black_level: f64,
    ) -> Result<(), ExposureActionError> {
        let mut state = ExposureModuleState::default();
        state.apply(ExposureAction::SetEnabled(enabled))?;
        state.apply(ExposureAction::SetExpanded(expanded))?;
        state.apply(ExposureAction::SetExposureEv(exposure_ev))?;
        state.apply(ExposureAction::SetBlackLevel(black_level))?;
        self.state.replace(state);
        self.module_revision.replace(revision);
        self.sync_widgets();
        Ok(())
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
        let controls = self.control_set();
        connect_switch_action(
            &self.enabled,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            controls.clone(),
            Rc::clone(&self.module_actions),
            Rc::clone(&self.module_revision),
            ExposureAction::SetEnabled,
        );
        connect_expander_action(
            &self.expander,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            controls.clone(),
            Rc::clone(&self.module_actions),
            Rc::clone(&self.module_revision),
        );
        connect_mode_action(
            &self.mode,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            controls.clone(),
            Rc::clone(&self.module_actions),
            Rc::clone(&self.module_revision),
        );
        connect_scale_action(
            &self.exposure,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            controls.clone(),
            Rc::clone(&self.module_actions),
            Rc::clone(&self.module_revision),
            ExposureAction::SetExposureEv,
        );
        connect_scale_action(
            &self.black,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            controls.clone(),
            Rc::clone(&self.module_actions),
            Rc::clone(&self.module_revision),
            ExposureAction::SetBlackLevel,
        );
        connect_switch_action(
            &self.compensate_exposure_bias,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            controls.clone(),
            Rc::clone(&self.module_actions),
            Rc::clone(&self.module_revision),
            ExposureAction::SetCompensateExposureBias,
        );
        connect_switch_action(
            &self.compensate_highlight_preservation,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            controls.clone(),
            Rc::clone(&self.module_actions),
            Rc::clone(&self.module_revision),
            ExposureAction::SetCompensateHighlightPreservation,
        );
        let state = Rc::clone(&self.state);
        let actions = Rc::clone(&self.actions);
        let module_actions = Rc::clone(&self.module_actions);
        let module_revision = Rc::clone(&self.module_revision);
        reset.connect_clicked(move |_| {
            dispatch(
                &state,
                &actions,
                &controls,
                &module_actions,
                &module_revision,
                ExposureAction::Reset,
            );
        });
    }

    fn sync_widgets(&self) {
        sync_controls(&self.state, &self.control_set());
    }

    fn control_set(&self) -> ControlSet {
        ControlSet {
            expander: self.expander.clone(),
            mode_stack: self.mode_stack.clone(),
            enabled: self.enabled.clone(),
            mode: self.mode.clone(),
            exposure: self.exposure.clone(),
            exposure_value: self.exposure_value.clone(),
            black: self.black.clone(),
            black_value: self.black_value.clone(),
            compensate_exposure_bias: self.compensate_exposure_bias.clone(),
            compensate_highlight_preservation: self.compensate_highlight_preservation.clone(),
            sync_guard: Rc::clone(&self.sync_guard),
        }
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
    mode_stack: gtk4::Stack,
    enabled: gtk4::Switch,
    mode: gtk4::DropDown,
    exposure: gtk4::Scale,
    exposure_value: gtk4::Label,
    black: gtk4::Scale,
    black_value: gtk4::Label,
    compensate_exposure_bias: gtk4::Switch,
    compensate_highlight_preservation: gtk4::Switch,
    sync_guard: Rc<Cell<bool>>,
}

fn sync_controls(state: &Rc<RefCell<ExposureModuleState>>, controls: &ControlSet) {
    let state = *state.borrow();
    controls.sync_guard.set(true);
    controls.enabled.set_active(state.enabled());
    controls.expander.set_expanded(state.expanded());
    controls.mode.set_selected(mode_index(state.mode()));
    controls
        .mode_stack
        .set_visible_child_name(match state.mode() {
            ExposureMode::Manual => "manual",
            ExposureMode::Automatic => "automatic",
        });
    controls.mode.set_sensitive(state.enabled());
    controls.mode_stack.set_sensitive(state.enabled());
    controls.black.set_sensitive(state.enabled());
    controls
        .compensate_exposure_bias
        .set_sensitive(state.enabled());
    controls
        .compensate_highlight_preservation
        .set_sensitive(state.enabled());
    controls.exposure.set_sensitive(state.enabled());
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
    controls.sync_guard.set(false);
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
    controls: ControlSet,
    module_actions: Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    module_revision: Rc<RefCell<Revision>>,
    action: F,
) where
    F: Fn(bool) -> ExposureAction + 'static,
{
    control.connect_active_notify(move |control| {
        dispatch(
            &state,
            &actions,
            &controls,
            &module_actions,
            &module_revision,
            action(control.is_active()),
        );
    });
}

fn connect_expander_action(
    control: &gtk4::Expander,
    state: Rc<RefCell<ExposureModuleState>>,
    actions: Rc<RefCell<Option<ExposureActionHandler>>>,
    controls: ControlSet,
    module_actions: Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    module_revision: Rc<RefCell<Revision>>,
) {
    control.connect_expanded_notify(move |control| {
        dispatch(
            &state,
            &actions,
            &controls,
            &module_actions,
            &module_revision,
            ExposureAction::SetExpanded(control.is_expanded()),
        );
    });
}

fn connect_mode_action(
    control: &gtk4::DropDown,
    state: Rc<RefCell<ExposureModuleState>>,
    actions: Rc<RefCell<Option<ExposureActionHandler>>>,
    controls: ControlSet,
    module_actions: Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    module_revision: Rc<RefCell<Revision>>,
) {
    control.connect_selected_notify(move |control| {
        let mode = if control.selected() == mode_index(ExposureMode::Manual) {
            ExposureMode::Manual
        } else {
            ExposureMode::Automatic
        };
        dispatch(
            &state,
            &actions,
            &controls,
            &module_actions,
            &module_revision,
            ExposureAction::SetMode(mode),
        );
    });
}

fn connect_scale_action<F>(
    control: &gtk4::Scale,
    state: Rc<RefCell<ExposureModuleState>>,
    actions: Rc<RefCell<Option<ExposureActionHandler>>>,
    controls: ControlSet,
    module_actions: Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    module_revision: Rc<RefCell<Revision>>,
    action: F,
) where
    F: Fn(f64) -> ExposureAction + 'static,
{
    control.connect_value_changed(move |control| {
        dispatch(
            &state,
            &actions,
            &controls,
            &module_actions,
            &module_revision,
            action(control.value()),
        );
    });
}

fn dispatch(
    state: &Rc<RefCell<ExposureModuleState>>,
    actions: &Rc<RefCell<Option<ExposureActionHandler>>>,
    controls: &ControlSet,
    module_actions: &Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    module_revision: &Rc<RefCell<Revision>>,
    action: ExposureAction,
) {
    if controls.sync_guard.get() || state.borrow_mut().apply(action).is_err() {
        return;
    }
    sync_controls(state, controls);
    if let Some(handler) = actions.borrow().as_ref() {
        handler(action);
    }
    if let Some(action) = exposure_module_action(action, *module_revision.borrow())
        && let Some(handler) = module_actions.borrow().as_ref()
        && let Ok(revision) = handler(action)
    {
        *module_revision.borrow_mut() = revision;
    }
}

fn exposure_module_action(
    action: ExposureAction,
    expected_revision: Revision,
) -> Option<DarkroomModuleAction> {
    let action = match action {
        ExposureAction::SetExpanded(expanded) => DarkroomModuleAction::Disclosure {
            module_id: "exposure".to_owned(),
            expected_revision,
            expanded,
        },
        ExposureAction::SetEnabled(enabled) => DarkroomModuleAction::Enable {
            module_id: "exposure".to_owned(),
            expected_revision,
            enabled,
        },
        ExposureAction::SetExposureEv(value) => DarkroomModuleAction::Control {
            module_id: "exposure".to_owned(),
            expected_revision,
            id: "exposure-stops".to_owned(),
            value: crate::presentation::DarkroomControlValue::Slider(value),
        },
        ExposureAction::SetBlackLevel(value) => DarkroomModuleAction::Control {
            module_id: "exposure".to_owned(),
            expected_revision,
            id: "exposure-black".to_owned(),
            value: crate::presentation::DarkroomControlValue::Slider(value),
        },
        ExposureAction::Reset => DarkroomModuleAction::Reset {
            module_id: "exposure".to_owned(),
            expected_revision,
        },
        ExposureAction::SetMode(_)
        | ExposureAction::SetCompensateExposureBias(_)
        | ExposureAction::SetCompensateHighlightPreservation(_) => return None,
    };
    Some(action)
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
