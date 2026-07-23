//! GTK4 Exposure IOP panel matching Darktable's manual controls.

use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::Revision;
use rusttable_processing::{
    BLACK_LEVEL_MAXIMUM, BLACK_LEVEL_MINIMUM, BLACK_LEVEL_SOFT_MAXIMUM, BLACK_LEVEL_SOFT_MINIMUM,
    EXPOSURE_EV_MAXIMUM, EXPOSURE_EV_MINIMUM, EXPOSURE_EV_SOFT_MAXIMUM, EXPOSURE_EV_SOFT_MINIMUM,
    ExposureAction, ExposureActionError, ExposureMode, ExposureModuleState,
};

use super::modules::{DarkroomModuleAction, DarkroomModuleActionHandler, DarkroomModuleError};
use super::{ThemeRole, apply_theme_role};
use crate::gui::darktable_components::{
    MODULE_GAP, dropdown, module_expander as shared_module_expander, module_row, scale_row, slider,
    switch,
};

type ExposureActionHandler = Rc<dyn Fn(ExposureAction)>;

/// Native GTK4 realization of one Darktable Exposure module panel.
#[derive(Clone)]
pub struct ExposurePanel {
    expander: gtk4::Expander,
    state: Rc<RefCell<ExposureModuleState>>,
    mode_stack: gtk4::Stack,
    enabled: gtk4::ToggleButton,
    mode: gtk4::DropDown,
    exposure: gtk4::Scale,
    exposure_value: gtk4::Label,
    black: gtk4::Scale,
    black_value: gtk4::Label,
    status: gtk4::Label,
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
        let enabled = module_header_toggle(
            "exposure-enabled",
            "system-shutdown-symbolic",
            "Enable exposure module",
        );
        let mode = dropdown("exposure-mode", &["manual", "automatic"]);
        let mode_stack = gtk4::Stack::new();
        mode_stack.set_widget_name("exposure-mode-stack");
        mode_stack.set_hhomogeneous(false);
        let exposure = slider(
            "exposure-ev",
            EXPOSURE_EV_MINIMUM,
            EXPOSURE_EV_MAXIMUM,
            0.001,
            false,
        );
        let black = slider(
            "exposure-black",
            BLACK_LEVEL_MINIMUM,
            BLACK_LEVEL_MAXIMUM,
            0.0001,
            false,
        );
        exposure.set_digits(3);
        exposure.set_tooltip_text(Some(&format!(
            "adjust exposure correction; soft range {EXPOSURE_EV_SOFT_MINIMUM:.0} to \
             {EXPOSURE_EV_SOFT_MAXIMUM:.0} EV"
        )));
        black.set_digits(4);
        black.set_tooltip_text(Some(&format!(
            "adjust black level; soft range {BLACK_LEVEL_SOFT_MINIMUM:.1} to \
             {BLACK_LEVEL_SOFT_MAXIMUM:.1}"
        )));
        let compensate_exposure_bias = switch("exposure-bias-compensation");
        let compensate_highlight_preservation = switch("exposure-highlight-compensation");
        let exposure_value = value_label("exposure-value", "Exposure value");
        let black_value = value_label("black-value", "Black-level value");
        let status = gtk4::Label::new(Some("Ready"));
        status.set_widget_name("exposure-status");
        status.set_halign(gtk4::Align::Start);
        status.set_hexpand(true);
        status.set_accessible_role(gtk4::AccessibleRole::Status);
        status.update_property(&[Property::Label("Exposure module status")]);
        status.add_css_class("dim-label");
        let manual = gtk4::Box::new(gtk4::Orientation::Vertical, MODULE_GAP);
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
        let presets = module_header_button(
            "exposure-presets",
            "view-more-symbolic",
            "Exposure presets unavailable",
        );
        presets.set_sensitive(false);
        let reset = module_header_button(
            "exposure-reset",
            "edit-undo-symbolic",
            "Reset exposure module",
        );
        let multi = module_header_button(
            "exposure-multi",
            "list-add-symbolic",
            "Multiple exposure instances are unavailable",
        );
        multi.set_sensitive(false);
        let content = gtk4::Box::new(gtk4::Orientation::Vertical, MODULE_GAP);
        content.set_width_request(0);
        content.set_hexpand(true);
        append_dropdown_row(&content, "mode", &mode);
        content.append(&mode_stack);
        append_scale_row(&content, "black", &black, &black_value, "");
        content.append(&status);

        let header = gtk4::Box::new(gtk4::Orientation::Horizontal, MODULE_GAP);
        header.set_width_request(0);
        header.set_hexpand(true);
        header.add_css_class("dt_darkroom_module_header");
        let title = gtk4::Label::new(Some("exposure"));
        title.set_halign(gtk4::Align::Start);
        title.set_hexpand(true);
        header.append(&enabled);
        header.append(&title);
        header.append(&presets);
        header.append(&reset);
        header.append(&multi);

        let expander = shared_module_expander(
            "exposure",
            "exposure",
            initial_state.expanded(),
            Some(&content),
        );
        expander.set_label_widget(Some(&header));
        expander.set_widget_name("exposure");
        apply_theme_role(&expander, ThemeRole::Module);
        expander.set_accessible_role(gtk4::AccessibleRole::Group);
        expander.update_property(&[Property::Label("Exposure processing module")]);
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
            status,
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
        self.actions.replace(Some(Rc::new(handler)));
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
        self.status
            .set_text(&format!("Ready · revision {revision}"));
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
        self.status.set_text("Ready");
        Ok(())
    }

    fn connect_actions(&self, reset: &gtk4::Button) {
        let controls = self.control_set();
        connect_enabled_action(
            &self.enabled,
            Rc::clone(&self.state),
            Rc::clone(&self.actions),
            controls.clone(),
            Rc::clone(&self.module_actions),
            Rc::clone(&self.module_revision),
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
            run_gtk_callback(|| {
                dispatch_from_gtk(
                    &state,
                    &actions,
                    &controls,
                    &module_actions,
                    &module_revision,
                    ExposureAction::Reset,
                );
            });
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
            status: self.status.clone(),
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
    enabled: gtk4::ToggleButton,
    mode: gtk4::DropDown,
    exposure: gtk4::Scale,
    exposure_value: gtk4::Label,
    black: gtk4::Scale,
    black_value: gtk4::Label,
    status: gtk4::Label,
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
    container.append(&module_row(label, control));
}

fn append_dropdown_row(container: &gtk4::Box, label: &str, control: &gtk4::DropDown) {
    container.append(&module_row(label, control));
}

fn append_scale_row(
    container: &gtk4::Box,
    label: &str,
    control: &gtk4::Scale,
    value: &gtk4::Label,
    unit: &str,
) {
    container.append(&scale_row(label, control, value, unit));
}

fn value_label(id: &str, accessible_name: &str) -> gtk4::Label {
    let label = gtk4::Label::new(None);
    label.set_widget_name(id);
    label.add_css_class("dt_module_value");
    label.set_halign(gtk4::Align::End);
    label.set_width_chars(1);
    label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
    label.update_property(&[Property::Label(accessible_name)]);
    label
}

fn module_header_button(id: &str, icon_name: &str, accessible_name: &str) -> gtk4::Button {
    let button = gtk4::Button::new();
    configure_module_header_control(&button, id, icon_name, accessible_name);
    button
}

fn module_header_toggle(id: &str, icon_name: &str, accessible_name: &str) -> gtk4::ToggleButton {
    let button = gtk4::ToggleButton::new();
    configure_module_header_control(&button, id, icon_name, accessible_name);
    button
}

fn configure_module_header_control(
    control: &(impl IsA<gtk4::Button> + IsA<gtk4::Widget> + IsA<gtk4::Accessible>),
    id: &str,
    icon_name: &str,
    accessible_name: &str,
) {
    control.set_widget_name(id);
    control.set_size_request(18, 18);
    control.set_focusable(false);
    control.add_css_class("dt_module_action");
    control.set_tooltip_text(Some(accessible_name));
    control.update_property(&[Property::Label(accessible_name)]);
    let icon = gtk4::Image::from_icon_name(icon_name);
    icon.set_pixel_size(12);
    control.set_child(Some(&icon));
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
        run_gtk_callback(|| {
            dispatch_from_gtk(
                &state,
                &actions,
                &controls,
                &module_actions,
                &module_revision,
                action(control.is_active()),
            );
        });
    });
}

fn connect_enabled_action(
    control: &gtk4::ToggleButton,
    state: Rc<RefCell<ExposureModuleState>>,
    actions: Rc<RefCell<Option<ExposureActionHandler>>>,
    controls: ControlSet,
    module_actions: Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    module_revision: Rc<RefCell<Revision>>,
) {
    control.connect_toggled(move |control| {
        run_gtk_callback(|| {
            dispatch_from_gtk(
                &state,
                &actions,
                &controls,
                &module_actions,
                &module_revision,
                ExposureAction::SetEnabled(control.is_active()),
            );
        });
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
        run_gtk_callback(|| {
            dispatch_from_gtk(
                &state,
                &actions,
                &controls,
                &module_actions,
                &module_revision,
                ExposureAction::SetExpanded(control.is_expanded()),
            );
        });
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
        run_gtk_callback(|| {
            let mode = if control.selected() == mode_index(ExposureMode::Manual) {
                ExposureMode::Manual
            } else {
                ExposureMode::Automatic
            };
            dispatch_from_gtk(
                &state,
                &actions,
                &controls,
                &module_actions,
                &module_revision,
                ExposureAction::SetMode(mode),
            );
        });
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
        run_gtk_callback(|| {
            dispatch_from_gtk(
                &state,
                &actions,
                &controls,
                &module_actions,
                &module_revision,
                action(control.value()),
            );
        });
    });
}

/// Seals the complete Rust body of every Exposure signal callback before it
/// is entered from GTK's `extern "C"` trampoline. This includes widget reads,
/// action construction, `RefCell` access, controller callbacks, and status
/// reporting; no panic may cross back into native GTK.
fn run_gtk_callback(callback: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(callback));
}

fn dispatch(
    state: &Rc<RefCell<ExposureModuleState>>,
    actions: &Rc<RefCell<Option<ExposureActionHandler>>>,
    controls: &ControlSet,
    module_actions: &Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    module_revision: &Rc<RefCell<Revision>>,
    action: ExposureAction,
) -> Result<(), ExposureDispatchError> {
    if controls.sync_guard.get() {
        return Ok(());
    }
    state
        .borrow_mut()
        .apply(action)
        .map_err(ExposureDispatchError::Action)?;
    sync_controls(state, controls);
    let requested_action = action;
    if let Some(handler) = actions.borrow().as_ref().map(Rc::clone) {
        catch_unwind(AssertUnwindSafe(|| handler(action))).map_err(|payload| {
            ExposureDispatchError::CallbackPanicked {
                action,
                message: panic_message(payload.as_ref()),
            }
        })?;
    }
    let expected_revision = *module_revision.borrow();
    let module_action = exposure_module_action(action, expected_revision);
    let module_handler = module_actions.borrow().clone();
    if let Some(action) = module_action
        && let Some(handler) = module_handler
    {
        let revision = catch_unwind(AssertUnwindSafe(|| handler(action))).map_err(|payload| {
            ExposureDispatchError::CallbackPanicked {
                action: requested_action,
                message: panic_message(payload.as_ref()),
            }
        })?;
        let revision = revision.map_err(ExposureDispatchError::Module)?;
        *module_revision.borrow_mut() = revision;
        controls
            .status
            .set_text(&format!("Ready · revision {revision}"));
    }
    Ok(())
}

fn dispatch_from_gtk(
    state: &Rc<RefCell<ExposureModuleState>>,
    actions: &Rc<RefCell<Option<ExposureActionHandler>>>,
    controls: &ControlSet,
    module_actions: &Rc<RefCell<Option<DarkroomModuleActionHandler>>>,
    module_revision: &Rc<RefCell<Revision>>,
    action: ExposureAction,
) {
    let result = catch_unwind(AssertUnwindSafe(|| {
        dispatch(
            state,
            actions,
            controls,
            module_actions,
            module_revision,
            action,
        )
    }));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => controls.status.set_text(&error.to_string()),
        Err(payload) => controls.status.set_text(
            &ExposureDispatchError::CallbackPanicked {
                action,
                message: panic_message(payload.as_ref()),
            }
            .to_string(),
        ),
    }
}

#[derive(Debug)]
enum ExposureDispatchError {
    Action(ExposureActionError),
    Module(DarkroomModuleError),
    CallbackPanicked {
        action: ExposureAction,
        message: String,
    },
}

impl std::fmt::Display for ExposureDispatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Action(error) => write!(formatter, "Exposure action rejected · {error}"),
            Self::Module(error) => write!(formatter, "Exposure module error · {error}"),
            Self::CallbackPanicked { action, message } => write!(
                formatter,
                "Exposure callback failed for {action:?} · {message}"
            ),
        }
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = payload.downcast_ref::<&str>() {
        return (*message).to_owned();
    }
    "non-string panic payload".to_owned()
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
