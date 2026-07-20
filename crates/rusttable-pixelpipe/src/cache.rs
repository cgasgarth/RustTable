#![allow(clippy::missing_errors_doc)]

use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::cache_value::{CacheValue, CancellationToken, ValueDescriptor};
use crate::{CacheKey, CacheKeyDigest, ImplementationIdentity};
use crate::{CancellationError, CancellationReason, CancellationStage, CleanupRegistration};

const DEFAULT_BUDGET: u64 = 64 * 1024 * 1024;
const DEFAULT_FAILURE_WINDOW: Duration = Duration::from_millis(250);
const MAX_RECEIPTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Segment {
    Probationary,
    Protected,
}

struct Entry {
    value: Arc<dyn Any + Send + Sync>,
    type_id: TypeId,
    descriptor: ValueDescriptor,
    cost: u64,
    segment: Segment,
    access: u64,
    pin_count: Arc<AtomicUsize>,
}

struct FailureEntry {
    expires: Instant,
    message: String,
}

struct FlightState {
    completed: bool,
    error: Option<CacheError>,
}

struct InFlight {
    token: CancellationToken,
    state: Mutex<FlightState>,
    wake: Condvar,
    consumers: Mutex<Consumers>,
}

struct Consumers {
    next_id: u64,
    active: BTreeSet<u64>,
}

struct ConsumerRegistration {
    flight: Arc<InFlight>,
    id: u64,
    hook: Option<CleanupRegistration>,
}

impl ConsumerRegistration {
    fn register(flight: &Arc<InFlight>, token: &CancellationToken) -> Result<Self, CacheError> {
        let id = {
            let mut consumers = flight.consumers.lock().map_err(|_| CacheError::Poisoned)?;
            let id = consumers.next_id;
            consumers.next_id = consumers.next_id.saturating_add(1);
            consumers.active.insert(id);
            id
        };
        let weak = Arc::downgrade(flight);
        let hook = token.register_cleanup(move |reason| {
            if let Some(flight) = weak.upgrade() {
                flight.release_consumer(id, reason);
            }
        });
        Ok(Self {
            flight: flight.clone(),
            id,
            hook: Some(hook),
        })
    }
}

impl Drop for ConsumerRegistration {
    fn drop(&mut self) {
        self.hook.take();
        self.flight
            .release_consumer(self.id, CancellationReason::NoConsumers);
    }
}

impl InFlight {
    fn release_consumer(&self, id: u64, reason: CancellationReason) {
        let empty = self.consumers.lock().map_or(true, |mut consumers| {
            consumers.active.remove(&id) && consumers.active.is_empty()
        });
        if empty {
            self.token.cancel_with_reason(reason);
        }
    }
}

struct Inner {
    entries: HashMap<CacheKey, Entry>,
    in_flight: HashMap<CacheKey, Arc<InFlight>>,
    failures: HashMap<CacheKey, FailureEntry>,
    receipts: VecDeque<CacheReceipt>,
    budget: u64,
    access: u64,
    invalidation_sequence: u64,
    invalidations: VecDeque<(u64, CacheScope)>,
    shutdown: bool,
}

/// Bounded in-memory segmented-LRU pixelpipe result cache.
pub struct Cache {
    inner: Mutex<Inner>,
    failure_window: Duration,
    receipt_sequence: AtomicU64,
}

impl fmt::Debug for Cache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Cache")
            .field("metrics", &self.metrics())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheConfig {
    budget_bytes: u64,
    failure_window: Duration,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            budget_bytes: DEFAULT_BUDGET,
            failure_window: DEFAULT_FAILURE_WINDOW,
        }
    }
}

impl CacheConfig {
    #[must_use]
    pub const fn new(budget_bytes: u64) -> Self {
        Self {
            budget_bytes,
            failure_window: DEFAULT_FAILURE_WINDOW,
        }
    }
    #[must_use]
    pub const fn with_failure_window(mut self, value: Duration) -> Self {
        self.failure_window = value;
        self
    }
}

impl Cache {
    #[must_use]
    pub fn new(config: CacheConfig) -> Self {
        Self {
            inner: Mutex::new(Inner {
                entries: HashMap::new(),
                in_flight: HashMap::new(),
                failures: HashMap::new(),
                receipts: VecDeque::new(),
                budget: config.budget_bytes,
                access: 0,
                invalidation_sequence: 0,
                invalidations: VecDeque::new(),
                shutdown: false,
            }),
            failure_window: config.failure_window.min(DEFAULT_FAILURE_WINDOW),
            receipt_sequence: AtomicU64::new(0),
        }
    }

    /// Returns a pinned typed value when the full structured key matches.
    pub fn lookup<T: CacheValue>(
        &self,
        key: &CacheKey,
    ) -> Result<Option<CacheLease<T>>, CacheError> {
        let mut inner = self.lock_inner()?;
        let Some(entry) = inner.entries.remove(key) else {
            self.record_locked(&mut inner, CacheEvent::Miss, key);
            return Ok(None);
        };
        if entry.type_id != TypeId::of::<T>() {
            inner.entries.insert(key.clone(), entry);
            return Err(CacheError::TypeMismatch);
        }
        let value = entry
            .value
            .clone()
            .downcast::<T>()
            .map_err(|_| CacheError::TypeMismatch)?;
        let promoted = entry.segment == Segment::Probationary;
        let pin_count = entry.pin_count.clone();
        pin_count.fetch_add(1, Ordering::AcqRel);
        inner.access = inner.access.saturating_add(1);
        let access = inner.access;
        let segment = if promoted {
            Segment::Protected
        } else {
            entry.segment
        };
        inner.entries.insert(
            key.clone(),
            Entry {
                value: value.clone(),
                type_id: TypeId::of::<T>(),
                descriptor: entry.descriptor,
                cost: entry.cost,
                segment,
                access,
                pin_count: pin_count.clone(),
            },
        );
        self.record_locked(
            &mut inner,
            if promoted {
                CacheEvent::Promotion
            } else {
                CacheEvent::Hit
            },
            key,
        );
        Ok(Some(CacheLease {
            key: key.clone(),
            value,
            descriptor: entry.descriptor,
            pin_count,
            cached: true,
        }))
    }

    /// Builds one exact key at most once at a time. Waiters can cancel their
    /// own wait without cancelling the shared build.
    #[allow(clippy::needless_pass_by_value)]
    pub fn get_or_build<T, F>(
        &self,
        key: CacheKey,
        cancellation: &CancellationToken,
        builder: F,
    ) -> Result<CacheLease<T>, CacheError>
    where
        T: CacheValue,
        F: FnOnce(&CancellationToken) -> Result<T, CacheError>,
    {
        if cancellation.is_cancelled() {
            return Err(CacheError::Cancelled);
        }
        let (flight, owner, started_at) = {
            let mut inner = self.lock_inner()?;
            if inner.shutdown {
                return Err(CacheError::Shutdown);
            }
            if let Some(failure) = inner.failures.get(&key)
                && failure.expires > Instant::now()
            {
                return Err(CacheError::SuppressedFailure(failure.message.clone()));
            }
            if !inner.failures.contains_key(&key)
                && let Some(lease) = self.lookup_locked::<T>(&mut inner, &key)?
            {
                return Ok(lease);
            }
            if let Some(flight) = inner.in_flight.get(&key) {
                (flight.clone(), false, inner.invalidation_sequence)
            } else {
                let flight = Arc::new(InFlight {
                    token: CancellationToken::for_generation(key.generation()),
                    state: Mutex::new(FlightState {
                        completed: false,
                        error: None,
                    }),
                    wake: Condvar::new(),
                    consumers: Mutex::new(Consumers {
                        next_id: 1,
                        active: BTreeSet::new(),
                    }),
                });
                inner.in_flight.insert(key.clone(), flight.clone());
                (flight, true, inner.invalidation_sequence)
            }
        };
        if !owner {
            let registration = ConsumerRegistration::register(&flight, cancellation)?;
            return self.wait_for_flight::<T>(&key, &flight, cancellation, registration);
        }

        let registration = ConsumerRegistration::register(&flight, cancellation)?;

        let result = catch_unwind(AssertUnwindSafe(|| builder(&flight.token)))
            .map_err(|_| CacheError::BuilderPanicked)
            .and_then(std::convert::identity)
            .and_then(|value| self.publish(key.clone(), value, started_at, &flight.token));
        drop(registration);
        {
            let mut inner = self.lock_inner()?;
            inner.in_flight.remove(&key);
            if let Err(error) = &result
                && matches!(
                    error,
                    CacheError::BuildFailed(_)
                        | CacheError::BuilderPanicked
                        | CacheError::InvalidValue(_)
                )
            {
                inner.failures.insert(
                    key.clone(),
                    FailureEntry {
                        expires: Instant::now() + self.failure_window,
                        message: error.to_string(),
                    },
                );
            }
        }
        let mut state = flight.state.lock().map_err(|_| CacheError::Poisoned)?;
        state.completed = true;
        state.error = result.as_ref().err().cloned();
        flight.wake.notify_all();
        drop(state);
        result
    }

    fn wait_for_flight<T: CacheValue>(
        &self,
        key: &CacheKey,
        flight: &Arc<InFlight>,
        cancellation: &CancellationToken,
        _registration: ConsumerRegistration,
    ) -> Result<CacheLease<T>, CacheError> {
        loop {
            if cancellation.is_cancelled() {
                return Err(CacheError::Cancelled);
            }
            let state = flight.state.lock().map_err(|_| CacheError::Poisoned)?;
            if state.completed {
                if let Some(error) = &state.error {
                    return Err(error.clone());
                }
                drop(state);
                return self
                    .lookup(key)
                    .and_then(|result| result.ok_or(CacheError::BuildNotPublished));
            }
            let (_guard, timeout) = flight
                .wake
                .wait_timeout(state, Duration::from_millis(5))
                .map_err(|_| CacheError::Poisoned)?;
            if timeout.timed_out() {
                thread::yield_now();
            }
        }
    }

    fn publish<T: CacheValue>(
        &self,
        key: CacheKey,
        value: T,
        started_at: u64,
        shared_token: &CancellationToken,
    ) -> Result<CacheLease<T>, CacheError> {
        shared_token
            .check(CancellationStage::CachePromotion)
            .map_err(CacheError::Cancellation)?;
        value.validate().map_err(CacheError::InvalidValue)?;
        let descriptor = value.descriptor();
        let cost = descriptor.total_bytes()?;
        if !descriptor.cacheable() {
            return Err(CacheError::NotCacheable);
        }
        let mut inner = self.lock_inner()?;
        inner.in_flight.remove(&key);
        if inner.shutdown {
            return Err(CacheError::Shutdown);
        }
        if inner
            .invalidations
            .iter()
            .any(|(sequence, scope)| *sequence > started_at && key.matches(scope))
        {
            self.record_locked(&mut inner, CacheEvent::StalePublication, &key);
            return Err(CacheError::StalePublication);
        }
        if cost > inner.budget {
            inner.failures.remove(&key);
            self.record_locked(&mut inner, CacheEvent::OversizeDirect, &key);
            let value = Arc::new(value);
            let pin_count = Arc::new(AtomicUsize::new(1));
            return Ok(CacheLease {
                key,
                value,
                descriptor,
                pin_count,
                cached: false,
            });
        }
        self.evict_for_locked(&mut inner, cost)?;
        inner.failures.remove(&key);
        inner.access = inner.access.saturating_add(1);
        let access = inner.access;
        let value = Arc::new(value);
        let pin_count = Arc::new(AtomicUsize::new(1));
        inner.entries.insert(
            key.clone(),
            Entry {
                value: value.clone(),
                type_id: TypeId::of::<T>(),
                descriptor,
                cost,
                segment: Segment::Probationary,
                access,
                pin_count: pin_count.clone(),
            },
        );
        self.record_locked(&mut inner, CacheEvent::Publish, &key);
        Ok(CacheLease {
            key,
            value,
            descriptor,
            pin_count,
            cached: true,
        })
    }

    fn lookup_locked<T: CacheValue>(
        &self,
        inner: &mut Inner,
        key: &CacheKey,
    ) -> Result<Option<CacheLease<T>>, CacheError> {
        let Some(entry) = inner.entries.remove(key) else {
            self.record_locked(inner, CacheEvent::Miss, key);
            return Ok(None);
        };
        if entry.type_id != TypeId::of::<T>() {
            inner.entries.insert(key.clone(), entry);
            return Err(CacheError::TypeMismatch);
        }
        let value = entry
            .value
            .clone()
            .downcast::<T>()
            .map_err(|_| CacheError::TypeMismatch)?;
        let promoted = entry.segment == Segment::Probationary;
        let pin_count = entry.pin_count.clone();
        pin_count.fetch_add(1, Ordering::AcqRel);
        inner.access = inner.access.saturating_add(1);
        let access = inner.access;
        let segment = if promoted {
            Segment::Protected
        } else {
            entry.segment
        };
        let descriptor = entry.descriptor;
        let cost = entry.cost;
        inner.entries.insert(
            key.clone(),
            Entry {
                value: value.clone(),
                type_id: TypeId::of::<T>(),
                descriptor,
                cost,
                segment,
                access,
                pin_count: pin_count.clone(),
            },
        );
        self.record_locked(
            inner,
            if promoted {
                CacheEvent::Promotion
            } else {
                CacheEvent::Hit
            },
            key,
        );
        Ok(Some(CacheLease {
            key: key.clone(),
            value,
            descriptor,
            pin_count,
            cached: true,
        }))
    }

    fn evict_for_locked(&self, inner: &mut Inner, requested: u64) -> Result<(), CacheError> {
        let resident = resident_bytes(inner);
        if resident
            .checked_add(requested)
            .ok_or(CacheError::CostOverflow)?
            > inner.budget
        {
            self.evict_until_locked(inner, requested)?;
        }
        if resident_bytes(inner)
            .checked_add(requested)
            .ok_or(CacheError::CostOverflow)?
            > inner.budget
        {
            return Err(CacheError::OverBudgetPinned);
        }
        Ok(())
    }

    fn evict_until_locked(&self, inner: &mut Inner, requested: u64) -> Result<(), CacheError> {
        loop {
            let total = resident_bytes(inner);
            if total
                .checked_add(requested)
                .ok_or(CacheError::CostOverflow)?
                <= inner.budget
            {
                return Ok(());
            }
            let candidate = inner
                .entries
                .iter()
                .filter(|(_, entry)| entry.pin_count.load(Ordering::Acquire) == 0)
                .min_by_key(|(key, entry)| {
                    (
                        entry.segment != Segment::Probationary,
                        entry.access,
                        key.canonical_bytes(),
                    )
                })
                .map(|(key, _)| key.clone());
            let Some(key) = candidate else {
                return Err(CacheError::OverBudgetPinned);
            };
            inner.entries.remove(&key);
            self.record_locked(inner, CacheEvent::Eviction, &key);
        }
    }

    /// Applies natural key misses for the requested scope and invalidates
    /// matching in-flight publications before they can become entries.
    pub fn invalidate(&self, scope: CacheScope) -> Result<InvalidationReceipt, CacheError> {
        let mut inner = self.lock_inner()?;
        inner.invalidation_sequence = inner.invalidation_sequence.saturating_add(1);
        let sequence = inner.invalidation_sequence;
        let mut removed = 0_usize;
        inner.entries.retain(|key, _| {
            let keep = !key.matches(&scope);
            if !keep {
                removed = removed.saturating_add(1);
            }
            keep
        });
        inner.invalidations.push_back((sequence, scope.clone()));
        while inner.invalidations.len() > MAX_RECEIPTS {
            inner.invalidations.pop_front();
        }
        self.record_locked(
            &mut inner,
            CacheEvent::Invalidation,
            &CacheKey::diagnostic_sentinel(),
        );
        Ok(InvalidationReceipt {
            sequence,
            removed,
            scope,
        })
    }

    pub fn set_budget(&self, budget: u64) -> Result<CacheMetrics, CacheError> {
        let mut inner = self.lock_inner()?;
        inner.budget = budget;
        let _ = self.evict_until_locked(&mut inner, 0);
        Ok(metrics_locked(&inner))
    }

    #[must_use]
    pub fn metrics(&self) -> CacheMetrics {
        self.inner
            .lock()
            .map(|inner| metrics_locked(&inner))
            .unwrap_or_default()
    }

    #[must_use]
    pub fn receipts(&self) -> Vec<CacheReceipt> {
        self.inner
            .lock()
            .map(|inner| inner.receipts.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn shutdown(&self) -> Result<ShutdownReport, CacheError> {
        let mut inner = self.lock_inner()?;
        inner.shutdown = true;
        let entries = inner.entries.len();
        let pinned = inner
            .entries
            .values()
            .filter(|entry| entry.pin_count.load(Ordering::Acquire) > 0)
            .count();
        inner
            .entries
            .retain(|_, entry| entry.pin_count.load(Ordering::Acquire) > 0);
        Ok(ShutdownReport {
            entries_released: entries.saturating_sub(pinned),
            pinned_entries: pinned,
            in_flight_builds: inner.in_flight.len(),
        })
    }

    fn record_locked(&self, inner: &mut Inner, event: CacheEvent, key: &CacheKey) {
        let sequence = self.receipt_sequence.fetch_add(1, Ordering::Relaxed);
        inner.receipts.push_back(CacheReceipt {
            sequence,
            event,
            key: key.diagnostic_sha256(),
            components: u8::try_from(key.components().len()).expect("component count is bounded"),
            segment: None,
            bytes: 0,
        });
        while inner.receipts.len() > MAX_RECEIPTS {
            inner.receipts.pop_front();
        }
    }

    fn lock_inner(&self) -> Result<std::sync::MutexGuard<'_, Inner>, CacheError> {
        self.inner.lock().map_err(|_| CacheError::Poisoned)
    }
}

/// A typed immutable lease. A lease pins its entry; dropping all clones makes
/// it eligible for deterministic eviction.
pub struct CacheLease<T: CacheValue> {
    key: CacheKey,
    value: Arc<T>,
    descriptor: ValueDescriptor,
    pin_count: Arc<AtomicUsize>,
    cached: bool,
}

impl<T: CacheValue> fmt::Debug for CacheLease<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CacheLease")
            .field("key", &self.key.diagnostic_sha256())
            .field("descriptor", &self.descriptor)
            .field("cached", &self.cached)
            .field("pin_count", &self.pin_count())
            .finish_non_exhaustive()
    }
}

impl<T: CacheValue> Clone for CacheLease<T> {
    fn clone(&self) -> Self {
        self.pin_count.fetch_add(1, Ordering::AcqRel);
        Self {
            key: self.key.clone(),
            value: self.value.clone(),
            descriptor: self.descriptor,
            pin_count: self.pin_count.clone(),
            cached: self.cached,
        }
    }
}

impl<T: CacheValue> Drop for CacheLease<T> {
    fn drop(&mut self) {
        self.pin_count.fetch_sub(1, Ordering::AcqRel);
    }
}

impl<T: CacheValue> CacheLease<T> {
    #[must_use]
    pub fn value(&self) -> &T {
        &self.value
    }
    #[must_use]
    pub fn key(&self) -> &CacheKey {
        &self.key
    }
    #[must_use]
    pub const fn descriptor(&self) -> ValueDescriptor {
        self.descriptor
    }
    #[must_use]
    pub const fn is_cached(&self) -> bool {
        self.cached
    }
    #[must_use]
    pub fn pin_count(&self) -> usize {
        self.pin_count.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheError {
    Cancelled,
    Cancellation(CancellationError),
    Shutdown,
    CostOverflow,
    OverBudgetPinned,
    TypeMismatch,
    BuildNotPublished,
    BuilderPanicked,
    BuildFailed(String),
    SuppressedFailure(String),
    InvalidValue(String),
    NotCacheable,
    StalePublication,
    Poisoned,
}

impl fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuildFailed(error) => write!(formatter, "pixelpipe cache build failed: {error}"),
            Self::SuppressedFailure(error) => {
                write!(formatter, "pixelpipe cache failure suppressed: {error}")
            }
            Self::InvalidValue(error) => {
                write!(formatter, "pixelpipe cache value is invalid: {error}")
            }
            other => write!(formatter, "pixelpipe cache error: {other:?}"),
        }
    }
}

impl std::error::Error for CacheError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheEvent {
    Hit,
    Miss,
    Promotion,
    Publish,
    Eviction,
    OversizeDirect,
    Invalidation,
    StalePublication,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheReceipt {
    sequence: u64,
    event: CacheEvent,
    key: CacheKeyDigest,
    components: u8,
    segment: Option<&'static str>,
    bytes: u64,
}

impl CacheReceipt {
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    #[must_use]
    pub const fn event(&self) -> CacheEvent {
        self.event
    }
    #[must_use]
    pub const fn key(&self) -> CacheKeyDigest {
        self.key
    }
    #[must_use]
    pub const fn component_count(&self) -> u8 {
        self.components
    }
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
    #[must_use]
    pub const fn segment(&self) -> Option<&'static str> {
        self.segment
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CacheMetrics {
    pub resident_bytes: u64,
    pub auxiliary_bytes: u64,
    pub entries: usize,
    pub probationary_entries: usize,
    pub protected_entries: usize,
    pub pinned_entries: usize,
    pub over_budget_pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidationReceipt {
    sequence: u64,
    removed: usize,
    scope: CacheScope,
}

impl InvalidationReceipt {
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
    #[must_use]
    pub const fn removed(&self) -> usize {
        self.removed
    }
    #[must_use]
    pub fn scope(&self) -> &CacheScope {
        &self.scope
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShutdownReport {
    pub entries_released: usize,
    pub pinned_entries: usize,
    pub in_flight_builds: usize,
}

/// The explicit invalidation domains supported by the product boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheScope {
    Source(crate::SourceIdentity),
    Snapshot(crate::PipelineGeneration),
    Implementation(ImplementationIdentity),
    Backend([u8; 32]),
    MemoryPressure,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureDiagnostic {
    pub key: CacheKeyDigest,
    pub message: String,
    pub expires_in: Duration,
}

fn resident_bytes(inner: &Inner) -> u64 {
    inner.entries.values().map(|entry| entry.cost).sum()
}

fn metrics_locked(inner: &Inner) -> CacheMetrics {
    let resident_bytes = resident_bytes(inner);
    let auxiliary_bytes = inner
        .entries
        .values()
        .map(|entry| entry.descriptor.auxiliary_bytes())
        .sum();
    let probationary_entries = inner
        .entries
        .values()
        .filter(|entry| entry.segment == Segment::Probationary)
        .count();
    let protected_entries = inner
        .entries
        .values()
        .filter(|entry| entry.segment == Segment::Protected)
        .count();
    let pinned_entries = inner
        .entries
        .values()
        .filter(|entry| entry.pin_count.load(Ordering::Acquire) > 0)
        .count();
    CacheMetrics {
        resident_bytes,
        auxiliary_bytes,
        entries: inner.entries.len(),
        probationary_entries,
        protected_entries,
        pinned_entries,
        over_budget_pinned: resident_bytes > inner.budget && pinned_entries > 0,
    }
}

impl CacheKey {
    fn diagnostic_sentinel() -> Self {
        // Only used to keep invalidation receipts privacy-safe; it is never
        // inserted or compared to a product key.
        CacheKey::builder()
            .source(crate::SourceIdentity::new([0; 32]))
            .source_descriptor([0])
            .snapshot(crate::PipelineSnapshotIdentity::from_bytes(&[0]))
            .generation(crate::PipelineGeneration::new(1).expect("nonzero"))
            .purpose(crate::PipelinePurpose::Preview)
            .quality(crate::CacheQuality::Draft)
            .precision(crate::CachePrecision::F32)
            .node(crate::NodeBoundary::whole(
                crate::ImplementationIdentity::new("sentinel", 1, "receipt").expect("identity"),
            ))
            .output(crate::OutputIdentity::new(
                rusttable_image::ImageDimensions::new(1, 1).expect("dimensions"),
                rusttable_image::Roi::full(
                    rusttable_image::ImageDimensions::new(1, 1).expect("dimensions"),
                ),
                rusttable_image::PixelFormat::rgba8(),
                crate::ColorIdentity::working(),
                [0; 32],
            ))
            .parameters(1, [0])
            .build()
            .expect("sentinel key")
    }
}
