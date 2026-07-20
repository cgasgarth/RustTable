use std::time::Duration;

use crate::{CpuPriority, TaskFailure, TaskId, TaskState};

/// Privacy-safe counters owned by one scheduler instance.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SchedulerMetrics {
    pub(crate) submitted: u64,
    pub(crate) admitted: u64,
    pub(crate) rejected: u64,
    pub(crate) started: u64,
    pub(crate) completed: u64,
    pub(crate) failed: u64,
    pub(crate) cancelled: u64,
    pub(crate) skipped: u64,
    pub(crate) panics: u64,
    pub(crate) queue_overflow: u64,
    pub(crate) memory_waits: u64,
    pub(crate) worker_waits: u64,
    pub(crate) dependency_skips: u64,
    pub(crate) fairness_choices: u64,
    pub(crate) max_queue_depth: usize,
    pub(crate) admitted_memory_bytes: u64,
    pub(crate) active_memory_bytes: u64,
}

impl SchedulerMetrics {
    #[must_use]
    pub const fn submitted(self) -> u64 {
        self.submitted
    }
    #[must_use]
    pub const fn admitted(self) -> u64 {
        self.admitted
    }
    #[must_use]
    pub const fn rejected(self) -> u64 {
        self.rejected
    }
    #[must_use]
    pub const fn started(self) -> u64 {
        self.started
    }
    #[must_use]
    pub const fn completed(self) -> u64 {
        self.completed
    }
    #[must_use]
    pub const fn failed(self) -> u64 {
        self.failed
    }
    #[must_use]
    pub const fn cancelled(self) -> u64 {
        self.cancelled
    }
    #[must_use]
    pub const fn skipped(self) -> u64 {
        self.skipped
    }
    #[must_use]
    pub const fn panics(self) -> u64 {
        self.panics
    }
    #[must_use]
    pub const fn queue_overflow(self) -> u64 {
        self.queue_overflow
    }
    #[must_use]
    pub const fn memory_waits(self) -> u64 {
        self.memory_waits
    }
    #[must_use]
    pub const fn worker_waits(self) -> u64 {
        self.worker_waits
    }
    #[must_use]
    pub const fn dependency_skips(self) -> u64 {
        self.dependency_skips
    }
    #[must_use]
    pub const fn fairness_choices(self) -> u64 {
        self.fairness_choices
    }
    #[must_use]
    pub const fn max_queue_depth(self) -> usize {
        self.max_queue_depth
    }
    #[must_use]
    pub const fn admitted_memory_bytes(self) -> u64 {
        self.admitted_memory_bytes
    }
    #[must_use]
    pub const fn active_memory_bytes(self) -> u64 {
        self.active_memory_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownReport {
    pub(crate) mode: crate::ShutdownMode,
    pub(crate) cancelled_queued: u64,
    pub(crate) cancellation_requested_active: u64,
    pub(crate) remaining_queued: usize,
    pub(crate) remaining_active: usize,
    pub(crate) leaked_memory_bytes: u64,
}

impl ShutdownReport {
    #[must_use]
    pub const fn mode(self) -> crate::ShutdownMode {
        self.mode
    }
    #[must_use]
    pub const fn cancelled_queued(self) -> u64 {
        self.cancelled_queued
    }
    #[must_use]
    pub const fn cancellation_requested_active(self) -> u64 {
        self.cancellation_requested_active
    }
    #[must_use]
    pub const fn remaining_queued(self) -> usize {
        self.remaining_queued
    }
    #[must_use]
    pub const fn remaining_active(self) -> usize {
        self.remaining_active
    }
    #[must_use]
    pub const fn leaked_memory_bytes(self) -> u64 {
        self.leaked_memory_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerSnapshot {
    pub queued: usize,
    pub running: usize,
    pub admitted_memory_bytes: u64,
    pub active_memory_bytes: u64,
    pub active_workers: u16,
    pub active_pipelines: u16,
    pub degraded: bool,
    pub shutting_down: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkUnitReceipt {
    pub task_id: TaskId,
    pub unit: u32,
    pub state: TaskState,
    pub failure: Option<TaskFailure>,
    pub elapsed: Duration,
    pub publication_allowed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FairnessReceipt {
    pub priority: CpuPriority,
    pub waited: Duration,
    pub aged: bool,
    pub deficit: i64,
}

pub(crate) struct MetricsMut;

impl MetricsMut {
    pub(crate) const fn record_outcome(metrics: &mut SchedulerMetrics, outcome: TaskState) {
        match outcome {
            TaskState::Passed => metrics.completed = metrics.completed.saturating_add(1),
            TaskState::Failed => metrics.failed = metrics.failed.saturating_add(1),
            TaskState::Cancelled => metrics.cancelled = metrics.cancelled.saturating_add(1),
            TaskState::Skipped => metrics.skipped = metrics.skipped.saturating_add(1),
            TaskState::Queued | TaskState::Running => {}
        }
    }

    pub(crate) const fn record_failure(metrics: &mut SchedulerMetrics, failure: &TaskFailure) {
        if matches!(failure, TaskFailure::PanicIsolated) {
            metrics.panics = metrics.panics.saturating_add(1);
        }
    }
}
