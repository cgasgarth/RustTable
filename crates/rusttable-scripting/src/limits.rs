use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ScriptError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptLimits {
    pub max_source_bytes: usize,
    pub max_result_bytes: usize,
    pub max_instructions: u64,
    pub max_wall_time_ms: u64,
    pub max_memory_bytes: usize,
    pub max_recursion: u32,
    pub max_stack_slots: u32,
    pub max_event_queue: usize,
    pub max_concurrent_calls: u32,
    pub max_storage_bytes: usize,
    pub max_storage_keys: usize,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 256 * 1024,
            max_result_bytes: 64 * 1024,
            max_instructions: 2_000_000,
            max_wall_time_ms: 2_000,
            max_memory_bytes: 16 * 1024 * 1024,
            max_recursion: 128,
            max_stack_slots: 4096,
            max_event_queue: 256,
            max_concurrent_calls: 4,
            max_storage_bytes: 1024 * 1024,
            max_storage_keys: 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LimitKind {
    SourceBytes,
    ResultBytes,
    Instructions,
    WallTime,
    MemoryBytes,
    Recursion,
    StackSlots,
    EventQueue,
    ConcurrentCalls,
    StorageBytes,
    StorageKeys,
}

#[derive(Debug, Clone)]
pub struct QuotaLedger {
    limits: ScriptLimits,
    instructions: u64,
    concurrent_calls: u32,
    storage_bytes: usize,
    storage_keys: usize,
    started: Instant,
}

impl QuotaLedger {
    #[must_use]
    pub fn new(limits: ScriptLimits) -> Self {
        Self {
            limits,
            instructions: 0,
            concurrent_calls: 0,
            storage_bytes: 0,
            storage_keys: 0,
            started: Instant::now(),
        }
    }

    #[must_use]
    pub const fn limits(&self) -> ScriptLimits {
        self.limits
    }

    /// # Errors
    ///
    /// Returns `LimitExceeded` when source bytes exceed the per-script bound.
    pub fn check_source(&self, bytes: usize) -> Result<(), ScriptError> {
        Self::check(
            bytes <= self.limits.max_source_bytes,
            LimitKind::SourceBytes,
        )
    }

    pub fn reset_invocation(&mut self) {
        self.instructions = 0;
        self.concurrent_calls = 0;
        self.started = Instant::now();
    }

    /// # Errors
    ///
    /// Returns `LimitExceeded` when the serialized result exceeds the bound.
    pub fn check_result(&self, bytes: usize) -> Result<(), ScriptError> {
        Self::check(
            bytes <= self.limits.max_result_bytes,
            LimitKind::ResultBytes,
        )
    }

    /// # Errors
    ///
    /// Returns `LimitExceeded` when instruction or wall-clock budgets are exhausted.
    pub fn tick(&mut self, amount: u64) -> Result<(), ScriptError> {
        self.instructions = self.instructions.saturating_add(amount);
        Self::check(
            self.instructions <= self.limits.max_instructions,
            LimitKind::Instructions,
        )?;
        self.check_deadline()
    }

    /// # Errors
    ///
    /// Returns `LimitExceeded` when the invocation deadline has passed.
    pub fn check_deadline(&self) -> Result<(), ScriptError> {
        Self::check(
            self.started.elapsed() <= Duration::from_millis(self.limits.max_wall_time_ms),
            LimitKind::WallTime,
        )
    }

    /// # Errors
    ///
    /// Returns `LimitExceeded` when concurrent host calls exceed the bound.
    pub fn enter_call(&mut self) -> Result<(), ScriptError> {
        self.concurrent_calls = self.concurrent_calls.saturating_add(1);
        Self::check(
            self.concurrent_calls <= self.limits.max_concurrent_calls,
            LimitKind::ConcurrentCalls,
        )
    }

    pub fn leave_call(&mut self) {
        self.concurrent_calls = self.concurrent_calls.saturating_sub(1);
    }

    /// # Errors
    ///
    /// Returns `LimitExceeded` when storage bytes or keys exceed their bounds.
    pub fn set_storage_usage(&mut self, bytes: usize, keys: usize) -> Result<(), ScriptError> {
        self.storage_bytes = bytes;
        self.storage_keys = keys;
        Self::check(
            bytes <= self.limits.max_storage_bytes,
            LimitKind::StorageBytes,
        )?;
        Self::check(keys <= self.limits.max_storage_keys, LimitKind::StorageKeys)
    }

    fn check(valid: bool, kind: LimitKind) -> Result<(), ScriptError> {
        valid.then_some(()).ok_or_else(|| {
            ScriptError::new(
                ErrorCode::LimitExceeded,
                format!("quota exceeded: {kind:?}"),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quota_ledger_fails_closed_for_every_budgeted_counter() {
        let limits = ScriptLimits {
            max_source_bytes: 1,
            max_result_bytes: 1,
            max_instructions: 1,
            max_concurrent_calls: 1,
            max_storage_bytes: 1,
            max_storage_keys: 1,
            ..ScriptLimits::default()
        };
        let mut ledger = QuotaLedger::new(limits);
        assert!(ledger.check_source(2).is_err());
        assert!(ledger.check_result(2).is_err());
        assert!(ledger.tick(2).is_err());
        assert!(ledger.enter_call().is_ok());
        assert!(ledger.enter_call().is_err());
        assert!(ledger.set_storage_usage(2, 1).is_err());
    }
}
