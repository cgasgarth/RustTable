//! GTK4 widgets for the typed darkroom module controls.

use std::{cell::RefCell, rc::Rc};

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::Revision;

use crate::presentation::PresentationText;
use crate::presentation::darkroom_controls::{DarkroomControlKind, DarkroomControlValue};

use super::{
    DarkroomControlViewModel, DarkroomModuleAction, DarkroomModuleActionHandler,
    DarkroomModuleError,
};

/// Builds one ordered control row from the typed presentation snapshot.
#[allow(clippy::too_many_lines)]
pub(super) fn build_control_row(
    control: &DarkroomControlViewModel,
    module_enabled: bool,
    action_handler: Option<DarkroomModuleActionHandler>,
    status: gtk4::Label,
    recover: gtk4::Button,
    current_revision: Rc<RefCell<Revision>>,
    module_id: String,
) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.set_widget_name(control.id().as_str());
    row.add_css_class("dt_module_row");
    let label = gtk4::Label::new(Some(control.label().as_str()));
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);
    row.append(&label);

    match control.kind() {
        DarkroomControlKind::Slider => {
            let spec = control.slider_spec().expect("slider has slider metadata");
            let slider = gtk4::Scale::with_range(
                gtk4::Orientation::Horizontal,
                spec.minimum(),
                spec.maximum(),
                spec.step(),
            );
            slider.set_value(spec.value());
            slider.set_sensitive(module_enabled);
            slider.set_hexpand(true);
            slider.set_digits(slider_digits(spec.step()));
            slider.set_draw_value(true);
            slider.set_value_pos(gtk4::PositionType::Right);
            slider.set_tooltip_text(Some(&format!(
                "{}; range {:.3} to {:.3}",
                control.label().as_str(),
                spec.minimum(),
                spec.maximum()
            )));
            identify_control(&slider, control, "Adjust slider");
            if let Some(handler) = action_handler {
                let id = control.id().to_string();
                slider.connect_value_changed(move |slider| {
                    dispatch_module_action(
                        &handler,
                        &status,
                        &recover,
                        &current_revision,
                        DarkroomModuleAction::Control {
                            module_id: module_id.clone(),
                            expected_revision: *current_revision.borrow(),
                            id: id.clone(),
                            value: DarkroomControlValue::Slider(slider.value()),
                        },
                    );
                });
            }
            row.append(&slider);
        }
        DarkroomControlKind::Choice => {
            let choices = control
                .choices()
                .map(PresentationText::as_str)
                .collect::<Vec<_>>();
            let choice = gtk4::DropDown::from_strings(&choices);
            if let DarkroomControlValue::Choice(selected) = control.value() {
                choice.set_selected(u32::try_from(selected).unwrap_or(u32::MAX));
            }
            choice.set_sensitive(module_enabled);
            choice.set_tooltip_text(Some(control.label().as_str()));
            identify_control(&choice, control, "Select option");
            if let Some(handler) = action_handler {
                let id = control.id().to_string();
                choice.connect_selected_notify(move |choice| {
                    let Ok(selected) = usize::try_from(choice.selected()) else {
                        return;
                    };
                    dispatch_module_action(
                        &handler,
                        &status,
                        &recover,
                        &current_revision,
                        DarkroomModuleAction::Control {
                            module_id: module_id.clone(),
                            expected_revision: *current_revision.borrow(),
                            id: id.clone(),
                            value: DarkroomControlValue::Choice(selected),
                        },
                    );
                });
            }
            row.append(&choice);
        }
        DarkroomControlKind::Toggle => {
            let toggle = gtk4::Switch::new();
            if let DarkroomControlValue::Toggle(active) = control.value() {
                toggle.set_active(active);
            }
            toggle.set_sensitive(module_enabled);
            toggle.set_tooltip_text(Some(control.label().as_str()));
            identify_control(&toggle, control, "Toggle option");
            if let Some(handler) = action_handler {
                let id = control.id().to_string();
                toggle.connect_active_notify(move |toggle| {
                    dispatch_module_action(
                        &handler,
                        &status,
                        &recover,
                        &current_revision,
                        DarkroomModuleAction::Control {
                            module_id: module_id.clone(),
                            expected_revision: *current_revision.borrow(),
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
            entry.set_sensitive(module_enabled);
            entry.set_hexpand(true);
            entry.set_tooltip_text(Some(control.label().as_str()));
            identify_control(&entry, control, "Edit text");
            if let Some(handler) = action_handler {
                let id = control.id().to_string();
                entry.connect_changed(move |entry| {
                    dispatch_module_action(
                        &handler,
                        &status,
                        &recover,
                        &current_revision,
                        DarkroomModuleAction::Control {
                            module_id: module_id.clone(),
                            expected_revision: *current_revision.borrow(),
                            id: id.clone(),
                            value: DarkroomControlValue::Text(entry.text().to_string()),
                        },
                    );
                });
            }
            row.append(&entry);
        }
    }
    row
}

fn identify_control<W>(widget: &W, control: &DarkroomControlViewModel, role: &str)
where
    W: IsA<gtk4::Widget> + IsA<gtk4::Accessible>,
{
    widget.set_widget_name(&format!("{}-widget", control.id()));
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
