use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::{ImageDescriptor, PixelFormat};

pub const MIN_HOST_ALIGNMENT: usize = 64;
const MAX_HOST_ALIGNMENT: usize = 512;

/// A supported host allocation alignment. Alignments are explicit classes so
/// callers cannot silently rely on an operation-specific SIMD alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BufferAlignment(usize);

impl BufferAlignment {
    pub const CACHE_LINE: Self = Self(MIN_HOST_ALIGNMENT);

    /// Creates a power-of-two alignment supported by the safe storage adapter.
    ///
    /// # Errors
    ///
    /// Returns [`HostPoolError::InvalidAlignment`] for an alignment below the
    /// cache-line guarantee, a non-power-of-two value, or an unsupported size.
    pub const fn new(value: usize) -> Result<Self, HostPoolError> {
        if value < MIN_HOST_ALIGNMENT || value > MAX_HOST_ALIGNMENT || !value.is_power_of_two() {
            return Err(HostPoolError::InvalidAlignment { requested: value });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn bytes(self) -> usize {
        self.0
    }
}

/// Ownership/accounting class for a host buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BufferUsage {
    Image,
    Temporary,
    Pixelpipe,
}

/// The validated image format is part of the pool key. Two formats never
/// reuse one another's storage, even when their byte counts happen to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AllocationClass {
    format: PixelFormat,
    alignment: BufferAlignment,
    usage: BufferUsage,
}

impl AllocationClass {
    #[must_use]
    pub const fn new(format: PixelFormat, alignment: BufferAlignment, usage: BufferUsage) -> Self {
        Self {
            format,
            alignment,
            usage,
        }
    }

    #[must_use]
    pub const fn format(self) -> PixelFormat {
        self.format
    }

    #[must_use]
    pub const fn alignment(self) -> BufferAlignment {
        self.alignment
    }

    #[must_use]
    pub const fn usage(self) -> BufferUsage {
        self.usage
    }
}

/// Initialization is tracked independently from the physical bytes. Even
/// though the safe adapter starts its blocks in a deterministic state, an
/// uninitialized lease cannot read them until its owner publishes validity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InitializationPolicy {
    Uninitialized,
    Zeroed,
    Callback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PriorityClass {
    Background,
    Normal,
    Interactive,
}

/// A checked request for one image or temporary allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferRequest {
    class: AllocationClass,
    bytes: usize,
    initialization: InitializationPolicy,
    priority: PriorityClass,
}

impl BufferRequest {
    /// Creates a request with exact planned bytes and a typed allocation class.
    ///
    /// # Errors
    ///
    /// Returns [`HostPoolError::InvalidRequest`] for a zero byte request.
    pub const fn new(
        class: AllocationClass,
        bytes: usize,
        initialization: InitializationPolicy,
        priority: PriorityClass,
    ) -> Result<Self, HostPoolError> {
        if bytes == 0 {
            return Err(HostPoolError::InvalidRequest);
        }
        Ok(Self {
            class,
            bytes,
            initialization,
            priority,
        })
    }

    /// Builds an image request from the complete checked descriptor.
    ///
    /// # Errors
    ///
    /// Returns an invalid request error if the descriptor has no bytes.
    pub fn for_image(
        descriptor: &ImageDescriptor,
        alignment: BufferAlignment,
        usage: BufferUsage,
        initialization: InitializationPolicy,
        priority: PriorityClass,
    ) -> Result<Self, HostPoolError> {
        Self::new(
            AllocationClass::new(descriptor.format(), alignment, usage),
            descriptor.byte_length(),
            initialization,
            priority,
        )
    }

    #[must_use]
    pub const fn class(self) -> AllocationClass {
        self.class
    }

    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    #[must_use]
    pub const fn initialization(self) -> InitializationPolicy {
        self.initialization
    }

    #[must_use]
    pub const fn priority(self) -> PriorityClass {
        self.priority
    }
}

/// Pool-wide hard limits. Allocated capacity includes both outstanding and
/// idle entries; it is never allowed to exceed `total_allocated_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolBudgets {
    total_allocated_bytes: usize,
    idle_bytes: usize,
    max_entries: usize,
    per_class_bytes: usize,
    per_request_bytes: usize,
    per_request_entries: usize,
}

impl PoolBudgets {
    /// Creates budgets with checked nonzero allocation and request limits.
    /// `idle_bytes` and `max_entries` may be zero to disable retention.
    ///
    /// # Errors
    ///
    /// Returns [`HostPoolError::InvalidBudgets`] for zero or inconsistent limits.
    pub const fn new(
        total_allocated_bytes: usize,
        idle_bytes: usize,
        max_entries: usize,
        per_class_bytes: usize,
        per_request_bytes: usize,
        per_request_entries: usize,
    ) -> Result<Self, HostPoolError> {
        if total_allocated_bytes == 0
            || per_class_bytes == 0
            || per_request_bytes == 0
            || per_request_entries == 0
            || idle_bytes > total_allocated_bytes
            || per_class_bytes > total_allocated_bytes
            || per_request_bytes > total_allocated_bytes
        {
            return Err(HostPoolError::InvalidBudgets);
        }
        Ok(Self {
            total_allocated_bytes,
            idle_bytes,
            max_entries,
            per_class_bytes,
            per_request_bytes,
            per_request_entries,
        })
    }

    #[must_use]
    pub const fn total_allocated_bytes(self) -> usize {
        self.total_allocated_bytes
    }
    #[must_use]
    pub const fn idle_bytes(self) -> usize {
        self.idle_bytes
    }
    #[must_use]
    pub const fn max_entries(self) -> usize {
        self.max_entries
    }
    #[must_use]
    pub const fn per_class_bytes(self) -> usize {
        self.per_class_bytes
    }
    #[must_use]
    pub const fn per_request_bytes(self) -> usize {
        self.per_request_bytes
    }
    #[must_use]
    pub const fn per_request_entries(self) -> usize {
        self.per_request_entries
    }
}

/// A cooperative cancellation source for a blocked acquire.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Waiting behavior for a blocking acquire.
#[derive(Debug, Clone, Default)]
pub struct AcquireOptions {
    priority: Option<PriorityClass>,
    timeout: Option<Duration>,
    cancellation: Option<CancellationToken>,
}

impl AcquireOptions {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            priority: None,
            timeout: None,
            cancellation: None,
        }
    }

    #[must_use]
    pub const fn with_priority(mut self, priority: PriorityClass) -> Self {
        self.priority = Some(priority);
        self
    }

    #[must_use]
    pub const fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    #[must_use]
    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    #[must_use]
    pub const fn priority(&self) -> Option<PriorityClass> {
        self.priority
    }

    #[must_use]
    pub const fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    #[must_use]
    pub const fn cancellation(&self) -> Option<&CancellationToken> {
        self.cancellation.as_ref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeaseState {
    ExclusiveUninitialized,
    ExclusiveInitialized,
    SharedImmutable,
    Returning,
    Poisoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PoolAccounting {
    pub(crate) allocated_bytes: usize,
    pub(crate) idle_bytes: usize,
    pub(crate) outstanding_bytes: usize,
    pub(crate) entries: usize,
    pub(crate) outstanding_entries: usize,
    pub(crate) reuse_count: u64,
    pub(crate) eviction_count: u64,
    pub(crate) poison_count: u64,
    pub(crate) failure_count: u64,
    pub(crate) wait_count: u64,
}

impl PoolAccounting {
    #[must_use]
    pub const fn allocated_bytes(self) -> usize {
        self.allocated_bytes
    }
    #[must_use]
    pub const fn idle_bytes(self) -> usize {
        self.idle_bytes
    }
    #[must_use]
    pub const fn outstanding_bytes(self) -> usize {
        self.outstanding_bytes
    }
    #[must_use]
    pub const fn entries(self) -> usize {
        self.entries
    }
    #[must_use]
    pub const fn outstanding_entries(self) -> usize {
        self.outstanding_entries
    }
    #[must_use]
    pub const fn reuse_count(self) -> u64 {
        self.reuse_count
    }
    #[must_use]
    pub const fn eviction_count(self) -> u64 {
        self.eviction_count
    }
    #[must_use]
    pub const fn poison_count(self) -> u64 {
        self.poison_count
    }
    #[must_use]
    pub const fn failure_count(self) -> u64 {
        self.failure_count
    }
    #[must_use]
    pub const fn wait_count(self) -> u64 {
        self.wait_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolEvent {
    Pressure {
        allocated_bytes: usize,
        total_bytes: usize,
    },
    BudgetUpdated,
    Evicted {
        bytes: usize,
    },
    Poisoned {
        bytes: usize,
    },
    AllocationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReturnReceipt {
    pub(crate) lease_id: u64,
    pub(crate) pooled: bool,
    pub(crate) capacity: usize,
    pub(crate) poisoned: bool,
}

impl ReturnReceipt {
    #[must_use]
    pub const fn lease_id(self) -> u64 {
        self.lease_id
    }
    #[must_use]
    pub const fn pooled(self) -> bool {
        self.pooled
    }
    #[must_use]
    pub const fn capacity(self) -> usize {
        self.capacity
    }
    #[must_use]
    pub const fn poisoned(self) -> bool {
        self.poisoned
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownReport {
    pub(crate) outstanding_entries: usize,
    pub(crate) outstanding_bytes: usize,
    pub(crate) idle_entries_freed: usize,
    pub(crate) idle_bytes_freed: usize,
    pub(crate) timed_out: bool,
}

impl ShutdownReport {
    #[must_use]
    pub const fn outstanding_entries(self) -> usize {
        self.outstanding_entries
    }
    #[must_use]
    pub const fn outstanding_bytes(self) -> usize {
        self.outstanding_bytes
    }
    #[must_use]
    pub const fn idle_entries_freed(self) -> usize {
        self.idle_entries_freed
    }
    #[must_use]
    pub const fn idle_bytes_freed(self) -> usize {
        self.idle_bytes_freed
    }
    #[must_use]
    pub const fn timed_out(self) -> bool {
        self.timed_out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPoolError {
    InvalidAlignment { requested: usize },
    UnsupportedAlignment { requested: usize },
    InvalidRequest,
    InvalidBudgets,
    ArithmeticOverflow,
    DescriptorMismatch,
    WouldBlock,
    Cancelled,
    DeadlineExceeded,
    Shutdown,
    BudgetExceeded,
    AllocationFailed,
    InvalidTransition { state: LeaseState },
    AlreadyReturned,
    Poisoned,
    CallbackFailed,
    CallbackPanicked,
    EntryLimitExceeded,
    AccountingCorruption,
}

impl fmt::Display for HostPoolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidAlignment { requested } => {
                write!(formatter, "invalid host alignment {requested}")
            }
            Self::UnsupportedAlignment { requested } => {
                write!(formatter, "unsupported host alignment {requested}")
            }
            Self::InvalidRequest => formatter.write_str("invalid host buffer request"),
            Self::InvalidBudgets => formatter.write_str("invalid host pool budgets"),
            Self::ArithmeticOverflow => formatter.write_str("host pool arithmetic overflowed"),
            Self::DescriptorMismatch => {
                formatter.write_str("host buffer descriptor does not match its request")
            }
            Self::WouldBlock => {
                formatter.write_str("host buffer pool is unavailable without waiting")
            }
            Self::Cancelled => formatter.write_str("host buffer acquire was cancelled"),
            Self::DeadlineExceeded => formatter.write_str("host buffer acquire deadline elapsed"),
            Self::Shutdown => formatter.write_str("host buffer pool is shut down"),
            Self::BudgetExceeded => formatter.write_str("host buffer request exceeds its budget"),
            Self::AllocationFailed => formatter.write_str("host buffer allocation failed"),
            Self::InvalidTransition { state } => {
                write!(formatter, "invalid host lease transition from {state:?}")
            }
            Self::AlreadyReturned => formatter.write_str("host lease was already returned"),
            Self::Poisoned => formatter.write_str("host lease is poisoned"),
            Self::CallbackFailed => {
                formatter.write_str("host buffer initialization callback failed")
            }
            Self::CallbackPanicked => {
                formatter.write_str("host buffer initialization callback panicked")
            }
            Self::EntryLimitExceeded => formatter.write_str("host pool entry limit exceeded"),
            Self::AccountingCorruption => {
                formatter.write_str("host pool accounting invariant failed")
            }
        }
    }
}

impl std::error::Error for HostPoolError {}

pub(crate) fn checked_bucket_request(bytes: usize) -> Result<usize, HostPoolError> {
    if bytes == 0 {
        return Err(HostPoolError::InvalidRequest);
    }
    let sixteen_mib = 16 * 1024 * 1024;
    if bytes <= sixteen_mib {
        bytes
            .checked_next_power_of_two()
            .ok_or(HostPoolError::ArithmeticOverflow)
    } else {
        let target = bytes
            .checked_add(bytes / 2)
            .ok_or(HostPoolError::ArithmeticOverflow)?;
        let mut bucket = bytes;
        while bucket < target {
            bucket = bucket
                .checked_add(bucket / 2)
                .ok_or(HostPoolError::ArithmeticOverflow)?;
        }
        Ok(bucket)
    }
}

pub(crate) fn within_waste_limit(capacity: usize, bytes: usize) -> bool {
    capacity <= bytes.saturating_add(bytes / 4)
}
