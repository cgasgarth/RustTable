#![allow(clippy::missing_errors_doc)]

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::{PipelineGeneration, PipelineSnapshotIdentity};

/// The scheduler's fixed priority classes. The order is also the deterministic
/// tie-break order used by the reference model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CpuPriority {
    InteractivePreview,
    VisibleFullView,
    UserExport,
    VisibleThumbnail,
    PrefetchThumbnail,
    BackgroundAnalysis,
    Maintenance,
}

impl CpuPriority {
    pub const ALL: [Self; 7] = [
        Self::InteractivePreview,
        Self::VisibleFullView,
        Self::UserExport,
        Self::VisibleThumbnail,
        Self::PrefetchThumbnail,
        Self::BackgroundAnalysis,
        Self::Maintenance,
    ];

    #[must_use]
    pub const fn weight(self) -> u32 {
        match self {
            Self::InteractivePreview => 16,
            Self::VisibleFullView => 8,
            Self::UserExport | Self::VisibleThumbnail => 4,
            Self::PrefetchThumbnail => 2,
            Self::BackgroundAnalysis | Self::Maintenance => 1,
        }
    }

    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::InteractivePreview => "interactive-preview",
            Self::VisibleFullView => "visible-full-view",
            Self::UserExport => "user-export",
            Self::VisibleThumbnail => "visible-thumbnail",
            Self::PrefetchThumbnail => "prefetch-thumbnail",
            Self::BackgroundAnalysis => "background-analysis",
            Self::Maintenance => "maintenance",
        }
    }

    #[must_use]
    pub const fn is_interactive(self) -> bool {
        matches!(self, Self::InteractivePreview | Self::VisibleFullView)
    }

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::InteractivePreview => 0,
            Self::VisibleFullView => 1,
            Self::UserExport => 2,
            Self::VisibleThumbnail => 3,
            Self::PrefetchThumbnail => 4,
            Self::BackgroundAnalysis => 5,
            Self::Maintenance => 6,
        }
    }
}

/// External cancellation authority supplied by #272 or a later runtime
/// adapter. The scheduler observes this boundary but never owns generation
/// counters or publication state.
pub trait CancellationBoundary: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

impl CancellationBoundary for crate::CancellationToken {
    fn is_cancelled(&self) -> bool {
        Self::is_cancelled(self)
    }
}

/// The publication destination is identity-only metadata. Actual publication
/// permits remain owned by #272 and the cache/render/output owners.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PublicationTargetKind {
    None,
    Cache,
    UserInterface,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicationTarget {
    kind: PublicationTargetKind,
    identity: [u8; 32],
}

impl PublicationTarget {
    #[must_use]
    pub const fn none() -> Self {
        Self {
            kind: PublicationTargetKind::None,
            identity: [0; 32],
        }
    }

    #[must_use]
    pub const fn new(kind: PublicationTargetKind, identity: [u8; 32]) -> Self {
        Self { kind, identity }
    }

    #[must_use]
    pub const fn kind(self) -> PublicationTargetKind {
        self.kind
    }

    #[must_use]
    pub const fn identity(self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(u64);

impl TaskId {
    /// Creates a nonzero task identifier.
    pub const fn new(value: u64) -> Result<Self, TaskError> {
        if value == 0 {
            Err(TaskError::ZeroId)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RequestId(u64);

impl RequestId {
    pub const fn new(value: u64) -> Result<Self, TaskError> {
        if value == 0 {
            Err(TaskError::ZeroRequestId)
        } else {
            Ok(Self(value))
        }
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One planned #269 lease family. The scheduler reserves the estimate before
/// execution; the owner confirms actual bytes at the execution boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LeaseClaim {
    planned_bytes: u64,
    lease_count: u16,
}

impl LeaseClaim {
    pub const fn new(planned_bytes: u64, lease_count: u16) -> Result<Self, TaskError> {
        if planned_bytes == 0 || lease_count == 0 {
            Err(TaskError::InvalidLeaseClaim)
        } else {
            Ok(Self {
                planned_bytes,
                lease_count,
            })
        }
    }

    #[must_use]
    pub const fn planned_bytes(self) -> u64 {
        self.planned_bytes
    }

    #[must_use]
    pub const fn lease_count(self) -> u16 {
        self.lease_count
    }
}

/// Resources admitted as one atomic task claim. No task can grow this claim
/// after admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceClaim {
    memory_bytes: u64,
    worker_tokens: u16,
    max_parallelism: u16,
    active_pipeline: bool,
    leases: Vec<LeaseClaim>,
}

impl ResourceClaim {
    pub fn new(
        memory_bytes: u64,
        worker_tokens: u16,
        max_parallelism: u16,
        active_pipeline: bool,
        leases: Vec<LeaseClaim>,
    ) -> Result<Self, TaskError> {
        if memory_bytes == 0
            || worker_tokens == 0
            || max_parallelism == 0
            || max_parallelism > worker_tokens
            || leases.len() > 64
        {
            return Err(TaskError::InvalidResourceClaim);
        }
        let lease_bytes = leases.iter().try_fold(0_u64, |total, lease| {
            total
                .checked_add(lease.planned_bytes)
                .ok_or(TaskError::ArithmeticOverflow)
        })?;
        if lease_bytes > memory_bytes {
            return Err(TaskError::LeaseExceedsEstimate);
        }
        Ok(Self {
            memory_bytes,
            worker_tokens,
            max_parallelism,
            active_pipeline,
            leases,
        })
    }

    #[must_use]
    pub const fn memory_bytes(&self) -> u64 {
        self.memory_bytes
    }

    #[must_use]
    pub const fn worker_tokens(&self) -> u16 {
        self.worker_tokens
    }

    #[must_use]
    pub const fn max_parallelism(&self) -> u16 {
        self.max_parallelism
    }

    #[must_use]
    pub const fn active_pipeline(&self) -> bool {
        self.active_pipeline
    }

    #[must_use]
    pub fn leases(&self) -> &[LeaseClaim] {
        &self.leases
    }
}

#[derive(Clone)]
pub struct TaskSpec {
    task_id: TaskId,
    request_id: RequestId,
    snapshot_identity: PipelineSnapshotIdentity,
    generation: PipelineGeneration,
    priority: CpuPriority,
    work_units: u32,
    dependencies: Vec<TaskId>,
    resources: ResourceClaim,
    publication_target: PublicationTarget,
    cancellation: Option<Arc<dyn CancellationBoundary>>,
}

impl fmt::Debug for TaskSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskSpec")
            .field("task_id", &self.task_id)
            .field("request_id", &self.request_id)
            .field("snapshot_identity", &self.snapshot_identity)
            .field("generation", &self.generation)
            .field("priority", &self.priority)
            .field("work_units", &self.work_units)
            .field("dependencies", &self.dependencies)
            .field("resources", &self.resources)
            .field("publication_target", &self.publication_target)
            .field("has_cancellation", &self.cancellation.is_some())
            .finish()
    }
}

impl TaskSpec {
    pub fn new(
        task_id: TaskId,
        request_id: RequestId,
        snapshot_identity: PipelineSnapshotIdentity,
        generation: PipelineGeneration,
        priority: CpuPriority,
        work_units: u32,
        resources: ResourceClaim,
    ) -> Result<Self, TaskError> {
        if work_units == 0 {
            return Err(TaskError::ZeroWorkUnits);
        }
        Ok(Self {
            task_id,
            request_id,
            snapshot_identity,
            generation,
            priority,
            work_units,
            dependencies: Vec::new(),
            resources,
            publication_target: PublicationTarget::none(),
            cancellation: None,
        })
    }

    pub fn with_dependencies(mut self, dependencies: Vec<TaskId>) -> Result<Self, TaskError> {
        if dependencies.len() > 64 || dependencies.contains(&self.task_id) {
            return Err(TaskError::InvalidDependencies);
        }
        let mut unique = dependencies.clone();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != dependencies.len() {
            return Err(TaskError::InvalidDependencies);
        }
        self.dependencies = dependencies;
        Ok(self)
    }

    #[must_use]
    pub const fn with_publication_target(mut self, target: PublicationTarget) -> Self {
        self.publication_target = target;
        self
    }

    #[must_use]
    pub fn with_cancellation(mut self, cancellation: Arc<dyn CancellationBoundary>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    #[must_use]
    pub const fn task_id(&self) -> TaskId {
        self.task_id
    }

    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }

    #[must_use]
    pub const fn snapshot_identity(&self) -> PipelineSnapshotIdentity {
        self.snapshot_identity
    }

    #[must_use]
    pub const fn generation(&self) -> PipelineGeneration {
        self.generation
    }

    #[must_use]
    pub const fn priority(&self) -> CpuPriority {
        self.priority
    }

    #[must_use]
    pub const fn work_units(&self) -> u32 {
        self.work_units
    }

    #[must_use]
    pub fn dependencies(&self) -> &[TaskId] {
        &self.dependencies
    }

    #[must_use]
    pub const fn resources(&self) -> &ResourceClaim {
        &self.resources
    }

    #[must_use]
    pub const fn publication_target(&self) -> PublicationTarget {
        self.publication_target
    }

    #[must_use]
    pub fn cancellation(&self) -> Option<&Arc<dyn CancellationBoundary>> {
        self.cancellation.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerConfig {
    total_queue_limit: usize,
    per_class_queue_limit: usize,
    active_pipeline_limit: u16,
    worker_limit: u16,
    memory_limit: u64,
    receipt_limit: usize,
    aging_after: Duration,
}

impl SchedulerConfig {
    pub fn new(
        total_queue_limit: usize,
        per_class_queue_limit: usize,
        active_pipeline_limit: u16,
        worker_limit: u16,
        memory_limit: u64,
    ) -> Result<Self, SchedulerConfigError> {
        if total_queue_limit == 0
            || per_class_queue_limit == 0
            || per_class_queue_limit > total_queue_limit
            || active_pipeline_limit == 0
            || worker_limit == 0
            || memory_limit == 0
        {
            return Err(SchedulerConfigError::InvalidLimits);
        }
        Ok(Self {
            total_queue_limit,
            per_class_queue_limit,
            active_pipeline_limit,
            worker_limit,
            memory_limit,
            receipt_limit: 256,
            aging_after: Duration::from_secs(2),
        })
    }

    #[must_use]
    pub const fn with_receipt_limit(mut self, limit: usize) -> Self {
        self.receipt_limit = limit;
        self
    }

    pub const fn with_aging_after(
        mut self,
        aging_after: Duration,
    ) -> Result<Self, SchedulerConfigError> {
        if aging_after.is_zero() {
            return Err(SchedulerConfigError::InvalidAging);
        }
        self.aging_after = aging_after;
        Ok(self)
    }

    #[must_use]
    pub const fn total_queue_limit(self) -> usize {
        self.total_queue_limit
    }
    #[must_use]
    pub const fn per_class_queue_limit(self) -> usize {
        self.per_class_queue_limit
    }
    #[must_use]
    pub const fn active_pipeline_limit(self) -> u16 {
        self.active_pipeline_limit
    }
    #[must_use]
    pub const fn worker_limit(self) -> u16 {
        self.worker_limit
    }
    #[must_use]
    pub const fn memory_limit(self) -> u64 {
        self.memory_limit
    }
    #[must_use]
    pub const fn receipt_limit(self) -> usize {
        self.receipt_limit
    }
    #[must_use]
    pub const fn aging_after(self) -> Duration {
        self.aging_after
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerConfigError {
    InvalidLimits,
    InvalidAging,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskError {
    ZeroId,
    ZeroRequestId,
    ZeroWorkUnits,
    InvalidDependencies,
    InvalidLeaseClaim,
    InvalidResourceClaim,
    LeaseExceedsEstimate,
    ArithmeticOverflow,
}

impl fmt::Display for TaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid scheduler task: {self:?}")
    }
}

impl std::error::Error for TaskError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    Queued,
    Running,
    Passed,
    Failed,
    Cancelled,
    Skipped,
}

impl TaskState {
    #[must_use]
    pub const fn terminal(self) -> bool {
        matches!(
            self,
            Self::Passed | Self::Failed | Self::Cancelled | Self::Skipped
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskFailure {
    ResourceRequestTooLarge,
    MemoryEstimateMismatch {
        planned_bytes: u64,
        actual_bytes: u64,
    },
    WorkUnitFailed,
    PanicIsolated,
    DependencyFailed,
    Shutdown,
    AccountingCorruption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkUnitBoundary {
    Continue,
    Cancelled,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownMode {
    DrainVisible,
    AbortAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunningTask {
    pub(crate) task_id: TaskId,
    pub(crate) worker_tokens: u16,
    pub(crate) max_parallelism: u16,
    pub(crate) work_units: u32,
}

impl RunningTask {
    #[must_use]
    pub const fn task_id(self) -> TaskId {
        self.task_id
    }
    #[must_use]
    pub const fn worker_tokens(self) -> u16 {
        self.worker_tokens
    }
    #[must_use]
    pub const fn max_parallelism(self) -> u16 {
        self.max_parallelism
    }
    #[must_use]
    pub const fn work_units(self) -> u32 {
        self.work_units
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmitReceipt {
    pub(crate) task_id: TaskId,
    pub(crate) request_id: RequestId,
    pub(crate) priority: CpuPriority,
    pub(crate) enqueue_sequence: u64,
    pub(crate) admitted_memory_bytes: u64,
}

impl AdmitReceipt {
    #[must_use]
    pub const fn task_id(self) -> TaskId {
        self.task_id
    }
    #[must_use]
    pub const fn request_id(self) -> RequestId {
        self.request_id
    }
    #[must_use]
    pub const fn priority(self) -> CpuPriority {
        self.priority
    }
    #[must_use]
    pub const fn enqueue_sequence(self) -> u64 {
        self.enqueue_sequence
    }
    #[must_use]
    pub const fn admitted_memory_bytes(self) -> u64 {
        self.admitted_memory_bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerReceipt {
    pub(crate) task_id: TaskId,
    pub(crate) request_id: RequestId,
    pub(crate) priority: CpuPriority,
    pub(crate) state: TaskState,
    pub(crate) enqueue_sequence: u64,
    pub(crate) queue_wait: Duration,
    pub(crate) run_time: Duration,
    pub(crate) admitted_memory_bytes: u64,
    pub(crate) worker_tokens: u16,
    pub(crate) dependency_count: u16,
    pub(crate) publication_allowed: bool,
    pub(crate) failure: Option<TaskFailure>,
}

impl SchedulerReceipt {
    #[must_use]
    pub const fn task_id(&self) -> TaskId {
        self.task_id
    }
    #[must_use]
    pub const fn request_id(&self) -> RequestId {
        self.request_id
    }
    #[must_use]
    pub const fn priority(&self) -> CpuPriority {
        self.priority
    }
    #[must_use]
    pub const fn state(&self) -> TaskState {
        self.state
    }
    #[must_use]
    pub const fn enqueue_sequence(&self) -> u64 {
        self.enqueue_sequence
    }
    #[must_use]
    pub const fn queue_wait(&self) -> Duration {
        self.queue_wait
    }
    #[must_use]
    pub const fn run_time(&self) -> Duration {
        self.run_time
    }
    #[must_use]
    pub const fn admitted_memory_bytes(&self) -> u64 {
        self.admitted_memory_bytes
    }
    #[must_use]
    pub const fn worker_tokens(&self) -> u16 {
        self.worker_tokens
    }
    #[must_use]
    pub const fn dependency_count(&self) -> u16 {
        self.dependency_count
    }
    #[must_use]
    pub const fn publication_allowed(&self) -> bool {
        self.publication_allowed
    }
    #[must_use]
    pub fn failure(&self) -> Option<&TaskFailure> {
        self.failure.as_ref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedulerError {
    InvalidConfig(SchedulerConfigError),
    DuplicateTask(TaskId),
    MissingDependency(TaskId),
    DependencyCycle(TaskId),
    QueueFull,
    PriorityQueueFull(CpuPriority),
    ResourceRequestTooLarge,
    MemoryReservationUnavailable,
    PostShutdown,
    UnknownTask(TaskId),
    NotRunning(TaskId),
    LeaseMismatch {
        planned_bytes: u64,
        actual_bytes: u64,
    },
    ReceiptLimit,
    AccountingCorruption,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "scheduler error: {self:?}")
    }
}

impl std::error::Error for SchedulerError {}
