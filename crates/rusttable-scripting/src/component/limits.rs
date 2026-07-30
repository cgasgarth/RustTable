use super::errors::{ErrorCode, ScriptError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum LimitKind {
    Memory,
    Table,
    Instances,
    Resources,
    Output,
    HostCalls,
    Fuel,
    Deadline,
    Storage,
    Concurrency,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ScriptLimits {
    pub memory_bytes: usize,
    pub table_elements: usize,
    pub instances: usize,
    pub resource_handles: usize,
    pub output_bytes: usize,
    pub host_calls: usize,
    pub fuel: u64,
    pub deadline_ms: u64,
    pub storage_bytes: usize,
    pub concurrent_instances: usize,
}

impl Default for ScriptLimits {
    fn default() -> Self {
        Self {
            memory_bytes: 64 * 1024 * 1024,
            table_elements: 1_024,
            instances: 8,
            resource_handles: 256,
            output_bytes: 64 * 1024,
            host_calls: 256,
            fuel: 10_000_000,
            deadline_ms: 2_000,
            storage_bytes: 8 * 1024 * 1024,
            concurrent_instances: 1,
        }
    }
}

impl ScriptLimits {
    /// Validates product policy before a component is compiled or activated.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when any quota is zero or concurrency exceeds the instance bound.
    pub fn validate(&self) -> Result<(), ScriptError> {
        let checks = [
            (self.memory_bytes > 0, LimitKind::Memory),
            (self.table_elements > 0, LimitKind::Table),
            (self.instances > 0, LimitKind::Instances),
            (self.resource_handles > 0, LimitKind::Resources),
            (self.output_bytes > 0, LimitKind::Output),
            (self.host_calls > 0, LimitKind::HostCalls),
            (self.fuel > 0, LimitKind::Fuel),
            (self.deadline_ms > 0, LimitKind::Deadline),
            (self.storage_bytes > 0, LimitKind::Storage),
            (self.concurrent_instances > 0, LimitKind::Concurrency),
        ];
        if let Some((false, kind)) = checks.into_iter().find(|(valid, _)| !valid) {
            return Err(ScriptError::new(
                ErrorCode::LimitExceeded,
                format!("{kind:?} must be non-zero"),
            ));
        }
        if self.concurrent_instances > self.instances {
            return Err(ScriptError::new(
                ErrorCode::LimitExceeded,
                "concurrent instances exceed instance limit",
            ));
        }
        Ok(())
    }
}
