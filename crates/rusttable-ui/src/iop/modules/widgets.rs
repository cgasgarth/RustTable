//! GTK4 widgets for the typed darkroom module controls.

use std::{cell::RefCell, rc::Rc};

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};

use crate::bauhaus::slider_input::SliderInputSpec;
use crate::gui::darktable_components::{
    CONTROL_GAP, dropdown, provisional_scale, slider_with_input_spec, switch,
};
use crate::presentation::PresentationText;
use crate::presentation::darkroom_controls::{DarkroomControlKind, DarkroomControlValue};

use super::{
    DarkroomControlViewModel, DarkroomModuleAction, DarkroomModuleActionHandler,
    DarkroomModuleError, presentation_control_id,
};

/// Owned action-routing state captured by one control row.
pub(super) struct ControlRowActionContext {
    pub(super) action_handler: Option<DarkroomModuleActionHandler>,
    pub(super) status: gtk4::Label,
    pub(super) recover: gtk4::Button,
    pub(super) current_revision: Rc<RefCell<Revision>>,
    pub(super) module_id: String,
    pub(super) operation_id: Option<OperationId>,
}

/// Builds one ordered control row from the typed presentation snapshot.
#[allow(clippy::too_many_lines)]
pub(super) fn build_control_row(
    control: &DarkroomControlViewModel,
    panel_widget_id: &str,
    module_enabled: bool,
    action_context: ControlRowActionContext,
) -> gtk4::Box {
    let ControlRowActionContext {
        action_handler,
        status,
        recover,
        current_revision,
        module_id,
        operation_id,
    } = action_context;
    let control_widget_id =
        presentation_control_id(module_id.as_str(), panel_widget_id, control.id().as_str());
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, CONTROL_GAP);
    row.set_width_request(0);
    row.set_hexpand(true);
    row.set_widget_name(&control_widget_id);
    row.add_css_class("dt_module_row");
    let label = gtk4::Label::new(Some(control.label().as_str()));
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);
    label.set_width_chars(1);
    row.append(&label);

    match control.kind() {
        DarkroomControlKind::Slider => {
            let spec = control.slider_spec().expect("slider has slider metadata");
            if let Some(source) = control.source_mapped_slider_spec() {
                let mut input_spec = SliderInputSpec::IDENTITY
                    .with_suffix(source.suffix())
                    .with_default_value(spec.default_value())
                    .with_digits(source.digits());
                if source.automatic_step() {
                    input_spec = input_spec.with_automatic_step();
                }
                let slider = slider_with_input_spec(
                    &format!("{control_widget_id}-widget"),
                    spec.minimum(),
                    spec.maximum(),
                    spec.step(),
                    true,
                    input_spec,
                );
                slider.set_value(spec.value());
                slider.scale().set_draw_value(true);
                slider.scale().set_value_pos(gtk4::PositionType::Right);
                slider.scale().set_tooltip_text(Some(source.tooltip()));
                identify_control(slider.scale(), control, &control_widget_id, "Adjust slider");
                if let Some(handler) = action_handler {
                    let id = control.id().to_string();
                    slider.scale().connect_value_changed(move |slider| {
                        let expected_revision = *current_revision.borrow();
                        dispatch_module_action(
                            &handler,
                            &status,
                            &recover,
                            &current_revision,
                            DarkroomModuleAction::Control {
                                module_id: module_id.clone(),
                                operation_id,
                                expected_revision,
                                id: id.clone(),
                                value: DarkroomControlValue::Slider(slider.value()),
                            },
                        );
                    });
                }
                row.append(slider.widget());
            } else {
                let slider = provisional_scale(
                    &format!("{control_widget_id}-widget"),
                    spec.minimum(),
                    spec.maximum(),
                    spec.step(),
                    true,
                );
                slider.set_value(spec.value());
                slider.set_digits(slider_digits(spec.step()));
                slider.set_draw_value(true);
                slider.set_value_pos(gtk4::PositionType::Right);
                slider.set_tooltip_text(Some(&format!(
                    "{}; range {:.3} to {:.3}",
                    control.label().as_str(),
                    spec.minimum(),
                    spec.maximum()
                )));
                identify_control(&slider, control, &control_widget_id, "Adjust slider");
                if let Some(handler) = action_handler {
                    let id = control.id().to_string();
                    slider.connect_value_changed(move |slider| {
                        let expected_revision = *current_revision.borrow();
                        dispatch_module_action(
                            &handler,
                            &status,
                            &recover,
                            &current_revision,
                            DarkroomModuleAction::Control {
                                module_id: module_id.clone(),
                                operation_id,
                                expected_revision,
                                id: id.clone(),
                                value: DarkroomControlValue::Slider(slider.value()),
                            },
                        );
                    });
                }
                row.append(&slider);
            }
        }
        DarkroomControlKind::Choice => {
            let choices = control
                .choices()
                .map(PresentationText::as_str)
                .collect::<Vec<_>>();
            let choice = dropdown(&format!("{control_widget_id}-widget"), &choices);
            if let DarkroomControlValue::Choice(selected) = control.value() {
                choice.set_selected(u32::try_from(selected).unwrap_or(u32::MAX));
            }
            choice.set_tooltip_text(Some(control.label().as_str()));
            identify_control(&choice, control, &control_widget_id, "Select option");
            if let Some(handler) = action_handler {
                let id = control.id().to_string();
                choice.connect_selected_notify(move |choice| {
                    let Ok(selected) = usize::try_from(choice.selected()) else {
                        return;
                    };
                    let expected_revision = *current_revision.borrow();
                    dispatch_module_action(
                        &handler,
                        &status,
                        &recover,
                        &current_revision,
                        DarkroomModuleAction::Control {
                            module_id: module_id.clone(),
                            operation_id,
                            expected_revision,
                            id: id.clone(),
                            value: DarkroomControlValue::Choice(selected),
                        },
                    );
                });
            }
            row.append(&choice);
        }
        DarkroomControlKind::Toggle => {
            let toggle = switch(&format!("{control_widget_id}-widget"));
            if let DarkroomControlValue::Toggle(active) = control.value() {
                toggle.set_active(active);
            }
            toggle.set_tooltip_text(Some(control.label().as_str()));
            identify_control(&toggle, control, &control_widget_id, "Toggle option");
            if let Some(handler) = action_handler {
                let id = control.id().to_string();
                toggle.connect_active_notify(move |toggle| {
                    let expected_revision = *current_revision.borrow();
                    dispatch_module_action(
                        &handler,
                        &status,
                        &recover,
                        &current_revision,
                        DarkroomModuleAction::Control {
                            module_id: module_id.clone(),
                            operation_id,
                            expected_revision,
                            id: id.clone(),
                            value: DarkroomControlValue::Toggle(toggle.is_active()),
                        },
                    );
                });
            }
            row.append(&toggle);
        }
        DarkroomControlKind::Text => {
            let entry = gtk4::Entry::new();
            if let DarkroomControlValue::Text(value) = control.value() {
                entry.set_text(&value);
            }
            entry.set_hexpand(true);
            entry.set_tooltip_text(Some(control.label().as_str()));
            identify_control(&entry, control, &control_widget_id, "Edit text");
            if let Some(handler) = action_handler {
                let id = control.id().to_string();
                entry.connect_changed(move |entry| {
                    let expected_revision = *current_revision.borrow();
                    dispatch_module_action(
                        &handler,
                        &status,
                        &recover,
                        &current_revision,
                        DarkroomModuleAction::Control {
                            module_id: module_id.clone(),
                            operation_id,
                            expected_revision,
                            id: id.clone(),
                            value: DarkroomControlValue::Text(entry.text().to_string()),
                        },
                    );
                });
            }
            row.append(&entry);
        }
    }
    row.set_sensitive(module_enabled);
    row
}

fn identify_control<W>(
    widget: &W,
    control: &DarkroomControlViewModel,
    control_widget_id: &str,
    role: &str,
) where
    W: IsA<gtk4::Widget> + IsA<gtk4::Accessible>,
{
    widget.set_widget_name(&format!("{control_widget_id}-widget"));
    widget.update_property(&[Property::Label(&format!(
        "{}: {}",
        control.label().as_str(),
        role
    ))]);
    widget.set_focusable(true);
}

fn slider_digits(step: f64) -> i32 {
    let mut digits = 0;
    let mut scaled = step.abs();
    while scaled < 1.0 && digits < 6 {
        scaled *= 10.0;
        digits += 1;
    }
    digits
}

pub(super) fn dispatch_module_action(
    handler: &DarkroomModuleActionHandler,
    status: &gtk4::Label,
    recover: &gtk4::Button,
    current_revision: &RefCell<Revision>,
    action: DarkroomModuleAction,
) {
    match handler(action) {
        Ok(revision) => {
            *current_revision.borrow_mut() = revision;
            status.set_label(&format!("Ready · revision {revision}"));
            recover.set_sensitive(false);
        }
        Err(error) => {
            if let Some(actual) = stale_actual_revision(&error) {
                *current_revision.borrow_mut() = actual;
                status.set_label("Stale callback · refresh required");
                recover.set_sensitive(true);
            } else {
                status.set_label(&format!("Module error · {error}"));
            }
        }
    }
}

fn stale_actual_revision(error: &DarkroomModuleError) -> Option<Revision> {
    match error {
        DarkroomModuleError::StaleRevision { actual, .. }
        | DarkroomModuleError::Control(
            crate::presentation::DarkroomControlError::StaleRevision { actual, .. },
        ) => Some(*actual),
        _ => None,
    }
}
