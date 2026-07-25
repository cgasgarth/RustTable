//! Darktable-style darkroom module columns and their GTK4 projection.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};

use crate::gui::darktable_components::{
    button as shared_button, dropdown as shared_dropdown,
    module_expander as shared_module_expander, module_title,
};
use crate::iop::colorcorrection::{
    COLORCORRECTION_MODULE_ID, ColorCorrectionGridGtkContext, ColorCorrectionGridState,
    build_grid as build_color_correction_grid,
};
use crate::presentation::PresentationTextError;
use crate::presentation::darkroom_controls::{
    ControlId, ControlIdError, ControlValidationError, DarkroomControlError, DarkroomControlValue,
    DarkroomControlViewModel, DarkroomControlsViewModel,
};

use super::{ThemeRole, apply_theme_role};

mod widgets;
use widgets::{
    ControlRowActionContext, InstanceActionContext, build_control_row, build_instance_menu,
    connect_instance_menu, dispatch_module_action,
};
mod reference;
pub use reference::{DarkroomModuleAvailability, reference_modules};

#[cfg(test)]
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

pub use super::actions::{
    DarkroomModuleAction, DarkroomModuleActionHandler, DarkroomModuleError, DarkroomModulePreset,
    DarkroomModuleStatus,
};

/// Registry-backed darkroom grouping used by the GTK rail filters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarkroomModuleGroup {
    Active,
    Favorites,
    Basic,
    Tone,
    Color,
    Correct,
    Effects,
    Grading,
    Technical,
    Deprecated,
}

impl DarkroomModuleGroup {
    #[must_use]
    pub fn matches(self, module: &DarkroomModuleViewModel) -> bool {
        let visible_in_regular_group =
            !module.is_hidden() && (!module.availability().is_deprecated() || module.enabled());
        match self {
            Self::Active => module.enabled() && !module.is_hidden(),
            Self::Favorites => module.is_favorite() && visible_in_regular_group,
            Self::Deprecated => module.availability().is_deprecated(),
            Self::Basic => visible_in_regular_group && module.belongs_to_group("group.basic"),
            Self::Tone => visible_in_regular_group && module.belongs_to_group("group.tone"),
            Self::Color => visible_in_regular_group && module.belongs_to_group("group.color"),
            Self::Correct => {
                visible_in_regular_group
                    && (module.belongs_to_group("group.correct")
                        || module.belongs_to_group("group.corrective"))
            }
            Self::Effects => visible_in_regular_group && module.belongs_to_group("group.effects"),
            Self::Grading => visible_in_regular_group && module.belongs_to_group("group.grading"),
            Self::Technical => {
                visible_in_regular_group && module.belongs_to_group("group.technical")
            }
        }
    }
}

/// One ordered, disclosure-capable module in a darkroom side panel.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, PartialEq)]
pub struct DarkroomModuleViewModel {
    id: String,
    operation_id: Option<OperationId>,
    instance_sequence: usize,
    instance_count: usize,
    title: String,
    side: DarkroomModuleSide,
    expanded: bool,
    enabled: bool,
    resettable: bool,
    revision: Revision,
    controls: DarkroomControlsViewModel,
    color_correction_grid: Option<ColorCorrectionGridState>,
    presets: Vec<DarkroomModulePreset>,
    presets_unavailable_reason: Option<String>,
    availability: DarkroomModuleAvailability,
    status: DarkroomModuleStatus,
    group_keys: Vec<String>,
    aliases: Vec<String>,
    style_eligible: bool,
    favorite: bool,
    hidden: bool,
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
            operation_id: None,
            instance_sequence: 0,
            instance_count: 1,
            title,
            side,
            expanded,
            enabled,
            resettable,
            revision,
            controls,
            color_correction_grid: None,
            presets: Vec::new(),
            presets_unavailable_reason: None,
            availability: DarkroomModuleAvailability::Supported,
            status: DarkroomModuleStatus::Ready,
            group_keys: vec!["group.basic".to_owned()],
            aliases: Vec::new(),
            style_eligible: false,
            favorite: false,
            hidden: false,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Persisted operation instance represented by this panel.
    #[must_use]
    pub const fn operation_id(&self) -> Option<OperationId> {
        self.operation_id
    }

    /// Zero-based order among instances of the same compatibility operation.
    #[must_use]
    pub const fn instance_sequence(&self) -> usize {
        self.instance_sequence
    }

    #[must_use]
    pub const fn instance_count(&self) -> usize {
        self.instance_count
    }

    /// Binds a registry template to one persisted operation instance.
    #[must_use]
    pub const fn with_operation_instance(
        mut self,
        operation_id: OperationId,
        instance_sequence: usize,
        instance_count: usize,
    ) -> Self {
        self.operation_id = Some(operation_id);
        self.instance_sequence = instance_sequence;
        self.instance_count = instance_count;
        self
    }

    /// Stable GTK identity derived from compatibility identity and persisted
    /// operation identity. A sole instance retains legacy widget names.
    #[must_use]
    pub fn widget_id(&self) -> String {
        if self.instance_count <= 1 {
            return self.id.clone();
        }
        self.operation_id.map_or_else(
            || format!("{}-instance-{}", self.id, self.instance_sequence + 1),
            |operation_id| format!("{}-instance-{operation_id}", self.id),
        )
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
    pub fn with_registry_metadata(
        mut self,
        group_key: impl Into<String>,
        style_eligible: bool,
        hidden: bool,
    ) -> Self {
        self.group_keys = vec![group_key.into()];
        self.style_eligible = style_eligible;
        self.hidden = hidden;
        self
    }

    #[must_use]
    pub fn with_group_keys<I, S>(mut self, group_keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let group_keys = group_keys.into_iter().map(Into::into).collect::<Vec<_>>();
        if !group_keys.is_empty() {
            self.group_keys = group_keys;
        }
        self
    }

    #[must_use]
    pub fn with_aliases<I, S>(mut self, aliases: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.aliases = aliases.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn group_key(&self) -> &str {
        self.group_keys
            .first()
            .map_or("group.basic", String::as_str)
    }

    #[must_use = "iterate over every source group in declaration order"]
    pub fn group_keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.group_keys.iter().map(String::as_str)
    }

    #[must_use]
    pub fn belongs_to_group(&self, group_key: &str) -> bool {
        self.group_keys.iter().any(|group| group == group_key)
    }

    #[must_use = "iterate over module search aliases"]
    pub fn aliases(&self) -> impl ExactSizeIterator<Item = &str> {
        self.aliases.iter().map(String::as_str)
    }

    #[must_use]
    pub const fn is_style_eligible(&self) -> bool {
        self.style_eligible
    }

    #[must_use]
    pub const fn with_favorite(mut self, favorite: bool) -> Self {
        self.favorite = favorite;
        self
    }

    #[must_use]
    pub const fn is_favorite(&self) -> bool {
        self.favorite
    }

    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        self.hidden
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

    #[must_use]
    pub const fn color_correction_grid(&self) -> Option<ColorCorrectionGridState> {
        self.color_correction_grid
    }

    #[must_use]
    pub const fn with_color_correction_grid(mut self, grid: ColorCorrectionGridState) -> Self {
        self.color_correction_grid = Some(grid);
        self
    }

    #[must_use]
    pub fn presets(&self) -> impl ExactSizeIterator<Item = &DarkroomModulePreset> {
        self.presets.iter()
    }

    #[must_use]
    pub fn presets_enable_module(&self) -> bool {
        self.presets
            .iter()
            .any(DarkroomModulePreset::enables_module)
    }

    #[must_use]
    pub fn presets_unavailable_reason(&self) -> Option<&str> {
        self.presets_unavailable_reason.as_deref()
    }

    #[must_use]
    pub fn with_presets(mut self, presets: Vec<DarkroomModulePreset>) -> Self {
        self.presets = presets;
        self.presets_unavailable_reason = None;
        self
    }

    #[must_use]
    pub fn with_presets_unavailable(mut self, reason: impl Into<String>) -> Self {
        self.presets.clear();
        self.presets_unavailable_reason = Some(reason.into());
        self
    }

    #[must_use]
    pub fn supports_multi_instance(&self) -> bool {
        rusttable_processing::builtin_registry()
            .definitions()
            .iter()
            .find(|definition| definition.descriptor().id.compatibility_name == self.id)
            .is_some_and(|definition| {
                definition
                    .descriptor()
                    .flags
                    .contains(rusttable_processing::descriptor::OperationFlags::MULTI_INSTANCE)
            })
    }

    #[must_use]
    pub fn can_add_instance(&self) -> bool {
        self.operation_id.is_some()
            && self.availability.is_supported()
            && self.supports_multi_instance()
    }

    #[must_use]
    pub fn can_delete_instance(&self) -> bool {
        self.can_add_instance() && self.instance_count > 1
    }

    /// Returns stable widget names in GTK keyboard traversal order.
    #[must_use]
    pub fn focus_order(&self) -> Vec<String> {
        let widget_id = self.widget_id();
        let mut order = vec![format!("{widget_id}-disclosure")];
        if self.can_add_instance() {
            order.push(format!("{widget_id}-actions"));
        }
        order.push(format!("{widget_id}-enabled"));
        if self.resettable {
            order.push(format!("{widget_id}-reset"));
        }
        if !self.presets.is_empty() {
            order.insert(2, format!("{widget_id}-presets"));
        }
        if self.color_correction_grid.is_some() {
            order.push(format!("{widget_id}-grid"));
        }
        order.extend(self.controls.controls().map(|control| {
            format!(
                "{}-widget",
                presentation_control_id(
                    self.id.as_str(),
                    widget_id.as_str(),
                    control.id().as_str(),
                )
            )
        }));
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
        if action.operation_id() != self.operation_id {
            return Err(self.record_error(DarkroomModuleError::WrongOperation {
                module_id: self.id.clone(),
                expected: self.operation_id,
                actual: action.operation_id(),
            }));
        }
        if !self.availability.is_supported()
            && matches!(
                action,
                DarkroomModuleAction::Enable { .. }
                    | DarkroomModuleAction::Reset { .. }
                    | DarkroomModuleAction::Preset { .. }
                    | DarkroomModuleAction::Control { .. }
                    | DarkroomModuleAction::ColorCorrectionGrid { .. }
                    | DarkroomModuleAction::ColorCorrectionResetParameters { .. }
                    | DarkroomModuleAction::NewInstance { .. }
                    | DarkroomModuleAction::DuplicateInstance { .. }
                    | DarkroomModuleAction::MoveInstanceUp { .. }
                    | DarkroomModuleAction::MoveInstanceDown { .. }
                    | DarkroomModuleAction::DeleteInstance { .. }
            )
        {
            return Err(self.record_error(DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: self
                    .availability
                    .reason()
                    .unwrap_or("registry capability is not qualified")
                    .to_owned(),
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
            DarkroomModuleAction::Preset {
                expected_revision,
                preset_id,
                ..
            } => self.apply_preset(expected_revision, &preset_id),
            DarkroomModuleAction::Control {
                expected_revision,
                id,
                value,
                ..
            } => self.set_control(expected_revision, &id, value),
            DarkroomModuleAction::ColorCorrectionGrid {
                expected_revision,
                grid,
                ..
            } => self.set_color_correction_grid(expected_revision, grid),
            DarkroomModuleAction::ColorCorrectionResetParameters {
                expected_revision, ..
            } => self.reset_color_correction_parameters(expected_revision),
            DarkroomModuleAction::NewInstance {
                expected_revision, ..
            } => self.apply_instance_action(
                expected_revision,
                "new instance",
                self.can_add_instance(),
                "the module has no exact persisted multi-instance target",
            ),
            DarkroomModuleAction::DuplicateInstance {
                expected_revision, ..
            } => self.apply_instance_action(
                expected_revision,
                "duplicate instance",
                false,
                "the current edit model cannot copy native blend and mask state",
            ),
            DarkroomModuleAction::MoveInstanceUp {
                expected_revision, ..
            } => self.apply_instance_action(
                expected_revision,
                "move up",
                false,
                "the current edit model cannot apply native adjacent-module ordering",
            ),
            DarkroomModuleAction::MoveInstanceDown {
                expected_revision, ..
            } => self.apply_instance_action(
                expected_revision,
                "move down",
                false,
                "the current edit model cannot apply native adjacent-module ordering",
            ),
            DarkroomModuleAction::DeleteInstance {
                expected_revision, ..
            } => self.apply_instance_action(
                expected_revision,
                "delete",
                self.can_delete_instance(),
                "the final instance cannot be deleted",
            ),
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

    /// Reconciles the module from persisted operation parameter values.
    ///
    /// The application controller supplies values keyed by the GTK control id;
    /// the construction-time defaults remain in place for absent optional
    /// parameters and unknown persisted extensions.
    ///
    /// # Errors
    ///
    /// Returns a typed control or revision error when the persisted projection
    /// cannot be represented by this module's descriptor-backed controls.
    pub fn reconcile_operation<I>(
        &mut self,
        revision: Revision,
        enabled: bool,
        values: I,
    ) -> Result<(), DarkroomModuleError>
    where
        I: IntoIterator<Item = (String, DarkroomControlValue)>,
    {
        let mut controls = self.controls.controls().cloned().collect::<Vec<_>>();
        for (id, value) in values {
            let Some(control) = controls
                .iter_mut()
                .find(|control| control.id().as_str() == id)
            else {
                let control_id = ControlId::new(id.clone())
                    .map_err(ControlValidationError::InvalidId)
                    .map_err(DarkroomControlError::Validation)
                    .map_err(|error| self.record_control_error(error))?;
                return Err(
                    self.record_control_error(DarkroomControlError::UnknownControl(control_id))
                );
            };
            control.set_persisted_value(value).map_err(|error| {
                self.record_control_error(DarkroomControlError::Validation(error))
            })?;
        }
        self.reconcile_snapshot(revision, self.expanded, enabled, controls)
    }

    /// Reconciles the four native Color Correction endpoint parameters as one
    /// state without advancing the persisted edit revision.
    ///
    /// # Errors
    ///
    /// Returns an unsupported error if this module has no Color Correction grid.
    pub fn reconcile_color_correction_grid(
        &mut self,
        revision: Revision,
        grid: ColorCorrectionGridState,
    ) -> Result<(), DarkroomModuleError> {
        if self.color_correction_grid.is_none() {
            return Err(self.record_error(DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: "module has no Color Correction endpoint grid".to_owned(),
            }));
        }
        if revision < self.revision {
            return Err(
                self.record_error(DarkroomModuleError::SnapshotRevisionRewind {
                    current: self.revision,
                    replacement: revision,
                }),
            );
        }
        self.revision = revision;
        self.color_correction_grid = Some(grid);
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
            DarkroomModuleAvailability::Unsupported { reason }
            | DarkroomModuleAvailability::DeprecatedUnavailable { reason } => {
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
    /// Returns an error when the caller's processing revision is stale.
    ///
    /// Disclosure is presentation-only, so it deliberately does not advance
    /// the edit revision shared with persistence callbacks.
    pub fn set_expanded(
        &mut self,
        expected_revision: Revision,
        expanded: bool,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        self.expanded = expanded;
        self.status = DarkroomModuleStatus::Ready;
        Ok(self.revision)
    }

    /// Restores controller-owned disclosure state after processing state is
    /// reprojected. This is presentation-only and never changes the edit
    /// revision.
    pub fn restore_expanded_presentation(&mut self, expanded: bool) {
        self.expanded = expanded;
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
        if self.id == COLORCORRECTION_MODULE_ID && id == "colorcorrection-saturation" {
            self.enabled = true;
        }
        self.status = DarkroomModuleStatus::Ready;
        Ok(revision)
    }

    /// Applies all four Color Correction endpoint parameters in one revision.
    ///
    /// # Errors
    ///
    /// Returns an error for stale callbacks or non-Color-Correction modules.
    pub fn set_color_correction_grid(
        &mut self,
        expected_revision: Revision,
        grid: ColorCorrectionGridState,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        if self.id != COLORCORRECTION_MODULE_ID || self.color_correction_grid.is_none() {
            return Err(self.record_error(DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: "module has no Color Correction endpoint grid".to_owned(),
            }));
        }
        self.color_correction_grid = Some(grid);
        self.enabled = true;
        self.advance_revision()
    }

    /// Restores the five native Color Correction parameters while preserving
    /// operation-owned blend state such as opacity.
    ///
    /// # Errors
    ///
    /// Returns an error for stale callbacks or non-Color-Correction modules.
    pub fn reset_color_correction_parameters(
        &mut self,
        expected_revision: Revision,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        if self.id != COLORCORRECTION_MODULE_ID || self.color_correction_grid.is_none() {
            return Err(self.record_error(DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: "module has no Color Correction endpoint grid".to_owned(),
            }));
        }
        let revision = self
            .controls
            .reset_all(expected_revision)
            .map_err(|error| self.record_control_error(error))?;
        self.revision = revision;
        self.color_correction_grid = Some(ColorCorrectionGridState::DEFAULT);
        self.enabled = true;
        self.status = DarkroomModuleStatus::Ready;
        Ok(revision)
    }

    /// Resets all controls and enables the module when it exposes the Darktable
    /// reset affordance.
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
        if self.color_correction_grid.is_some() {
            self.color_correction_grid = Some(ColorCorrectionGridState::DEFAULT);
        }
        self.enabled = true;
        self.status = DarkroomModuleStatus::Ready;
        Ok(revision)
    }

    fn apply_preset(
        &mut self,
        expected_revision: Revision,
        preset_id: &str,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        if let Some(reason) = self.presets_unavailable_reason.as_deref() {
            let error = DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: reason.to_owned(),
            };
            return Err(self.record_error(error));
        }
        let Some(preset) = self.presets.iter().find(|preset| preset.id() == preset_id) else {
            return Err(self.record_error(DarkroomModuleError::UnknownPreset {
                module_id: self.id.clone(),
                preset_id: preset_id.to_owned(),
            }));
        };
        let values = preset.values().to_vec();
        let color_correction_grid = preset.color_correction_grid();
        let enables_module = preset.enables_module();
        let mut revision = expected_revision;
        for (control_id, value) in values {
            revision = self
                .controls
                .set_value(revision, &control_id, value)
                .map_err(|error| self.record_control_error(error))?;
        }
        if let Some(grid) = color_correction_grid {
            self.color_correction_grid = Some(grid);
        }
        if enables_module {
            self.enabled = true;
        }
        self.revision = revision;
        self.status = DarkroomModuleStatus::Ready;
        Ok(revision)
    }

    fn apply_instance_action(
        &mut self,
        expected_revision: Revision,
        action: &'static str,
        permitted: bool,
        unavailable_reason: &'static str,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        if !self.supports_multi_instance() {
            return Err(
                self.record_error(DarkroomModuleError::InstanceActionUnavailable {
                    module_id: self.id.clone(),
                    action,
                    reason: "the operation is single-instance",
                }),
            );
        }
        if self.operation_id.is_none() || !permitted {
            return Err(
                self.record_error(DarkroomModuleError::InstanceActionUnavailable {
                    module_id: self.id.clone(),
                    action,
                    reason: unavailable_reason,
                }),
            );
        }
        self.advance_revision()
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
            if left.iter().chain(right.iter()).any(|item| {
                item.id() == module.id() && item.operation_id() == module.operation_id()
            }) {
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
        let mut matches = self
            .left
            .iter()
            .chain(self.right.iter())
            .filter(|module| module.id() == id);
        let module = matches.next()?;
        matches.next().is_none().then_some(module)
    }

    #[must_use]
    pub fn module_mut(&mut self, id: &str) -> Option<&mut DarkroomModuleViewModel> {
        let matches = self
            .left
            .iter()
            .chain(self.right.iter())
            .filter(|module| module.id() == id)
            .count();
        if matches != 1 {
            return None;
        }
        self.left
            .iter_mut()
            .chain(self.right.iter_mut())
            .find(|module| module.id() == id)
    }

    #[must_use]
    pub fn module_target(
        &self,
        id: &str,
        operation_id: Option<OperationId>,
    ) -> Option<&DarkroomModuleViewModel> {
        match operation_id {
            Some(operation_id) => self
                .left
                .iter()
                .chain(self.right.iter())
                .find(|module| module.id() == id && module.operation_id() == Some(operation_id)),
            None => self.module(id),
        }
    }

    #[must_use]
    pub fn module_target_mut(
        &mut self,
        id: &str,
        operation_id: Option<OperationId>,
    ) -> Option<&mut DarkroomModuleViewModel> {
        match operation_id {
            Some(operation_id) => self
                .left
                .iter_mut()
                .chain(self.right.iter_mut())
                .find(|module| module.id() == id && module.operation_id() == Some(operation_id)),
            None => self.module_mut(id),
        }
    }

    #[must_use = "iterate over every persisted instance in stack order"]
    pub fn instances<'a>(
        &'a self,
        id: &'a str,
    ) -> impl Iterator<Item = &'a DarkroomModuleViewModel> + 'a {
        self.left
            .iter()
            .chain(self.right.iter())
            .filter(move |module| module.id() == id)
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
pub fn build_module_panel_with_actions(
    module: &DarkroomModuleViewModel,
    action_handler: Option<DarkroomModuleActionHandler>,
) -> gtk4::Expander {
    let current_revision = Rc::new(RefCell::new(module.revision()));
    build_module_panel_with_action_revision(module, action_handler, &current_revision)
}

#[must_use]
#[allow(clippy::too_many_lines)]
fn build_module_panel_with_action_revision(
    module: &DarkroomModuleViewModel,
    action_handler: Option<DarkroomModuleActionHandler>,
    current_revision: &Rc<RefCell<Revision>>,
) -> gtk4::Expander {
    let module_id = module.id().to_owned();
    let operation_id = module.operation_id();
    let widget_id = module.widget_id();
    let module_available = module.availability().is_supported();
    let module_resettable = module_available && module.resettable();
    let presets_enable_module = module.presets_enable_module();
    let deprecation_message = match module.availability() {
        DarkroomModuleAvailability::Deprecated { reason } => Some(reason.as_str()),
        DarkroomModuleAvailability::Supported
        | DarkroomModuleAvailability::DeprecatedUnavailable { .. }
        | DarkroomModuleAvailability::Unsupported { .. } => None,
    };
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    content.set_widget_name(&format!("{widget_id}-content"));
    apply_theme_role(&content, ThemeRole::Module);

    let status_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    status_row.set_widget_name(&format!("{widget_id}-status-row"));
    let status = gtk4::Label::new(Some(&module.status_text()));
    status.set_widget_name(&format!("{widget_id}-status"));
    status.set_halign(gtk4::Align::Start);
    status.set_hexpand(true);
    status.set_accessible_role(gtk4::AccessibleRole::Status);
    status.update_property(&[Property::Label("Module status")]);
    let recover = shared_button(&format!("{widget_id}-recover"), "Refresh");
    recover.set_sensitive(false);
    recover.set_focus_on_click(false);
    recover.update_property(&[Property::Label("Refresh module snapshot")]);
    if deprecation_message.is_none() {
        status_row.append(&status);
        status_row.append(&recover);
    }
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    header.set_widget_name(&format!("{widget_id}-header"));
    header.add_css_class("dt_module_header");
    let enabled = gtk4::CheckButton::new();
    enabled.set_widget_name(&format!("{widget_id}-enabled"));
    enabled.set_label(Some("Enabled"));
    enabled.set_active(module.enabled());
    enabled.set_sensitive(module_available);
    enabled.set_focusable(true);
    enabled.update_property(&[Property::Label("Enable module")]);
    header.append(&enabled);
    let presets = if module.presets().len() == 0 {
        let button = shared_button(&format!("{widget_id}-presets"), "Presets");
        button.set_tooltip_text(Some(
            module
                .presets_unavailable_reason()
                .unwrap_or("Presets are unavailable for this module"),
        ));
        button.set_sensitive(false);
        button.set_focusable(false);
        button.update_property(&[Property::Label("Module presets unavailable")]);
        header.append(&button);
        None
    } else {
        let labels = module
            .presets()
            .map(DarkroomModulePreset::label)
            .collect::<Vec<_>>();
        let dropdown = shared_dropdown(&format!("{widget_id}-presets"), &labels);
        dropdown.set_selected(u32::MAX);
        dropdown.set_sensitive(module_available && (module.enabled() || presets_enable_module));
        dropdown.set_focusable(true);
        dropdown.update_property(&[Property::Label("Choose module preset")]);
        header.append(&dropdown);
        Some(dropdown)
    };
    let reset = module.resettable().then(|| {
        let reset = shared_button(&format!("{widget_id}-reset"), "Reset");
        reset.set_sensitive(module_resettable);
        reset.set_focus_on_click(false);
        reset.set_halign(gtk4::Align::End);
        reset.update_property(&[Property::Label("Reset module to defaults")]);
        header.append(&reset);
        reset
    });
    content.append(&header);
    if let Some(deprecation_message) = deprecation_message {
        let warning = gtk4::Label::new(Some(deprecation_message));
        warning.set_widget_name(&format!("{widget_id}-deprecation-warning"));
        warning.set_halign(gtk4::Align::Start);
        warning.set_hexpand(true);
        warning.set_wrap(true);
        warning.set_max_width_chars(0);
        warning.set_xalign(0.0);
        warning.add_css_class("dt_warning");
        warning.set_accessible_role(gtk4::AccessibleRole::Status);
        warning.update_property(&[Property::Label("Module deprecation warning")]);
        content.append(&warning);
    } else {
        // RustTable's actionable backend status occupies Darktable's trouble
        // row only when there is no persistent source deprecation warning.
        content.append(&status_row);
    }

    let color_correction_controls_enable_module = module.id() == COLORCORRECTION_MODULE_ID;
    let synchronizing_enabled = Rc::new(Cell::new(false));
    let interaction_action_handler = if color_correction_controls_enable_module {
        action_handler.as_ref().map(|handler| {
            let handler = handler.clone();
            let enabled = enabled.clone();
            let synchronizing_enabled = Rc::clone(&synchronizing_enabled);
            Rc::new(move |action| {
                let result = handler(action);
                if result.is_ok() {
                    synchronizing_enabled.set(true);
                    enabled.set_active(true);
                    synchronizing_enabled.set(false);
                }
                result
            }) as DarkroomModuleActionHandler
        })
    } else {
        action_handler.clone()
    };
    let mut control_rows = Vec::new();
    for control in module.controls().controls() {
        let row = build_control_row(
            control,
            &widget_id,
            module_available && (module.enabled() || color_correction_controls_enable_module),
            ControlRowActionContext {
                action_handler: interaction_action_handler.clone(),
                status: status.clone(),
                recover: recover.clone(),
                current_revision: current_revision.clone(),
                module_id: module_id.clone(),
                operation_id,
            },
        );
        control_rows.push(row);
    }
    let color_correction_grid_widget = if let Some(grid_state) = module.color_correction_grid() {
        let saturation_widget_id = format!(
            "{}-widget",
            presentation_control_id(
                module.id(),
                widget_id.as_str(),
                "colorcorrection-saturation",
            )
        );
        let saturation = control_rows
            .iter()
            .find_map(|row| {
                let root: gtk4::Widget = row.clone().upcast();
                descendant_named(&root, &saturation_widget_id)
            })
            .and_then(|widget| widget.downcast::<gtk4::Scale>().ok())
            .expect("Color Correction source map retains its saturation scale");
        let DarkroomControlValue::Slider(initial_saturation) = module
            .controls()
            .control("colorcorrection-saturation")
            .expect("Color Correction source map retains its saturation value")
            .value()
        else {
            unreachable!("Color Correction saturation remains a scalar");
        };
        let commit_grid = interaction_action_handler.as_ref().map(|handler| {
            let handler = handler.clone();
            let status = status.clone();
            let recover = recover.clone();
            let current_revision = current_revision.clone();
            let module_id = module_id.clone();
            Rc::new(move |grid| {
                let expected_revision = *current_revision.borrow();
                dispatch_module_action(
                    &handler,
                    &status,
                    &recover,
                    &current_revision,
                    DarkroomModuleAction::ColorCorrectionGrid {
                        module_id: module_id.clone(),
                        operation_id,
                        expected_revision,
                        grid,
                    },
                )
            }) as crate::iop::colorcorrection::ColorCorrectionGridCommit
        });
        let reset_all = interaction_action_handler.as_ref().map(|handler| {
            let handler = handler.clone();
            let status = status.clone();
            let recover = recover.clone();
            let current_revision = current_revision.clone();
            let module_id = module_id.clone();
            Rc::new(move || {
                let expected_revision = *current_revision.borrow();
                dispatch_module_action(
                    &handler,
                    &status,
                    &recover,
                    &current_revision,
                    DarkroomModuleAction::ColorCorrectionResetParameters {
                        module_id: module_id.clone(),
                        operation_id,
                        expected_revision,
                    },
                )
            }) as crate::iop::colorcorrection::ColorCorrectionResetCommit
        });
        let grid = build_color_correction_grid(ColorCorrectionGridGtkContext {
            widget_id: widget_id.clone(),
            state: grid_state,
            saturation,
            initial_saturation,
            sensitive: module_available
                && (module.enabled() || color_correction_controls_enable_module),
            commit_grid,
            reset_all,
        });
        content.append(&grid);
        Some(grid)
    } else {
        None
    };
    for row in &control_rows {
        content.append(row);
    }

    let expander = shared_module_expander(
        &widget_id,
        module.title(),
        module.expanded(),
        Some(&content),
    );
    let title = if module.id() == crate::iop::velvia::VELVIA_MODULE_ID {
        crate::iop::velvia::module_title_widget()
    } else {
        module_title(&widget_id, module.title())
    };
    if let Some(deprecation_message) = deprecation_message {
        let mut child = title.first_child();
        while let Some(current) = child {
            if let Ok(label) = current.clone().downcast::<gtk4::Label>() {
                label.set_tooltip_text(Some(deprecation_message));
                break;
            }
            child = current.next_sibling();
        }
    }
    let instance_menu = build_instance_menu(&title, module, action_handler.is_some());
    expander.set_label_widget(Some(&title));
    expander.set_accessible_role(gtk4::AccessibleRole::Group);
    expander.update_property(&[Property::Label(module.title())]);
    apply_theme_role(&expander, ThemeRole::Module);

    if let Some(handler) = action_handler {
        if let (Some(instance_menu), Some(operation_id)) = (instance_menu.as_ref(), operation_id) {
            connect_instance_menu(
                instance_menu,
                InstanceActionContext {
                    action_handler: handler.clone(),
                    status: status.clone(),
                    recover: recover.clone(),
                    current_revision: current_revision.clone(),
                    module_id: module_id.clone(),
                    operation_id,
                },
            );
        }
        let status_for_expander = status.clone();
        let recover_for_expander = recover.clone();
        let current_revision_for_expander = current_revision.clone();
        let handler_for_expander = handler.clone();
        let module_id_for_expander = module_id.clone();
        expander.connect_notify_local(Some("expanded"), move |expander, _| {
            let expected_revision = *current_revision_for_expander.borrow();
            dispatch_module_action(
                &handler_for_expander,
                &status_for_expander,
                &recover_for_expander,
                &current_revision_for_expander,
                DarkroomModuleAction::Disclosure {
                    module_id: module_id_for_expander.clone(),
                    operation_id,
                    expected_revision,
                    expanded: expander.is_expanded(),
                },
            );
        });

        let handler_for_enabled = handler.clone();
        let status_for_enabled = status.clone();
        let recover_for_enabled = recover.clone();
        let reset_for_enabled = reset.clone();
        let presets_for_enabled = presets.clone();
        let current_revision_for_enabled = current_revision.clone();
        let module_id_for_enabled = module_id.clone();
        let control_rows_for_enabled = control_rows.clone();
        let grid_for_enabled = color_correction_grid_widget.clone();
        let synchronizing_enabled_for_toggle = Rc::clone(&synchronizing_enabled);
        enabled.connect_toggled(move |enabled| {
            if let Some(reset) = reset_for_enabled.as_ref() {
                reset.set_sensitive(module_resettable);
            }
            if let Some(presets) = presets_for_enabled.as_ref() {
                presets.set_sensitive(
                    module_available && (enabled.is_active() || presets_enable_module),
                );
            }
            for row in &control_rows_for_enabled {
                row.set_sensitive(
                    module_available
                        && (enabled.is_active() || color_correction_controls_enable_module),
                );
            }
            if let Some(grid) = grid_for_enabled.as_ref() {
                grid.set_sensitive(
                    module_available
                        && (enabled.is_active() || color_correction_controls_enable_module),
                );
            }
            if synchronizing_enabled_for_toggle.get() {
                return;
            }
            let expected_revision = *current_revision_for_enabled.borrow();
            dispatch_module_action(
                &handler_for_enabled,
                &status_for_enabled,
                &recover_for_enabled,
                &current_revision_for_enabled,
                DarkroomModuleAction::Enable {
                    module_id: module_id_for_enabled.clone(),
                    operation_id,
                    expected_revision,
                    enabled: enabled.is_active(),
                },
            );
        });

        if let Some(reset) = reset {
            let status_for_reset = status.clone();
            let recover_for_reset = recover.clone();
            let routed_handler_for_reset = handler.clone();
            let reset_succeeded = Rc::new(Cell::new(false));
            let reset_succeeded_for_handler = Rc::clone(&reset_succeeded);
            let handler_for_reset: DarkroomModuleActionHandler = Rc::new(move |action| {
                let result = routed_handler_for_reset(action);
                reset_succeeded_for_handler.set(result.is_ok());
                result
            });
            let current_revision_for_reset = current_revision.clone();
            let module_id_for_reset = module_id.clone();
            let enabled_for_reset = enabled.clone();
            let synchronizing_enabled_for_reset = Rc::clone(&synchronizing_enabled);
            reset.connect_clicked(move |_| {
                reset_succeeded.set(false);
                let expected_revision = *current_revision_for_reset.borrow();
                dispatch_module_action(
                    &handler_for_reset,
                    &status_for_reset,
                    &recover_for_reset,
                    &current_revision_for_reset,
                    DarkroomModuleAction::Reset {
                        module_id: module_id_for_reset.clone(),
                        operation_id,
                        expected_revision,
                    },
                );
                if reset_succeeded.get() {
                    synchronizing_enabled_for_reset.set(true);
                    enabled_for_reset.set_active(true);
                    synchronizing_enabled_for_reset.set(false);
                }
            });
        }

        if let Some(presets) = presets {
            let status_for_presets = status.clone();
            let recover_for_presets = recover.clone();
            let handler_for_presets = handler.clone();
            let current_revision_for_presets = current_revision.clone();
            let module_id_for_presets = module_id.clone();
            let preset_ids = module
                .presets()
                .map(|preset| preset.id().to_owned())
                .collect::<Vec<_>>();
            let preset_enables = module
                .presets()
                .map(DarkroomModulePreset::enables_module)
                .collect::<Vec<_>>();
            let enabled_for_presets = enabled.clone();
            let synchronizing_enabled_for_presets = Rc::clone(&synchronizing_enabled);
            presets.connect_selected_notify(move |presets| {
                let Ok(index) = usize::try_from(presets.selected()) else {
                    return;
                };
                let Some(preset_id) = preset_ids.get(index) else {
                    return;
                };
                let expected_revision = *current_revision_for_presets.borrow();
                let succeeded = dispatch_module_action(
                    &handler_for_presets,
                    &status_for_presets,
                    &recover_for_presets,
                    &current_revision_for_presets,
                    DarkroomModuleAction::Preset {
                        module_id: module_id_for_presets.clone(),
                        operation_id,
                        expected_revision,
                        preset_id: preset_id.clone(),
                    },
                );
                if succeeded && preset_enables.get(index).copied().unwrap_or(false) {
                    synchronizing_enabled_for_presets.set(true);
                    enabled_for_presets.set_active(true);
                    synchronizing_enabled_for_presets.set(false);
                }
                presets.set_selected(u32::MAX);
            });
        }

        if deprecation_message.is_none() {
            let current_revision_for_recovery = current_revision.clone();
            let handler_for_recovery = handler.clone();
            let status_for_recovery = status.clone();
            let recover_for_recovery = recover.clone();
            let module_id_for_recovery = module_id.clone();
            recover.connect_clicked(move |_| {
                let expected_revision = *current_revision_for_recovery.borrow();
                dispatch_module_action(
                    &handler_for_recovery,
                    &status_for_recovery,
                    &recover_for_recovery,
                    &current_revision_for_recovery,
                    DarkroomModuleAction::Recover {
                        module_id: module_id_for_recovery.clone(),
                        operation_id,
                        expected_revision,
                    },
                );
            });
        }
    }
    expander
}

// Substitute only the presentation prefix. The logical control ID remains the
// registry/persistence key captured by `DarkroomModuleAction::Control`.
fn presentation_control_id(module_id: &str, panel_widget_id: &str, control_id: &str) -> String {
    if panel_widget_id == module_id {
        return control_id.to_owned();
    }
    control_id
        .strip_prefix(module_id)
        .filter(|suffix| suffix.is_empty() || suffix.starts_with('-'))
        .map_or_else(
            || format!("{panel_widget_id}-{control_id}"),
            |suffix| format!("{panel_widget_id}{suffix}"),
        )
}

fn descendant_named(root: &gtk4::Widget, name: &str) -> Option<gtk4::Widget> {
    if root.widget_name() == name {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(current) = child {
        if let Some(found) = descendant_named(&current, name) {
            return Some(found);
        }
        child = current.next_sibling();
    }
    None
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
    build_module_column_with_filter_and_revision(modules, side, query, action_handler, None)
}

pub(crate) fn build_module_column_with_filter_at_revision<'a>(
    modules: impl Iterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
    query: &str,
    action_handler: Option<&DarkroomModuleActionHandler>,
    current_revision: &Rc<RefCell<Revision>>,
) -> gtk4::Box {
    build_module_column_with_filter_and_revision(
        modules,
        side,
        query,
        action_handler,
        Some(current_revision),
    )
}

fn build_module_column_with_filter_and_revision<'a>(
    modules: impl Iterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
    query: &str,
    action_handler: Option<&DarkroomModuleActionHandler>,
    current_revision: Option<&Rc<RefCell<Revision>>>,
) -> gtk4::Box {
    let column = build_module_column_without_empty_and_revision(
        modules,
        side,
        query,
        action_handler,
        current_revision,
    );
    let query = query.trim().to_ascii_lowercase();
    if column.first_child().is_none() {
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

pub(crate) fn build_module_column_without_empty_at_revision<'a>(
    modules: impl Iterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
    query: &str,
    action_handler: Option<&DarkroomModuleActionHandler>,
    current_revision: &Rc<RefCell<Revision>>,
) -> gtk4::Box {
    build_module_column_without_empty_and_revision(
        modules,
        side,
        query,
        action_handler,
        Some(current_revision),
    )
}

fn build_module_column_without_empty_and_revision<'a>(
    modules: impl Iterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
    query: &str,
    action_handler: Option<&DarkroomModuleActionHandler>,
    current_revision: Option<&Rc<RefCell<Revision>>>,
) -> gtk4::Box {
    let column = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    column.set_widget_name(side.widget_name());
    column.set_vexpand(true);
    let query = if matches!(side, DarkroomModuleSide::Left) {
        String::new()
    } else {
        query.trim().to_ascii_lowercase()
    };
    for module in modules {
        if !module_matches_query(module, &query) {
            continue;
        }
        let current_revision = current_revision
            .cloned()
            .unwrap_or_else(|| Rc::new(RefCell::new(module.revision())));
        column.append(&build_module_panel_with_action_revision(
            module,
            action_handler.cloned(),
            &current_revision,
        ));
    }
    column
}

pub(crate) fn module_matches_search(module: &DarkroomModuleViewModel, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    module_matches_query(module, &query)
}

fn module_matches_query(module: &DarkroomModuleViewModel, query: &str) -> bool {
    let aliases = module.aliases().collect::<Vec<_>>();
    search_matches(query, module.title(), module.id(), &aliases)
}

pub(crate) fn search_matches(query: &str, title: &str, id: &str, aliases: &[&str]) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    let values = std::iter::once(title)
        .chain(std::iter::once(id))
        .chain(aliases.iter().copied())
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();
    query
        .split_whitespace()
        .all(|token| values.iter().any(|value| value.contains(token)))
}
