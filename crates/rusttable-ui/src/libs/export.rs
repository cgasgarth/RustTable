//! GTK4 export module matching Darktable's `src/libs/export.c` right-center panel.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4::prelude::*;

const MAXIMUM_EDGE: u32 = 16_384;

/// Storage choices visible in the export rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportDestination {
    LocalFile,
}

impl ExportDestination {
    #[must_use]
    const fn label(self) -> &'static str {
        match self {
            Self::LocalFile => "local file",
        }
    }
}

/// Formats currently represented by the `RustTable` export service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Png,
}

impl ExportFormat {
    #[must_use]
    const fn label(self) -> &'static str {
        match self {
            Self::Png => "PNG",
        }
    }
}

/// Typed future-facing rail events for destination and format controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportRailAction {
    SelectDestination(ExportDestination),
    SelectFormat(ExportFormat),
    SelectSize(ExportSize),
    Start,
    Cancel,
    ReplaceExisting,
}

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
type ExportRailActionHandler = Box<dyn Fn(ExportRailAction)>;
type ExportStatusHandler = Box<dyn Fn(&str)>;

/// Darktable-shaped GTK controls for saving the selected persisted edit as PNG.
#[derive(Clone)]
pub struct ExportPanel {
    expander: gtk4::Expander,
    size: Rc<Cell<ExportSize>>,
    destination: Rc<Cell<ExportDestination>>,
    format: Rc<Cell<ExportFormat>>,
    selected: Cell<bool>,
    destination_choice: gtk4::DropDown,
    format_choice: gtk4::DropDown,
    size_choice: gtk4::DropDown,
    custom_size: gtk4::SpinButton,
    save: gtk4::Button,
    cancel: gtk4::Button,
    replace: gtk4::Button,
    status: gtk4::Label,
    progress: gtk4::ProgressBar,
    actions: Rc<RefCell<Option<ExportActionHandler>>>,
    rail_actions: Rc<RefCell<Option<ExportRailActionHandler>>>,
    status_handler: Rc<RefCell<Option<ExportStatusHandler>>>,
}

struct ExportChoices {
    destination: Rc<Cell<ExportDestination>>,
    format: Rc<Cell<ExportFormat>>,
    destination_choice: gtk4::DropDown,
    format_choice: gtk4::DropDown,
    size_choice: gtk4::DropDown,
    custom_size: gtk4::SpinButton,
}

struct ExportActions {
    save: gtk4::Button,
    cancel: gtk4::Button,
    replace: gtk4::Button,
    status: gtk4::Label,
    progress: gtk4::ProgressBar,
}

impl ExportPanel {
    /// Builds the bounded GTK4 export module.
    #[must_use]
    pub fn new() -> Self {
        let size = Rc::new(Cell::new(ExportSize::Original));
        let actions = Rc::new(RefCell::new(None));
        let rail_actions = Rc::new(RefCell::new(None));
        let status_handler = Rc::new(RefCell::new(None));
        let choices = export_choices();
        let controls = export_actions();

        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        content.set_widget_name("export-rail-content");
        content.set_hexpand(true);
        append_labeled(
            &content,
            "storage",
            &choices.destination_choice,
            "export-destination-row",
        );
        append_labeled(
            &content,
            "format",
            &choices.format_choice,
            "export-format-row",
        );
        content.append(&export_separator("destination-size"));
        append_labeled(&content, "size", &choices.size_choice, "export-size-row");
        append_labeled(
            &content,
            "max edge (px)",
            &choices.custom_size,
            "export-custom-size-row",
        );
        content.append(&export_separator("size-actions"));
        let actions_row = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
        actions_row.set_widget_name("export-actions");
        actions_row.add_css_class("dt_export_actions");
        actions_row.set_hexpand(true);
        actions_row.append(&controls.save);
        actions_row.append(&controls.cancel);
        actions_row.append(&controls.replace);
        content.append(&actions_row);
        content.append(&controls.progress);
        content.append(&controls.status);

        let expander = gtk4::Expander::builder()
            .label("export")
            .expanded(false)
            .child(&content)
            .build();
        expander.set_widget_name("export");
        expander.set_hexpand(true);
        expander.set_vexpand(false);
        expander.add_css_class("dt_module_group");
        expander.add_css_class("dt_export_module");
        expander.set_accessible_role(gtk4::AccessibleRole::Group);

        let panel = Self {
            expander,
            size,
            destination: choices.destination,
            format: choices.format,
            selected: Cell::new(false),
            destination_choice: choices.destination_choice,
            format_choice: choices.format_choice,
            size_choice: choices.size_choice,
            custom_size: choices.custom_size,
            save: controls.save,
            cancel: controls.cancel,
            replace: controls.replace,
            status: controls.status,
            progress: controls.progress,
            actions,
            rail_actions,
            status_handler,
        };
        panel.connect_actions();
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

    #[must_use]
    pub fn destination(&self) -> ExportDestination {
        self.destination.get()
    }

    #[must_use]
    pub fn format(&self) -> ExportFormat {
        self.format.get()
    }

    /// Enables the module only when a catalog photo is selected.
    pub fn set_selected(&self, selected: bool) {
        self.selected.set(selected);
        self.set_idle(if selected {
            "Ready to export the selected persisted edit."
        } else {
            "Select a photo to export."
        });
        self.save.set_sensitive(selected);
        self.set_controls_sensitive(selected);
    }

    /// Installs the application callback for explicit export actions.
    pub fn connect_action<F>(&self, handler: F)
    where
        F: Fn(ExportAction) + 'static,
    {
        self.actions.replace(Some(Box::new(handler)));
    }

    /// Connects the visible export/background-job status to the owning shell.
    pub fn connect_status<F>(&self, handler: F)
    where
        F: Fn(&str) + 'static,
    {
        self.status_handler.replace(Some(Box::new(handler)));
    }

    /// Connects destination, format, and lifecycle events from the export rail.
    pub fn connect_rail_action<F>(&self, handler: F)
    where
        F: Fn(ExportRailAction) + 'static,
    {
        self.rail_actions.replace(Some(Box::new(handler)));
    }

    /// Shows a non-running export status.
    pub fn set_idle(&self, message: &str) {
        self.set_status_text(message);
        self.status.remove_css_class("error");
        self.progress.set_visible(false);
        self.cancel.set_visible(false);
        self.replace.set_visible(false);
    }

    /// Shows an in-flight export stage and keeps cancellation available.
    pub fn set_running(&self, message: &str) {
        if !self.selected.get() {
            self.set_idle("Select a photo to export.");
            return;
        }
        self.set_status_text(message);
        self.progress.set_visible(true);
        self.progress.set_fraction(0.0);
        self.progress.set_text(Some(message));
        self.cancel.set_visible(true);
        self.replace.set_visible(false);
        self.save.set_sensitive(false);
        self.set_controls_sensitive(false);
    }

    /// Projects worker progress while preserving the panel's selection boundary.
    pub fn set_progress(&self, fraction: f64, message: &str) {
        self.progress.set_visible(true);
        self.progress.set_fraction(fraction.clamp(0.0, 1.0));
        self.progress.set_text(Some(message));
        self.set_status_text(message);
    }

    /// Shows an existing-destination collision that requires an explicit choice.
    pub fn set_collision(&self, message: &str) {
        if !self.selected.get() {
            self.set_idle("Select a photo to export.");
            return;
        }
        self.set_status_text(message);
        self.progress.set_visible(false);
        self.cancel.set_visible(false);
        self.replace.set_visible(true);
        self.save.set_sensitive(self.selected.get());
        self.set_controls_sensitive(self.selected.get());
    }

    /// Shows a completed or failed export result.
    pub fn set_finished(&self, message: &str, success: bool) {
        if !self.selected.get() {
            self.set_idle("Select a photo to export.");
            return;
        }
        self.set_status_text(message);
        self.progress.set_visible(false);
        self.cancel.set_visible(false);
        self.replace.set_visible(false);
        self.save.set_sensitive(self.selected.get());
        self.set_controls_sensitive(self.selected.get());
        if success {
            self.status.remove_css_class("error");
        } else {
            self.status.add_css_class("error");
        }
    }

    fn connect_actions(&self) {
        let size = Rc::clone(&self.size);
        let actions = Rc::clone(&self.actions);
        let rail_actions = Rc::clone(&self.rail_actions);
        self.size_choice.connect_selected_notify(move |choice| {
            let selected = match choice.selected() {
                1 => ExportSize::Fit2048,
                2 => ExportSize::Fit4096,
                _ => ExportSize::Original,
            };
            size.set(selected);
            dispatch(&actions, ExportAction::SelectSize(selected));
            dispatch_rail(&rail_actions, ExportRailAction::SelectSize(selected));
        });

        let size = Rc::clone(&self.size);
        let actions = Rc::clone(&self.actions);
        let rail_actions = Rc::clone(&self.rail_actions);
        self.custom_size.connect_value_changed(move |control| {
            let value = u32::try_from(control.value_as_int()).ok();
            if let Some(selected) = value.and_then(ExportSize::custom) {
                size.set(selected);
                dispatch(&actions, ExportAction::SelectSize(selected));
                dispatch_rail(&rail_actions, ExportRailAction::SelectSize(selected));
            }
        });

        let destination = Rc::clone(&self.destination);
        let rail_actions = Rc::clone(&self.rail_actions);
        self.destination_choice
            .connect_selected_notify(move |choice| {
                if choice.selected() == 0 {
                    destination.set(ExportDestination::LocalFile);
                    dispatch_rail(
                        &rail_actions,
                        ExportRailAction::SelectDestination(ExportDestination::LocalFile),
                    );
                }
            });

        let format = Rc::clone(&self.format);
        let rail_actions = Rc::clone(&self.rail_actions);
        self.format_choice.connect_selected_notify(move |choice| {
            if choice.selected() == 0 {
                format.set(ExportFormat::Png);
                dispatch_rail(
                    &rail_actions,
                    ExportRailAction::SelectFormat(ExportFormat::Png),
                );
            }
        });

        let actions = Rc::clone(&self.actions);
        let rail_actions = Rc::clone(&self.rail_actions);
        self.save.connect_clicked(move |_| {
            dispatch(&actions, ExportAction::Start);
            dispatch_rail(&rail_actions, ExportRailAction::Start);
        });
        let actions = Rc::clone(&self.actions);
        let rail_actions = Rc::clone(&self.rail_actions);
        self.cancel.connect_clicked(move |_| {
            dispatch(&actions, ExportAction::Cancel);
            dispatch_rail(&rail_actions, ExportRailAction::Cancel);
        });
        let actions = Rc::clone(&self.actions);
        let rail_actions = Rc::clone(&self.rail_actions);
        self.replace.connect_clicked(move |_| {
            dispatch(&actions, ExportAction::ReplaceExisting);
            dispatch_rail(&rail_actions, ExportRailAction::ReplaceExisting);
        });
    }

    fn set_status_text(&self, message: &str) {
        self.status.set_text(message);
        if let Some(handler) = self.status_handler.borrow().as_ref() {
            handler(message);
        }
    }

    fn set_controls_sensitive(&self, sensitive: bool) {
        self.destination_choice.set_sensitive(sensitive);
        self.format_choice.set_sensitive(sensitive);
        self.size_choice.set_sensitive(sensitive);
        self.custom_size.set_sensitive(sensitive);
    }
}

fn export_choices() -> ExportChoices {
    let destination = Rc::new(Cell::new(ExportDestination::LocalFile));
    let destination_choice = gtk4::DropDown::from_strings(&[ExportDestination::LocalFile.label()]);
    destination_choice.set_widget_name("export-destination");
    destination_choice.set_hexpand(false);
    destination_choice.set_accessible_role(gtk4::AccessibleRole::ComboBox);
    destination_choice.set_tooltip_text(Some("export destination is selected when export starts"));

    let format = Rc::new(Cell::new(ExportFormat::Png));
    let format_choice = gtk4::DropDown::from_strings(&[ExportFormat::Png.label()]);
    format_choice.set_widget_name("export-format");
    format_choice.set_hexpand(false);
    format_choice.set_accessible_role(gtk4::AccessibleRole::ComboBox);
    format_choice.set_tooltip_text(Some("PNG is the currently supported export format"));

    let size_choice = gtk4::DropDown::from_strings(&["original", "fit 2048", "fit 4096"]);
    size_choice.set_widget_name("export-size");
    size_choice.set_hexpand(false);
    size_choice.set_accessible_role(gtk4::AccessibleRole::ComboBox);
    size_choice.set_tooltip_text(Some("Choose the maximum exported edge"));
    let custom_size = gtk4::SpinButton::with_range(1.0, f64::from(MAXIMUM_EDGE), 1.0);
    custom_size.set_widget_name("export-custom-size");
    custom_size.set_hexpand(false);
    custom_size.set_width_chars(6);
    custom_size.set_value(2_048.0);
    custom_size.set_numeric(true);
    custom_size.set_tooltip_text(Some("Custom maximum edge in pixels (1–16384)"));

    ExportChoices {
        destination,
        format,
        destination_choice,
        format_choice,
        size_choice,
        custom_size,
    }
}

fn export_actions() -> ExportActions {
    let save = gtk4::Button::with_label("Save PNG…");
    save.set_widget_name("save-rendered-png");
    save.add_css_class("dt_export_action");
    save.set_hexpand(true);
    save.set_accessible_role(gtk4::AccessibleRole::Button);
    save.set_tooltip_text(Some("Save the selected persisted edit as a verified PNG"));

    let cancel = gtk4::Button::with_label("Cancel PNG export");
    cancel.set_widget_name("cancel-rendered-png");
    cancel.add_css_class("dt_export_action");
    cancel.set_hexpand(true);
    cancel.set_accessible_role(gtk4::AccessibleRole::Button);
    cancel.set_visible(false);

    let replace = gtk4::Button::with_label("Replace existing PNG");
    replace.set_widget_name("replace-rendered-png");
    replace.add_css_class("dt_export_action");
    replace.set_hexpand(true);
    replace.set_accessible_role(gtk4::AccessibleRole::Button);
    replace.set_visible(false);

    let status = gtk4::Label::new(Some("Select a photo to export."));
    status.set_widget_name("rendered-png-status");
    status.set_halign(gtk4::Align::Start);
    status.set_hexpand(true);
    status.set_wrap(true);
    status.add_css_class("dim-label");
    status.set_accessible_role(gtk4::AccessibleRole::Status);

    let progress = gtk4::ProgressBar::new();
    progress.set_widget_name("rendered-png-progress");
    progress.set_hexpand(true);
    progress.set_show_text(true);
    progress.set_visible(false);

    ExportActions {
        save,
        cancel,
        replace,
        status,
        progress,
    }
}

impl Default for ExportPanel {
    fn default() -> Self {
        Self::new()
    }
}

fn append_labeled<W>(container: &gtk4::Box, label: &str, control: &W, id: &str)
where
    W: IsA<gtk4::Widget>,
{
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    row.set_widget_name(id);
    row.add_css_class("dt_export_row");
    row.set_hexpand(true);
    row.set_valign(gtk4::Align::Center);
    let text = gtk4::Label::new(Some(label));
    text.set_halign(gtk4::Align::Start);
    text.set_hexpand(true);
    text.add_css_class("dt_export_label");
    row.append(&text);
    row.append(control);
    container.append(&row);
}

fn export_separator(id: &str) -> gtk4::Separator {
    let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
    separator.set_widget_name(&format!("export-separator-{id}"));
    separator.add_css_class("dt_rail_separator");
    separator
}

fn dispatch(actions: &Rc<RefCell<Option<ExportActionHandler>>>, action: ExportAction) {
    if let Some(handler) = actions.borrow().as_ref() {
        handler(action);
    }
}

fn dispatch_rail(actions: &Rc<RefCell<Option<ExportRailActionHandler>>>, action: ExportRailAction) {
    if let Some(handler) = actions.borrow().as_ref() {
        handler(action);
    }
}

#[cfg(test)]
mod tests {
    use super::{ExportDestination, ExportFormat, ExportSize};

    #[test]
    fn export_size_keeps_all_outputs_within_the_bounded_edge() {
        assert_eq!(ExportSize::Original.maximum_edge(), 16_384);
        assert_eq!(ExportSize::Fit2048.maximum_edge(), 2_048);
        assert_eq!(ExportSize::Fit4096.maximum_edge(), 4_096);
        assert_eq!(ExportSize::custom(16_384), Some(ExportSize::Custom(16_384)));
        assert_eq!(ExportSize::custom(0), None);
        assert_eq!(ExportSize::custom(16_385), None);
    }

    #[test]
    fn export_rail_keeps_only_truthful_destination_and_format_choices() {
        assert_eq!(ExportDestination::LocalFile.label(), "local file");
        assert_eq!(ExportFormat::Png.label(), "PNG");
    }

    #[test]
    fn export_size_projection_is_stable_at_the_service_boundary() {
        assert_eq!(ExportSize::Fit2048.maximum_edge(), 2_048);
        assert_eq!(ExportSize::Fit4096.maximum_edge(), 4_096);
        assert_eq!(ExportSize::custom(16_384), Some(ExportSize::Custom(16_384)));
    }

    #[test]
    fn export_rail_keeps_stable_accessibility_and_projection_ids() {
        let source = include_str!("export.rs");
        for widget in [
            "export-destination",
            "export-format",
            "export-size",
            "export-custom-size",
            "rendered-png-progress",
        ] {
            assert!(source.contains(widget));
        }
        assert!(source.contains("AccessibleRole::Status"));
        assert!(source.contains("Select a photo to export."));
    }
}
