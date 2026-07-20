//! GTK4 export module matching Darktable's `src/libs/export.c` right-center panel.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;

const MAXIMUM_EDGE: u32 = 16_384;

/// Bounded output-size choices exposed by the GTK export module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSize {
    Original,
    Fit2048,
    Fit4096,
    Custom(u32),
}

impl ExportSize {
    /// Creates a custom maximum within the PNG export contract.
    #[must_use]
    pub const fn custom(value: u32) -> Option<Self> {
        if value == 0 || value > MAXIMUM_EDGE {
            None
        } else {
            Some(Self::Custom(value))
        }
    }

    #[must_use]
    pub const fn maximum_edge(self) -> u32 {
        match self {
            Self::Original => MAXIMUM_EDGE,
            Self::Fit2048 => 2_048,
            Self::Fit4096 => 4_096,
            Self::Custom(value) => value,
        }
    }
}

/// Actions emitted by the GTK export module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportAction {
    SelectSize(ExportSize),
    Start,
    Cancel,
    ReplaceExisting,
}

type ExportActionHandler = Box<dyn Fn(ExportAction)>;

/// Darktable-shaped GTK controls for saving the selected persisted edit as PNG.
#[derive(Clone)]
pub struct ExportPanel {
    expander: gtk4::Expander,
    size: Rc<Cell<ExportSize>>,
    save: gtk4::Button,
    cancel: gtk4::Button,
    replace: gtk4::Button,
    status: gtk4::Label,
    progress: gtk4::ProgressBar,
    actions: Rc<RefCell<Option<ExportActionHandler>>>,
}

impl ExportPanel {
    /// Builds the bounded GTK4 export module.
    #[must_use]
    pub fn new() -> Self {
        let size = Rc::new(Cell::new(ExportSize::Original));
        let actions = Rc::new(RefCell::new(None));
        let size_choice = gtk4::DropDown::from_strings(&["original", "fit 2048", "fit 4096"]);
        size_choice.set_selected(0);
        size_choice.set_tooltip_text(Some("Choose the maximum exported edge"));
        let custom = gtk4::SpinButton::with_range(1.0, f64::from(MAXIMUM_EDGE), 1.0);
        custom.set_value(2_048.0);
        custom.set_numeric(true);
        custom.set_tooltip_text(Some("Custom maximum edge in pixels (1–16384)"));

        let save = gtk4::Button::with_label("Save PNG…");
        save.set_widget_name("save-rendered-png");
        save.set_tooltip_text(Some("Save the selected persisted edit as a verified PNG"));
        let cancel = gtk4::Button::with_label("Cancel PNG export");
        cancel.set_widget_name("cancel-rendered-png");
        cancel.set_visible(false);
        let replace = gtk4::Button::with_label("Replace existing PNG");
        replace.set_widget_name("replace-rendered-png");
        replace.set_visible(false);
        let status = gtk4::Label::new(Some("Select a photo to export."));
        status.set_widget_name("rendered-png-status");
        status.set_halign(gtk4::Align::Start);
        status.set_wrap(true);
        status.add_css_class("dim-label");
        let progress = gtk4::ProgressBar::new();
        progress.set_widget_name("rendered-png-progress");
        progress.set_show_text(true);
        progress.set_visible(false);

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        append_labeled(&content, "size", &size_choice);
        append_labeled(&content, "custom maximum (px)", &custom);
        let actions_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        actions_row.append(&save);
        actions_row.append(&cancel);
        actions_row.append(&replace);
        content.append(&actions_row);
        content.append(&progress);
        content.append(&status);

        let expander = gtk4::Expander::builder()
            .label("export")
            .expanded(false)
            .child(&content)
            .build();
        expander.set_widget_name("export");
        expander.add_css_class("dt_module_group");

        let panel = Self {
            expander,
            size,
            save,
            cancel,
            replace,
            status,
            progress,
            actions,
        };
        panel.connect_actions(&size_choice, &custom);
        panel.set_selected(false);
        panel
    }

    /// Returns the root GTK widget.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Expander {
        &self.expander
    }

    /// Returns the currently selected output-size choice.
    #[must_use]
    pub fn size(&self) -> ExportSize {
        self.size.get()
    }

    /// Enables the module only when a catalog photo is selected.
    pub fn set_selected(&self, selected: bool) {
        self.set_idle(if selected {
            "Ready to export the selected persisted edit."
        } else {
            "Select a photo to export."
        });
        self.save.set_sensitive(selected);
    }

    /// Installs the application callback for explicit export actions.
    pub fn connect_action<F>(&self, handler: F)
    where
        F: Fn(ExportAction) + 'static,
    {
        self.actions.replace(Some(Box::new(handler)));
    }

    /// Shows a non-running export status.
    pub fn set_idle(&self, message: &str) {
        self.status.set_text(message);
        self.status.remove_css_class("error");
        self.progress.set_visible(false);
        self.cancel.set_visible(false);
        self.replace.set_visible(false);
    }

    /// Shows an in-flight export stage and keeps cancellation available.
    pub fn set_running(&self, message: &str) {
        self.status.set_text(message);
        self.progress.set_visible(true);
        self.progress.set_fraction(0.0);
        self.progress.set_text(Some(message));
        self.cancel.set_visible(true);
        self.replace.set_visible(false);
        self.save.set_sensitive(false);
    }

    /// Shows an existing-destination collision that requires an explicit choice.
    pub fn set_collision(&self, message: &str) {
        self.status.set_text(message);
        self.progress.set_visible(false);
        self.cancel.set_visible(false);
        self.replace.set_visible(true);
        self.save.set_sensitive(true);
    }

    /// Shows a completed or failed export result.
    pub fn set_finished(&self, message: &str, success: bool) {
        self.status.set_text(message);
        self.progress.set_visible(false);
        self.cancel.set_visible(false);
        self.replace.set_visible(false);
        self.save.set_sensitive(true);
        if success {
            self.status.remove_css_class("error");
        } else {
            self.status.add_css_class("error");
        }
    }

    fn connect_actions(&self, size_choice: &gtk4::DropDown, custom: &gtk4::SpinButton) {
        let size = Rc::clone(&self.size);
        let actions = Rc::clone(&self.actions);
        size_choice.connect_selected_notify(move |choice| {
            let selected = match choice.selected() {
                1 => ExportSize::Fit2048,
                2 => ExportSize::Fit4096,
                _ => ExportSize::Original,
            };
            size.set(selected);
            dispatch(&actions, ExportAction::SelectSize(selected));
        });

        let size = Rc::clone(&self.size);
        let actions = Rc::clone(&self.actions);
        custom.connect_value_changed(move |control| {
            let value = u32::try_from(control.value_as_int()).ok();
            if let Some(selected) = value.and_then(ExportSize::custom) {
                size.set(selected);
                dispatch(&actions, ExportAction::SelectSize(selected));
            }
        });

        let actions = Rc::clone(&self.actions);
        self.save
            .connect_clicked(move |_| dispatch(&actions, ExportAction::Start));
        let actions = Rc::clone(&self.actions);
        self.cancel
            .connect_clicked(move |_| dispatch(&actions, ExportAction::Cancel));
        let actions = Rc::clone(&self.actions);
        self.replace
            .connect_clicked(move |_| dispatch(&actions, ExportAction::ReplaceExisting));
    }
}

impl Default for ExportPanel {
    fn default() -> Self {
        Self::new()
    }
}

fn append_labeled<W>(container: &gtk4::Box, label: &str, control: &W)
where
    W: IsA<gtk4::Widget>,
{
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let text = gtk4::Label::new(Some(label));
    text.set_halign(gtk4::Align::Start);
    text.set_hexpand(true);
    row.append(&text);
    row.append(control);
    container.append(&row);
}

fn dispatch(actions: &Rc<RefCell<Option<ExportActionHandler>>>, action: ExportAction) {
    if let Some(handler) = actions.borrow().as_ref() {
        handler(action);
    }
}

#[cfg(test)]
mod tests {
    use super::ExportSize;

    #[test]
    fn export_size_keeps_all_outputs_within_the_bounded_edge() {
        assert_eq!(ExportSize::Original.maximum_edge(), 16_384);
        assert_eq!(ExportSize::Fit2048.maximum_edge(), 2_048);
        assert_eq!(ExportSize::Fit4096.maximum_edge(), 4_096);
        assert_eq!(ExportSize::custom(16_384), Some(ExportSize::Custom(16_384)));
        assert_eq!(ExportSize::custom(0), None);
        assert_eq!(ExportSize::custom(16_385), None);
    }
}
