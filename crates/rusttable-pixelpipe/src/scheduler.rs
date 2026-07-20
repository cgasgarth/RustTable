#![allow(clippy::missing_errors_doc, clippy::needless_pass_by_value)]

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use crate::scheduler_executor::isolate_work_unit;
use crate::scheduler_metrics::{MetricsMut, SchedulerMetrics, SchedulerSnapshot, ShutdownReport};
use crate::{
    AdmitReceipt, CpuPriority, ResourceClaim, RunningTask, SchedulerConfig, SchedulerError,
    SchedulerReceipt, ShutdownMode, TaskFailure, TaskId, TaskSpec, TaskState, WorkUnitBoundary,
};

#[derive(Debug)]
struct TaskRecord {
    spec: TaskSpec,
    state: TaskState,
    enqueue_sequence: u64,
    enqueued_at: Instant,
    started_at: Option<Instant>,
    cancel_requested: bool,
    actual_lease_bytes: Option<u64>,
}

impl TaskRecord {
    fn new(spec: TaskSpec, enqueue_sequence: u64, now: Instant) -> Self {
        Self {
            spec,
            state: TaskState::Queued,
            enqueue_sequence,
            enqueued_at: now,
            started_at: None,
            cancel_requested: false,
            actual_lease_bytes: None,
        }
    }

    fn queue_wait(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.enqueued_at)
    }
}

#[derive(Debug)]
struct QueueState {
    queues: [VecDeque<TaskId>; 7],
    deficits: [i64; 7],
}

impl Default for QueueState {
    fn default() -> Self {
        Self {
            queues: std::array::from_fn(|_| VecDeque::new()),
            deficits: [0; 7],
        }
    }
}

impl QueueState {
    fn len(&self) -> usize {
        self.queues.iter().map(VecDeque::len).sum()
    }

    fn push(&mut self, priority: CpuPriority, task_id: TaskId) {
        self.queues[priority.index()].push_back(task_id);
    }

    fn remove(&mut self, priority: CpuPriority, task_id: TaskId) -> bool {
        let queue = &mut self.queues[priority.index()];
        let Some(index) = queue.iter().position(|candidate| *candidate == task_id) else {
            return false;
        };
        queue.remove(index).is_some()
    }
}

/// Deterministic bounded CPU scheduler reference model.
#[derive(Debug)]
pub struct CpuScheduler {
    config: SchedulerConfig,
    tasks: BTreeMap<TaskId, TaskRecord>,
    queue: QueueState,
    next_enqueue_sequence: u64,
    admitted_memory_bytes: u64,
    active_memory_bytes: u64,
    active_workers: u16,
    active_pipelines: u16,
    metrics: SchedulerMetrics,
    receipts: VecDeque<SchedulerReceipt>,
    degraded: bool,
    shutting_down: bool,
}

impl CpuScheduler {
    #[must_use]
    pub fn new(config: SchedulerConfig) -> Self {
        Self {
            receipts: VecDeque::with_capacity(config.receipt_limit()),
            config,
            tasks: BTreeMap::new(),
            queue: QueueState::default(),
            next_enqueue_sequence: 1,
            admitted_memory_bytes: 0,
            active_memory_bytes: 0,
            active_workers: 0,
            active_pipelines: 0,
            metrics: SchedulerMetrics::default(),
            degraded: false,
            shutting_down: false,
        }
    }

    #[must_use]
    pub const fn config(&self) -> SchedulerConfig {
        self.config
    }

    pub fn submit(&mut self, spec: TaskSpec) -> Result<AdmitReceipt, SchedulerError> {
        self.submit_at(spec, Instant::now())
    }

    pub fn submit_at(
        &mut self,
        spec: TaskSpec,
        now: Instant,
    ) -> Result<AdmitReceipt, SchedulerError> {
        self.metrics.submitted = self.metrics.submitted.saturating_add(1);
        if self.shutting_down {
            self.metrics.rejected = self.metrics.rejected.saturating_add(1);
            return Err(SchedulerError::PostShutdown);
        }
        if self.degraded {
            self.metrics.rejected = self.metrics.rejected.saturating_add(1);
            return Err(SchedulerError::AccountingCorruption);
        }
        let task_id = spec.task_id();
        if self.tasks.contains_key(&task_id) {
            self.metrics.rejected = self.metrics.rejected.saturating_add(1);
            return Err(SchedulerError::DuplicateTask(task_id));
        }
        for dependency in spec.dependencies() {
            if !self.tasks.contains_key(dependency) {
                self.metrics.rejected = self.metrics.rejected.saturating_add(1);
                return Err(SchedulerError::MissingDependency(*dependency));
            }
        }
        if self.queue.len() >= self.config.total_queue_limit() {
            self.metrics.queue_overflow = self.metrics.queue_overflow.saturating_add(1);
            self.metrics.rejected = self.metrics.rejected.saturating_add(1);
            return Err(SchedulerError::QueueFull);
        }
        let class = spec.priority();
        if self.queue.queues[class.index()].len() >= self.config.per_class_queue_limit() {
            self.metrics.queue_overflow = self.metrics.queue_overflow.saturating_add(1);
            self.metrics.rejected = self.metrics.rejected.saturating_add(1);
            return Err(SchedulerError::PriorityQueueFull(class));
        }
        let claim = spec.resources();
        self.validate_claim(claim)?;
        if self
            .admitted_memory_bytes
            .checked_add(claim.memory_bytes())
            .is_none_or(|total| total > self.config.memory_limit())
        {
            self.metrics.rejected = self.metrics.rejected.saturating_add(1);
            return Err(SchedulerError::MemoryReservationUnavailable);
        }
        if !class.is_interactive()
            && self.high_priority_queued()
            && self.admitted_memory_bytes + claim.memory_bytes()
                > self
                    .config
                    .memory_limit()
                    .saturating_sub(self.memory_reservation())
        {
            self.metrics.rejected = self.metrics.rejected.saturating_add(1);
            return Err(SchedulerError::MemoryReservationUnavailable);
        }
        let sequence = self.next_enqueue_sequence;
        self.next_enqueue_sequence = self.next_enqueue_sequence.saturating_add(1);
        let receipt = AdmitReceipt {
            task_id,
            request_id: spec.request_id(),
            priority: class,
            enqueue_sequence: sequence,
            admitted_memory_bytes: claim.memory_bytes(),
        };
        self.admitted_memory_bytes += claim.memory_bytes();
        self.queue.push(class, task_id);
        self.tasks
            .insert(task_id, TaskRecord::new(spec, sequence, now));
        self.metrics.admitted = self.metrics.admitted.saturating_add(1);
        self.metrics.admitted_memory_bytes = self.admitted_memory_bytes;
        self.metrics.max_queue_depth = self.metrics.max_queue_depth.max(self.queue.len());
        Ok(receipt)
    }

    fn validate_claim(&self, claim: &ResourceClaim) -> Result<(), SchedulerError> {
        if claim.worker_tokens() > self.config.worker_limit()
            || claim.max_parallelism() > self.config.worker_limit()
            || claim.memory_bytes() > self.config.memory_limit()
        {
            return Err(SchedulerError::ResourceRequestTooLarge);
        }
        Ok(())
    }

    fn memory_reservation(&self) -> u64 {
        (self.config.memory_limit() / 5).max(1)
    }

    fn high_priority_queued(&self) -> bool {
        self.queue.queues[CpuPriority::InteractivePreview.index()]
            .iter()
            .chain(self.queue.queues[CpuPriority::VisibleFullView.index()].iter())
            .any(|task_id| {
                self.tasks
                    .get(task_id)
                    .is_some_and(|task| task.state == TaskState::Queued && !task.cancel_requested)
            })
    }

    /// Starts the next runnable task when workers, pipelines, and admission
    /// memory all fit. The returned task is metadata only; #180 dispatches it.
    pub fn start_next(&mut self) -> Option<RunningTask> {
        self.start_next_at(Instant::now())
    }

    pub fn start_next_at(&mut self, now: Instant) -> Option<RunningTask> {
        if self.shutting_down && self.queue.len() == 0 {
            return None;
        }
        self.refresh_queued(now);
        let priority = self.select_priority(now)?;
        let task_id = self.queue.queues[priority.index()].front().copied()?;
        let claim = self.tasks.get(&task_id)?.spec.resources().clone();
        if !self.fits_active(&claim) {
            self.record_resource_wait(&claim);
            return None;
        }
        let _ = self.queue.queues[priority.index()].pop_front();
        let record = self.tasks.get_mut(&task_id)?;
        record.state = TaskState::Running;
        record.started_at = Some(now);
        self.active_workers += claim.worker_tokens();
        self.active_memory_bytes += claim.memory_bytes();
        if claim.active_pipeline() {
            self.active_pipelines += 1;
        }
        self.metrics.started = self.metrics.started.saturating_add(1);
        self.metrics.active_memory_bytes = self.active_memory_bytes;
        Some(RunningTask {
            task_id,
            worker_tokens: claim.worker_tokens(),
            max_parallelism: claim.max_parallelism(),
            work_units: record.spec.work_units(),
        })
    }

    fn fits_active(&self, claim: &ResourceClaim) -> bool {
        self.active_workers
            .checked_add(claim.worker_tokens())
            .is_some_and(|workers| workers <= self.config.worker_limit())
            && self
                .active_memory_bytes
                .checked_add(claim.memory_bytes())
                .is_some_and(|bytes| bytes <= self.config.memory_limit())
            && (!claim.active_pipeline()
                || self.active_pipelines < self.config.active_pipeline_limit())
    }

    fn record_resource_wait(&mut self, claim: &ResourceClaim) {
        if self.active_workers + claim.worker_tokens() > self.config.worker_limit() {
            self.metrics.worker_waits = self.metrics.worker_waits.saturating_add(1);
        }
        if self.active_memory_bytes + claim.memory_bytes() > self.config.memory_limit() {
            self.metrics.memory_waits = self.metrics.memory_waits.saturating_add(1);
        }
    }

    fn refresh_queued(&mut self, now: Instant) {
        let ids: Vec<TaskId> = self
            .queue
            .queues
            .iter()
            .flat_map(|queue| queue.iter().copied())
            .collect();
        for task_id in ids {
            let Some(record) = self.tasks.get(&task_id) else {
                continue;
            };
            let externally_cancelled = record
                .spec
                .cancellation()
                .is_some_and(|token| token.is_cancelled());
            if externally_cancelled {
                let _ = self.finish_queued(task_id, TaskState::Cancelled, None, now);
                continue;
            }
            if self.dependency_failed(task_id) {
                self.metrics.dependency_skips = self.metrics.dependency_skips.saturating_add(1);
                let _ = self.finish_queued(
                    task_id,
                    TaskState::Skipped,
                    Some(TaskFailure::DependencyFailed),
                    now,
                );
            }
        }
    }

    fn dependency_failed(&self, task_id: TaskId) -> bool {
        self.tasks.get(&task_id).is_some_and(|record| {
            record.spec.dependencies().iter().any(|dependency| {
                self.tasks.get(dependency).is_some_and(|task| {
                    matches!(
                        task.state,
                        TaskState::Failed | TaskState::Cancelled | TaskState::Skipped
                    )
                })
            })
        })
    }

    fn dependencies_passed(&self, task_id: TaskId) -> bool {
        self.tasks.get(&task_id).is_some_and(|record| {
            record.spec.dependencies().iter().all(|dependency| {
                self.tasks
                    .get(dependency)
                    .is_some_and(|task| task.state == TaskState::Passed)
            })
        })
    }

    fn select_priority(&mut self, now: Instant) -> Option<CpuPriority> {
        let aging_after = self.config.aging_after();
        for priority in CpuPriority::ALL {
            if !self.queue.queues[priority.index()].is_empty() {
                self.queue.deficits[priority.index()] += i64::from(priority.weight());
            }
        }
        let mut best: Option<(CpuPriority, i64, u64)> = None;
        for priority in CpuPriority::ALL {
            let Some(task_id) = self.queue.queues[priority.index()].front() else {
                continue;
            };
            if !self.dependencies_passed(*task_id) {
                continue;
            }
            let waited = self
                .tasks
                .get(task_id)
                .map_or(Duration::ZERO, |task| task.queue_wait(now));
            let age = if waited >= aging_after {
                i64::try_from(waited.as_millis() / aging_after.as_millis().max(1))
                    .unwrap_or(i64::MAX)
            } else {
                0
            };
            let score = self.queue.deficits[priority.index()].saturating_add(age);
            let sequence = self
                .tasks
                .get(task_id)
                .map_or(u64::MAX, |task| task.enqueue_sequence);
            if best.is_none_or(|(_, best_score, best_sequence)| {
                score > best_score || (score == best_score && sequence < best_sequence)
            }) {
                best = Some((priority, score, sequence));
            }
        }
        let (priority, _, _) = best?;
        self.queue.deficits[priority.index()] =
            self.queue.deficits[priority.index()].saturating_sub(1);
        self.metrics.fairness_choices = self.metrics.fairness_choices.saturating_add(1);
        Some(priority)
    }

    pub fn cancel(&mut self, task_id: TaskId) -> Result<TaskState, SchedulerError> {
        self.cancel_at(task_id, Instant::now())
    }

    pub fn cancel_at(
        &mut self,
        task_id: TaskId,
        now: Instant,
    ) -> Result<TaskState, SchedulerError> {
        let state = self
            .tasks
            .get(&task_id)
            .ok_or(SchedulerError::UnknownTask(task_id))?
            .state;
        match state {
            TaskState::Queued => {
                let _ = self.finish_queued(task_id, TaskState::Cancelled, None, now);
                Ok(TaskState::Cancelled)
            }
            TaskState::Running => {
                if let Some(record) = self.tasks.get_mut(&task_id) {
                    record.cancel_requested = true;
                }
                Ok(TaskState::Running)
            }
            terminal => Ok(terminal),
        }
    }

    pub fn work_unit_boundary(
        &mut self,
        task_id: TaskId,
    ) -> Result<WorkUnitBoundary, SchedulerError> {
        let record = self
            .tasks
            .get(&task_id)
            .ok_or(SchedulerError::UnknownTask(task_id))?;
        if record.state != TaskState::Running {
            return Err(SchedulerError::NotRunning(task_id));
        }
        if self.shutting_down
            || record.cancel_requested
            || record
                .spec
                .cancellation()
                .is_some_and(|token| token.is_cancelled())
        {
            if let Some(record) = self.tasks.get_mut(&task_id) {
                record.cancel_requested = true;
            }
            return Ok(if self.shutting_down {
                WorkUnitBoundary::Shutdown
            } else {
                WorkUnitBoundary::Cancelled
            });
        }
        Ok(WorkUnitBoundary::Continue)
    }

    pub fn run_task<F>(
        &mut self,
        task_id: TaskId,
        now: Instant,
        mut work: F,
    ) -> Result<TaskState, SchedulerError>
    where
        F: FnMut(u32) -> Result<(), ()>,
    {
        let running = self
            .tasks
            .get(&task_id)
            .ok_or(SchedulerError::UnknownTask(task_id))?;
        if running.state != TaskState::Running {
            return Err(SchedulerError::NotRunning(task_id));
        }
        let units = running.spec.work_units();
        for unit in 0..units {
            if self.work_unit_boundary(task_id)? != WorkUnitBoundary::Continue {
                return self.complete_cancelled(task_id, now);
            }
            if let Err(failure) = isolate_work_unit(|| work(unit)) {
                return self.complete_failure(task_id, now, failure);
            }
        }
        self.complete_success(task_id, now)
    }

    pub fn complete_success(
        &mut self,
        task_id: TaskId,
        now: Instant,
    ) -> Result<TaskState, SchedulerError> {
        let cancelled = self
            .tasks
            .get(&task_id)
            .ok_or(SchedulerError::UnknownTask(task_id))?
            .cancel_requested;
        if cancelled || self.work_unit_boundary(task_id)? != WorkUnitBoundary::Continue {
            return self.complete_cancelled(task_id, now);
        }
        self.finish_running(task_id, TaskState::Passed, None, now)
    }

    pub fn complete_cancelled(
        &mut self,
        task_id: TaskId,
        now: Instant,
    ) -> Result<TaskState, SchedulerError> {
        self.finish_running(task_id, TaskState::Cancelled, None, now)
    }

    pub fn complete_failure(
        &mut self,
        task_id: TaskId,
        now: Instant,
        failure: TaskFailure,
    ) -> Result<TaskState, SchedulerError> {
        self.finish_running(task_id, TaskState::Failed, Some(failure), now)
    }

    pub fn confirm_leases(
        &mut self,
        task_id: TaskId,
        actual_bytes: u64,
        now: Instant,
    ) -> Result<(), SchedulerError> {
        let record = self
            .tasks
            .get(&task_id)
            .ok_or(SchedulerError::UnknownTask(task_id))?;
        if record.state != TaskState::Running {
            return Err(SchedulerError::NotRunning(task_id));
        }
        let planned = record.spec.resources().memory_bytes();
        if planned != actual_bytes {
            self.complete_failure(
                task_id,
                now,
                TaskFailure::MemoryEstimateMismatch {
                    planned_bytes: planned,
                    actual_bytes,
                },
            )?;
            return Err(SchedulerError::LeaseMismatch {
                planned_bytes: planned,
                actual_bytes,
            });
        }
        if let Some(record) = self.tasks.get_mut(&task_id) {
            record.actual_lease_bytes = Some(actual_bytes);
        }
        Ok(())
    }

    fn finish_queued(
        &mut self,
        task_id: TaskId,
        state: TaskState,
        failure: Option<TaskFailure>,
        now: Instant,
    ) -> Result<(), SchedulerError> {
        let record = self
            .tasks
            .get(&task_id)
            .ok_or(SchedulerError::UnknownTask(task_id))?;
        let priority = record.spec.priority();
        let removed = self.queue.remove(priority, task_id);
        if !removed {
            self.degraded = true;
            return Err(SchedulerError::UnknownTask(task_id));
        }
        self.finish_record(task_id, state, failure, now, None)
    }

    fn finish_running(
        &mut self,
        task_id: TaskId,
        state: TaskState,
        failure: Option<TaskFailure>,
        now: Instant,
    ) -> Result<TaskState, SchedulerError> {
        let record = self
            .tasks
            .get(&task_id)
            .ok_or(SchedulerError::UnknownTask(task_id))?;
        if record.state != TaskState::Running {
            return Err(SchedulerError::NotRunning(task_id));
        }
        let claim = record.spec.resources().clone();
        self.active_workers = self
            .active_workers
            .checked_sub(claim.worker_tokens())
            .ok_or_else(|| {
                self.degraded = true;
                SchedulerError::UnknownTask(task_id)
            })?;
        self.active_memory_bytes = self
            .active_memory_bytes
            .checked_sub(claim.memory_bytes())
            .ok_or_else(|| {
                self.degraded = true;
                SchedulerError::UnknownTask(task_id)
            })?;
        if claim.active_pipeline() {
            self.active_pipelines = self.active_pipelines.checked_sub(1).ok_or_else(|| {
                self.degraded = true;
                SchedulerError::UnknownTask(task_id)
            })?;
        }
        self.finish_record(task_id, state, failure, now, record.started_at)?;
        Ok(state)
    }

    fn finish_record(
        &mut self,
        task_id: TaskId,
        state: TaskState,
        failure: Option<TaskFailure>,
        now: Instant,
        started_at: Option<Instant>,
    ) -> Result<(), SchedulerError> {
        let record = self
            .tasks
            .get_mut(&task_id)
            .ok_or(SchedulerError::UnknownTask(task_id))?;
        let claim = record.spec.resources().clone();
        self.admitted_memory_bytes = self
            .admitted_memory_bytes
            .checked_sub(claim.memory_bytes())
            .ok_or_else(|| {
                self.degraded = true;
                SchedulerError::UnknownTask(task_id)
            })?;
        record.state = state;
        let receipt = SchedulerReceipt {
            task_id,
            request_id: record.spec.request_id(),
            priority: record.spec.priority(),
            state,
            enqueue_sequence: record.enqueue_sequence,
            queue_wait: record.queue_wait(now),
            run_time: started_at.map_or(Duration::ZERO, |started| {
                now.saturating_duration_since(started)
            }),
            admitted_memory_bytes: claim.memory_bytes(),
            worker_tokens: claim.worker_tokens(),
            dependency_count: u16::try_from(record.spec.dependencies().len()).unwrap_or(u16::MAX),
            publication_allowed: state == TaskState::Passed
                && record.spec.publication_target().kind() != crate::PublicationTargetKind::None,
            failure: failure.clone(),
        };
        self.push_receipt(receipt);
        MetricsMut::record_outcome(&mut self.metrics, state);
        if let Some(failure) = failure.as_ref() {
            MetricsMut::record_failure(&mut self.metrics, failure);
        }
        self.metrics.admitted_memory_bytes = self.admitted_memory_bytes;
        self.metrics.active_memory_bytes = self.active_memory_bytes;
        Ok(())
    }

    fn push_receipt(&mut self, receipt: SchedulerReceipt) {
        if self.config.receipt_limit() == 0 {
            return;
        }
        if self.receipts.len() >= self.config.receipt_limit() {
            let _ = self.receipts.pop_front();
        }
        self.receipts.push_back(receipt);
    }

    #[must_use]
    pub fn state(&self, task_id: TaskId) -> Option<TaskState> {
        self.tasks.get(&task_id).map(|task| task.state)
    }

    #[must_use]
    pub const fn metrics(&self) -> SchedulerMetrics {
        self.metrics
    }

    #[must_use]
    pub fn receipts(&self) -> impl ExactSizeIterator<Item = &SchedulerReceipt> {
        self.receipts.iter()
    }

    #[must_use]
    pub fn snapshot(&self) -> SchedulerSnapshot {
        SchedulerSnapshot {
            queued: self.queue.len(),
            running: self
                .tasks
                .values()
                .filter(|task| task.state == TaskState::Running)
                .count(),
            admitted_memory_bytes: self.admitted_memory_bytes,
            active_memory_bytes: self.active_memory_bytes,
            active_workers: self.active_workers,
            active_pipelines: self.active_pipelines,
            degraded: self.degraded,
            shutting_down: self.shutting_down,
        }
    }

    pub fn shutdown(&mut self, mode: ShutdownMode, now: Instant) -> ShutdownReport {
        self.shutting_down = true;
        let ids: Vec<TaskId> = self
            .queue
            .queues
            .iter()
            .flat_map(|queue| queue.iter().copied())
            .filter(|task_id| {
                mode == ShutdownMode::AbortAll
                    || self
                        .tasks
                        .get(task_id)
                        .is_some_and(|task| !task.spec.priority().is_interactive())
            })
            .collect();
        for task_id in &ids {
            let _ = self.finish_queued(
                *task_id,
                TaskState::Cancelled,
                Some(TaskFailure::Shutdown),
                now,
            );
        }
        let mut requested = 0_u64;
        for task in self
            .tasks
            .values_mut()
            .filter(|task| task.state == TaskState::Running)
        {
            if mode == ShutdownMode::AbortAll || !task.spec.priority().is_interactive() {
                task.cancel_requested = true;
                requested = requested.saturating_add(1);
            }
        }
        let snapshot = self.snapshot();
        ShutdownReport {
            mode,
            cancelled_queued: u64::try_from(ids.len()).unwrap_or(u64::MAX),
            cancellation_requested_active: requested,
            remaining_queued: snapshot.queued,
            remaining_active: snapshot.running,
            leaked_memory_bytes: 0,
        }
    }

    #[must_use]
    pub const fn is_shutting_down(&self) -> bool {
        self.shutting_down
    }
}
