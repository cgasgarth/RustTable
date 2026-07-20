#![allow(
    clippy::assigning_clones,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::match_same_arms,
    clippy::semicolon_if_nothing_returned,
    clippy::wildcard_imports
)]

use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::model::*;

type ActionHandler = Rc<dyn Fn(AiBatchAction)>;

#[derive(Clone)]
pub struct AiBatchPanel {
    root: gtk4::Box,
    task: gtk4::DropDown,
    strength: gtk4::Scale,
    policy: gtk4::DropDown,
    review_button: gtk4::Button,
    confirm_button: gtk4::Button,
    pause_button: gtk4::Button,
    resume_button: gtk4::Button,
    cancel_button: gtk4::Button,
    retry_button: gtk4::Button,
    reconcile_button: gtk4::Button,
    remove_history_button: gtk4::Button,
    table: gtk4::ListBox,
    progress: gtk4::ProgressBar,
    status: gtk4::Label,
}

impl AiBatchPanel {
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        root.set_widget_name("ai-batch");
        root.set_accessible_role(gtk4::AccessibleRole::Region);
        root.update_property(&[Property::Label("AI restoration batch workflow")]);
        let heading = gtk4::Label::new(Some("AI restoration batch"));
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("title-4");
        root.append(&heading);
        let hint = gtk4::Label::new(Some(
            "Review revision-pinned selections before service-owned processing.",
        ));
        hint.set_halign(gtk4::Align::Start);
        hint.set_wrap(true);
        hint.add_css_class("dim-label");
        root.append(&hint);
        let task = gtk4::DropDown::from_strings(&AiBatchTask::all().map(AiBatchTask::label));
        task.set_widget_name("ai-batch-task");
        root.append(&row("Task", &task));
        let strength = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.0, 100.0, 1.0);
        strength.set_widget_name("ai-batch-strength");
        strength.set_value(50.0);
        strength.set_draw_value(true);
        strength.set_hexpand(true);
        root.append(&row("Strength", &strength));
        let policy = gtk4::DropDown::from_strings(&[
            "Process eligible; retain skipped",
            "Require all eligible",
        ]);
        policy.set_widget_name("ai-batch-policy");
        root.append(&row("Enqueue policy", &policy));
        let review_button = gtk4::Button::with_label("Review eligibility");
        review_button.set_widget_name("ai-batch-review");
        let confirm_button = gtk4::Button::with_label("Confirm preflight");
        confirm_button.set_widget_name("ai-batch-confirm");
        confirm_button.add_css_class("suggested-action");
        confirm_button.set_sensitive(false);
        let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        actions.append(&review_button);
        actions.append(&confirm_button);
        root.append(&actions);
        let table = gtk4::ListBox::new();
        table.set_widget_name("ai-batch-table");
        table.set_selection_mode(gtk4::SelectionMode::None);
        table.set_accessible_role(gtk4::AccessibleRole::List);
        root.append(&table);
        let progress = gtk4::ProgressBar::new();
        progress.set_widget_name("ai-batch-progress");
        progress.set_show_text(true);
        progress.set_accessible_role(gtk4::AccessibleRole::ProgressBar);
        root.append(&progress);
        let controls = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        let pause_button = button("Pause new work", "ai-batch-pause");
        let resume_button = button("Resume", "ai-batch-resume");
        let cancel_button = button("Cancel", "ai-batch-cancel");
        let retry_button = button("Retry failed", "ai-batch-retry");
        let reconcile_button = button("Reconcile", "ai-batch-reconcile");
        let remove_history_button = button("Remove history", "ai-batch-remove-history");
        for control in [
            &pause_button,
            &resume_button,
            &cancel_button,
            &retry_button,
            &reconcile_button,
            &remove_history_button,
        ] {
            controls.append(control);
        }
        root.append(&controls);
        let status = gtk4::Label::new(Some("AI batch service unavailable"));
        status.set_widget_name("ai-batch-status");
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.set_accessible_role(gtk4::AccessibleRole::Status);
        status.update_property(&[Property::Label("AI batch workflow status")]);
        root.append(&status);
        Self {
            root,
            task,
            strength,
            policy,
            review_button,
            confirm_button,
            pause_button,
            resume_button,
            cancel_button,
            retry_button,
            reconcile_button,
            remove_history_button,
            table,
            progress,
            status,
        }
    }
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }
    pub fn set_state(&self, state: &AiBatchState) {
        clear(&self.table);
        let mut fraction = 0.0;
        let mut status = "AI batch service unavailable".to_owned();
        let mut can_confirm = false;
        match state {
            AiBatchState::Empty => {
                status = "Select one or more photos to review an AI batch.".to_owned()
            }
            AiBatchState::Unavailable { detail } => status.clone_from(detail),
            AiBatchState::Failed { detail } => status.clone_from(detail),
            AiBatchState::Reviewing(review) => {
                status = format!(
                    "{} eligible · {} skipped",
                    review.eligible_count(),
                    review.skipped_count()
                );
                can_confirm = review.can_enqueue();
                for item in review.items() {
                    self.add_item(item);
                }
            }
            AiBatchState::Preflight { review, summary } => {
                status = format!(
                    "Preflight: {} eligible · {} skipped · {} bytes · {} memory",
                    summary.eligible(),
                    summary.skipped(),
                    summary.estimate_bytes(),
                    summary.estimate_memory_bytes()
                );
                can_confirm = true;
                for item in review.items() {
                    self.add_item(item);
                }
            }
            AiBatchState::Queued { summary, .. } => {
                status = format!(
                    "Queued: {} eligible · {} skipped",
                    summary.eligible(),
                    summary.skipped()
                )
            }
            AiBatchState::Running {
                completed,
                total,
                stage,
                ..
            } => {
                fraction = if *total == 0 {
                    0.0
                } else {
                    *completed as f64 / *total as f64
                };
                status = stage.label().to_owned();
            }
            AiBatchState::Paused { .. } => {
                status = "Paused; durable work remains available to resume.".to_owned()
            }
            AiBatchState::Recovering { failed, .. } => {
                status = format!("Recovery required for {failed} item(s).")
            }
            AiBatchState::Complete { .. } => {
                fraction = 1.0;
                status = "Batch complete; receipts and supported recovery actions are available."
                    .to_owned();
            }
        }
        self.progress.set_fraction(fraction);
        self.progress.set_text(Some(&status));
        self.status.set_text(&status);
        self.confirm_button.set_sensitive(can_confirm);
    }
    fn add_item(&self, item: &AiBatchItem) {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let label = gtk4::Label::new(Some(&format!(
            "Photo {} · {} · {}",
            item.selection().photo_id,
            item.eligibility().label(),
            item.reason().unwrap_or(item.stage().label())
        )));
        label.set_halign(gtk4::Align::Start);
        label.set_hexpand(true);
        row.append(&label);
        let progress = gtk4::ProgressBar::new();
        progress.set_fraction(f64::from(item.progress()) / 100.0);
        progress.set_text(Some(item.stage().label()));
        progress.set_show_text(true);
        row.append(&progress);
        self.table.append(&row);
    }
    pub fn connect_action<F>(&self, handler: F)
    where
        F: Fn(AiBatchAction) + 'static,
    {
        let handler: ActionHandler = Rc::new(handler);
        let callback = Rc::clone(&handler);
        self.task.connect_selected_notify(move |dropdown| {
            if let Some(task) = AiBatchTask::all().get(dropdown.selected() as usize) {
                callback(AiBatchAction::SelectTask(*task));
            }
        });
        let callback = Rc::clone(&handler);
        self.strength.connect_value_changed(move |scale| {
            callback(AiBatchAction::SetStrength(
                scale.value().round().clamp(0.0, 100.0) as u8,
            ))
        });
        let callback = Rc::clone(&handler);
        self.policy.connect_selected_notify(move |dropdown| {
            callback(AiBatchAction::SetPolicy(if dropdown.selected() == 1 {
                AiBatchEnqueuePolicy::RequireAllEligible
            } else {
                AiBatchEnqueuePolicy::ProcessEligible
            }))
        });
        for (button, action) in [
            (&self.review_button, AiBatchAction::Review),
            (&self.confirm_button, AiBatchAction::Confirm),
            (&self.pause_button, AiBatchAction::Pause),
            (&self.resume_button, AiBatchAction::Resume),
            (&self.cancel_button, AiBatchAction::Cancel),
            (&self.retry_button, AiBatchAction::RetryFailed),
            (&self.reconcile_button, AiBatchAction::Reconcile),
            (&self.remove_history_button, AiBatchAction::RemoveHistory),
        ] {
            let callback = Rc::clone(&handler);
            button.connect_clicked(move |_| callback(action.clone()));
        }
    }
}
impl Default for AiBatchPanel {
    fn default() -> Self {
        Self::new()
    }
}
fn button(label: &str, id: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button
}
fn row(label: &str, widget: &impl IsA<gtk4::Widget>) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let label = gtk4::Label::new(Some(label));
    label.set_width_chars(14);
    label.set_halign(gtk4::Align::Start);
    row.append(&label);
    row.append(widget);
    row
}
fn clear(container: &gtk4::ListBox) {
    while let Some(child) = container.first_child() {
        child.unparent();
    }
}
