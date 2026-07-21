use super::types::{
    ActionContext, ActionDefinition, ActionId, ApplyResult, Binding, BindingSource, ConflictKind,
    Curve, DeviceDescriptor, EditorError, EditorMessage, EditorStatus, EditorView, KeyChord,
    LEARN_TIMEOUT_TICKS, LearnTarget, MAX_SEQUENCE_LENGTH, MappingConflict, MappingProfile,
    MappingSnapshot, ResetScope,
};
use std::collections::BTreeSet;

/// Display-independent state machine backing the GTK editor.
#[derive(Debug, Clone)]
pub struct EditorState {
    defaults: MappingSnapshot,
    baseline: MappingSnapshot,
    draft: MappingSnapshot,
    pub view: EditorView,
    pub search: String,
    pub selected_action: Option<ActionId>,
    pub selected_device: Option<String>,
    pub learn: Option<LearnTarget>,
    pub learn_idle_ticks: u8,
    pub test_preview: Option<String>,
    pub status: EditorStatus,
    pub inactive_imports: Vec<Binding>,
}

impl EditorState {
    /// Creates an editor over one immutable live snapshot.
    #[must_use]
    pub fn new(snapshot: MappingSnapshot) -> Self {
        let selected_action = snapshot.actions.first().map(|action| action.id.clone());
        let mut defaults = snapshot.clone();
        defaults.bindings.retain(|binding| binding.built_in);
        Self {
            defaults,
            baseline: snapshot.clone(),
            draft: snapshot,
            view: EditorView::Actions,
            search: String::new(),
            selected_action,
            selected_device: None,
            learn: None,
            learn_idle_ticks: 0,
            test_preview: None,
            status: EditorStatus::Clean,
            inactive_imports: Vec::new(),
        }
    }

    /// Returns the current draft snapshot for service commit or preview.
    #[must_use]
    pub fn snapshot(&self) -> &MappingSnapshot {
        &self.draft
    }

    /// Returns actions matching localized text, stable ID, and category.
    #[must_use]
    pub fn visible_actions(&self) -> Vec<&ActionDefinition> {
        let needle = self.search.trim().to_lowercase();
        self.draft
            .actions
            .iter()
            .filter(|action| {
                needle.is_empty()
                    || action.label.to_lowercase().contains(&needle)
                    || action.category.to_lowercase().contains(&needle)
                    || action.id.as_str().to_lowercase().contains(&needle)
            })
            .collect()
    }

    /// Returns devices matching the same privacy-safe search.
    #[must_use]
    pub fn visible_devices(&self) -> Vec<&DeviceDescriptor> {
        let needle = self.search.trim().to_lowercase();
        self.draft
            .devices
            .iter()
            .filter(|device| {
                needle.is_empty()
                    || device.label.to_lowercase().contains(&needle)
                    || device.alias.to_lowercase().contains(&needle)
                    || device.kind.label().to_lowercase().contains(&needle)
            })
            .collect()
    }

    /// Returns the bindings currently attached to an action.
    #[must_use]
    pub fn bindings_for(&self, action_id: &ActionId) -> Vec<&Binding> {
        self.draft
            .bindings
            .iter()
            .filter(|binding| &binding.action_id == action_id)
            .collect()
    }

    /// Computes exact and shadow conflicts with context precedence.
    #[must_use]
    pub fn conflicts(&self) -> Vec<MappingConflict> {
        let mut conflicts = Vec::new();
        for (index, left) in self.draft.bindings.iter().enumerate() {
            if !left.enabled {
                continue;
            }
            for right in self.draft.bindings.iter().skip(index + 1) {
                if !right.enabled
                    || left.device_alias != right.device_alias
                    || left.source.identity() != right.source.identity()
                {
                    continue;
                }
                if left.context == right.context {
                    conflicts.push(MappingConflict {
                        left_binding_id: left.id.clone(),
                        right_binding_id: right.id.clone(),
                        kind: ConflictKind::Exact,
                        explanation: format!(
                            "{} and {} claim the same input in {}",
                            left.action_id,
                            right.action_id,
                            left.context.label()
                        ),
                    });
                } else {
                    let winner = if left.context.priority() >= right.context.priority() {
                        left.context
                    } else {
                        right.context
                    };
                    conflicts.push(MappingConflict {
                        left_binding_id: left.id.clone(),
                        right_binding_id: right.id.clone(),
                        kind: ConflictKind::Shadowed { winner },
                        explanation: format!(
                            "{} wins over the lower-priority context for this input",
                            winner.label()
                        ),
                    });
                }
            }
        }
        conflicts
    }

    /// Applies one typed editor operation and updates only presentation state.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, learn-mode, conflict, or generation error;
    /// failed apply operations leave the baseline generation untouched.
    pub fn update(&mut self, message: EditorMessage) -> Result<Option<ApplyResult>, EditorError> {
        match message {
            EditorMessage::SetView(view) => self.view = view,
            EditorMessage::SetSearch(search) => self.search = search,
            EditorMessage::SelectAction(action_id) => self.selected_action = Some(action_id),
            EditorMessage::SelectDevice(alias) => self.selected_device = Some(alias),
            EditorMessage::BeginLearn(target) => {
                self.learn = Some(target);
                self.learn_idle_ticks = 0;
                self.status = EditorStatus::Learning(target);
            }
            EditorMessage::CaptureKeyboard(chord) => self.capture_keyboard(chord)?,
            EditorMessage::LearnTick => self.tick_learn(),
            EditorMessage::CancelLearn => {
                self.learn = None;
                self.learn_idle_ticks = 0;
                self.status = EditorStatus::Dirty;
            }
            EditorMessage::TestBinding(binding_id) => {
                if self
                    .draft
                    .bindings
                    .iter()
                    .any(|binding| binding.id == binding_id)
                {
                    self.test_preview =
                        Some(format!("Previewing {binding_id}; no action will execute"));
                    self.status = EditorStatus::Testing;
                }
            }
            EditorMessage::StopTest => {
                self.test_preview = None;
                self.status = EditorStatus::Dirty;
            }
            EditorMessage::RemoveBinding(binding_id) => self.remove_binding(&binding_id)?,
            EditorMessage::ToggleBinding {
                binding_id,
                enabled,
            } => {
                if let Some(binding) = self
                    .draft
                    .bindings
                    .iter_mut()
                    .find(|binding| binding.id == binding_id)
                {
                    binding.enabled = enabled;
                    self.status = EditorStatus::Dirty;
                }
            }
            EditorMessage::Reset(scope) => self.reset(scope),
            EditorMessage::Apply { live_generation } => {
                return self.apply(live_generation).map(Some);
            }
            EditorMessage::Revert => {
                self.draft = self.baseline.clone();
                self.inactive_imports.clear();
                self.learn = None;
                self.status = EditorStatus::Clean;
            }
        }
        Ok(None)
    }

    /// Adds a keyboard sequence from a recorder or a deterministic test.
    ///
    /// # Errors
    ///
    /// Returns an error when the action is unknown or the sequence exceeds the
    /// recorder limit.
    pub fn add_keyboard_sequence(
        &mut self,
        action_id: &ActionId,
        sequence: Vec<KeyChord>,
    ) -> Result<(), EditorError> {
        if !self
            .draft
            .actions
            .iter()
            .any(|action| &action.id == action_id)
        {
            return Err(EditorError::NoSelectedAction);
        }
        if sequence.is_empty() || sequence.len() > MAX_SEQUENCE_LENGTH {
            return Err(EditorError::SequenceTooLong);
        }
        let id = format!("user-{}-{}", action_id.as_str(), self.draft.bindings.len());
        self.draft.bindings.push(Binding::user(
            id,
            action_id.clone(),
            "keyboard",
            ActionContext::Global,
            BindingSource::Keyboard { sequence },
        ));
        self.status = EditorStatus::Dirty;
        Ok(())
    }

    /// Imports a profile, preserving unknown records as inactive recovery data.
    pub fn import_profile(&mut self, profile: MappingProfile) {
        let known_actions: BTreeSet<_> =
            self.draft.actions.iter().map(|action| &action.id).collect();
        let known_devices: BTreeSet<_> = self
            .draft
            .devices
            .iter()
            .map(|device| &device.alias)
            .collect();
        let mut changed = 0;
        let mut unknown = 0;
        self.inactive_imports.clear();
        for binding in profile.mappings {
            if known_actions.contains(&binding.action_id)
                && known_devices.contains(&binding.device_alias)
            {
                if let Some(existing) = self
                    .draft
                    .bindings
                    .iter_mut()
                    .find(|item| item.id == binding.id)
                {
                    *existing = binding;
                } else {
                    self.draft.bindings.push(binding);
                }
                changed += 1;
            } else {
                self.inactive_imports.push(binding);
                unknown += 1;
            }
        }
        self.status = EditorStatus::Imported { changed, unknown };
    }

    /// Returns the canonical profile including inactive unknown records.
    #[must_use]
    pub fn export_profile(&self, name: impl Into<String>) -> MappingProfile {
        let mut profile = MappingProfile::from_snapshot(&self.draft, name);
        profile
            .mappings
            .extend(self.inactive_imports.iter().cloned());
        profile
            .mappings
            .sort_by(|left, right| left.id.cmp(&right.id));
        profile
    }

    fn capture_keyboard(&mut self, chord: KeyChord) -> Result<(), EditorError> {
        if self.learn != Some(LearnTarget::Keyboard) {
            return Err(EditorError::LearnNotActive);
        }
        let action_id = self
            .selected_action
            .clone()
            .ok_or(EditorError::NoSelectedAction)?;
        self.add_keyboard_sequence(&action_id, vec![chord])?;
        self.learn = None;
        self.learn_idle_ticks = 0;
        self.status = EditorStatus::LearnCaptured;
        Ok(())
    }

    fn tick_learn(&mut self) {
        if self.learn.is_none() {
            return;
        }
        self.learn_idle_ticks = self.learn_idle_ticks.saturating_add(1);
        if self.learn_idle_ticks >= LEARN_TIMEOUT_TICKS {
            self.learn = None;
            self.learn_idle_ticks = 0;
            self.status = EditorStatus::LearnTimedOut;
        }
    }

    fn remove_binding(&mut self, binding_id: &str) -> Result<(), EditorError> {
        if let Some(index) = self
            .draft
            .bindings
            .iter()
            .position(|binding| binding.id == binding_id)
        {
            let action_id = self.draft.bindings[index].action_id.clone();
            let requires_fallback = self
                .draft
                .actions
                .iter()
                .find(|action| action.id == action_id)
                .is_some_and(|action| action.nonremovable);
            let enabled_for_action = self
                .draft
                .bindings
                .iter()
                .filter(|binding| binding.action_id == action_id && binding.enabled)
                .count();
            if requires_fallback && enabled_for_action <= 1 {
                return Err(EditorError::NonRemovableFallback);
            }
            if self.draft.bindings[index].built_in {
                self.draft.bindings[index].enabled = false;
            } else {
                self.draft.bindings.remove(index);
            }
            self.status = EditorStatus::Dirty;
        }
        Ok(())
    }

    fn reset(&mut self, scope: ResetScope) {
        match scope {
            ResetScope::All => self.draft.bindings = self.defaults.bindings.clone(),

            ResetScope::Action => {
                if let Some(action_id) = self.selected_action.as_ref() {
                    self.draft
                        .bindings
                        .retain(|binding| binding.action_id != *action_id);
                    self.draft.bindings.extend(
                        self.defaults
                            .bindings
                            .iter()
                            .filter(|binding| binding.action_id == *action_id)
                            .cloned(),
                    );
                }
            }
            ResetScope::Device(kind) => {
                let aliases: BTreeSet<_> = self
                    .draft
                    .devices
                    .iter()
                    .filter(|device| device.kind == kind)
                    .map(|device| device.alias.as_str())
                    .collect();
                self.draft
                    .bindings
                    .retain(|binding| !aliases.contains(binding.device_alias.as_str()));
                self.draft.bindings.extend(
                    self.defaults
                        .bindings
                        .iter()
                        .filter(|binding| aliases.contains(binding.device_alias.as_str()))
                        .cloned(),
                );
            }
        }
        self.inactive_imports.clear();
        self.status = EditorStatus::Dirty;
    }

    fn apply(&mut self, live_generation: u64) -> Result<ApplyResult, EditorError> {
        if live_generation != self.baseline.generation {
            self.status = EditorStatus::StaleGeneration;
            return Err(EditorError::StaleGeneration);
        }
        if self.conflicts().iter().any(MappingConflict::blocks_apply) {
            self.status = EditorStatus::ValidationError(EditorError::ExactConflict.to_string());
            return Err(EditorError::ExactConflict);
        }
        validate_bindings(&self.draft)?;
        let changed_bindings = self
            .draft
            .bindings
            .iter()
            .filter(|binding| {
                self.baseline
                    .bindings
                    .iter()
                    .find(|old| old.id == binding.id)
                    != Some(binding)
            })
            .count();
        self.draft.generation = self.baseline.generation.saturating_add(1);
        self.baseline = self.draft.clone();
        self.status = EditorStatus::Applied(self.draft.generation);
        Ok(ApplyResult {
            generation: self.draft.generation,
            changed_bindings,
        })
    }
}

fn validate_bindings(snapshot: &MappingSnapshot) -> Result<(), EditorError> {
    for binding in &snapshot.bindings {
        if let BindingSource::Keyboard { sequence } = &binding.source
            && (sequence.is_empty() || sequence.len() > MAX_SEQUENCE_LENGTH)
        {
            return Err(EditorError::SequenceTooLong);
        }
        if let Some(continuous) = &binding.continuous {
            if !continuous.input_min.is_finite()
                || !continuous.input_max.is_finite()
                || continuous.input_min >= continuous.input_max
            {
                return Err(EditorError::InvalidContinuous(
                    "input range must be finite and increasing".to_owned(),
                ));
            }
            if !continuous.target_min.is_finite()
                || !continuous.target_max.is_finite()
                || continuous.target_min >= continuous.target_max
            {
                return Err(EditorError::InvalidContinuous(
                    "target range must be finite and increasing".to_owned(),
                ));
            }
            if !(0.0..1.0).contains(&continuous.deadzone) {
                return Err(EditorError::InvalidContinuous(
                    "deadzone must be between 0 and 1".to_owned(),
                ));
            }
            if continuous.step <= 0.0 || !continuous.step.is_finite() {
                return Err(EditorError::InvalidContinuous(
                    "target step must be positive".to_owned(),
                ));
            }
            if let Curve::Exponential {
                exponent_hundredths,
            } = continuous.curve
                && !(100..=400).contains(&exponent_hundredths)
            {
                return Err(EditorError::InvalidContinuous(
                    "exponential curve exponent must be between 1.00 and 4.00".to_owned(),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
