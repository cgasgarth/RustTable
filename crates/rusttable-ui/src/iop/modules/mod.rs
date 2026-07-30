//! Darktable-style darkroom module columns and their GTK4 projection.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::{OperationId, Revision};

use crate::gui::darktable_components::{
    image_operation_module_header, module_expander as shared_module_expander,
};
use crate::iop::bloom::{BloomGtkHandlerOutcome, BloomGtkLeaf, BloomGtkState, build_bloom_gtk};
use crate::iop::colorcorrection::{
    COLORCORRECTION_MODULE_ID, ColorCorrectionGridGtkContext, ColorCorrectionGridState,
    build_grid as build_color_correction_grid,
};
use crate::iop::colorreconstruct::{
    ColorReconstructionGtkActionHandler, ColorReconstructionGtkHandlerOutcome,
    ColorReconstructionGtkLeaf, ColorReconstructionGtkState, build_colorreconstruction_gtk,
};
use crate::iop::colorzones::{
    ColorZonesGtkActionHandler, ColorZonesGtkHandlerOutcome, ColorZonesGtkLeaf,
    ColorZonesGtkPreferencesHandler, ColorZonesGtkState, build_colorzones_gtk,
};
use crate::iop::soften::{
    SoftenGtkActionHandler, SoftenGtkHandlerOutcome, SoftenGtkLeaf, SoftenGtkState,
    build_soften_gtk,
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

/// Source-specific editor payload carried by a registry module instead of
/// descriptor-generated controls.
#[derive(Debug, Clone, PartialEq)]
enum DarkroomCustomEditorPayload {
    Bloom(Option<BloomGtkState>),
    ColorReconstruction(Option<ColorReconstructionGtkState>),
    ColorZones(Option<ColorZonesGtkState>),
    Soften(Option<SoftenGtkState>),
}

impl DarkroomCustomEditorPayload {
    const fn bloom_state(&self) -> Option<&BloomGtkState> {
        match self {
            Self::Bloom(state) => state.as_ref(),
            Self::ColorReconstruction(_) | Self::ColorZones(_) | Self::Soften(_) => None,
        }
    }

    const fn colorreconstruct_state(&self) -> Option<&ColorReconstructionGtkState> {
        match self {
            Self::ColorReconstruction(state) => state.as_ref(),
            Self::Bloom(_) | Self::ColorZones(_) | Self::Soften(_) => None,
        }
    }

    const fn colorzones_state(&self) -> Option<&ColorZonesGtkState> {
        match self {
            Self::Bloom(_) | Self::ColorReconstruction(_) | Self::Soften(_) => None,
            Self::ColorZones(state) => state.as_ref(),
        }
    }

    const fn soften_state(&self) -> Option<&SoftenGtkState> {
        match self {
            Self::Soften(state) => state.as_ref(),
            Self::Bloom(_) | Self::ColorReconstruction(_) | Self::ColorZones(_) => None,
        }
    }
}

/// Mounted custom leaves are retained across processing-snapshot reconciliation.
#[derive(Clone, Default)]
pub(crate) struct DarkroomCustomEditorMounts {
    bloom: Rc<RefCell<BTreeMap<OperationId, BloomGtkLeaf>>>,
    colorreconstruct: Rc<RefCell<BTreeMap<OperationId, ColorReconstructionGtkLeaf>>>,
    colorzones: Rc<RefCell<BTreeMap<OperationId, ColorZonesGtkLeaf>>>,
    soften: Rc<RefCell<BTreeMap<OperationId, SoftenGtkLeaf>>>,
    colorzones_handler: Rc<RefCell<Option<ColorZonesGtkActionHandler>>>,
    colorzones_preferences_handler: Rc<RefCell<Option<ColorZonesGtkPreferencesHandler>>>,
}

impl DarkroomCustomEditorMounts {
    pub(crate) fn set_colorzones_handler(&self, handler: Option<ColorZonesGtkActionHandler>) {
        self.colorzones_handler.replace(handler);
    }

    pub(crate) fn set_colorzones_preferences_handler(
        &self,
        handler: Option<ColorZonesGtkPreferencesHandler>,
    ) {
        self.colorzones_preferences_handler.replace(handler);
        let handler = self.colorzones_preferences_handler.borrow().clone();
        for leaf in self.colorzones.borrow().values() {
            leaf.set_preferences_handler(handler.clone());
        }
    }

    pub(crate) fn reconcile(&self, modules: &DarkroomModulesViewModel) {
        for module in modules.left_modules().chain(modules.right_modules()) {
            if let Some(state) = module.bloom_editor_state()
                && let Some(leaf) = self.bloom.borrow().get(&state.operation_id())
            {
                leaf.reconcile(*state);
            }
            if let Some(state) = module.colorreconstruct_editor_state()
                && let Some(leaf) = self.colorreconstruct.borrow().get(&state.operation_id())
            {
                leaf.reconcile(*state);
            }
            if let Some(state) = module.colorzones_editor_state()
                && let Some(leaf) = self.colorzones.borrow().get(&state.operation_id())
            {
                let output_channel = leaf.state().editor().output_channel();
                leaf.reconcile(state.clone().with_output_channel(output_channel));
            }
            if let Some(state) = module.soften_editor_state()
                && let Some(leaf) = self.soften.borrow().get(&state.operation_id())
            {
                leaf.reconcile(*state);
            }
        }
    }

    fn bloom_leaf(
        &self,
        widget_id: &str,
        state: BloomGtkState,
        action_handler: Option<&DarkroomModuleActionHandler>,
    ) -> BloomGtkLeaf {
        if let Some(leaf) = self.bloom.borrow().get(&state.operation_id()).cloned() {
            leaf.reconcile(state);
            return leaf;
        }
        let handler = action_handler.cloned().map(|handler| {
            Rc::new(move |settled: crate::iop::bloom::BloomSettledAction| {
                let action = DarkroomModuleAction::BloomSettled {
                    module_id: crate::iop::bloom::BLOOM_MODULE_ID.to_owned(),
                    operation_id: (!settled.materialization_required()).then_some(settled.target()),
                    expected_revision: settled.expected_revision(),
                    parameters: settled.parameters(),
                    enable_required: settled.enable_required(),
                };
                match handler(action) {
                    Ok(revision) => BloomGtkHandlerOutcome::Commit { revision },
                    Err(_) => BloomGtkHandlerOutcome::Rollback,
                }
            }) as crate::iop::bloom::BloomGtkActionHandler
        });
        let leaf = build_bloom_gtk(widget_id, state, handler);
        self.bloom
            .borrow_mut()
            .insert(state.operation_id(), leaf.clone());
        leaf
    }

    fn colorreconstruct_leaf(
        &self,
        widget_id: &str,
        state: ColorReconstructionGtkState,
        action_handler: Option<&DarkroomModuleActionHandler>,
    ) -> ColorReconstructionGtkLeaf {
        if let Some(leaf) = self
            .colorreconstruct
            .borrow()
            .get(&state.operation_id())
            .cloned()
        {
            leaf.reconcile(state);
            return leaf;
        }
        let handler = action_handler.cloned().map(|handler| {
            Rc::new(
                move |settled: crate::iop::colorreconstruct::ColorReconstructionSettledAction| {
                    let action = DarkroomModuleAction::ColorReconstructionSettled {
                        module_id: crate::iop::colorreconstruct::COLORRECONSTRUCTION_MODULE_ID
                            .to_owned(),
                        operation_id: (!settled.materialization_required())
                            .then_some(settled.target()),
                        expected_revision: settled.expected_revision(),
                        parameters: settled.parameters(),
                        enable_required: settled.enable_required(),
                    };
                    match handler(action) {
                        Ok(revision) => ColorReconstructionGtkHandlerOutcome::Commit { revision },
                        Err(_) => ColorReconstructionGtkHandlerOutcome::Rollback,
                    }
                },
            ) as ColorReconstructionGtkActionHandler
        });
        let leaf = build_colorreconstruction_gtk(widget_id, state, handler);
        self.colorreconstruct
            .borrow_mut()
            .insert(state.operation_id(), leaf.clone());
        leaf
    }

    fn colorzones_leaf(&self, widget_id: &str, state: &ColorZonesGtkState) -> ColorZonesGtkLeaf {
        if let Some(leaf) = self.colorzones.borrow().get(&state.operation_id()).cloned() {
            let output_channel = leaf.state().editor().output_channel();
            leaf.reconcile(state.clone().with_output_channel(output_channel));
            return leaf;
        }
        let handler_slot = Rc::clone(&self.colorzones_handler);
        let handler: ColorZonesGtkActionHandler = Rc::new(move |action| {
            let handler = handler_slot.borrow().clone();
            handler.map_or(ColorZonesGtkHandlerOutcome::Rollback, |handler| {
                handler(action)
            })
        });
        let leaf = build_colorzones_gtk(widget_id, state.clone(), Some(handler));
        leaf.set_preferences_handler(self.colorzones_preferences_handler.borrow().clone());
        self.colorzones
            .borrow_mut()
            .insert(state.operation_id(), leaf.clone());
        leaf
    }

    fn soften_leaf(
        &self,
        widget_id: &str,
        state: SoftenGtkState,
        action_handler: Option<&DarkroomModuleActionHandler>,
    ) -> SoftenGtkLeaf {
        if let Some(leaf) = self.soften.borrow().get(&state.operation_id()).cloned() {
            leaf.reconcile(state);
            return leaf;
        }
        let handler = action_handler.cloned().map(|handler| {
            Rc::new(move |settled: crate::iop::soften::SoftenSettledAction| {
                let action = DarkroomModuleAction::Control {
                    module_id: crate::iop::soften::SOFTEN_MODULE_ID.to_owned(),
                    operation_id: (!settled.materialization_required()).then_some(settled.target()),
                    expected_revision: settled.expected_revision(),
                    id: format!(
                        "{}-{}",
                        crate::iop::soften::SOFTEN_MODULE_ID,
                        settled.parameter().id()
                    ),
                    value: DarkroomControlValue::Slider(f64::from(match settled.parameter() {
                        crate::iop::soften::SoftenParameter::Size => settled.parameters().size,
                        crate::iop::soften::SoftenParameter::Saturation => {
                            settled.parameters().saturation
                        }
                        crate::iop::soften::SoftenParameter::Brightness => {
                            settled.parameters().brightness
                        }
                        crate::iop::soften::SoftenParameter::Amount => settled.parameters().amount,
                    })),
                };
                match handler(action) {
                    Ok(revision) => SoftenGtkHandlerOutcome::Commit { revision },
                    Err(_) => SoftenGtkHandlerOutcome::Rollback,
                }
            }) as SoftenGtkActionHandler
        });
        let leaf = build_soften_gtk(widget_id, state, handler);
        self.soften
            .borrow_mut()
            .insert(state.operation_id(), leaf.clone());
        leaf
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
    description: Option<String>,
    side: DarkroomModuleSide,
    expanded: bool,
    enabled: bool,
    resettable: bool,
    revision: Revision,
    controls: DarkroomControlsViewModel,
    color_correction_grid: Option<ColorCorrectionGridState>,
    custom_editor: Option<DarkroomCustomEditorPayload>,
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
            description: None,
            side,
            expanded,
            enabled,
            resettable,
            revision,
            controls,
            color_correction_grid: None,
            custom_editor: None,
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
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Marks this module as a source-specific Bloom editor and removes every
    /// descriptor-generated control.
    ///
    /// # Panics
    ///
    /// Panics only if constructing an empty control snapshot is rejected.
    #[must_use]
    pub fn with_bloom_custom_editor(mut self) -> Self {
        debug_assert_eq!(self.id, crate::iop::bloom::BLOOM_MODULE_ID);
        self.controls
            .replace_snapshot(self.revision, Vec::new())
            .expect("Bloom custom projection retains zero generic controls");
        self.custom_editor = Some(DarkroomCustomEditorPayload::Bloom(None));
        self
    }

    /// Installs the exact Bloom operation projection consumed by its
    /// source-specific GTK leaf.
    ///
    /// # Panics
    ///
    /// Panics only if constructing an empty control snapshot is rejected.
    #[must_use]
    pub fn with_bloom_editor_state(mut self, state: BloomGtkState) -> Self {
        debug_assert_eq!(self.id, crate::iop::bloom::BLOOM_MODULE_ID);
        self.operation_id = (!state.materialization_required()).then_some(state.operation_id());
        self.revision = state.revision();
        self.enabled = state.enabled();
        self.controls
            .replace_snapshot(state.revision(), Vec::new())
            .expect("Bloom custom projection retains zero generic controls");
        self.custom_editor = Some(DarkroomCustomEditorPayload::Bloom(Some(state)));
        self
    }

    #[must_use]
    pub fn has_bloom_custom_editor(&self) -> bool {
        matches!(
            &self.custom_editor,
            Some(DarkroomCustomEditorPayload::Bloom(_))
        )
    }

    #[must_use]
    pub fn bloom_editor_state(&self) -> Option<&BloomGtkState> {
        self.custom_editor
            .as_ref()
            .and_then(DarkroomCustomEditorPayload::bloom_state)
    }

    /// Marks this module as a source-specific Soften editor and removes every
    /// descriptor-generated control.
    ///
    /// # Panics
    ///
    /// Panics only if replacing the generic controls with an empty snapshot is
    /// rejected by the shared control model.
    #[must_use]
    pub fn with_soften_custom_editor(mut self) -> Self {
        debug_assert_eq!(self.id, crate::iop::soften::SOFTEN_MODULE_ID);
        self.controls
            .replace_snapshot(self.revision, Vec::new())
            .expect("Soften custom projection retains zero generic controls");
        self.custom_editor = Some(DarkroomCustomEditorPayload::Soften(None));
        self
    }

    /// Installs the exact Soften operation projection consumed by its
    /// source-specific GTK leaf.
    ///
    /// # Panics
    ///
    /// Panics only if replacing the generic controls with an empty snapshot is
    /// rejected by the shared control model.
    #[must_use]
    pub fn with_soften_editor_state(mut self, state: SoftenGtkState) -> Self {
        debug_assert_eq!(self.id, crate::iop::soften::SOFTEN_MODULE_ID);
        self.operation_id = (!state.materialization_required()).then_some(state.operation_id());
        self.revision = state.revision();
        self.enabled = state.enabled();
        self.controls
            .replace_snapshot(state.revision(), Vec::new())
            .expect("Soften custom projection retains zero generic controls");
        self.custom_editor = Some(DarkroomCustomEditorPayload::Soften(Some(state)));
        self
    }

    #[must_use]
    pub fn has_soften_custom_editor(&self) -> bool {
        matches!(
            &self.custom_editor,
            Some(DarkroomCustomEditorPayload::Soften(_))
        )
    }

    #[must_use]
    pub fn soften_editor_state(&self) -> Option<&SoftenGtkState> {
        self.custom_editor
            .as_ref()
            .and_then(DarkroomCustomEditorPayload::soften_state)
    }

    /// Marks this module as a source-specific Color Reconstruction editor and removes
    /// every descriptor-generated control.
    ///
    /// # Panics
    ///
    /// Panics if the custom projection cannot replace its zero generic controls.
    #[must_use]
    pub fn with_colorreconstruct_custom_editor(mut self) -> Self {
        debug_assert_eq!(
            self.id,
            crate::iop::colorreconstruct::COLORRECONSTRUCTION_MODULE_ID
        );
        self.controls
            .replace_snapshot(self.revision, Vec::new())
            .expect("Color Reconstruction custom projection retains zero generic controls");
        self.custom_editor = Some(DarkroomCustomEditorPayload::ColorReconstruction(None));
        self
    }

    ///
    /// # Panics
    ///
    /// Panics if the custom projection cannot replace its zero generic controls.
    #[must_use]
    pub fn with_colorreconstruct_editor_state(
        mut self,
        state: ColorReconstructionGtkState,
    ) -> Self {
        debug_assert_eq!(
            self.id,
            crate::iop::colorreconstruct::COLORRECONSTRUCTION_MODULE_ID
        );
        self.operation_id = (!state.materialization_required()).then_some(state.operation_id());
        self.revision = state.revision();
        self.enabled = state.enabled();
        self.controls
            .replace_snapshot(state.revision(), Vec::new())
            .expect("Color Reconstruction custom projection retains zero generic controls");
        self.custom_editor = Some(DarkroomCustomEditorPayload::ColorReconstruction(Some(
            state,
        )));
        self
    }

    #[must_use]
    pub fn has_colorreconstruct_custom_editor(&self) -> bool {
        matches!(
            &self.custom_editor,
            Some(DarkroomCustomEditorPayload::ColorReconstruction(_))
        )
    }

    #[must_use]
    pub fn colorreconstruct_editor_state(&self) -> Option<&ColorReconstructionGtkState> {
        self.custom_editor
            .as_ref()
            .and_then(DarkroomCustomEditorPayload::colorreconstruct_state)
    }

    /// Marks this module as a source-specific Color Zones editor and removes
    /// every descriptor-generated control.
    ///
    /// # Panics
    ///
    /// Panics only if constructing an empty control snapshot is rejected.
    #[must_use]
    pub fn with_colorzones_custom_editor(mut self) -> Self {
        debug_assert_eq!(self.id, crate::iop::colorzones::COLORZONES_MODULE_ID);
        self.controls
            .replace_snapshot(self.revision, Vec::new())
            .expect("Color Zones custom projection retains zero generic controls");
        self.custom_editor = Some(DarkroomCustomEditorPayload::ColorZones(None));
        self
    }

    /// Installs the exact Color Zones operation projection consumed by its
    /// source-specific GTK leaf.
    ///
    /// # Panics
    ///
    /// Panics only if constructing an empty control snapshot is rejected.
    #[must_use]
    pub fn with_colorzones_editor_state(mut self, state: ColorZonesGtkState) -> Self {
        debug_assert_eq!(self.id, crate::iop::colorzones::COLORZONES_MODULE_ID);
        self.operation_id = (!state.materialization_required()).then_some(state.operation_id());
        self.revision = state.revision();
        self.enabled = state.enabled();
        self.controls
            .replace_snapshot(state.revision(), Vec::new())
            .expect("Color Zones custom projection retains zero generic controls");
        self.custom_editor = Some(DarkroomCustomEditorPayload::ColorZones(Some(state)));
        self
    }

    #[must_use]
    pub fn has_colorzones_custom_editor(&self) -> bool {
        matches!(
            &self.custom_editor,
            Some(DarkroomCustomEditorPayload::ColorZones(_))
        )
    }

    #[must_use]
    pub fn colorzones_editor_state(&self) -> Option<&ColorZonesGtkState> {
        self.custom_editor
            .as_ref()
            .and_then(DarkroomCustomEditorPayload::colorzones_state)
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
        let mut order = vec![
            format!("{widget_id}-disclosure"),
            format!("{widget_id}-enabled"),
        ];
        if self.can_add_instance() {
            order.push(format!("{widget_id}-actions"));
        }
        if self.resettable {
            order.push(format!("{widget_id}-reset"));
        }
        if self.color_correction_grid.is_some() {
            order.push(format!("{widget_id}-grid"));
        }
        if self.bloom_editor_state().is_some() {
            order.extend(
                crate::iop::bloom::BLOOM_SLIDERS.map(|slider| slider.widget_name(&widget_id)),
            );
        }
        if self.soften_editor_state().is_some() {
            order.extend(
                crate::iop::soften::SOFTEN_SLIDERS.map(|slider| slider.widget_name(&widget_id)),
            );
        }
        if self.colorreconstruct_editor_state().is_some() {
            order.extend(
                crate::iop::colorreconstruct::COLORRECONSTRUCTION_CONTROLS.map(|control| {
                    match control {
                        crate::iop::colorreconstruct::ColorReconstructionControl::Threshold => {
                            crate::iop::colorreconstruct::COLORRECONSTRUCTION_THRESHOLD_SLIDER
                                .widget_name(&widget_id)
                        }
                        crate::iop::colorreconstruct::ColorReconstructionControl::Spatial => {
                            crate::iop::colorreconstruct::COLORRECONSTRUCTION_SPATIAL_SLIDER
                                .widget_name(&widget_id)
                        }
                        crate::iop::colorreconstruct::ColorReconstructionControl::Range => {
                            crate::iop::colorreconstruct::COLORRECONSTRUCTION_RANGE_SLIDER
                                .widget_name(&widget_id)
                        }
                        crate::iop::colorreconstruct::ColorReconstructionControl::Precedence => {
                            format!("{widget_id}-precedence")
                        }
                        crate::iop::colorreconstruct::ColorReconstructionControl::Hue => {
                            crate::iop::colorreconstruct::COLORRECONSTRUCTION_HUE_SLIDER
                                .widget_name(&widget_id)
                        }
                    }
                }),
            );
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
                    | DarkroomModuleAction::BloomSettled { .. }
                    | DarkroomModuleAction::ColorReconstructionSettled { .. }
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
            DarkroomModuleAction::BloomSettled {
                expected_revision,
                parameters,
                enable_required,
                ..
            } => self.set_bloom_parameters(expected_revision, parameters, enable_required),
            DarkroomModuleAction::ColorReconstructionSettled {
                expected_revision,
                parameters,
                enable_required,
                ..
            } => {
                self.set_colorreconstruct_parameters(expected_revision, parameters, enable_required)
            }
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
            DarkroomModuleAvailability::PartiallySupported { reason, .. } => {
                return format!("Partial · {reason}");
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
        if self.id == crate::iop::soften::SOFTEN_MODULE_ID && self.has_soften_custom_editor() {
            return self.set_soften_control(expected_revision, id, &value);
        }
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

    fn set_soften_control(
        &mut self,
        expected_revision: Revision,
        id: &str,
        value: &DarkroomControlValue,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        let parameter = match id {
            "soften-size" => crate::iop::soften::SoftenParameter::Size,
            "soften-saturation" => crate::iop::soften::SoftenParameter::Saturation,
            "soften-brightness" => crate::iop::soften::SoftenParameter::Brightness,
            "soften-amount" => crate::iop::soften::SoftenParameter::Amount,
            _ => {
                return Err(self.record_error(DarkroomModuleError::Unsupported {
                    module_id: self.id.clone(),
                    reason: format!("unknown Soften control {id}"),
                }));
            }
        };
        let DarkroomControlValue::Slider(value) = value else {
            return Err(self.record_error(DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: format!("Soften control {id} requires a slider value"),
            }));
        };
        let current = self.soften_editor_state().copied().ok_or_else(|| {
            self.record_error(DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: "module has no Soften custom editor".to_owned(),
            })
        })?;
        #[allow(
            clippy::cast_possible_truncation,
            reason = "the descriptor-backed slider is bounded to the native f32 range"
        )]
        let value = *value as f32;
        let mut editor = current.editor();
        editor.set(parameter, value).map_err(|error| {
            self.record_error(DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: error.to_string(),
            })
        })?;
        let revision = self.advance_revision()?;
        self.enabled = true;
        self.custom_editor = Some(DarkroomCustomEditorPayload::Soften(Some(
            SoftenGtkState::new(
                current.operation_id(),
                revision,
                editor,
                true,
                current.sensitive(),
                current.materialization_required(),
            ),
        )));
        Ok(revision)
    }

    /// Applies all three source Bloom parameters in one settled revision.
    ///
    /// # Errors
    ///
    /// Returns an error for stale callbacks, invalid parameters, or non-Bloom modules.
    pub fn set_bloom_parameters(
        &mut self,
        expected_revision: Revision,
        parameters: rusttable_processing::operations::bloom::BloomParametersV1,
        enable_required: bool,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        let Some(current) = self.bloom_editor_state().copied() else {
            return Err(self.record_error(DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: "module has no Bloom custom editor".to_owned(),
            }));
        };
        let editor = crate::iop::bloom::BloomEditorState::new(parameters).map_err(|error| {
            self.record_error(DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: error.to_string(),
            })
        })?;
        let revision = self.advance_revision()?;
        self.enabled = current.enabled() || enable_required;
        self.custom_editor = Some(DarkroomCustomEditorPayload::Bloom(Some(
            BloomGtkState::new(
                current.operation_id(),
                revision,
                editor,
                self.enabled,
                current.sensitive(),
                current.materialization_required(),
            ),
        )));
        Ok(revision)
    }

    /// Applies all native Color Reconstruction parameters in one settled revision.
    ///
    /// # Errors
    pub fn set_colorreconstruct_parameters(
        &mut self,
        expected_revision: Revision,
        parameters: rusttable_processing::operations::colorreconstruction::ColorReconstructionV3,
        enable_required: bool,
    ) -> Result<Revision, DarkroomModuleError> {
        self.check_revision(expected_revision)?;
        let Some(current) = self.colorreconstruct_editor_state().copied() else {
            return Err(self.record_error(DarkroomModuleError::Unsupported {
                module_id: self.id.clone(),
                reason: "module has no Color Reconstruction custom editor".to_owned(),
            }));
        };
        let editor = crate::iop::colorreconstruct::ColorReconstructionEditorState::new(parameters)
            .map_err(|error| {
                self.record_error(DarkroomModuleError::Unsupported {
                    module_id: self.id.clone(),
                    reason: error.to_string(),
                })
            })?;
        let revision = self.advance_revision()?;
        self.enabled = current.enabled() || enable_required;
        self.custom_editor = Some(DarkroomCustomEditorPayload::ColorReconstruction(Some(
            ColorReconstructionGtkState::new(
                current.operation_id(),
                revision,
                editor,
                self.enabled,
                current.sensitive(),
                current.materialization_required(),
            ),
        )));
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
        if let Some(current) = self.bloom_editor_state().copied() {
            self.custom_editor = Some(DarkroomCustomEditorPayload::Bloom(Some(
                BloomGtkState::new(
                    current.operation_id(),
                    revision,
                    crate::iop::bloom::BloomEditorState::default(),
                    true,
                    current.sensitive(),
                    current.materialization_required(),
                ),
            )));
        }
        if let Some(current) = self.colorreconstruct_editor_state().copied() {
            self.custom_editor = Some(DarkroomCustomEditorPayload::ColorReconstruction(Some(
                ColorReconstructionGtkState::new(
                    current.operation_id(),
                    revision,
                    crate::iop::colorreconstruct::ColorReconstructionEditorState::default(),
                    true,
                    current.sensitive(),
                    current.materialization_required(),
                ),
            )));
        }
        if let Some(current) = self.soften_editor_state().copied() {
            self.custom_editor = Some(DarkroomCustomEditorPayload::Soften(Some(
                SoftenGtkState::new(
                    current.operation_id(),
                    revision,
                    crate::iop::soften::SoftenEditorState::default(),
                    true,
                    current.sensitive(),
                    current.materialization_required(),
                ),
            )));
        }
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
    build_module_panel_with_action_revision(module, action_handler, &current_revision, None)
}

#[must_use]
#[allow(clippy::too_many_lines)]
fn build_module_panel_with_action_revision(
    module: &DarkroomModuleViewModel,
    action_handler: Option<DarkroomModuleActionHandler>,
    current_revision: &Rc<RefCell<Revision>>,
    custom_mounts: Option<&DarkroomCustomEditorMounts>,
) -> gtk4::Expander {
    let module_id = module.id().to_owned();
    let operation_id = module.operation_id();
    let widget_id = module.widget_id();
    let module_available = module.availability().is_supported();
    let module_resettable = module_available && module.resettable();
    let deprecation_message = match module.availability() {
        DarkroomModuleAvailability::Deprecated { reason } => Some(reason.as_str()),
        DarkroomModuleAvailability::Supported
        | DarkroomModuleAvailability::PartiallySupported { .. }
        | DarkroomModuleAvailability::DeprecatedUnavailable { .. }
        | DarkroomModuleAvailability::Unsupported { .. } => None,
    };

    let module_header = image_operation_module_header(
        &widget_id,
        module.id(),
        module.title(),
        module.supports_multi_instance(),
        module.resettable(),
    );
    let title = module_header.widget;
    let enabled = module_header.enabled;
    let icon_slot = module_header.icon_slot;
    let reset = module_header.reset;
    enabled.set_active(module.enabled());
    enabled.set_sensitive(module_available);
    if let Some(reset) = reset.as_ref() {
        reset.set_sensitive(module_resettable);
    }
    if module.id() == crate::iop::velvia::VELVIA_MODULE_ID {
        let velvia_title = crate::iop::velvia::module_title_widget();
        if let Some(icon) = velvia_title.first_child() {
            icon.unparent();
            icon_slot.append(&icon);
        }
    }
    if let Some(description) = module.description() {
        title.set_tooltip_text(Some(description));
    }

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.set_widget_name(&format!("{widget_id}-content"));
    content.add_css_class("dt_plugin_ui");
    apply_theme_role(&content, ThemeRole::Module);
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
    }
    let operation_root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    operation_root.set_widget_name(&format!("{widget_id}-operation-root"));
    operation_root.add_css_class("dt_plugin_ui_main");

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
            let current_revision = current_revision.clone();
            let module_id = module_id.clone();
            Rc::new(move |grid| {
                let expected_revision = *current_revision.borrow();
                dispatch_module_action(
                    &handler,
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
            let current_revision = current_revision.clone();
            let module_id = module_id.clone();
            Rc::new(move || {
                let expected_revision = *current_revision.borrow();
                dispatch_module_action(
                    &handler,
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
        operation_root.append(&grid);
        Some(grid)
    } else {
        None
    };
    if let (Some(custom_mounts), Some(state)) = (custom_mounts, module.bloom_editor_state()) {
        let leaf =
            custom_mounts.bloom_leaf(&widget_id, *state, interaction_action_handler.as_ref());
        if leaf.widget().parent().is_some() {
            leaf.widget().unparent();
        }
        operation_root.append(leaf.widget());
    }
    if let (Some(custom_mounts), Some(state)) =
        (custom_mounts, module.colorreconstruct_editor_state())
    {
        let leaf = custom_mounts.colorreconstruct_leaf(
            &widget_id,
            *state,
            interaction_action_handler.as_ref(),
        );
        if leaf.widget().parent().is_some() {
            leaf.widget().unparent();
        }
        operation_root.append(leaf.widget());
    }
    if let (Some(custom_mounts), Some(state)) = (custom_mounts, module.colorzones_editor_state()) {
        let leaf = custom_mounts.colorzones_leaf(&widget_id, state);
        if leaf.widget().parent().is_some() {
            leaf.widget().unparent();
        }
        operation_root.append(leaf.widget());
    }
    if let (Some(custom_mounts), Some(state)) = (custom_mounts, module.soften_editor_state()) {
        let leaf =
            custom_mounts.soften_leaf(&widget_id, *state, interaction_action_handler.as_ref());
        if leaf.widget().parent().is_some() {
            leaf.widget().unparent();
        }
        operation_root.append(leaf.widget());
    }
    for row in &control_rows {
        operation_root.append(row);
    }
    content.append(&operation_root);

    let expander = shared_module_expander(
        &widget_id,
        module.title(),
        module.expanded(),
        Some(&content),
    );
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
                    current_revision: current_revision.clone(),
                    module_id: module_id.clone(),
                    operation_id,
                },
            );
        }
        let current_revision_for_expander = current_revision.clone();
        let handler_for_expander = handler.clone();
        let module_id_for_expander = module_id.clone();
        expander.connect_notify_local(Some("expanded"), move |expander, _| {
            let expected_revision = *current_revision_for_expander.borrow();
            dispatch_module_action(
                &handler_for_expander,
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
        let reset_for_enabled = reset.clone();
        let current_revision_for_enabled = current_revision.clone();
        let module_id_for_enabled = module_id.clone();
        let control_rows_for_enabled = control_rows.clone();
        let grid_for_enabled = color_correction_grid_widget.clone();
        let synchronizing_enabled_for_toggle = Rc::clone(&synchronizing_enabled);
        enabled.connect_toggled(move |enabled| {
            if let Some(reset) = reset_for_enabled.as_ref() {
                reset.set_sensitive(module_resettable);
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
    build_module_column_with_filter_and_revision(modules, side, query, action_handler, None, None)
}

pub(crate) fn build_module_column_with_filter_at_revision<'a>(
    modules: impl Iterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
    query: &str,
    action_handler: Option<&DarkroomModuleActionHandler>,
    current_revision: &Rc<RefCell<Revision>>,
    custom_mounts: &DarkroomCustomEditorMounts,
) -> gtk4::Box {
    build_module_column_with_filter_and_revision(
        modules,
        side,
        query,
        action_handler,
        Some(current_revision),
        Some(custom_mounts),
    )
}

fn build_module_column_with_filter_and_revision<'a>(
    modules: impl Iterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
    query: &str,
    action_handler: Option<&DarkroomModuleActionHandler>,
    current_revision: Option<&Rc<RefCell<Revision>>>,
    custom_mounts: Option<&DarkroomCustomEditorMounts>,
) -> gtk4::Box {
    let column = build_module_column_without_empty_and_revision(
        modules,
        side,
        query,
        action_handler,
        current_revision,
        custom_mounts,
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
    custom_mounts: &DarkroomCustomEditorMounts,
) -> gtk4::Box {
    build_module_column_without_empty_and_revision(
        modules,
        side,
        query,
        action_handler,
        Some(current_revision),
        Some(custom_mounts),
    )
}

fn build_module_column_without_empty_and_revision<'a>(
    modules: impl Iterator<Item = &'a DarkroomModuleViewModel>,
    side: DarkroomModuleSide,
    query: &str,
    action_handler: Option<&DarkroomModuleActionHandler>,
    current_revision: Option<&Rc<RefCell<Revision>>>,
    custom_mounts: Option<&DarkroomCustomEditorMounts>,
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
            custom_mounts,
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
