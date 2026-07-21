//! Bounded asynchronous submission lifecycle for GPU work.
//!
//! This module deliberately does not contain WGPU command encoding or queue
//! access. A caller supplies a small [`SubmissionBackend`] adapter at the
//! runtime boundary. The model owns dependency ordering and resource
//! retirement, while the adapter only reports whether a submission was
//! accepted. Completion is reported later through [`SubmissionQueue::complete`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use crate::{
    DeviceGeneration, FaultState, GpuResourcePool, GpuRuntime, PoolError, ResourceId,
    ResourceLease, SubmissionId, SubmissionToken,
};

const MAX_DIAGNOSTIC_BYTES: usize = 256;

/// Limits applied before a submission enters the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubmissionLimits {
    /// Maximum number of pending or submitted submissions.
    pub max_active_submissions: usize,
    /// Maximum number of resources owned by pending or submitted work.
    pub max_active_resources: usize,
    /// Maximum dependency edges on one submission.
    pub max_dependencies_per_submission: usize,
}

impl SubmissionLimits {
    /// Creates limits. Every limit must be nonzero.
    pub const fn new(
        max_active_submissions: usize,
        max_active_resources: usize,
        max_dependencies_per_submission: usize,
    ) -> Result<Self, SubmissionLimitError> {
        if max_active_submissions == 0 {
            return Err(SubmissionLimitError::ActiveSubmissions);
        }
        if max_active_resources == 0 {
            return Err(SubmissionLimitError::ActiveResources);
        }
        if max_dependencies_per_submission == 0 {
            return Err(SubmissionLimitError::Dependencies);
        }
        Ok(Self {
            max_active_submissions,
            max_active_resources,
            max_dependencies_per_submission,
        })
    }
}

impl Default for SubmissionLimits {
    fn default() -> Self {
        Self {
            max_active_submissions: 64,
            max_active_resources: 256,
            max_dependencies_per_submission: 32,
        }
    }
}

/// Configuration errors for [`SubmissionLimits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionLimitError {
    ActiveSubmissions,
    ActiveResources,
    Dependencies,
}

impl fmt::Display for SubmissionLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::ActiveSubmissions => "active submission limit must be nonzero",
            Self::ActiveResources => "active resource limit must be nonzero",
            Self::Dependencies => "dependency limit must be nonzero",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for SubmissionLimitError {}

/// One unit of work admitted to a [`SubmissionQueue`].
///
/// Resource leases move into the queue and remain owned by it until either a
/// pre-submit cancellation drops them or a completed submission retires their
/// corresponding [`SubmissionToken`]s.
#[derive(Debug)]
pub struct SubmissionRequest {
    dependencies: Vec<SubmissionId>,
    resources: Vec<ResourceLease>,
}

impl SubmissionRequest {
    /// Creates work with no dependencies.
    #[must_use]
    pub const fn new(resources: Vec<ResourceLease>) -> Self {
        Self {
            dependencies: Vec::new(),
            resources,
        }
    }

    /// Creates work with the supplied dependency IDs.
    #[must_use]
    pub fn with_dependencies(
        resources: Vec<ResourceLease>,
        dependencies: Vec<SubmissionId>,
    ) -> Self {
        Self {
            dependencies,
            resources,
        }
    }

    /// Returns dependency IDs in declaration order.
    #[must_use]
    pub fn dependencies(&self) -> &[SubmissionId] {
        &self.dependencies
    }

    /// Returns the number of resources held by this request.
    #[must_use]
    pub const fn resource_count(&self) -> usize {
        self.resources.len()
    }
}

/// A read-only packet passed to the runtime's queue adapter.
///
/// The adapter must not retain these borrowed slices. It should submit its
/// already-prepared opaque work and return; completion is reported separately.
#[derive(Debug, Clone, Copy)]
pub struct SubmissionPacket<'a> {
    pub id: SubmissionId,
    pub dependencies: &'a [SubmissionId],
    pub resources: &'a [ResourceId],
}

/// Runtime boundary used by the model before calling a queue adapter.
pub trait SubmissionRuntime {
    /// Rejects work when the runtime cannot accept GPU submissions.
    fn validate_submission(&self) -> Result<(), RuntimeSubmissionError>;
}

impl SubmissionRuntime for GpuRuntime {
    fn validate_submission(&self) -> Result<(), RuntimeSubmissionError> {
        if self.is_cpu_only() {
            return Err(RuntimeSubmissionError::CpuOnly);
        }
        let snapshot = self.fault_snapshot();
        if matches!(snapshot.state, FaultState::Healthy | FaultState::Degraded) {
            Ok(())
        } else {
            Err(RuntimeSubmissionError::Faulted(snapshot.state))
        }
    }
}

/// Runtime states that prevent submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeSubmissionError {
    CpuOnly,
    Faulted(FaultState),
}

impl fmt::Display for RuntimeSubmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CpuOnly => formatter.write_str("GPU runtime is CPU-only"),
            Self::Faulted(state) => {
                write!(formatter, "GPU runtime is not accepting work: {state:?}")
            }
        }
    }
}

impl std::error::Error for RuntimeSubmissionError {}

/// Queue adapter that hides actual WGPU queue integration behind the runtime.
pub trait SubmissionBackend {
    type Error: fmt::Display;

    /// Attempts to hand opaque work to the runtime queue.
    fn submit(&mut self, packet: SubmissionPacket<'_>) -> Result<(), Self::Error>;
}

/// State of a submission retained by the bounded queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionState {
    Pending,
    Submitted { cancellation_requested: bool },
}

/// Signals delivered by the asynchronous runtime after submission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionSignal {
    Completed,
    Failed(String),
    Obsolete,
}

/// Terminal result visible to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionOutcome {
    Completed,
    Failed(String),
    Obsolete,
    CancelledBeforeSubmit,
}

/// Evidence that all resources owned by a submission were retired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionReceipt {
    pub id: SubmissionId,
    pub outcome: CompletionOutcome,
    pub retired_resources: usize,
}

/// Result of cancellation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CancellationOutcome {
    CancelledBeforeSubmit {
        invalidated_dependents: Vec<SubmissionId>,
    },
    RequestedAfterSubmit,
    AlreadyRequested,
}

/// Successful dispatch or a backend rejection that was safely retired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DispatchOutcome {
    Submitted(SubmissionId),
    Rejected(CompletionReceipt),
}

/// Queue admission and lifecycle errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmissionError {
    AdmissionLimit {
        limit: usize,
        kind: AdmissionLimitKind,
    },
    TooManyDependencies {
        count: usize,
        limit: usize,
    },
    DuplicateDependency(SubmissionId),
    UnknownSubmission(SubmissionId),
    UnknownDependency(SubmissionId),
    SelfDependency(SubmissionId),
    DependencyCycle {
        submission: SubmissionId,
        dependency: SubmissionId,
    },
    DependenciesMustBePending(SubmissionId),
    NotReady(SubmissionId),
    ResourceGeneration {
        expected: DeviceGeneration,
        actual: DeviceGeneration,
    },
    Runtime(RuntimeSubmissionError),
    Resource(PoolError),
    IdExhausted,
}

/// Which queue admission bound was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionLimitKind {
    ActiveSubmissions,
    ActiveResources,
}

impl fmt::Display for SubmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AdmissionLimit { limit, kind } => {
                write!(
                    formatter,
                    "submission admission limit reached: {kind:?} (limit {limit})"
                )
            }
            Self::TooManyDependencies { count, limit } => {
                write!(
                    formatter,
                    "submission has {count} dependencies; limit is {limit}"
                )
            }
            Self::DuplicateDependency(id) => write!(formatter, "duplicate dependency: {id:?}"),
            Self::UnknownSubmission(id) => write!(formatter, "unknown submission: {id:?}"),
            Self::UnknownDependency(id) => write!(formatter, "unknown dependency: {id:?}"),
            Self::SelfDependency(id) => write!(formatter, "submission depends on itself: {id:?}"),
            Self::DependencyCycle {
                submission,
                dependency,
            } => write!(
                formatter,
                "dependency cycle: {submission:?} -> {dependency:?}"
            ),
            Self::DependenciesMustBePending(id) => {
                write!(
                    formatter,
                    "dependencies can only change while {id:?} is pending"
                )
            }
            Self::NotReady(id) => {
                write!(formatter, "submission is waiting on dependencies: {id:?}")
            }
            Self::ResourceGeneration { expected, actual } => write!(
                formatter,
                "submission resource generation mismatch: expected {}, got {}",
                expected.value(),
                actual.value()
            ),
            Self::Runtime(error) => error.fmt(formatter),
            Self::Resource(error) => error.fmt(formatter),
            Self::IdExhausted => formatter.write_str("submission ID space exhausted"),
        }
    }
}

impl std::error::Error for SubmissionError {}

impl From<PoolError> for SubmissionError {
    fn from(error: PoolError) -> Self {
        Self::Resource(error)
    }
}

#[derive(Debug)]
struct SubmissionRecord {
    state: SubmissionState,
    dependencies: BTreeSet<SubmissionId>,
    unresolved: BTreeSet<SubmissionId>,
    resource_ids: Vec<ResourceId>,
    resources: Vec<ResourceLease>,
    tokens: Vec<SubmissionToken>,
}

/// Bounded dependency-aware asynchronous submission state machine.
#[derive(Debug)]
pub struct SubmissionQueue {
    pool: GpuResourcePool,
    limits: SubmissionLimits,
    records: BTreeMap<SubmissionId, SubmissionRecord>,
    next_id: u64,
    active_resources: usize,
}

impl SubmissionQueue {
    /// Creates a queue tied to the resource pool whose leases it owns.
    #[must_use]
    pub fn new(pool: GpuResourcePool, limits: SubmissionLimits) -> Self {
        Self {
            pool,
            limits,
            records: BTreeMap::new(),
            next_id: 1,
            active_resources: 0,
        }
    }

    /// Returns the immutable queue limits.
    #[must_use]
    pub const fn limits(&self) -> SubmissionLimits {
        self.limits
    }

    /// Returns active submission count, including pending work.
    #[must_use]
    pub fn active_submissions(&self) -> usize {
        self.records.len()
    }

    /// Returns active resource count, including pending work.
    #[must_use]
    pub const fn active_resources(&self) -> usize {
        self.active_resources
    }

    /// Returns a snapshot of a live submission's state.
    #[must_use]
    pub fn state(&self, id: SubmissionId) -> Option<SubmissionState> {
        self.records.get(&id).map(|record| record.state)
    }

    /// Admits work after checking all bounds and dependency identities.
    pub fn admit(&mut self, request: SubmissionRequest) -> Result<SubmissionId, SubmissionError> {
        if self.records.len() >= self.limits.max_active_submissions {
            return Err(SubmissionError::AdmissionLimit {
                limit: self.limits.max_active_submissions,
                kind: AdmissionLimitKind::ActiveSubmissions,
            });
        }
        if request.resources.len() > self.limits.max_active_resources {
            return Err(SubmissionError::AdmissionLimit {
                limit: self.limits.max_active_resources,
                kind: AdmissionLimitKind::ActiveResources,
            });
        }
        let next_resources = self
            .active_resources
            .checked_add(request.resources.len())
            .ok_or(SubmissionError::IdExhausted)?;
        if next_resources > self.limits.max_active_resources {
            return Err(SubmissionError::AdmissionLimit {
                limit: self.limits.max_active_resources,
                kind: AdmissionLimitKind::ActiveResources,
            });
        }
        if request.dependencies.len() > self.limits.max_dependencies_per_submission {
            return Err(SubmissionError::TooManyDependencies {
                count: request.dependencies.len(),
                limit: self.limits.max_dependencies_per_submission,
            });
        }

        let id = SubmissionId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(SubmissionError::IdExhausted)?;
        let mut dependencies = BTreeSet::new();
        for dependency in request.dependencies {
            if dependency == id {
                return Err(SubmissionError::SelfDependency(id));
            }
            if !dependencies.insert(dependency) {
                return Err(SubmissionError::DuplicateDependency(dependency));
            }
            if !self.records.contains_key(&dependency) {
                return Err(SubmissionError::UnknownDependency(dependency));
            }
        }

        let generation = self.pool.generation();
        let resource_ids = request
            .resources
            .iter()
            .map(ResourceLease::id)
            .collect::<Vec<_>>();
        if let Some(actual) = resource_ids
            .iter()
            .map(|resource| resource.generation)
            .find(|actual| *actual != generation)
        {
            return Err(SubmissionError::ResourceGeneration {
                expected: generation,
                actual,
            });
        }

        self.active_resources = next_resources;
        self.records.insert(
            id,
            SubmissionRecord {
                state: SubmissionState::Pending,
                unresolved: dependencies.clone(),
                dependencies,
                resource_ids,
                resources: request.resources,
                tokens: Vec::new(),
            },
        );
        Ok(id)
    }

    /// Adds a dependency while preserving an acyclic graph.
    pub fn add_dependency(
        &mut self,
        submission: SubmissionId,
        dependency: SubmissionId,
    ) -> Result<(), SubmissionError> {
        let state = self
            .records
            .get(&submission)
            .ok_or(SubmissionError::UnknownSubmission(submission))?
            .state;
        if state != SubmissionState::Pending {
            return Err(SubmissionError::DependenciesMustBePending(submission));
        }
        if submission == dependency {
            return Err(SubmissionError::SelfDependency(submission));
        }
        if !self.records.contains_key(&dependency) {
            return Err(SubmissionError::UnknownDependency(dependency));
        }
        let Some(record) = self.records.get(&submission) else {
            return Err(SubmissionError::UnknownSubmission(submission));
        };
        if record.dependencies.contains(&dependency) {
            return Err(SubmissionError::DuplicateDependency(dependency));
        }
        if record.dependencies.len() >= self.limits.max_dependencies_per_submission {
            return Err(SubmissionError::TooManyDependencies {
                count: record.dependencies.len().saturating_add(1),
                limit: self.limits.max_dependencies_per_submission,
            });
        }
        if self.reaches(dependency, submission) {
            return Err(SubmissionError::DependencyCycle {
                submission,
                dependency,
            });
        }
        let Some(record) = self.records.get_mut(&submission) else {
            return Err(SubmissionError::UnknownSubmission(submission));
        };
        record.dependencies.insert(dependency);
        record.unresolved.insert(dependency);
        Ok(())
    }

    /// Returns pending submissions whose dependencies have all completed, in
    /// deterministic typed-ID order.
    #[must_use]
    pub fn ready(&self) -> Vec<SubmissionId> {
        self.records
            .iter()
            .filter_map(|(id, record)| {
                (record.state == SubmissionState::Pending && record.unresolved.is_empty())
                    .then_some(*id)
            })
            .collect()
    }

    /// Dispatches one ready submission through the opaque runtime adapter.
    pub fn submit<R, B>(
        &mut self,
        runtime: &R,
        id: SubmissionId,
        backend: &mut B,
    ) -> Result<DispatchOutcome, SubmissionError>
    where
        R: SubmissionRuntime,
        B: SubmissionBackend,
    {
        runtime
            .validate_submission()
            .map_err(SubmissionError::Runtime)?;
        let mut record = self
            .records
            .remove(&id)
            .ok_or(SubmissionError::UnknownSubmission(id))?;
        if record.state != SubmissionState::Pending {
            self.records.insert(id, record);
            return Err(SubmissionError::NotReady(id));
        }
        if !record.unresolved.is_empty() {
            self.records.insert(id, record);
            return Err(SubmissionError::NotReady(id));
        }

        let dependencies = record.dependencies.iter().copied().collect::<Vec<_>>();
        let resource_ids = record.resource_ids.clone();
        let leases = std::mem::take(&mut record.resources);
        for lease in leases {
            match lease.submit() {
                Ok(token) => record.tokens.push(token),
                Err(error) => {
                    let retirement = retire_tokens(std::mem::take(&mut record.tokens));
                    self.active_resources = self
                        .active_resources
                        .saturating_sub(record.resource_ids.len());
                    if let Err(retirement_error) = retirement {
                        return Err(SubmissionError::Resource(retirement_error));
                    }
                    return Err(SubmissionError::Resource(error));
                }
            }
        }

        record.state = SubmissionState::Submitted {
            cancellation_requested: false,
        };
        let packet = SubmissionPacket {
            id,
            dependencies: &dependencies,
            resources: &resource_ids,
        };
        match backend.submit(packet) {
            Ok(()) => {
                self.records.insert(id, record);
                Ok(DispatchOutcome::Submitted(id))
            }
            Err(error) => {
                let message = bounded_message(&error.to_string());
                let retired_resources = record.resource_ids.len();
                let retirement = retire_tokens(std::mem::take(&mut record.tokens));
                self.active_resources = self.active_resources.saturating_sub(retired_resources);
                if let Err(retirement_error) = retirement {
                    return Err(SubmissionError::Resource(retirement_error));
                }
                let invalidated = self.invalidate_dependents(id);
                let _ = invalidated;
                Ok(DispatchOutcome::Rejected(CompletionReceipt {
                    id,
                    outcome: CompletionOutcome::Failed(message),
                    retired_resources,
                }))
            }
        }
    }

    /// Requests cancellation. Submitted work cannot be removed from the GPU;
    /// its eventual result is therefore reported as [`CompletionOutcome::Obsolete`].
    pub fn cancel(&mut self, id: SubmissionId) -> Result<CancellationOutcome, SubmissionError> {
        let state = self
            .records
            .get(&id)
            .ok_or(SubmissionError::UnknownSubmission(id))?
            .state;
        match state {
            SubmissionState::Pending => {
                let Some(record) = self.records.remove(&id) else {
                    return Err(SubmissionError::UnknownSubmission(id));
                };
                self.active_resources = self
                    .active_resources
                    .saturating_sub(record.resource_ids.len());
                drop(record);
                let invalidated_dependents = self.invalidate_dependents(id);
                Ok(CancellationOutcome::CancelledBeforeSubmit {
                    invalidated_dependents,
                })
            }
            SubmissionState::Submitted {
                cancellation_requested: false,
            } => {
                let Some(record) = self.records.get_mut(&id) else {
                    return Err(SubmissionError::UnknownSubmission(id));
                };
                record.state = SubmissionState::Submitted {
                    cancellation_requested: true,
                };
                Ok(CancellationOutcome::RequestedAfterSubmit)
            }
            SubmissionState::Submitted {
                cancellation_requested: true,
            } => Ok(CancellationOutcome::AlreadyRequested),
        }
    }

    /// Retires all resource tokens and records the asynchronous completion.
    pub fn complete(
        &mut self,
        id: SubmissionId,
        signal: CompletionSignal,
    ) -> Result<CompletionReceipt, SubmissionError> {
        let record = self
            .records
            .remove(&id)
            .ok_or(SubmissionError::UnknownSubmission(id))?;
        let cancellation_requested = match record.state {
            SubmissionState::Pending => {
                self.records.insert(id, record);
                return Err(SubmissionError::NotReady(id));
            }
            SubmissionState::Submitted {
                cancellation_requested,
            } => cancellation_requested,
        };
        let retired_resources = record.resource_ids.len();
        retire_tokens(record.tokens)?;
        self.active_resources = self.active_resources.saturating_sub(retired_resources);
        let outcome = if cancellation_requested || signal == CompletionSignal::Obsolete {
            CompletionOutcome::Obsolete
        } else {
            match signal {
                CompletionSignal::Completed => CompletionOutcome::Completed,
                CompletionSignal::Failed(error) => {
                    CompletionOutcome::Failed(bounded_message(&error))
                }
                CompletionSignal::Obsolete => CompletionOutcome::Obsolete,
            }
        };
        if outcome == CompletionOutcome::Completed {
            for record in self.records.values_mut() {
                record.unresolved.remove(&id);
            }
        } else {
            let _ = self.invalidate_dependents(id);
        }
        Ok(CompletionReceipt {
            id,
            outcome,
            retired_resources,
        })
    }

    fn reaches(&self, start: SubmissionId, target: SubmissionId) -> bool {
        let mut stack = vec![start];
        let mut visited = BTreeSet::new();
        while let Some(id) = stack.pop() {
            if id == target {
                return true;
            }
            if !visited.insert(id) {
                continue;
            }
            if let Some(record) = self.records.get(&id) {
                stack.extend(record.dependencies.iter().copied());
            }
        }
        false
    }

    fn invalidate_dependents(&mut self, dependency: SubmissionId) -> Vec<SubmissionId> {
        let mut invalidated = Vec::new();
        let mut queue = VecDeque::from([dependency]);
        while let Some(completed_id) = queue.pop_front() {
            let dependents = self
                .records
                .iter()
                .filter_map(|(id, record)| record.unresolved.contains(&completed_id).then_some(*id))
                .collect::<Vec<_>>();
            for dependent in dependents {
                if let Some(record) = self.records.remove(&dependent) {
                    self.active_resources = self
                        .active_resources
                        .saturating_sub(record.resource_ids.len());
                    drop(record);
                    invalidated.push(dependent);
                    queue.push_back(dependent);
                }
            }
        }
        invalidated
    }
}

fn retire_tokens(tokens: Vec<SubmissionToken>) -> Result<(), PoolError> {
    let mut first_error = None;
    for token in tokens {
        if let Err(error) = token.retire()
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn bounded_message(message: &str) -> String {
    if message.len() <= MAX_DIAGNOSTIC_BYTES {
        return message.to_owned();
    }
    let mut end = MAX_DIAGNOSTIC_BYTES.saturating_sub(3);
    while !message.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    format!("{}...", &message[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DeviceGeneration, InitializationPolicy, ResourceClass, ResourcePoolConfig, ResourceRequest,
    };

    struct HealthyRuntime;

    impl SubmissionRuntime for HealthyRuntime {
        fn validate_submission(&self) -> Result<(), RuntimeSubmissionError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct Backend {
        submitted: Vec<SubmissionId>,
        reject: bool,
    }

    impl SubmissionBackend for Backend {
        type Error = &'static str;

        fn submit(&mut self, packet: SubmissionPacket<'_>) -> Result<(), Self::Error> {
            if self.reject {
                return Err("backend rejected submission");
            }
            self.submitted.push(packet.id);
            Ok(())
        }
    }

    fn pool() -> GpuResourcePool {
        GpuResourcePool::new(
            DeviceGeneration::new(1),
            ResourcePoolConfig {
                configured_cap: 1 << 20,
                memory_hint: None,
                addressable_limit: u64::MAX,
                backend_overhead: 0,
                soft_percent: 100,
            },
        )
    }

    fn resource(pool: &GpuResourcePool) -> ResourceLease {
        pool.try_acquire(ResourceRequest::new(
            ResourceClass::buffer(DeviceGeneration::new(1), 64, 0),
            InitializationPolicy::Zeroed,
        ))
        .expect("resource")
    }

    #[test]
    fn dependency_cycle_is_rejected() {
        let mut queue = SubmissionQueue::new(pool(), SubmissionLimits::default());
        let first = queue
            .admit(SubmissionRequest::new(Vec::new()))
            .expect("first");
        let second = queue
            .admit(SubmissionRequest::new(Vec::new()))
            .expect("second");
        queue.add_dependency(second, first).expect("edge");
        assert!(matches!(
            queue.add_dependency(first, second),
            Err(SubmissionError::DependencyCycle { .. })
        ));
    }

    #[test]
    fn ready_order_and_exact_retirement_are_bounded() {
        let pool = pool();
        let limits = SubmissionLimits::new(2, 1, 2).expect("limits");
        let mut queue = SubmissionQueue::new(pool.clone(), limits);
        let first = queue
            .admit(SubmissionRequest::new(vec![resource(&pool)]))
            .expect("first");
        let second = queue
            .admit(SubmissionRequest::with_dependencies(
                Vec::new(),
                vec![first],
            ))
            .expect("second");
        assert!(matches!(
            queue.admit(SubmissionRequest::new(Vec::new())),
            Err(SubmissionError::AdmissionLimit { .. })
        ));
        assert_eq!(queue.ready(), vec![first]);

        let mut backend = Backend::default();
        let runtime = HealthyRuntime;
        assert_eq!(
            queue.submit(&runtime, first, &mut backend),
            Ok(DispatchOutcome::Submitted(first))
        );
        let receipt = queue
            .complete(first, CompletionSignal::Completed)
            .expect("completion");
        assert_eq!(receipt.retired_resources, 1);
        assert_eq!(receipt.outcome, CompletionOutcome::Completed);
        assert_eq!(queue.ready(), vec![second]);
        assert_eq!(pool.metrics().accounting.in_flight_bytes, 0);
    }

    #[test]
    fn cancellation_has_pre_and_post_submit_contracts() {
        let pool = pool();
        let mut queue = SubmissionQueue::new(pool.clone(), SubmissionLimits::default());
        let before = queue
            .admit(SubmissionRequest::new(vec![resource(&pool)]))
            .expect("before");
        assert!(matches!(
            queue.cancel(before),
            Ok(CancellationOutcome::CancelledBeforeSubmit { .. })
        ));
        assert_eq!(pool.metrics().accounting.leases, 0);

        let after = queue
            .admit(SubmissionRequest::new(vec![resource(&pool)]))
            .expect("after");
        let runtime = HealthyRuntime;
        let mut backend = Backend::default();
        queue.submit(&runtime, after, &mut backend).expect("submit");
        assert_eq!(
            queue.cancel(after),
            Ok(CancellationOutcome::RequestedAfterSubmit)
        );
        let receipt = queue
            .complete(after, CompletionSignal::Completed)
            .expect("completion");
        assert_eq!(receipt.outcome, CompletionOutcome::Obsolete);
        assert_eq!(pool.metrics().accounting.in_flight_bytes, 0);
    }
}
