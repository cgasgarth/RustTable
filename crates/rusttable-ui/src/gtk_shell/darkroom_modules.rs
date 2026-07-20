//! Darktable-style darkroom module columns and their GTK4 projection.

use gtk4::prelude::*;
use rusttable_core::Revision;

use crate::presentation::darkroom_controls::{
    ControlIdError, ControlValidationError, DarkroomControlError, DarkroomControlKind,
    DarkroomControlValue, DarkroomControlViewModel, DarkroomControlsViewModel,
};
use crate::presentation::{PresentationText, PresentationTextError};

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
    RevisionOverflow,
}

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
    /// Validates side assignments while preserving insertion order within each side.
    ///
    /// # Errors
    ///
    /// Returns an error when a module's control snapshot is invalid.
    pub fn new(modules: Vec<DarkroomModuleViewModel>) -> Result<Self, DarkroomModuleError> {
        let mut left = Vec::new();
        let mut right = Vec::new();
        for module in modules {
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

/// Builds one native GTK4 expander for a module snapshot.
#[must_use]
pub fn build_module_panel(module: &DarkroomModuleViewModel) -> gtk4::Expander {
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    content.set_widget_name(&format!("{}-content", module.id()));

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let enabled = gtk4::CheckButton::new();
    enabled.set_label(Some("Enabled"));
    enabled.set_active(module.enabled());
    header.append(&enabled);
    if module.resettable() {
        let reset = gtk4::Button::with_label("Reset");
        reset.set_widget_name(&format!("{}-reset", module.id()));
        reset.set_sensitive(module.enabled());
        reset.set_halign(gtk4::Align::End);
        header.append(&reset);
    }
    content.append(&header);

    for control in module.controls().controls() {
        content.append(&build_control_row(control, module.enabled()));
    }

    let expander = gtk4::Expander::builder()
        .label(module.title())
        .expanded(module.expanded())
        .child(&content)
        .build();
    expander.set_widget_name(module.id());
    expander
}

fn build_control_row(control: &DarkroomControlViewModel, module_enabled: bool) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    row.set_widget_name(control.id().as_str());
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
            row.append(&choice);
        }
        DarkroomControlKind::Toggle => {
            let toggle = gtk4::Switch::new();
            if let DarkroomControlValue::Toggle(active) = control.value() {
                toggle.set_active(active);
            }
            toggle.set_sensitive(module_enabled);
            row.append(&toggle);
        }
    }
    row
}

/// Builds a native GTK4 vertical module column in model order.
#[must_use]
pub fn build_module_column<'a>(
    modules: impl ExactSizeIterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
) -> gtk4::Box {
    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    column.set_widget_name(side.widget_name());
    for module in modules {
        column.append(&build_module_panel(module));
    }
    column
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(id: &str, side: DarkroomModuleSide) -> DarkroomModuleViewModel {
        DarkroomModuleViewModel::new(
            id,
            id,
            side,
            true,
            true,
            true,
            Revision::from_u64(7),
            vec![
                DarkroomControlViewModel::slider(
                    id.to_owned() + "-amount",
                    "Amount",
                    0.0,
                    1.0,
                    0.01,
                    0.5,
                    0.0,
                )
                .expect("valid slider"),
            ],
        )
        .expect("valid module")
    }

    #[test]
    fn left_and_right_columns_keep_insertion_order() {
        let model = DarkroomModulesViewModel::new(vec![
            module("crop", DarkroomModuleSide::Right),
            module("navigation", DarkroomModuleSide::Left),
            module("exposure", DarkroomModuleSide::Right),
            module("snapshots", DarkroomModuleSide::Left),
        ])
        .expect("valid modules");
        assert_eq!(
            model
                .left_modules()
                .map(DarkroomModuleViewModel::id)
                .collect::<Vec<_>>(),
            ["navigation", "snapshots"]
        );
        assert_eq!(
            model
                .right_modules()
                .map(DarkroomModuleViewModel::id)
                .collect::<Vec<_>>(),
            ["crop", "exposure"]
        );
    }

    #[test]
    fn disclosure_enabled_and_reset_are_revision_guarded() {
        let mut model = module("exposure", DarkroomModuleSide::Right);
        let revision = model
            .set_expanded(Revision::from_u64(7), false)
            .expect("disclosure update");
        assert!(!model.expanded());
        let revision = model.set_enabled(revision, false).expect("enabled update");
        assert!(!model.enabled());
        let revision = model.reset(revision).expect("reset update");
        assert_eq!(revision, Revision::from_u64(10));
        assert!(matches!(model.status(), DarkroomModuleStatus::Ready));
    }

    #[test]
    fn stale_module_action_and_control_validation_are_visible() {
        let mut model = module("exposure", DarkroomModuleSide::Right);
        model
            .set_control(
                Revision::from_u64(7),
                "exposure-amount",
                DarkroomControlValue::Slider(0.75),
            )
            .expect("typed control update");
        let error = model
            .set_enabled(Revision::from_u64(7), false)
            .expect_err("old revision must be rejected");
        assert_eq!(
            error,
            DarkroomModuleError::StaleRevision {
                expected: Revision::from_u64(7),
                actual: Revision::from_u64(8),
            }
        );
        assert!(matches!(model.status(), DarkroomModuleStatus::Stale { .. }));
        let error = model
            .set_control(
                Revision::from_u64(8),
                "exposure-amount",
                DarkroomControlValue::Slider(4.0),
            )
            .expect_err("out of range slider must be rejected");
        assert!(matches!(
            error,
            DarkroomModuleError::Control(DarkroomControlError::Validation(_))
        ));
    }
}
