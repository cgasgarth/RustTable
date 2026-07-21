//! Darktable-style darkroom module columns and their GTK4 projection.

use std::{cell::RefCell, fmt, rc::Rc};

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::Revision;

use crate::presentation::PresentationTextError;
use crate::presentation::darkroom_controls::{
    ControlIdError, ControlValidationError, DarkroomControlError, DarkroomControlValue,
    DarkroomControlViewModel, DarkroomControlsViewModel,
};

use super::{ThemeRole, apply_theme_role};

#[path = "darkroom_controls/module_widgets.rs"]
mod module_widgets;
use module_widgets::{build_control_row, dispatch_module_action};
#[path = "darkroom_reference.rs"]
mod reference;
pub use reference::{DarkroomModuleAvailability, reference_modules};

#[cfg(test)]
#[path = "darkroom_modules_tests.rs"]
mod tests;

/// The side of the darkroom shell that owns a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarkroomModuleSide {
    Left,
    Right,
}

impl DarkroomModuleSide {
    #[must_use]
    pub const fn widget_name(self) -> &'static str {
        match self {
            Self::Left => "darkroom-left-modules",
            Self::Right => "darkroom-right-modules",
        }
    }
}

/// Error returned by a module-level action.
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomModuleError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    Control(DarkroomControlError),
    NotResettable,
    SnapshotRevisionRewind {
        current: Revision,
        replacement: Revision,
    },
    WrongModule {
        expected: String,
        actual: String,
    },
    DuplicateModule {
        id: String,
    },
    RevisionOverflow,
}

impl fmt::Display for DarkroomModuleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => {
                write!(
                    formatter,
                    "stale module callback: expected {expected}, current {actual}"
                )
            }
            Self::Control(error) => write!(formatter, "control error: {error:?}"),
            Self::NotResettable => formatter.write_str("module does not support reset"),
            Self::SnapshotRevisionRewind {
                current,
                replacement,
            } => write!(
                formatter,
                "module snapshot revision {replacement} is older than current {current}"
            ),
            Self::WrongModule { expected, actual } => {
                write!(
                    formatter,
                    "action targets module {expected}, received {actual}"
                )
            }
            Self::DuplicateModule { id } => write!(formatter, "duplicate darkroom module: {id}"),
            Self::RevisionOverflow => formatter.write_str("module revision counter overflowed"),
        }
    }
}

impl std::error::Error for DarkroomModuleError {}

/// Last-known module state exposed to a GTK status row.
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomModuleStatus {
    Ready,
    Stale {
        expected: Revision,
        actual: Revision,
    },
    Error(DarkroomModuleError),
}

/// A revision-safe action emitted by a module widget.
#[derive(Debug, Clone, PartialEq)]
pub enum DarkroomModuleAction {
    Disclosure {
        module_id: String,
        expected_revision: Revision,
        expanded: bool,
    },
    Enable {
        module_id: String,
        expected_revision: Revision,
        enabled: bool,
    },
    Reset {
        module_id: String,
        expected_revision: Revision,
    },
    Control {
        module_id: String,
        expected_revision: Revision,
        id: String,
        value: DarkroomControlValue,
    },
    Recover {
        module_id: String,
        expected_revision: Revision,
    },
}

impl DarkroomModuleAction {
    #[must_use]
    pub fn module_id(&self) -> &str {
        match self {
            Self::Disclosure { module_id, .. }
            | Self::Enable { module_id, .. }
            | Self::Reset { module_id, .. }
            | Self::Control { module_id, .. }
            | Self::Recover { module_id, .. } => module_id,
        }
    }
}

/// Callback type used by action-aware GTK module builders.
pub type DarkroomModuleActionHandler =
    Rc<dyn Fn(DarkroomModuleAction) -> Result<Revision, DarkroomModuleError>>;

/// One ordered, disclosure-capable module in a darkroom side panel.
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomModuleViewModel {
    id: String,
    title: String,
    side: DarkroomModuleSide,
    expanded: bool,
    enabled: bool,
    resettable: bool,
    revision: Revision,
    controls: DarkroomControlsViewModel,
    availability: DarkroomModuleAvailability,
    status: DarkroomModuleStatus,
}

impl DarkroomModuleViewModel {
    /// Creates a module and preserves the control order supplied by its owner.
    ///
    /// # Errors
    ///
    /// Returns an error when the module identity, title, or controls are invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        side: DarkroomModuleSide,
        expanded: bool,
        enabled: bool,
        resettable: bool,
        revision: Revision,
        controls: Vec<DarkroomControlViewModel>,
    ) -> Result<Self, ControlValidationError> {
        let id = id.into();
        let title = title.into();
        if id.trim().is_empty() {
            return Err(ControlValidationError::InvalidId(ControlIdError::Empty));
        }
        if title.trim().is_empty() {
            return Err(ControlValidationError::InvalidLabel(
                PresentationTextError::WhitespaceOnly,
            ));
        }
        let controls = DarkroomControlsViewModel::new(revision, controls)?;
        Ok(Self {
            id,
            title,
            side,
            expanded,
            enabled,
            resettable,
            revision,
            controls,
            availability: DarkroomModuleAvailability::Supported,
            status: DarkroomModuleStatus::Ready,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }

    #[must_use]
    pub const fn side(&self) -> DarkroomModuleSide {
        self.side
    }

    #[must_use]
    pub const fn expanded(&self) -> bool {
        self.expanded
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn resettable(&self) -> bool {
        self.resettable
    }

    #[must_use]
    pub const fn availability(&self) -> &DarkroomModuleAvailability {
        &self.availability
    }

    #[must_use]
    pub fn with_availability(mut self, availability: DarkroomModuleAvailability) -> Self {
        self.availability = availability;
        self
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn status(&self) -> &DarkroomModuleStatus {
        &self.status
    }

    #[must_use]
    pub const fn controls(&self) -> &DarkroomControlsViewModel {
        &self.controls
    }

    /// Returns stable widget names in GTK keyboard traversal order.
    #[must_use]
    pub fn focus_order(&self) -> Vec<String> {
        let mut order = vec![
            format!("{}-disclosure", self.id),
            format!("{}-enabled", self.id),
        ];
        if self.resettable {
            order.push(format!("{}-reset", self.id));
        }
        order.extend(
            self.controls
                .controls()
                .map(|control| format!("{}-widget", control.id())),
        );
        order
    }

    /// Applies a widget action after checking the revision captured by GTK.
    ///
    /// # Errors
    ///
    /// Returns a stale, wrong-module, validation, reset, or overflow error
    /// without applying an invalid action.
    pub fn apply(&mut self, action: DarkroomModuleAction) -> Result<Revision, DarkroomModuleError> {
        if action.module_id() != self.id {
            return Err(self.record_error(DarkroomModuleError::WrongModule {
                expected: self.id.clone(),
                actual: action.module_id().to_owned(),
            }));
        }
        match action {
            DarkroomModuleAction::Disclosure {
                expected_revision,
                expanded,
                ..
            } => self.set_expanded(expected_revision, expanded),
            DarkroomModuleAction::Enable {
                expected_revision,
                enabled,
                ..
            } => self.set_enabled(expected_revision, enabled),
            DarkroomModuleAction::Reset {
                expected_revision, ..
            } => self.reset(expected_revision),
            DarkroomModuleAction::Control {
                expected_revision,
                id,
                value,
                ..
            } => self.set_control(expected_revision, &id, value),
            DarkroomModuleAction::Recover {
                expected_revision, ..
            } => self.recover_stale(expected_revision),
        }
    }

    /// Reconciles a stale callback against a newer controller snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the replacement revision moves backward or its
    /// controls fail presentation validation.
    pub fn reconcile_snapshot(
        &mut self,
        revision: Revision,
        expanded: bool,
        enabled: bool,
        controls: Vec<DarkroomControlViewModel>,
    ) -> Result<(), DarkroomModuleError> {
        if revision < self.revision {
            return Err(
                self.record_error(DarkroomModuleError::SnapshotRevisionRewind {
                    current: self.revision,
                    replacement: revision,
                }),
            );
        }
        let replacement = DarkroomControlsViewModel::new(revision, controls)
            .map_err(DarkroomControlError::Validation)
            .map_err(|error| self.record_control_error(error))?;
        self.revision = revision;
        self.expanded = expanded;
        self.enabled = enabled;
        self.controls = replacement;
        self.status = DarkroomModuleStatus::Ready;
        Ok(())
    }

    /// Clears a stale status after the owner confirms that its snapshot is current.
    ///
    /// # Errors
    ///
    /// Returns a stale-revision error when the confirmation does not match the
    /// current module revision.
    pub fn recover_stale(
        &mut self,
        expected_revision: Revision,
    ) -> Result<Revision, DarkroomModuleError> {
        if expected_revision != self.revision {
            let error = DarkroomModuleError::StaleRevision {
                expected: expected_revision,
                actual: self.revision,
            };
            self.status = DarkroomModuleStatus::Stale {
                expected: expected_revision,
                actual: self.revision,
            };
            return Err(error);
        }
        self.status = DarkroomModuleStatus::Ready;
        Ok(self.revision)
    }

    #[must_use]
    pub fn status_text(&self) -> String {
        match &self.availability {
            DarkroomModuleAvailability::Unsupported { reason } => {
                return format!("Unavailable · {reason}");
            }
            DarkroomModuleAvailability::Deprecated { reason } => {
                return format!("Deprecated · {reason}");
            }
            DarkroomModuleAvailability::Supported => {}
        }
        match &self.status {
            DarkroomModuleStatus::Ready => format!("Ready · revision {}", self.revision),
            DarkroomModuleStatus::Stale { expected, actual } => {
                format!("Stale callback · refresh required (expected {expected}, current {actual})")
            }
            DarkroomModuleStatus::Error(error) => format!("Module error · {error}"),
        }
    }

    /// Changes disclosure without changing the ordered module list.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller's revision is stale or cannot advance.
    pub fn set_expanded(
        &mut self,
        expected_revision: Revision,
        expanded: bool,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        self.expanded = expanded;
        self.advance_revision()
    }

    /// Enables/disables the module and leaves its typed controls intact.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller's revision is stale or cannot advance.
    pub fn set_enabled(
        &mut self,
        expected_revision: Revision,
        enabled: bool,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        self.enabled = enabled;
        self.advance_revision()
    }

    /// Applies one typed slider, choice, or toggle action.
    ///
    /// # Errors
    ///
    /// Returns an error when the caller's revision or control value is invalid.
    pub fn set_control(
        &mut self,
        expected_revision: Revision,
        id: &str,
        value: DarkroomControlValue,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        let revision = self
            .controls
            .set_value(expected_revision, id, value)
            .map_err(|error| self.record_control_error(error))?;
        self.revision = revision;
        self.status = DarkroomModuleStatus::Ready;
        Ok(revision)
    }

    /// Resets all controls when the module exposes the Darktable reset affordance.
    ///
    /// # Errors
    ///
    /// Returns an error when the revision is stale, the module is not resettable,
    /// or a control cannot be reset.
    pub fn reset(&mut self, expected_revision: Revision) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        if !self.resettable {
            return Err(self.record_error(DarkroomModuleError::NotResettable));
        }
        let revision = self
            .controls
            .reset_all(expected_revision)
            .map_err(|error| self.record_control_error(error))?;
        self.revision = revision;
        self.status = DarkroomModuleStatus::Ready;
        Ok(revision)
    }

    fn check_revision(&mut self, expected: Revision) -> Result<(), DarkroomModuleError> {
        if expected != self.revision {
            let error = DarkroomModuleError::StaleRevision {
                expected,
                actual: self.revision,
            };
            self.status = DarkroomModuleStatus::Stale {
                expected,
                actual: self.revision,
            };
            return Err(error);
        }
        Ok(())
    }

    fn advance_revision(&mut self) -> Result<Revision, DarkroomModuleError> {
        let revision = self
            .revision
            .checked_increment()
            .map_err(|_| self.record_error(DarkroomModuleError::RevisionOverflow))?;
        self.revision = revision;
        self.controls
            .replace_snapshot(revision, self.controls.controls().cloned().collect())
            .map_err(|_| self.record_error(DarkroomModuleError::RevisionOverflow))?;
        self.status = DarkroomModuleStatus::Ready;
        Ok(revision)
    }

    fn record_control_error(&mut self, error: DarkroomControlError) -> DarkroomModuleError {
        let module_error = DarkroomModuleError::Control(error);
        self.status = DarkroomModuleStatus::Error(module_error.clone());
        module_error
    }

    fn record_error(&mut self, error: DarkroomModuleError) -> DarkroomModuleError {
        self.status = DarkroomModuleStatus::Error(error.clone());
        error
    }
}

/// Ordered left/right darkroom module columns.
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomModulesViewModel {
    left: Vec<DarkroomModuleViewModel>,
    right: Vec<DarkroomModuleViewModel>,
}

impl DarkroomModulesViewModel {
    /// Validates side assignments and identities while preserving insertion order within each side.
    ///
    /// # Errors
    ///
    /// Returns an error when a module's control snapshot is invalid.
    pub fn new(modules: Vec<DarkroomModuleViewModel>) -> Result<Self, DarkroomModuleError> {
        let mut left: Vec<DarkroomModuleViewModel> = Vec::new();
        let mut right: Vec<DarkroomModuleViewModel> = Vec::new();
        for module in modules {
            if left
                .iter()
                .chain(right.iter())
                .any(|item| item.id() == module.id())
            {
                return Err(DarkroomModuleError::DuplicateModule {
                    id: module.id().to_owned(),
                });
            }
            match module.side() {
                DarkroomModuleSide::Left => left.push(module),
                DarkroomModuleSide::Right => right.push(module),
            }
        }
        Ok(Self { left, right })
    }

    #[must_use = "iterate over left modules in deterministic order"]
    pub fn left_modules(&self) -> impl ExactSizeIterator<Item = &DarkroomModuleViewModel> {
        self.left.iter()
    }

    #[must_use = "iterate over right modules in deterministic order"]
    pub fn right_modules(&self) -> impl ExactSizeIterator<Item = &DarkroomModuleViewModel> {
        self.right.iter()
    }

    #[must_use]
    pub fn module(&self, id: &str) -> Option<&DarkroomModuleViewModel> {
        self.left
            .iter()
            .chain(self.right.iter())
            .find(|module| module.id() == id)
    }

    #[must_use]
    pub fn module_mut(&mut self, id: &str) -> Option<&mut DarkroomModuleViewModel> {
        self.left
            .iter_mut()
            .chain(self.right.iter_mut())
            .find(|module| module.id() == id)
    }
}

/// Builds one native GTK4 expander for a module snapshot without callbacks.
#[must_use]
pub fn build_module_panel(module: &DarkroomModuleViewModel) -> gtk4::Expander {
    build_module_panel_with_actions(module, None)
}

/// Builds a module panel and routes every interactive widget through a
/// revision-carrying action callback.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn build_module_panel_with_actions(
    module: &DarkroomModuleViewModel,
    action_handler: Option<DarkroomModuleActionHandler>,
) -> gtk4::Expander {
    let expected_revision = module.revision();
    let current_revision = Rc::new(RefCell::new(expected_revision));
    let module_id = module.id().to_owned();
    let module_available = module.availability().is_supported();
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    content.set_widget_name(&format!("{}-content", module.id()));
    apply_theme_role(&content, ThemeRole::Module);

    let status_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    status_row.set_widget_name(&format!("{}-status-row", module.id()));
    let status = gtk4::Label::new(Some(&module.status_text()));
    status.set_widget_name(&format!("{}-status", module.id()));
    status.set_halign(gtk4::Align::Start);
    status.set_hexpand(true);
    status.set_accessible_role(gtk4::AccessibleRole::Status);
    status.update_property(&[Property::Label("Module status")]);
    let recover = gtk4::Button::with_label("Refresh");
    recover.set_widget_name(&format!("{}-recover", module.id()));
    recover.set_sensitive(false);
    recover.set_focus_on_click(false);
    recover.update_property(&[Property::Label("Refresh module snapshot")]);
    status_row.append(&status);
    status_row.append(&recover);
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    header.set_widget_name(&format!("{}-header", module.id()));
    header.add_css_class("dt_module_header");
    let enabled = gtk4::CheckButton::new();
    enabled.set_widget_name(&format!("{}-enabled", module.id()));
    enabled.set_label(Some("Enabled"));
    enabled.set_active(module.enabled());
    enabled.set_sensitive(module_available);
    enabled.set_focusable(true);
    enabled.update_property(&[Property::Label("Enable module")]);
    header.append(&enabled);
    let presets = gtk4::Button::with_label("Presets");
    presets.set_widget_name(&format!("{}-presets", module.id()));
    presets.set_tooltip_text(Some("Presets are unavailable for this module"));
    presets.set_sensitive(false);
    presets.set_focusable(false);
    presets.update_property(&[Property::Label("Module presets unavailable")]);
    header.append(&presets);
    let reset = module.resettable().then(|| {
        let reset = gtk4::Button::with_label("Reset");
        reset.set_widget_name(&format!("{}-reset", module.id()));
        reset.set_sensitive(module_available && module.enabled());
        reset.set_focus_on_click(false);
        reset.set_halign(gtk4::Align::End);
        reset.update_property(&[Property::Label("Reset module to defaults")]);
        header.append(&reset);
        reset
    });
    content.append(&header);
    // Darktable inserts trouble/status content directly below the module header.
    content.append(&status_row);

    let mut control_rows = Vec::new();
    for control in module.controls().controls() {
        let row = build_control_row(
            control,
            module_available && module.enabled(),
            action_handler.clone(),
            status.clone(),
            recover.clone(),
            current_revision.clone(),
            module_id.clone(),
        );
        content.append(&row);
        control_rows.push(row);
    }

    let expander = gtk4::Expander::builder()
        .label(module.title())
        .expanded(module.expanded())
        .child(&content)
        .build();
    expander.set_widget_name(module.id());
    expander.set_focusable(true);
    expander.set_accessible_role(gtk4::AccessibleRole::Group);
    expander.update_property(&[Property::Label(module.title())]);
    apply_theme_role(&expander, ThemeRole::Module);

    if let Some(handler) = action_handler {
        let status_for_expander = status.clone();
        let recover_for_expander = recover.clone();
        let current_revision_for_expander = current_revision.clone();
        let handler_for_expander = handler.clone();
        let module_id_for_expander = module_id.clone();
        expander.connect_notify_local(Some("expanded"), move |expander, _| {
            dispatch_module_action(
                &handler_for_expander,
                &status_for_expander,
                &recover_for_expander,
                &current_revision_for_expander,
                DarkroomModuleAction::Disclosure {
                    module_id: module_id_for_expander.clone(),
                    expected_revision: *current_revision_for_expander.borrow(),
                    expanded: expander.is_expanded(),
                },
            );
        });

        let handler_for_enabled = handler.clone();
        let status_for_enabled = status.clone();
        let recover_for_enabled = recover.clone();
        let reset_for_enabled = reset.clone();
        let current_revision_for_enabled = current_revision.clone();
        let module_id_for_enabled = module_id.clone();
        enabled.connect_toggled(move |enabled| {
            if let Some(reset) = reset_for_enabled.as_ref() {
                reset.set_sensitive(enabled.is_active());
            }
            for row in &control_rows {
                row.set_sensitive(module_available && enabled.is_active());
            }
            dispatch_module_action(
                &handler_for_enabled,
                &status_for_enabled,
                &recover_for_enabled,
                &current_revision_for_enabled,
                DarkroomModuleAction::Enable {
                    module_id: module_id_for_enabled.clone(),
                    expected_revision: *current_revision_for_enabled.borrow(),
                    enabled: enabled.is_active(),
                },
            );
        });

        if let Some(reset) = reset {
            let status_for_reset = status.clone();
            let recover_for_reset = recover.clone();
            let handler_for_reset = handler.clone();
            let current_revision_for_reset = current_revision.clone();
            let module_id_for_reset = module_id.clone();
            reset.connect_clicked(move |_| {
                dispatch_module_action(
                    &handler_for_reset,
                    &status_for_reset,
                    &recover_for_reset,
                    &current_revision_for_reset,
                    DarkroomModuleAction::Reset {
                        module_id: module_id_for_reset.clone(),
                        expected_revision: *current_revision_for_reset.borrow(),
                    },
                );
            });
        }

        let current_revision_for_recovery = current_revision.clone();
        let handler_for_recovery = handler.clone();
        let status_for_recovery = status.clone();
        let recover_for_recovery = recover.clone();
        let module_id_for_recovery = module_id.clone();
        recover.connect_clicked(move |_| {
            dispatch_module_action(
                &handler_for_recovery,
                &status_for_recovery,
                &recover_for_recovery,
                &current_revision_for_recovery,
                DarkroomModuleAction::Recover {
                    module_id: module_id_for_recovery.clone(),
                    expected_revision: *current_revision_for_recovery.borrow(),
                },
            );
        });
    }
    expander
}

/// Builds a native GTK4 vertical module column in model order.
#[must_use]
pub fn build_module_column<'a>(
    modules: impl ExactSizeIterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
) -> gtk4::Box {
    build_module_column_with_actions(modules, side, None)
}

/// Builds a module column while preserving caller-supplied module order.
#[must_use]
pub fn build_module_column_with_actions<'a>(
    modules: impl ExactSizeIterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
    action_handler: Option<&DarkroomModuleActionHandler>,
) -> gtk4::Box {
    build_module_column_with_filter(modules, side, "", action_handler)
}

/// Builds a module column after applying a case-insensitive title/id search.
///
/// The empty-state label is intentionally explicit so a search never leaves a
/// blank rail that could be mistaken for missing module data.
#[must_use]
pub fn build_module_column_with_filter<'a>(
    modules: impl Iterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
    query: &str,
    action_handler: Option<&DarkroomModuleActionHandler>,
) -> gtk4::Box {
    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    column.set_widget_name(side.widget_name());
    column.set_vexpand(true);
    let query = if matches!(side, DarkroomModuleSide::Left) {
        String::new()
    } else {
        query.trim().to_ascii_lowercase()
    };
    let mut rendered = 0;
    for module in modules {
        if !module_matches_query(module, &query) {
            continue;
        }
        column.append(&build_module_panel_with_actions(
            module,
            action_handler.cloned(),
        ));
        rendered += 1;
    }
    if rendered == 0 {
        let empty = gtk4::Label::new(Some(if query.is_empty() {
            "No modules available"
        } else {
            "No modules match this search"
        }));
        empty.set_widget_name("darkroom-module-search-empty");
        empty.set_halign(gtk4::Align::Start);
        empty.add_css_class("dim-label");
        empty.set_accessible_role(gtk4::AccessibleRole::Status);
        column.append(&empty);
    }
    column
}

fn module_matches_query(module: &DarkroomModuleViewModel, query: &str) -> bool {
    query.is_empty()
        || module.title().to_ascii_lowercase().contains(query)
        || module.id().to_ascii_lowercase().contains(query)
}
