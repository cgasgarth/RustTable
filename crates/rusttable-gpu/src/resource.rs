use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::DeviceClass;

#[path = "resource_admin.rs"]
mod resource_admin;

const DISCRETE_DEFAULT_BUDGET: u64 = 1024 * 1024 * 1024;
const UNIFIED_DEFAULT_BUDGET: u64 = 512 * 1024 * 1024;
const SOFT_BUDGET_PERCENT: u8 = 80;
const DEFAULT_BACKEND_OVERHEAD: u64 = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceGeneration(u64);

impl DeviceGeneration {
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceKind {
    Buffer,
    Texture,
    TextureView,
    BindGroup,
    Staging,
    Readback,
}

impl ResourceKind {
    const fn is_byte_backed(self) -> bool {
        matches!(
            self,
            Self::Buffer | Self::Texture | Self::Staging | Self::Readback
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceState {
    Creating,
    ExclusiveLeased,
    ImmutableLeased,
    InFlight,
    Pooled,
    Mapped,
    Evicted,
    Lost,
    Poisoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceFormat {
    Raw,
    Rgba16Float,
    R32Float,
    Rgba32Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceClass {
    pub generation: DeviceGeneration,
    pub kind: ResourceKind,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub format: ResourceFormat,
    pub usage: u64,
    pub mip_level_count: u32,
    pub sample_count: u32,
    pub alignment: u64,
    pub mapped: bool,
    pub compatibility: u64,
}

impl ResourceClass {
    #[must_use]
    pub const fn buffer(generation: DeviceGeneration, size_bytes: u64, usage: u64) -> Self {
        Self {
            generation,
            kind: ResourceKind::Buffer,
            size_bytes,
            width: 0,
            height: 0,
            depth: 0,
            format: ResourceFormat::Raw,
            usage,
            mip_level_count: 1,
            sample_count: 1,
            alignment: 1,
            mapped: false,
            compatibility: 0,
        }
    }

    #[must_use]
    pub const fn texture(
        generation: DeviceGeneration,
        dimensions: [u32; 3],
        format: ResourceFormat,
        usage: u64,
        mip_level_count: u32,
        sample_count: u32,
    ) -> Self {
        Self {
            generation,
            kind: ResourceKind::Texture,
            size_bytes: 0,
            width: dimensions[0],
            height: dimensions[1],
            depth: dimensions[2],
            format,
            usage,
            mip_level_count,
            sample_count,
            alignment: 256,
            mapped: false,
            compatibility: 0,
        }
    }

    #[must_use]
    pub const fn with_size(mut self, size_bytes: u64) -> Self {
        self.size_bytes = size_bytes;
        self
    }

    #[must_use]
    pub const fn with_alignment(mut self, alignment: u64) -> Self {
        self.alignment = alignment;
        self
    }

    #[must_use]
    pub const fn with_mapping(mut self, mapped: bool) -> Self {
        self.mapped = mapped;
        self
    }

    #[must_use]
    pub const fn with_compatibility(mut self, compatibility: u64) -> Self {
        self.compatibility = compatibility;
        self
    }

    #[must_use]
    pub const fn accounted_bytes(self, overhead: u64) -> Option<u64> {
        if self.kind.is_byte_backed() {
            self.size_bytes.checked_add(overhead)
        } else {
            Some(overhead)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InitializationPolicy {
    FullOverwrite,
    Zeroed,
    Uploaded,
    PreservedImmutable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourcePriority {
    Background,
    Normal,
    Interactive,
    Protected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourcePoolConfig {
    pub configured_cap: u64,
    pub memory_hint: Option<u64>,
    pub addressable_limit: u64,
    pub backend_overhead: u64,
    pub soft_percent: u8,
}

impl ResourcePoolConfig {
    #[must_use]
    pub const fn conservative(device_class: DeviceClass) -> Self {
        let configured_cap = match device_class {
            DeviceClass::Discrete => DISCRETE_DEFAULT_BUDGET,
            DeviceClass::Integrated
            | DeviceClass::Virtual
            | DeviceClass::Cpu
            | DeviceClass::Other => UNIFIED_DEFAULT_BUDGET,
        };
        Self {
            configured_cap,
            memory_hint: None,
            addressable_limit: u64::MAX,
            backend_overhead: DEFAULT_BACKEND_OVERHEAD,
            soft_percent: SOFT_BUDGET_PERCENT,
        }
    }

    #[must_use]
    pub const fn hard_budget(self) -> u64 {
        let with_hint = match self.memory_hint {
            Some(value) if value < self.configured_cap => value,
            _ => self.configured_cap,
        };
        if self.addressable_limit < with_hint {
            self.addressable_limit
        } else {
            with_hint
        }
    }

    #[must_use]
    pub const fn soft_budget(self) -> u64 {
        self.hard_budget().saturating_mul(self.soft_percent as u64) / 100
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceId {
    pub generation: DeviceGeneration,
    pub index: u64,
    pub kind: ResourceKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SubmissionId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceRequest {
    pub class: ResourceClass,
    pub initialization: InitializationPolicy,
    pub priority: ResourcePriority,
    pub pinned: bool,
    pub one_shot: bool,
}

impl ResourceRequest {
    #[must_use]
    pub const fn new(class: ResourceClass, initialization: InitializationPolicy) -> Self {
        Self {
            class,
            initialization,
            priority: ResourcePriority::Normal,
            pinned: false,
            one_shot: false,
        }
    }
    #[must_use]
    pub const fn priority(mut self, value: ResourcePriority) -> Self {
        self.priority = value;
        self
    }
    #[must_use]
    pub const fn pinned(mut self, value: bool) -> Self {
        self.pinned = value;
        self
    }
    #[must_use]
    pub const fn one_shot(mut self, value: bool) -> Self {
        self.one_shot = value;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResourceAccounting {
    pub resident_bytes: u64,
    pub idle_bytes: u64,
    pub in_flight_bytes: u64,
    pub staging_bytes: u64,
    pub readback_bytes: u64,
    pub view_bytes: u64,
    pub bind_group_bytes: u64,
    pub leases: usize,
    pub pooled: usize,
    pub evictions: u64,
    pub reuse_count: u64,
    pub submissions: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ResourceMetrics {
    pub accounting: ResourceAccounting,
    pub soft_pressure: bool,
    pub hard_budget: u64,
    pub soft_budget: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolEvent {
    Acquired(ResourceId),
    Reused(ResourceId),
    Evicted(ResourceId),
    Submitted(ResourceId, SubmissionId),
    Retired(ResourceId, SubmissionId),
    Lost(DeviceGeneration),
    Poisoned(ResourceId),
    BudgetRejected(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolError {
    InvalidDescriptor(&'static str),
    ArithmeticOverflow,
    BudgetExceeded {
        requested: u64,
        hard: u64,
    },
    UnsupportedGeneration {
        expected: DeviceGeneration,
        actual: DeviceGeneration,
    },
    InvalidTransition {
        id: ResourceId,
        state: ResourceState,
        operation: &'static str,
    },
    UnknownResource(ResourceId),
    DoubleReturn(ResourceId),
    AlreadyRetired(SubmissionId),
    DeviceLost(DeviceGeneration),
    Shutdown,
    AllocationFailed(String),
}

impl fmt::Display for PoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDescriptor(reason) => {
                write!(f, "invalid GPU resource descriptor: {reason}")
            }
            Self::ArithmeticOverflow => f.write_str("GPU resource accounting overflow"),
            Self::BudgetExceeded { requested, hard } => {
                write!(f, "GPU resource budget exceeded: {requested} > {hard}")
            }
            Self::UnsupportedGeneration { expected, actual } => write!(
                f,
                "GPU resource generation mismatch: expected {}, got {}",
                expected.value(),
                actual.value()
            ),
            Self::InvalidTransition {
                id,
                state,
                operation,
            } => write!(
                f,
                "invalid GPU resource transition {id:?} from {state:?} during {operation}"
            ),
            Self::UnknownResource(id) => write!(f, "unknown GPU resource {id:?}"),
            Self::DoubleReturn(id) => write!(f, "GPU resource was returned twice: {id:?}"),
            Self::AlreadyRetired(id) => write!(f, "submission was retired twice: {id:?}"),
            Self::DeviceLost(generation) => {
                write!(f, "GPU device generation {} is lost", generation.value())
            }
            Self::Shutdown => f.write_str("GPU resource pool is shut down"),
            Self::AllocationFailed(reason) => write!(f, "GPU resource allocation failed: {reason}"),
        }
    }
}

impl std::error::Error for PoolError {}

#[derive(Debug)]
struct Entry {
    request: ResourceRequest,
    state: ResourceState,
    accounted_bytes: u64,
    leases: usize,
    last_used: u64,
    submission: Option<SubmissionId>,
}

#[derive(Debug)]
struct PoolState {
    generation: DeviceGeneration,
    config: ResourcePoolConfig,
    entries: BTreeMap<ResourceId, Entry>,
    idle: BTreeSet<ResourceId>,
    submissions: BTreeMap<SubmissionId, Vec<ResourceId>>,
    events: VecDeque<PoolEvent>,
    accounting: ResourceAccounting,
    next_index: u64,
    next_submission: u64,
    clock: u64,
    shutdown: bool,
}

#[derive(Debug)]
struct PoolShared {
    state: Mutex<PoolState>,
    wake: Condvar,
}

#[derive(Debug, Clone)]
pub struct GpuResourcePool {
    shared: Arc<PoolShared>,
}

impl GpuResourcePool {
    #[must_use]
    pub fn new(generation: DeviceGeneration, config: ResourcePoolConfig) -> Self {
        Self {
            shared: Arc::new(PoolShared {
                state: Mutex::new(PoolState {
                    generation,
                    config,
                    entries: BTreeMap::new(),
                    idle: BTreeSet::new(),
                    submissions: BTreeMap::new(),
                    events: VecDeque::new(),
                    accounting: ResourceAccounting::default(),
                    next_index: 1,
                    next_submission: 1,
                    clock: 0,
                    shutdown: false,
                }),
                wake: Condvar::new(),
            }),
        }
    }

    #[must_use]
    pub fn generation(&self) -> DeviceGeneration {
        lock(&self.shared.state).generation
    }

    pub fn try_acquire(&self, request: ResourceRequest) -> Result<ResourceLease, PoolError> {
        let mut state = lock(&self.shared.state);
        let id = acquire_entry(&mut state, request)?;
        Ok(ResourceLease::from_id(Arc::clone(&self.shared), id))
    }

    /// # Panics
    ///
    /// Panics only if the internal resource-pool mutex is poisoned.
    pub fn acquire(
        &self,
        request: ResourceRequest,
        timeout: Duration,
    ) -> Result<ResourceLease, PoolError> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.try_acquire(request) {
                Ok(lease) => return Ok(lease),
                Err(PoolError::BudgetExceeded { .. }) if timeout > Duration::ZERO => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return self.try_acquire(request);
                    }
                    let state = lock(&self.shared.state);
                    let (state, _) = self
                        .shared
                        .wake
                        .wait_timeout(state, remaining.min(Duration::from_millis(10)))
                        .expect("resource pool mutex is not poisoned");
                    drop(state);
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub fn share(&self, lease: &ResourceLease) -> Result<ResourceLease, PoolError> {
        let id = lease.id.ok_or(PoolError::DoubleReturn(lease.original_id))?;
        let mut state = lock(&self.shared.state);
        let generation = state.generation;
        let entry = state
            .entries
            .get_mut(&id)
            .ok_or(PoolError::UnknownResource(id))?;
        if entry.request.class.generation != generation {
            return Err(PoolError::DeviceLost(generation));
        }
        if entry.state != ResourceState::ImmutableLeased
            || entry.request.initialization == InitializationPolicy::FullOverwrite
        {
            return Err(PoolError::InvalidTransition {
                id,
                state: entry.state,
                operation: "share",
            });
        }
        entry.leases = entry
            .leases
            .checked_add(1)
            .ok_or(PoolError::ArithmeticOverflow)?;
        state.accounting.leases = state.accounting.leases.saturating_add(1);
        Ok(ResourceLease::from_id(Arc::clone(&self.shared), id))
    }

    #[must_use]
    pub fn metrics(&self) -> ResourceMetrics {
        let state = lock(&self.shared.state);
        ResourceMetrics {
            accounting: state.accounting,
            soft_pressure: state.accounting.resident_bytes > state.config.soft_budget(),
            hard_budget: state.config.hard_budget(),
            soft_budget: state.config.soft_budget(),
        }
    }

    #[must_use]
    pub fn drain_events(&self) -> Vec<PoolEvent> {
        lock(&self.shared.state).events.drain(..).collect()
    }

    pub fn lose_device(&self, generation: DeviceGeneration) -> Result<(), PoolError> {
        let mut state = lock(&self.shared.state);
        if generation != state.generation {
            return Err(PoolError::UnsupportedGeneration {
                expected: state.generation,
                actual: generation,
            });
        }
        let ids = state.entries.keys().copied().collect::<Vec<_>>();
        for entry in state.entries.values_mut() {
            entry.state = ResourceState::Lost;
        }
        for id in ids {
            state.idle.remove(&id);
        }
        state.events.push_back(PoolEvent::Lost(generation));
        state.generation = DeviceGeneration::new(generation.value().saturating_add(1));
        self.shared.wake.notify_all();
        Ok(())
    }

    /// # Panics
    ///
    /// Panics only if the internal resource-pool mutex is poisoned.
    pub fn shutdown(&self, timeout: Duration) -> Result<ResourceAccounting, PoolError> {
        let deadline = Instant::now() + timeout;
        let mut state = lock(&self.shared.state);
        state.shutdown = true;
        evict_idle(&mut state, u64::MAX);
        while state.accounting.leases != 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            state = self
                .shared
                .wake
                .wait_timeout(state, remaining.min(Duration::from_millis(10)))
                .expect("resource pool mutex is not poisoned")
                .0;
        }
        if state.accounting.leases != 0 {
            return Err(PoolError::AllocationFailed(
                "shutdown timed out with live leases".to_owned(),
            ));
        }
        Ok(state.accounting)
    }
}

#[derive(Debug)]
pub struct ResourceLease {
    pool: Arc<PoolShared>,
    id: Option<ResourceId>,
    original_id: ResourceId,
}

impl ResourceLease {
    fn new(pool: Arc<PoolShared>, id: ResourceId) -> Self {
        Self {
            pool,
            id: Some(id),
            original_id: id,
        }
    }

    #[must_use]
    pub fn id(&self) -> ResourceId {
        self.id.unwrap_or(self.original_id)
    }

    pub fn initialize(&mut self) -> Result<(), PoolError> {
        transition(
            &self.pool,
            self.id(),
            ResourceState::ExclusiveLeased,
            "initialize",
            |entry| entry.request.initialization != InitializationPolicy::FullOverwrite,
        )
    }

    pub fn freeze(&mut self) -> Result<(), PoolError> {
        transition(
            &self.pool,
            self.id(),
            ResourceState::ImmutableLeased,
            "freeze",
            |entry| entry.state == ResourceState::ExclusiveLeased,
        )
    }

    pub fn map(&mut self) -> Result<(), PoolError> {
        transition(
            &self.pool,
            self.id(),
            ResourceState::Mapped,
            "map",
            |entry| {
                matches!(
                    entry.state,
                    ResourceState::ExclusiveLeased | ResourceState::ImmutableLeased
                ) && entry.submission.is_none()
            },
        )
    }

    pub fn unmap(&mut self) -> Result<(), PoolError> {
        transition(
            &self.pool,
            self.id(),
            ResourceState::ExclusiveLeased,
            "unmap",
            |entry| entry.state == ResourceState::Mapped,
        )
    }

    pub fn submit(mut self) -> Result<SubmissionToken, PoolError> {
        let id = self
            .id
            .take()
            .ok_or(PoolError::DoubleReturn(self.original_id))?;
        let mut state = lock(&self.pool.state);
        let submission = SubmissionId(state.next_submission);
        state.next_submission = state.next_submission.saturating_add(1);
        let bytes = {
            let entry = state
                .entries
                .get_mut(&id)
                .ok_or(PoolError::UnknownResource(id))?;
            if !matches!(
                entry.state,
                ResourceState::ExclusiveLeased | ResourceState::ImmutableLeased
            ) || entry.submission.is_some()
            {
                return Err(PoolError::InvalidTransition {
                    id,
                    state: entry.state,
                    operation: "submit",
                });
            }
            entry.state = ResourceState::InFlight;
            entry.submission = Some(submission);
            entry.accounted_bytes
        };
        state.submissions.entry(submission).or_default().push(id);
        state.accounting.submissions = state.accounting.submissions.saturating_add(1);
        state.accounting.in_flight_bytes = state.accounting.in_flight_bytes.saturating_add(bytes);
        state.events.push_back(PoolEvent::Submitted(id, submission));
        Ok(SubmissionToken {
            pool: Arc::clone(&self.pool),
            id: submission,
            retired: false,
        })
    }

    pub fn release(mut self) -> Result<(), PoolError> {
        let id = self
            .id
            .take()
            .ok_or(PoolError::DoubleReturn(self.original_id))?;
        release_id(&self.pool, id)
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            let _ = release_id(&self.pool, id);
        }
    }
}

#[derive(Debug)]
pub struct SubmissionToken {
    pool: Arc<PoolShared>,
    id: SubmissionId,
    retired: bool,
}

impl SubmissionToken {
    #[must_use]
    pub const fn id(&self) -> SubmissionId {
        self.id
    }

    pub fn retire(mut self) -> Result<(), PoolError> {
        if self.retired {
            return Err(PoolError::AlreadyRetired(self.id));
        }
        self.retired = true;
        retire_submission(&self.pool, self.id)
    }
}

impl Drop for SubmissionToken {
    fn drop(&mut self) {
        if !self.retired {
            let _ = retire_submission(&self.pool, self.id);
            self.retired = true;
        }
    }
}

fn acquire_entry(state: &mut PoolState, request: ResourceRequest) -> Result<ResourceId, PoolError> {
    validate_request(state, request)?;
    state.clock = state.clock.saturating_add(1);
    let reusable = state
        .idle
        .iter()
        .copied()
        .filter(|id| {
            state.entries.get(id).is_some_and(|entry| {
                entry.request.class == request.class
                    && entry.request.priority == request.priority
                    && !entry.request.pinned
            })
        })
        .min_by_key(|id| {
            state
                .entries
                .get(id)
                .map_or(u64::MAX, |entry| entry.accounted_bytes)
        });
    if let Some(id) = reusable {
        state.idle.remove(&id);
        let entry = state
            .entries
            .get_mut(&id)
            .ok_or(PoolError::UnknownResource(id))?;
        entry.state = if request.initialization == InitializationPolicy::PreservedImmutable {
            ResourceState::ImmutableLeased
        } else {
            ResourceState::ExclusiveLeased
        };
        entry.leases = 1;
        entry.last_used = state.clock;
        state.accounting.idle_bytes = state
            .accounting
            .idle_bytes
            .saturating_sub(entry.accounted_bytes);
        state.accounting.leases = state.accounting.leases.saturating_add(1);
        state.accounting.pooled = state.accounting.pooled.saturating_sub(1);
        state.accounting.reuse_count = state.accounting.reuse_count.saturating_add(1);
        state.events.push_back(PoolEvent::Reused(id));
        return Ok(id);
    }
    let bytes = request
        .class
        .accounted_bytes(state.config.backend_overhead)
        .ok_or(PoolError::ArithmeticOverflow)?;
    if bytes == 0 {
        return Err(PoolError::InvalidDescriptor("resource has zero size"));
    }
    if bytes > state.config.hard_budget() && !request.one_shot {
        return Err(PoolError::BudgetExceeded {
            requested: bytes,
            hard: state.config.hard_budget(),
        });
    }
    evict_idle(state, bytes);
    let resident = state
        .accounting
        .resident_bytes
        .checked_add(bytes)
        .ok_or(PoolError::ArithmeticOverflow)?;
    if resident > state.config.hard_budget() && !request.one_shot {
        state.events.push_back(PoolEvent::BudgetRejected(bytes));
        return Err(PoolError::BudgetExceeded {
            requested: resident,
            hard: state.config.hard_budget(),
        });
    }
    let id = ResourceId {
        generation: state.generation,
        index: state.next_index,
        kind: request.class.kind,
    };
    state.next_index = state.next_index.saturating_add(1);
    let entry = Entry {
        request,
        state: if request.initialization == InitializationPolicy::PreservedImmutable {
            ResourceState::ImmutableLeased
        } else {
            ResourceState::ExclusiveLeased
        },
        accounted_bytes: bytes,
        leases: 1,
        last_used: state.clock,
        submission: None,
    };
    state.entries.insert(id, entry);
    state.accounting.resident_bytes = resident;
    state.accounting.leases = state.accounting.leases.saturating_add(1);
    state.events.push_back(PoolEvent::Acquired(id));
    Ok(id)
}

fn validate_request(state: &PoolState, request: ResourceRequest) -> Result<(), PoolError> {
    if state.shutdown {
        return Err(PoolError::Shutdown);
    }
    if request.class.generation != state.generation {
        return Err(PoolError::UnsupportedGeneration {
            expected: state.generation,
            actual: request.class.generation,
        });
    }
    if request.class.alignment == 0 || !request.class.alignment.is_power_of_two() {
        return Err(PoolError::InvalidDescriptor(
            "alignment must be a nonzero power of two",
        ));
    }
    if request.class.kind == ResourceKind::Texture
        && (request.class.width == 0 || request.class.height == 0 || request.class.depth == 0)
    {
        return Err(PoolError::InvalidDescriptor(
            "texture dimensions must be nonzero",
        ));
    }
    Ok(())
}

fn evict_idle(state: &mut PoolState, needed: u64) {
    while state.accounting.resident_bytes.saturating_add(needed) > state.config.hard_budget()
        || state.accounting.resident_bytes > state.config.soft_budget()
    {
        let candidate = state
            .idle
            .iter()
            .copied()
            .filter(|id| {
                state
                    .entries
                    .get(id)
                    .is_some_and(|entry| !entry.request.pinned)
            })
            .max_by_key(|id| {
                state.entries.get(id).map_or((0, 0, 0), |entry| {
                    (
                        entry.request.priority as u8,
                        entry.accounted_bytes,
                        u8::MAX - entry.request.class.kind as u8,
                    )
                })
            });
        let Some(id) = candidate else { break };
        if let Some(entry) = state.entries.remove(&id) {
            state.idle.remove(&id);
            state.accounting.resident_bytes = state
                .accounting
                .resident_bytes
                .saturating_sub(entry.accounted_bytes);
            state.accounting.idle_bytes = state
                .accounting
                .idle_bytes
                .saturating_sub(entry.accounted_bytes);
            state.accounting.pooled = state.accounting.pooled.saturating_sub(1);
            state.accounting.evictions = state.accounting.evictions.saturating_add(1);
            state.events.push_back(PoolEvent::Evicted(id));
        }
    }
}

fn transition<F: FnOnce(&Entry) -> bool>(
    pool: &Arc<PoolShared>,
    id: ResourceId,
    next: ResourceState,
    operation: &'static str,
    valid: F,
) -> Result<(), PoolError> {
    let mut state = lock(&pool.state);
    let entry = state
        .entries
        .get_mut(&id)
        .ok_or(PoolError::UnknownResource(id))?;
    if !valid(entry) {
        return Err(PoolError::InvalidTransition {
            id,
            state: entry.state,
            operation,
        });
    }
    entry.state = next;
    Ok(())
}

fn release_id(pool: &Arc<PoolShared>, id: ResourceId) -> Result<(), PoolError> {
    let mut state = lock(&pool.state);
    let (bytes, remove) = {
        let entry = state
            .entries
            .get_mut(&id)
            .ok_or(PoolError::UnknownResource(id))?;
        if entry.leases == 0 {
            return Err(PoolError::DoubleReturn(id));
        }
        if entry.state == ResourceState::InFlight || entry.submission.is_some() {
            return Err(PoolError::InvalidTransition {
                id,
                state: entry.state,
                operation: "release before retire",
            });
        }
        entry.leases -= 1;
        let remove = entry.leases == 0
            && matches!(
                entry.state,
                ResourceState::Lost | ResourceState::Poisoned | ResourceState::Evicted
            );
        (entry.accounted_bytes, remove)
    };
    state.accounting.leases = state.accounting.leases.saturating_sub(1);
    if remove {
        state.entries.remove(&id);
        state.accounting.resident_bytes = state.accounting.resident_bytes.saturating_sub(bytes);
    } else if let Some(entry) = state.entries.get_mut(&id)
        && entry.leases == 0
    {
        entry.state = ResourceState::Pooled;
        state.idle.insert(id);
        state.accounting.idle_bytes = state.accounting.idle_bytes.saturating_add(bytes);
        state.accounting.pooled = state.accounting.pooled.saturating_add(1);
    }
    state.clock = state.clock.saturating_add(1);
    evict_idle(&mut state, 0);
    pool.wake.notify_all();
    Ok(())
}

fn retire_submission(pool: &Arc<PoolShared>, submission: SubmissionId) -> Result<(), PoolError> {
    let mut state = lock(&pool.state);
    let ids = state
        .submissions
        .remove(&submission)
        .ok_or(PoolError::AlreadyRetired(submission))?;
    for id in ids {
        let bytes = if let Some(entry) = state.entries.get_mut(&id) {
            if entry.submission != Some(submission) {
                continue;
            }
            entry.submission = None;
            entry.state = ResourceState::Pooled;
            entry.leases = 0;
            entry.accounted_bytes
        } else {
            continue;
        };
        state.idle.insert(id);
        state.accounting.in_flight_bytes = state.accounting.in_flight_bytes.saturating_sub(bytes);
        state.accounting.idle_bytes = state.accounting.idle_bytes.saturating_add(bytes);
        state.accounting.pooled = state.accounting.pooled.saturating_add(1);
        state.events.push_back(PoolEvent::Retired(id, submission));
    }
    state.accounting.submissions = state.accounting.submissions.saturating_sub(1);
    pool.wake.notify_all();
    Ok(())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().expect("resource pool mutex is not poisoned")
}

impl ResourceLease {
    fn from_id(pool: Arc<PoolShared>, id: ResourceId) -> Self {
        Self::new(pool, id)
    }
}
