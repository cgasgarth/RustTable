use std::cell::RefCell;
use std::rc::Rc;

use gtk4::accessible::Property;
use gtk4::prelude::*;

use super::session_controller::ImportSessionAction;
use super::session_model::{ImportSessionState, ImportSessionViewModel};

/// Stable keyboard/focus order for import review and recovery.
pub const IMPORT_SESSION_FOCUS_ORDER: [&str; 9] = [
    "import-session-review",
    "import-session-start",
    "import-session-pause",
    "import-session-resume",
    "import-session-retry",
    "import-session-recover",
    "import-session-rollback",
    "import-session-progress",
    "import-session-status",
];

type ActionHandler = Rc<dyn Fn(ImportSessionAction)>;

/// Darktable-shaped import-session review module.
#[derive(Clone)]
pub struct ImportSessionPanel {
    root: gtk4::Box,
    status: gtk4::Label,
    rows: gtk4::ListBox,
    progress: gtk4::ProgressBar,
    review: gtk4::Button,
    start: gtk4::Button,
    pause: gtk4::Button,
    resume: gtk4::Button,
    recover: gtk4::Button,
    rollback: gtk4::Button,
    receipt: gtk4::Label,
    action: Rc<RefCell<Option<ActionHandler>>>,
}

impl Default for ImportSessionPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSessionPanel {
    #[must_use]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 5);
        root.set_widget_name("import-session-panel");
        root.set_accessible_role(gtk4::AccessibleRole::Region);
        root.update_property(&[Property::Label("Import session review")]);

        let heading = gtk4::Label::new(Some("import session"));
        heading.set_halign(gtk4::Align::Start);
        heading.add_css_class("dt_module_heading");
        root.append(&heading);
        let status_label = status("import-session-status", "No import session");
        root.append(&status_label);

        let review = button("import-session-review", "Review discovered items");
        let start = button("import-session-start", "Import reviewed items");
        let pause = button("import-session-pause", "Pause");
        let resume = button("import-session-resume", "Resume");
        let recovery = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        let recover = button("import-session-recover", "Recover");
        let rollback = button("import-session-rollback", "Rollback safe changes");
        recovery.append(&recover);
        recovery.append(&rollback);
        root.append(&review);
        root.append(&start);
        root.append(&pause);
        root.append(&resume);
        root.append(&recovery);

        let rows = gtk4::ListBox::new();
        rows.set_widget_name("import-session-rows");
        rows.set_selection_mode(gtk4::SelectionMode::None);
        rows.set_accessible_role(gtk4::AccessibleRole::List);
        let scroll = gtk4::ScrolledWindow::builder()
            .child(&rows)
            .min_content_height(120)
            .vexpand(true)
            .build();
        root.append(&scroll);

        let progress = gtk4::ProgressBar::new();
        progress.set_widget_name("import-session-progress");
        progress.set_show_text(true);
        progress.set_accessible_role(gtk4::AccessibleRole::ProgressBar);
        root.append(&progress);
        let receipt = status("import-session-receipt", "No import receipt yet");
        root.append(&receipt);

        let action = Rc::new(RefCell::new(None));
        let panel = Self {
            root,
            status: status_label,
            rows,
            progress,
            review,
            start,
            pause,
            resume,
            recover,
            rollback,
            receipt,
            action,
        };
        panel.connect_static_actions();
        panel
    }

    #[must_use]
    pub const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    pub fn connect_action<F>(&self, handler: F)
    where
        F: Fn(ImportSessionAction) + 'static,
    {
        self.action.replace(Some(Rc::new(handler)));
    }

    /// Projects aliases, duplicate/failure rows, progress, receipts, and recovery state.
    pub fn set_state(&self, state: &ImportSessionViewModel) {
        self.status
            .set_text(state.diagnostic.as_deref().unwrap_or(state.state.label()));
        self.progress.set_fraction(if state.total == 0 {
            0.0
        } else {
            f64::from(state.completed) / f64::from(state.total)
        });
        self.progress
            .set_text(Some(&format!("{} of {}", state.completed, state.total)));
        self.start.set_sensitive(matches!(
            state.state,
            ImportSessionState::Reviewing | ImportSessionState::Paused
        ));
        self.pause
            .set_sensitive(state.state == ImportSessionState::Running);
        self.resume
            .set_sensitive(state.state == ImportSessionState::Paused);
        self.recover
            .set_sensitive(state.state == ImportSessionState::Failed);
        clear_children(&self.rows);
        for row in &state.rows {
            self.rows.append(&review_row(row, Rc::clone(&self.action)));
        }
        if let Some(id) = state.receipt_id.as_deref() {
            self.receipt.set_text(&format!("Privacy-safe receipt {id}"));
        } else {
            self.receipt.set_text("No import receipt yet");
        }
    }

    fn connect_static_actions(&self) {
        for (button, action) in [
            (&self.review, ImportSessionAction::Review),
            (&self.start, ImportSessionAction::Start),
            (&self.pause, ImportSessionAction::Pause),
            (&self.resume, ImportSessionAction::Resume),
            (&self.recover, ImportSessionAction::Recover),
            (&self.rollback, ImportSessionAction::Rollback),
        ] {
            let handler = Rc::clone(&self.action);
            button.connect_clicked(move |_| emit(&handler, action.clone()));
        }
    }
}

fn review_row(
    row: &super::session_model::ImportReviewRow,
    handler: Rc<RefCell<Option<ActionHandler>>>,
) -> gtk4::ListBoxRow {
    let list_row = gtk4::ListBoxRow::new();
    let box_widget = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    box_widget.set_margin_top(3);
    box_widget.set_margin_bottom(3);
    let alias = gtk4::Label::new(Some(&row.alias));
    alias.set_halign(gtk4::Align::Start);
    alias.set_hexpand(true);
    alias.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
    box_widget.append(&alias);
    let result = gtk4::Label::new(Some(row.outcome.label()));
    result.set_halign(gtk4::Align::End);
    box_widget.append(&result);
    if let Some(detail) = &row.detail {
        result.set_tooltip_text(Some(detail));
    }
    if row.outcome.can_retry() {
        let retry = button("import-session-retry", "Retry");
        let item_id = row.item_id.clone();
        retry.connect_clicked(move |_| emit(&handler, ImportSessionAction::Retry(item_id.clone())));
        box_widget.append(&retry);
    }
    list_row.set_child(Some(&box_widget));
    list_row
}

fn status(id: &str, text: &str) -> gtk4::Label {
    let label = gtk4::Label::new(Some(text));
    label.set_widget_name(id);
    label.set_halign(gtk4::Align::Start);
    label.set_wrap(true);
    label.set_accessible_role(gtk4::AccessibleRole::Status);
    label
}

fn button(id: &str, label: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.set_hexpand(true);
    button.set_focus_on_click(false);
    button.update_property(&[Property::Label(label)]);
    button
}

fn emit(action: &RefCell<Option<ActionHandler>>, value: ImportSessionAction) {
    if let Some(handler) = action.borrow().as_ref() {
        handler(value);
    }
}

fn clear_children(container: &impl IsA<gtk4::Widget>) {
    let mut child = container.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        current.unparent();
    }
}

#[cfg(test)]
mod tests {
    use super::IMPORT_SESSION_FOCUS_ORDER;

    #[test]
    fn import_focus_order_covers_retry_and_recovery() {
        assert!(IMPORT_SESSION_FOCUS_ORDER.contains(&"import-session-retry"));
        assert!(IMPORT_SESSION_FOCUS_ORDER.contains(&"import-session-recover"));
        assert!(IMPORT_SESSION_FOCUS_ORDER.contains(&"import-session-status"));
    }
}
