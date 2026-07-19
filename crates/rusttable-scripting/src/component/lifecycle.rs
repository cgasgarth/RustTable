use std::collections::BTreeMap;

use super::{
    api::ExtensionId,
    errors::{ErrorCode, ScriptError},
    receipt::ReceiptStatus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum ExtensionState {
    Disabled,
    Enabled,
    Quarantined,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LifecycleReceipt {
    pub extension: ExtensionId,
    pub generation: u64,
    pub state: ExtensionState,
    pub status: ReceiptStatus,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    generation: u64,
    state: ExtensionState,
    faults: u32,
}

#[derive(Debug, Default)]
pub struct Lifecycle {
    entries: BTreeMap<ExtensionId, Entry>,
}

impl Lifecycle {
    pub fn install(&mut self, extension: &ExtensionId) {
        self.entries.insert(
            extension.clone(),
            Entry {
                generation: 1,
                state: ExtensionState::Disabled,
                faults: 0,
            },
        );
    }

    pub fn enable(&mut self, extension: &ExtensionId) -> Result<LifecycleReceipt, ScriptError> {
        let entry = self
            .entries
            .get_mut(extension)
            .ok_or_else(|| ScriptError::new(ErrorCode::NotFound, "extension is not registered"))?;
        if entry.state == ExtensionState::Quarantined {
            return Err(ScriptError::new(
                ErrorCode::Quarantined,
                "extension is quarantined",
            ));
        }
        entry.state = ExtensionState::Enabled;
        Ok(self.receipt(extension, ReceiptStatus::Completed))
    }

    pub fn disable(&mut self, extension: &ExtensionId) -> Result<LifecycleReceipt, ScriptError> {
        let entry = self
            .entries
            .get_mut(extension)
            .ok_or_else(|| ScriptError::new(ErrorCode::NotFound, "extension is not registered"))?;
        entry.state = ExtensionState::Disabled;
        entry.generation = entry.generation.saturating_add(1);
        Ok(self.receipt(extension, ReceiptStatus::Cancelled))
    }

    pub fn reload(&mut self, extension: &ExtensionId) -> Result<LifecycleReceipt, ScriptError> {
        let entry = self
            .entries
            .get_mut(extension)
            .ok_or_else(|| ScriptError::new(ErrorCode::NotFound, "extension is not registered"))?;
        entry.generation = entry.generation.saturating_add(1);
        entry.state = ExtensionState::Enabled;
        Ok(self.receipt(extension, ReceiptStatus::Completed))
    }

    pub fn fault(&mut self, extension: &ExtensionId) -> Result<LifecycleReceipt, ScriptError> {
        let entry = self
            .entries
            .get_mut(extension)
            .ok_or_else(|| ScriptError::new(ErrorCode::NotFound, "extension is not registered"))?;
        entry.faults = entry.faults.saturating_add(1);
        if entry.faults >= 3 {
            entry.state = ExtensionState::Quarantined;
        }
        Ok(self.receipt(extension, ReceiptStatus::Trapped))
    }

    pub fn check(&self, extension: &ExtensionId, generation: u64) -> Result<(), ScriptError> {
        let entry = self
            .entries
            .get(extension)
            .ok_or_else(|| ScriptError::new(ErrorCode::NotFound, "extension is not registered"))?;
        if entry.generation != generation {
            return Err(ScriptError::new(
                ErrorCode::Reloaded,
                "invocation belongs to an older generation",
            ));
        }
        match entry.state {
            ExtensionState::Enabled => Ok(()),
            ExtensionState::Quarantined => Err(ScriptError::new(
                ErrorCode::Quarantined,
                "extension is quarantined",
            )),
            ExtensionState::Disabled => Err(ScriptError::new(
                ErrorCode::PermissionDenied,
                "extension is disabled",
            )),
        }
    }

    #[must_use]
    pub fn generation(&self, extension: &ExtensionId) -> Option<u64> {
        self.entries.get(extension).map(|entry| entry.generation)
    }

    #[must_use]
    pub fn state(&self, extension: &ExtensionId) -> Option<ExtensionState> {
        self.entries.get(extension).map(|entry| entry.state)
    }

    fn receipt(&self, extension: &ExtensionId, status: ReceiptStatus) -> LifecycleReceipt {
        let entry = self.entries.get(extension).expect("lifecycle entry exists");
        LifecycleReceipt {
            extension: extension.clone(),
            generation: entry.generation,
            state: entry.state,
            status,
        }
    }
}
