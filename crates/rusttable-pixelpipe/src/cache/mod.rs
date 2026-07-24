#![allow(clippy::missing_errors_doc)]

use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

use rusttable_image::{
    CacheAllocation as CommonCacheAllocation, CacheCallbacks as CommonCacheCallbacks,
    CacheError as CommonCacheError, CacheLease as CommonCacheLease, CacheMode as CommonCacheMode,
    CacheReadLease as CommonCacheReadLease, CacheRemoveResult as CommonCacheRemoveResult,
    ConcurrentCache,
};

pub(crate) mod key;
pub(crate) mod value;

use self::value::{CacheValue, CancellationToken, ValueDescriptor};
use crate::{CacheKey, CacheKeyDigest, ImplementationIdentity};
use crate::{
    CancellationDeadline, CancellationError, CancellationReason, CancellationStage,
    CleanupRegistration,
};

const DEFAULT_BUDGET: u64 = 64 * 1024 * 1024;
const DEFAULT_FAILURE_WINDOW: Duration = Duration::from_millis(250);
const MAX_RECEIPTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Segment {
    Probationary,
    Protected,
}

struct StoredEntry {
    value: Arc<dyn Any + Send + Sync>,
    type_id: TypeId,
    descriptor: ValueDescriptor,
    segment: AtomicUsize,
    pin_count: Arc<AtomicUsize>,
}

impl StoredEntry {
    fn segment(&self) -> Segment {
        if self.segment.load(Ordering::Acquire) == 0 {
            Segment::Probationary
        } else {
            Segment::Protected
        }
    }

    fn promote(&self) -> bool {
        self.segment
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

struct ResidentState {
    pending: Mutex<HashMap<CacheKey, CommonCacheAllocation<StoredEntry>>>,
    explicit_removals: Mutex<HashSet<CacheKey>>,
    invalidating: Mutex<HashSet<CacheKey>>,
    receipts: Mutex<VecDeque<CacheReceipt>>,
    receipt_sequence: AtomicU64,
}

impl ResidentState {
    fn record(
        &self,
        event: CacheEvent,
        key: &CacheKey,
        descriptor: Option<ValueDescriptor>,
        segment: Option<Segment>,
    ) {
        let sequence = self.receipt_sequence.fetch_add(1, Ordering::Relaxed);
        let receipt = CacheReceipt {
            sequence,
            event,
            key: key.diagnostic_sha256(),
            components: u8::try_from(key.components().len()).expect("component count is bounded"),
            segment: segment.map(|value| match value {
                Segment::Probationary => "probationary",
                Segment::Protected => "protected",
            }),
            bytes: descriptor
                .and_then(|value| value.total_bytes().ok())
                .unwrap_or(0),
        };
        if let Ok(mut receipts) = self.receipts.lock() {
            receipts.push_back(receipt);
            while receipts.len() > MAX_RECEIPTS {
                receipts.pop_front();
            }
        }
    }
}

#[derive(Clone)]
struct ResidentCallbacks {
    state: Arc<ResidentState>,
}

impl CommonCacheCallbacks<CacheKey, StoredEntry> for ResidentCallbacks {
    type Error = CacheError;

    fn allocate(&self, key: &CacheKey) -> Result<CommonCacheAllocation<StoredEntry>, Self::Error> {
        self.state
            .pending
            .lock()
            .map_err(|_| CacheError::Poisoned)?
            .remove(key)
            .ok_or(CacheError::BuildNotPublished)
    }

    fn cleanup(&self, key: &CacheKey, allocation: CommonCacheAllocation<StoredEntry>) {
        let explicit = self
            .state
            .explicit_removals
            .lock()
            .is_ok_and(|mut removals| removals.remove(key));
        if !explicit {
            self.state.record(
                CacheEvent::Eviction,
                key,
                Some(allocation.value().descriptor),
                Some(allocation.value().segment()),
            );
        }
    }
}

type ResidentCache = ConcurrentCache<CacheKey, StoredEntry, ResidentCallbacks>;
type ResidentReadLease = CommonCacheReadLease<CacheKey, StoredEntry, ResidentCallbacks>;

struct FailureEntry {
    expires: Instant,
    message: String,
}

struct FlightSuccess {
    value: Arc<dyn Any + Send + Sync>,
    type_id: TypeId,
    descriptor: ValueDescriptor,
    pin_count: Arc<AtomicUsize>,
}

struct FlightState {
    completed: bool,
    error: Option<CacheError>,
    success: Option<FlightSuccess>,
}

struct InFlight {
    token: CancellationToken,
    state: Mutex<FlightState>,
    wake: Condvar,
    consumers: Mutex<Consumers>,
    deadlines: Arc<FlightDeadlines>,
    stale: AtomicBool,
}

struct Consumers {
    next_id: u64,
    active: BTreeSet<u64>,
}

struct FlightDeadlineState {
    deadlines: HashMap<u64, CancellationDeadline>,
    worker_started: bool,
    stopped: bool,
}

struct FlightDeadlines {
    state: Mutex<FlightDeadlineState>,
    wake: Condvar,
}

impl FlightDeadlines {
    fn new() -> Self {
        Self {
            state: Mutex::new(FlightDeadlineState {
                deadlines: HashMap::new(),
                worker_started: false,
                stopped: false,
            }),
            wake: Condvar::new(),
        }
    }

    fn register(
        self: &Arc<Self>,
        flight: &Arc<InFlight>,
        id: u64,
        deadline: CancellationDeadline,
    ) -> Result<(), CacheError> {
        let mut state = self.state.lock().map_err(|_| CacheError::Poisoned)?;
        state.deadlines.insert(id, deadline);
        if !state.worker_started {
            state.worker_started = true;
            let deadlines = self.clone();
            let flight = Arc::downgrade(flight);
            if thread::Builder::new()
                .name("rusttable-cache-deadlines".to_owned())
                .spawn(move || deadlines.run(&flight))
                .is_err()
            {
                state.worker_started = false;
                state.deadlines.remove(&id);
                return Err(CacheError::Poisoned);
            }
        }
        self.wake.notify_all();
        Ok(())
    }

    fn remove(&self, id: u64) {
        if let Ok(mut state) = self.state.lock()
            && state.deadlines.remove(&id).is_some()
        {
            self.wake.notify_all();
        }
    }

    fn stop(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.stopped = true;
            state.deadlines.clear();
            self.wake.notify_all();
        }
    }

    fn run(self: Arc<Self>, flight: &Weak<InFlight>) {
        loop {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            loop {
                if state.stopped {
                    return;
                }
                let Some(next) = state
                    .deadlines
                    .values()
                    .map(|deadline| deadline.instant())
                    .min()
                else {
                    let Ok(next_state) = self.wake.wait(state) else {
                        return;
                    };
                    state = next_state;
                    continue;
                };
                let remaining = next.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let Ok((next_state, _)) = self.wake.wait_timeout(state, remaining) else {
                    return;
                };
                state = next_state;
            }

            let now = Instant::now();
            let expired = state
                .deadlines
                .iter()
                .filter_map(|(id, deadline)| (deadline.instant() <= now).then_some(*id))
                .collect::<Vec<_>>();
            for id in &expired {
                state.deadlines.remove(id);
            }
            drop(state);

            let Some(flight) = flight.upgrade() else {
                return;
            };
            for id in expired {
                flight.release_consumer(id, CancellationReason::DeadlineExceeded);
            }
        }
    }
}

struct ConsumerRegistration {
    flight: Arc<InFlight>,
    id: u64,
    hook: Option<CleanupRegistration>,
}

impl ConsumerRegistration {
    fn register(
        flight: &Arc<InFlight>,
        token: &CancellationToken,
        deadline: Option<CancellationDeadline>,
    ) -> Result<Self, CacheError> {
        let id = {
            let mut consumers = flight.consumers.lock().map_err(|_| CacheError::Poisoned)?;
            let id = consumers.next_id;
            consumers.next_id = consumers.next_id.saturating_add(1);
            consumers.active.insert(id);
            id
        };
        Self::register_reserved(flight, id, token, deadline)
    }

    fn register_reserved(
        flight: &Arc<InFlight>,
        id: u64,
        token: &CancellationToken,
        deadline: Option<CancellationDeadline>,
    ) -> Result<Self, CacheError> {
        if let Some(deadline) = deadline
            && let Err(error) = flight.deadlines.register(flight, id, deadline)
        {
            flight.release_consumer(id, CancellationReason::NoConsumers);
            return Err(error);
        }
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
        self.deadlines.remove(id);
        let empty = self.consumers.lock().map_or(true, |mut consumers| {
            consumers.active.remove(&id) && consumers.active.is_empty()
        });
        if empty {
            self.token.cancel_with_reason(reason);
        }
    }
}

impl Drop for InFlight {
    fn drop(&mut self) {
        self.deadlines.stop();
    }
}

struct Inner {
    in_flight: HashMap<CacheKey, Arc<InFlight>>,
    failures: HashMap<CacheKey, FailureEntry>,
    invalidation_sequence: u64,
    shutdown: bool,
}

#[cfg(test)]
struct TestGate {
    reached: std::sync::Barrier,
    release: std::sync::Barrier,
}

#[cfg(test)]
impl TestGate {
    fn new() -> Self {
        Self {
            reached: std::sync::Barrier::new(2),
            release: std::sync::Barrier::new(2),
        }
    }

    fn pause(&self) {
        self.reached.wait();
        self.release.wait();
    }
}

/// Typed pixelpipe cache backed by Darktable's soft-quota common MRU cache.
pub struct Cache {
    inner: Mutex<Inner>,
    publication: Mutex<()>,
    resident: ResidentCache,
    resident_state: Arc<ResidentState>,
    failure_window: Duration,
    #[cfg(test)]
    owner_registration_gate: Mutex<Option<Arc<TestGate>>>,
    #[cfg(test)]
    completion_gate: Mutex<Option<Arc<TestGate>>>,
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
        let resident_state = Arc::new(ResidentState {
            pending: Mutex::new(HashMap::new()),
            explicit_removals: Mutex::new(HashSet::new()),
            invalidating: Mutex::new(HashSet::new()),
            receipts: Mutex::new(VecDeque::new()),
            receipt_sequence: AtomicU64::new(0),
        });
        let callbacks = ResidentCallbacks {
            state: resident_state.clone(),
        };
        Self {
            inner: Mutex::new(Inner {
                in_flight: HashMap::new(),
                failures: HashMap::new(),
                invalidation_sequence: 0,
                shutdown: false,
            }),
            publication: Mutex::new(()),
            resident: ResidentCache::with_callbacks(
                usize::try_from(config.budget_bytes).unwrap_or(usize::MAX),
                callbacks,
            ),
            resident_state,
            failure_window: config.failure_window.min(DEFAULT_FAILURE_WINDOW),
            #[cfg(test)]
            owner_registration_gate: Mutex::new(None),
            #[cfg(test)]
            completion_gate: Mutex::new(None),
        }
    }

    #[cfg(test)]
    fn pause_at_test_gate(slot: &Mutex<Option<Arc<TestGate>>>) {
        let gate = slot.lock().ok().and_then(|gate| gate.clone());
        if let Some(gate) = gate {
            gate.pause();
        }
    }

    /// Returns a pinned typed value when the full structured key matches.
    pub fn lookup<T: CacheValue>(
        &self,
        key: &CacheKey,
    ) -> Result<Option<CacheLease<T>>, CacheError> {
        if self.is_invalidating(key)? {
            self.record(CacheEvent::Miss, key, None, None);
            return Ok(None);
        }
        if !self.resident.contains(key).map_err(map_common_error)? {
            self.record(CacheEvent::Miss, key, None, None);
            return Ok(None);
        }
        let lease = match self.resident.acquire(key.clone(), CommonCacheMode::Read) {
            Ok(lease) => lease,
            Err(CommonCacheError::Allocation(CacheError::BuildNotPublished)) => {
                self.record(CacheEvent::Miss, key, None, None);
                return Ok(None);
            }
            Err(error) => return Err(map_common_error(error)),
        };
        let CommonCacheLease::Read(lease) = lease else {
            return Err(CacheError::Poisoned);
        };
        if self.is_invalidating(key)? {
            drop(lease);
            self.record(CacheEvent::Miss, key, None, None);
            return Ok(None);
        }
        let (value, descriptor, pin_count, promoted, segment) = lease
            .with_value(|entry| {
                if entry.type_id != TypeId::of::<T>() {
                    return Err(CacheError::TypeMismatch);
                }
                let value = entry
                    .value
                    .clone()
                    .downcast::<T>()
                    .map_err(|_| CacheError::TypeMismatch)?;
                let segment = entry.segment();
                Ok((
                    value,
                    entry.descriptor,
                    entry.pin_count.clone(),
                    entry.promote(),
                    segment,
                ))
            })
            .map_err(|_| CacheError::Poisoned)??;
        pin_count.fetch_add(1, Ordering::AcqRel);
        self.record(
            if promoted {
                CacheEvent::Promotion
            } else {
                CacheEvent::Hit
            },
            key,
            Some(descriptor),
            Some(if promoted {
                Segment::Protected
            } else {
                segment
            }),
        );
        Ok(Some(CacheLease {
            key: key.clone(),
            value,
            descriptor,
            pin_count,
            cached: true,
            resident_lease: Some(Arc::new(lease)),
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
        self.get_or_build_until(&key, cancellation, None, builder)
    }

    pub(crate) fn get_or_build_until<T, F>(
        &self,
        key: &CacheKey,
        cancellation: &CancellationToken,
        deadline: Option<CancellationDeadline>,
        builder: F,
    ) -> Result<CacheLease<T>, CacheError>
    where
        T: CacheValue,
        F: FnOnce(&CancellationToken) -> Result<T, CacheError>,
    {
        if consumer_cancelled(cancellation, deadline) {
            return Err(CacheError::Cancelled);
        }
        {
            let inner = self.lock_inner()?;
            if inner.shutdown {
                return Err(CacheError::Shutdown);
            }
            if let Some(failure) = inner.failures.get(key)
                && failure.expires > Instant::now()
            {
                return Err(CacheError::SuppressedFailure(failure.message.clone()));
            }
        }
        if let Some(lease) = self.lookup::<T>(key)? {
            return Ok(lease);
        }
        let (flight, owner) = {
            let mut inner = self.lock_inner()?;
            if inner.shutdown {
                return Err(CacheError::Shutdown);
            }
            if let Some(failure) = inner.failures.get(key)
                && failure.expires > Instant::now()
            {
                return Err(CacheError::SuppressedFailure(failure.message.clone()));
            }
            if let Some(flight) = inner.in_flight.get(key) {
                (flight.clone(), false)
            } else {
                let flight = Arc::new(InFlight {
                    token: CancellationToken::for_generation(key.generation()),
                    state: Mutex::new(FlightState {
                        completed: false,
                        error: None,
                        success: None,
                    }),
                    wake: Condvar::new(),
                    consumers: Mutex::new(Consumers {
                        next_id: 2,
                        active: BTreeSet::from([1]),
                    }),
                    deadlines: Arc::new(FlightDeadlines::new()),
                    stale: AtomicBool::new(false),
                });
                inner.in_flight.insert(key.clone(), flight.clone());
                (flight, true)
            }
        };
        if !owner {
            let registration = ConsumerRegistration::register(&flight, cancellation, deadline)?;
            return self.wait_for_flight::<T>(key, &flight, cancellation, deadline, registration);
        }

        #[cfg(test)]
        Self::pause_at_test_gate(&self.owner_registration_gate);
        let registration =
            match ConsumerRegistration::register_reserved(&flight, 1, cancellation, deadline) {
                Ok(registration) => registration,
                Err(error) => {
                    if let Ok(mut state) = flight.state.lock() {
                        state.completed = true;
                        state.error = Some(error.clone());
                        state.success = None;
                        flight.wake.notify_all();
                    }
                    self.lock_inner()?.in_flight.remove(key);
                    return Err(error);
                }
            };

        let mut result = match self.lookup::<T>(key) {
            Ok(Some(lease)) => Ok(lease),
            Ok(None) => flight
                .token
                .check(CancellationStage::CacheBuild)
                .map_err(CacheError::Cancellation)
                .and_then(|()| {
                    catch_unwind(AssertUnwindSafe(|| builder(&flight.token)))
                        .map_err(|_| CacheError::BuilderPanicked)
                        .and_then(std::convert::identity)
                })
                .and_then(|value| {
                    let _ = consumer_cancelled(cancellation, deadline);
                    self.publish(key.clone(), value, &flight)
                }),
            Err(error) => Err(error),
        };
        #[cfg(test)]
        if result.is_ok() {
            Self::pause_at_test_gate(&self.completion_gate);
        }
        {
            let _publication = self.publication.lock().map_err(|_| CacheError::Poisoned)?;
            let mut inner = self.lock_inner()?;
            if result.is_ok() && flight.stale.load(Ordering::Acquire) {
                result = Err(CacheError::StalePublication);
                self.record(CacheEvent::StalePublication, key, None, None);
            }
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
            let mut state = flight.state.lock().map_err(|_| CacheError::Poisoned)?;
            state.completed = true;
            state.error = result.as_ref().err().cloned();
            state.success = result.as_ref().ok().map(CacheLease::flight_success);
            if inner
                .in_flight
                .get(key)
                .is_some_and(|current| Arc::ptr_eq(current, &flight))
            {
                inner.in_flight.remove(key);
            }
            flight.wake.notify_all();
        }
        drop(registration);
        if consumer_cancelled(cancellation, deadline) {
            Err(CacheError::Cancelled)
        } else {
            result
        }
    }

    fn wait_for_flight<T: CacheValue>(
        &self,
        key: &CacheKey,
        flight: &Arc<InFlight>,
        cancellation: &CancellationToken,
        deadline: Option<CancellationDeadline>,
        _registration: ConsumerRegistration,
    ) -> Result<CacheLease<T>, CacheError> {
        loop {
            if consumer_cancelled(cancellation, deadline) {
                return Err(CacheError::Cancelled);
            }
            let state = flight.state.lock().map_err(|_| CacheError::Poisoned)?;
            if state.completed {
                if let Some(error) = &state.error {
                    return Err(error.clone());
                }
                let direct = state
                    .success
                    .as_ref()
                    .map(|success| {
                        if success.type_id != TypeId::of::<T>() {
                            return Err(CacheError::TypeMismatch);
                        }
                        let value = success
                            .value
                            .clone()
                            .downcast::<T>()
                            .map_err(|_| CacheError::TypeMismatch)?;
                        success.pin_count.fetch_add(1, Ordering::AcqRel);
                        Ok(CacheLease {
                            key: key.clone(),
                            value,
                            descriptor: success.descriptor,
                            pin_count: success.pin_count.clone(),
                            cached: false,
                            resident_lease: None,
                        })
                    })
                    .transpose()?;
                drop(state);
                if let Some(lease) = self.lookup(key)? {
                    return Ok(lease);
                }
                return direct.ok_or(CacheError::BuildNotPublished);
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
        flight: &InFlight,
    ) -> Result<CacheLease<T>, CacheError> {
        flight
            .token
            .check(CancellationStage::CachePromotion)
            .map_err(CacheError::Cancellation)?;
        value.validate().map_err(CacheError::InvalidValue)?;
        let descriptor = value.descriptor();
        let cost = descriptor.total_bytes()?;
        if !descriptor.cacheable() {
            return Err(CacheError::NotCacheable);
        }
        let _publication = self.publication.lock().map_err(|_| CacheError::Poisoned)?;
        {
            let mut inner = self.lock_inner()?;
            if inner.shutdown {
                return Err(CacheError::Shutdown);
            }
            if self.is_invalidating(&key)? {
                self.record(CacheEvent::StalePublication, &key, None, None);
                return Err(CacheError::StalePublication);
            }
            if flight.stale.load(Ordering::Acquire) {
                self.record(CacheEvent::StalePublication, &key, None, None);
                return Err(CacheError::StalePublication);
            }
            inner.failures.remove(&key);
        }
        let cost = usize::try_from(cost).map_err(|_| CacheError::CostOverflow)?;
        let quota = self
            .resident
            .accounting()
            .map_err(map_common_error)?
            .cost_quota;
        if cost > quota {
            self.record(CacheEvent::OversizeDirect, &key, Some(descriptor), None);
            let value = Arc::new(value);
            let pin_count = Arc::new(AtomicUsize::new(1));
            return Ok(CacheLease {
                key,
                value,
                descriptor,
                pin_count,
                cached: false,
                resident_lease: None,
            });
        }
        let value = Arc::new(value);
        let pin_count = Arc::new(AtomicUsize::new(1));
        let allocation = CommonCacheAllocation::with_cost(
            StoredEntry {
                value: value.clone(),
                type_id: TypeId::of::<T>(),
                descriptor,
                segment: AtomicUsize::new(0),
                pin_count: pin_count.clone(),
            },
            cost,
        );
        self.resident_state
            .pending
            .lock()
            .map_err(|_| CacheError::Poisoned)?
            .insert(key.clone(), allocation);
        let lease = match self.resident.acquire(key.clone(), CommonCacheMode::Read) {
            Ok(lease) => lease,
            Err(error) => {
                if let Ok(mut pending) = self.resident_state.pending.lock() {
                    pending.remove(&key);
                }
                return Err(map_common_error(error));
            }
        };
        let was_created = lease.was_created();
        if let Ok(mut pending) = self.resident_state.pending.lock() {
            pending.remove(&key);
        }
        if !was_created {
            drop(lease);
            return self.lookup(&key)?.ok_or(CacheError::BuildNotPublished);
        }
        let lease = lease.into_read().map_err(|_| CacheError::Poisoned)?;
        self.record(
            CacheEvent::Publish,
            &key,
            Some(descriptor),
            Some(Segment::Probationary),
        );
        Ok(CacheLease {
            key,
            value,
            descriptor,
            pin_count,
            cached: true,
            resident_lease: Some(Arc::new(lease)),
        })
    }

    /// Applies natural key misses for the requested scope and invalidates
    /// matching in-flight publications before they can become entries.
    pub fn invalidate(&self, scope: CacheScope) -> Result<InvalidationReceipt, CacheError> {
        let (sequence, keys) = {
            let _publication = self.publication.lock().map_err(|_| CacheError::Poisoned)?;
            let mut inner = self.lock_inner()?;
            inner.invalidation_sequence = inner.invalidation_sequence.saturating_add(1);
            let sequence = inner.invalidation_sequence;
            for (key, flight) in &inner.in_flight {
                if key.matches(&scope) {
                    flight.stale.store(true, Ordering::Release);
                }
            }
            let keys = self
                .resident
                .keys()
                .map_err(map_common_error)?
                .into_iter()
                .filter(|key| key.matches(&scope))
                .collect::<Vec<_>>();
            self.resident_state
                .invalidating
                .lock()
                .map_err(|_| CacheError::Poisoned)?
                .extend(keys.iter().cloned());
            (sequence, keys)
        };
        let mut removed = 0_usize;
        let mut deferred_error = None;
        for key in keys {
            if let Err(error) = self.mark_explicit_removal(&key) {
                self.unmark_invalidating(&key);
                deferred_error.get_or_insert(error);
                continue;
            }
            let result = self.resident.remove(&key).map_err(map_common_error);
            let result = match result {
                Ok(result) => result,
                Err(error) => {
                    self.unmark_explicit_removal(&key);
                    self.unmark_invalidating(&key);
                    deferred_error.get_or_insert(error);
                    continue;
                }
            };
            match result {
                CommonCacheRemoveResult::Removed { .. } => removed = removed.saturating_add(1),
                CommonCacheRemoveResult::Missing => self.unmark_explicit_removal(&key),
            }
            self.unmark_invalidating(&key);
        }
        if let Some(error) = deferred_error {
            return Err(error);
        }
        self.record(
            CacheEvent::Invalidation,
            &CacheKey::diagnostic_sentinel(),
            None,
            None,
        );
        Ok(InvalidationReceipt {
            sequence,
            removed,
            scope,
        })
    }

    pub fn set_budget(&self, budget: u64) -> Result<CacheMetrics, CacheError> {
        let _publication = self.publication.lock().map_err(|_| CacheError::Poisoned)?;
        self.resident
            .set_cost_quota(usize::try_from(budget).unwrap_or(usize::MAX))
            .map_err(map_common_error)?;
        self.resident.gc(1.0).map_err(map_common_error)?;
        Ok(self.metrics())
    }

    #[must_use]
    pub fn metrics(&self) -> CacheMetrics {
        self.metrics_result().unwrap_or_default()
    }

    #[must_use]
    pub fn receipts(&self) -> Vec<CacheReceipt> {
        self.resident_state
            .receipts
            .lock()
            .map(|receipts| receipts.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn shutdown(&self) -> Result<ShutdownReport, CacheError> {
        let (in_flight_builds, keys) = {
            let _publication = self.publication.lock().map_err(|_| CacheError::Poisoned)?;
            let mut inner = self.lock_inner()?;
            inner.shutdown = true;
            let in_flight_builds = inner.in_flight.len();
            let keys = self.resident.keys().map_err(map_common_error)?;
            (in_flight_builds, keys)
        };
        let entries = keys.len();
        let mut pinned = 0_usize;
        for key in keys {
            let is_pinned = self
                .resident
                .try_acquire(&key, CommonCacheMode::Read)
                .map_err(map_common_error)?
                .and_then(|lease| {
                    lease
                        .with_value(|entry| pin_count(&entry.pin_count) > 0)
                        .ok()
                })
                .unwrap_or(true);
            if is_pinned {
                pinned = pinned.saturating_add(1);
            } else {
                self.mark_explicit_removal(&key)?;
                match self.resident.remove(&key).map_err(map_common_error) {
                    Ok(CommonCacheRemoveResult::Removed { .. }) => {}
                    Ok(CommonCacheRemoveResult::Missing) => self.unmark_explicit_removal(&key),
                    Err(error) => {
                        self.unmark_explicit_removal(&key);
                        return Err(error);
                    }
                }
            }
        }
        Ok(ShutdownReport {
            entries_released: entries.saturating_sub(pinned),
            pinned_entries: pinned,
            in_flight_builds,
        })
    }

    fn metrics_result(&self) -> Result<CacheMetrics, CacheError> {
        let accounting = self.resident.accounting().map_err(map_common_error)?;
        let mut metrics = CacheMetrics {
            resident_bytes: u64::try_from(accounting.cost).unwrap_or(u64::MAX),
            entries: accounting.entries,
            ..CacheMetrics::default()
        };
        for key in self.resident.keys().map_err(map_common_error)? {
            let Some(lease) = self
                .resident
                .try_acquire(&key, CommonCacheMode::Read)
                .map_err(map_common_error)?
            else {
                continue;
            };
            lease
                .with_value(|entry| {
                    metrics.auxiliary_bytes = metrics
                        .auxiliary_bytes
                        .saturating_add(entry.descriptor.auxiliary_bytes());
                    match entry.segment() {
                        Segment::Probationary => {
                            metrics.probationary_entries =
                                metrics.probationary_entries.saturating_add(1);
                        }
                        Segment::Protected => {
                            metrics.protected_entries = metrics.protected_entries.saturating_add(1);
                        }
                    }
                    if pin_count(&entry.pin_count) > 0 {
                        metrics.pinned_entries = metrics.pinned_entries.saturating_add(1);
                    }
                })
                .map_err(|_| CacheError::Poisoned)?;
        }
        metrics.over_budget_pinned =
            accounting.cost > accounting.cost_quota && metrics.pinned_entries > 0;
        Ok(metrics)
    }

    fn record(
        &self,
        event: CacheEvent,
        key: &CacheKey,
        descriptor: Option<ValueDescriptor>,
        segment: Option<Segment>,
    ) {
        self.resident_state.record(event, key, descriptor, segment);
    }

    fn mark_explicit_removal(&self, key: &CacheKey) -> Result<(), CacheError> {
        self.resident_state
            .explicit_removals
            .lock()
            .map_err(|_| CacheError::Poisoned)?
            .insert(key.clone());
        Ok(())
    }

    fn unmark_explicit_removal(&self, key: &CacheKey) {
        if let Ok(mut removals) = self.resident_state.explicit_removals.lock() {
            removals.remove(key);
        }
    }

    fn is_invalidating(&self, key: &CacheKey) -> Result<bool, CacheError> {
        self.resident_state
            .invalidating
            .lock()
            .map(|keys| keys.contains(key))
            .map_err(|_| CacheError::Poisoned)
    }

    fn unmark_invalidating(&self, key: &CacheKey) {
        if let Ok(mut keys) = self.resident_state.invalidating.lock() {
            keys.remove(key);
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
    resident_lease: Option<Arc<ResidentReadLease>>,
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
            resident_lease: self.resident_lease.clone(),
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

    fn flight_success(&self) -> FlightSuccess {
        FlightSuccess {
            value: self.value.clone(),
            type_id: TypeId::of::<T>(),
            descriptor: self.descriptor,
            pin_count: self.pin_count.clone(),
        }
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

fn consumer_cancelled(
    cancellation: &CancellationToken,
    deadline: Option<CancellationDeadline>,
) -> bool {
    if deadline.is_some_and(CancellationDeadline::expired) {
        cancellation.cancel_with_reason(CancellationReason::DeadlineExceeded);
    }
    cancellation.is_cancelled()
}

fn pin_count(value: &AtomicUsize) -> usize {
    value.load(Ordering::Acquire)
}

fn map_common_error(error: CommonCacheError<CacheError>) -> CacheError {
    match error {
        CommonCacheError::Allocation(error) => error,
        CommonCacheError::CostOverflow { .. } | CommonCacheError::CostUnderflow { .. } => {
            CacheError::CostOverflow
        }
        CommonCacheError::Poisoned(_)
        | CommonCacheError::AllocationPanicked
        | CommonCacheError::CleanupPanicked
        | CommonCacheError::CapacityAllocation { .. }
        | CommonCacheError::InvalidFillRatio { .. } => CacheError::Poisoned,
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

#[cfg(test)]
mod deadline_tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::mpsc;

    use super::*;

    #[derive(Debug)]
    struct DeadlineValue(u64);

    impl CacheValue for DeadlineValue {
        fn descriptor(&self) -> ValueDescriptor {
            ValueDescriptor::new(
                value::ValueKind::Analysis,
                self.0,
                0,
                value::CreationCost::Normal,
                true,
            )
        }

        fn validate(&self) -> Result<(), String> {
            Ok(())
        }
    }

    fn active_consumers(cache: &Cache, key: &CacheKey) -> Option<usize> {
        let flight = cache.inner.lock().ok()?.in_flight.get(key).cloned()?;
        flight
            .consumers
            .lock()
            .ok()
            .map(|consumers| consumers.active.len())
    }

    fn wait_for_active_consumers(
        cache: &Cache,
        key: &CacheKey,
        expected: usize,
        timeout: Duration,
    ) {
        let expires = Instant::now() + timeout;
        while Instant::now() < expires {
            if active_consumers(cache, key) == Some(expected) {
                return;
            }
            thread::yield_now();
        }
        panic!(
            "expected {expected} active consumers, found {:?}",
            active_consumers(cache, key)
        );
    }

    #[test]
    fn sole_owner_deadline_cancels_shared_build_before_publication() {
        let cache = Cache::new(CacheConfig::new(64));
        let key = CacheKey::diagnostic_sentinel();
        let token = CancellationToken::new();
        let observed_reason = Arc::new(Mutex::new(None));
        let builder_reason = observed_reason.clone();

        let result = cache.get_or_build_until(
            &key,
            &token,
            Some(CancellationDeadline::after(Duration::from_millis(25))),
            move |shared| {
                let timeout = Instant::now() + Duration::from_secs(2);
                while !shared.is_cancelled() && Instant::now() < timeout {
                    let _ = shared.wait_timeout(Duration::from_millis(5));
                }
                *builder_reason.lock().expect("reason slot") = shared.reason();
                Ok(DeadlineValue(8))
            },
        );

        assert!(matches!(result, Err(CacheError::Cancelled)));
        assert_eq!(token.reason(), Some(CancellationReason::DeadlineExceeded));
        assert_eq!(
            *observed_reason.lock().expect("reason slot"),
            Some(CancellationReason::DeadlineExceeded)
        );
        assert!(
            cache
                .lookup::<DeadlineValue>(&key)
                .expect("cache lookup")
                .is_none()
        );
        assert!(
            !cache
                .receipts()
                .iter()
                .any(|receipt| receipt.event() == CacheEvent::Publish)
        );
    }

    #[test]
    fn owner_deadline_preserves_shared_build_for_live_waiter() {
        let cache = Arc::new(Cache::new(CacheConfig::new(64)));
        let key = CacheKey::diagnostic_sentinel();
        let owner_token = CancellationToken::new();
        let owner_token_for_thread = owner_token.clone();
        let deadline = CancellationDeadline::after(Duration::from_secs(1));
        let builds = Arc::new(AtomicUsize::new(0));
        let shared_cancelled = Arc::new(AtomicBool::new(false));
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();

        let owner_cache = cache.clone();
        let owner_key = key.clone();
        let owner_builds = builds.clone();
        let owner_shared_cancelled = shared_cancelled.clone();
        let owner = thread::spawn(move || {
            owner_cache.get_or_build_until(
                &owner_key,
                &owner_token_for_thread,
                Some(deadline),
                move |shared| {
                    owner_builds.fetch_add(1, Ordering::AcqRel);
                    started_tx.send(()).expect("signal owner builder");
                    release_rx.recv().expect("release owner builder");
                    owner_shared_cancelled.store(shared.is_cancelled(), Ordering::Release);
                    Ok(DeadlineValue(8))
                },
            )
        });
        started_rx.recv().expect("owner builder started");

        let waiter_cache = cache.clone();
        let waiter_key = key.clone();
        let waiter_builds = builds.clone();
        let waiter = thread::spawn(move || {
            waiter_cache.get_or_build_until(
                &waiter_key,
                &CancellationToken::new(),
                None,
                move |_| {
                    waiter_builds.fetch_add(1, Ordering::AcqRel);
                    Ok(DeadlineValue(99))
                },
            )
        });

        wait_for_active_consumers(&cache, &key, 2, Duration::from_millis(500));
        wait_for_active_consumers(&cache, &key, 1, Duration::from_secs(2));
        release_tx.send(()).expect("release shared builder");

        assert!(matches!(
            owner.join().expect("owner thread"),
            Err(CacheError::Cancelled)
        ));
        let waiter = waiter
            .join()
            .expect("waiter thread")
            .expect("waiter result");
        assert_eq!(waiter.value().0, 8);
        assert_eq!(
            owner_token.reason(),
            Some(CancellationReason::DeadlineExceeded)
        );
        assert!(!shared_cancelled.load(Ordering::Acquire));
        assert_eq!(builds.load(Ordering::Acquire), 1);
    }

    #[test]
    fn oversize_success_is_not_exposed_after_invalidation_wins() {
        let cache = Arc::new(Cache::new(CacheConfig::new(4)));
        let key = CacheKey::diagnostic_sentinel();
        let gate = Arc::new(TestGate::new());
        *cache.completion_gate.lock().expect("completion gate") = Some(gate.clone());
        let builds = Arc::new(AtomicUsize::new(0));

        let owner_cache = cache.clone();
        let owner_key = key.clone();
        let owner_builds = builds.clone();
        let owner = thread::spawn(move || {
            owner_cache.get_or_build::<DeadlineValue, _>(
                owner_key,
                &CancellationToken::new(),
                move |_| {
                    owner_builds.fetch_add(1, Ordering::AcqRel);
                    Ok(DeadlineValue(8))
                },
            )
        });
        gate.reached.wait();

        cache
            .invalidate(CacheScope::All)
            .expect("invalidation wins publication gap");
        let waiter_cache = cache.clone();
        let waiter_key = key.clone();
        let waiter_builds = builds.clone();
        let waiter = thread::spawn(move || {
            waiter_cache.get_or_build::<DeadlineValue, _>(
                waiter_key,
                &CancellationToken::new(),
                move |_| {
                    waiter_builds.fetch_add(1, Ordering::AcqRel);
                    Ok(DeadlineValue(99))
                },
            )
        });
        wait_for_active_consumers(&cache, &key, 2, Duration::from_secs(1));
        gate.release.wait();

        assert!(matches!(
            owner.join().expect("owner thread"),
            Err(CacheError::StalePublication)
        ));
        assert!(matches!(
            waiter.join().expect("waiter thread"),
            Err(CacheError::StalePublication)
        ));
        assert_eq!(builds.load(Ordering::Acquire), 1);
        assert!(
            cache
                .lookup::<DeadlineValue>(&key)
                .expect("cache lookup")
                .is_none()
        );
    }

    #[test]
    fn cancelled_waiter_cannot_cancel_reserved_owner_before_registration() {
        let cache = Arc::new(Cache::new(CacheConfig::new(64)));
        let key = CacheKey::diagnostic_sentinel();
        let gate = Arc::new(TestGate::new());
        *cache
            .owner_registration_gate
            .lock()
            .expect("owner registration gate") = Some(gate.clone());
        let builds = Arc::new(AtomicUsize::new(0));

        let owner_cache = cache.clone();
        let owner_key = key.clone();
        let owner_builds = builds.clone();
        let owner = thread::spawn(move || {
            owner_cache.get_or_build::<DeadlineValue, _>(
                owner_key,
                &CancellationToken::new(),
                move |_| {
                    owner_builds.fetch_add(1, Ordering::AcqRel);
                    Ok(DeadlineValue(8))
                },
            )
        });
        gate.reached.wait();

        let active_before_waiter = active_consumers(&cache, &key).expect("reserved owner consumer");
        let waiter_token = CancellationToken::new();
        let waiter_token_for_thread = waiter_token.clone();
        let waiter_cache = cache.clone();
        let waiter_key = key.clone();
        let waiter = thread::spawn(move || {
            waiter_cache.get_or_build::<DeadlineValue, _>(
                waiter_key,
                &waiter_token_for_thread,
                |_| Ok(DeadlineValue(99)),
            )
        });
        wait_for_active_consumers(
            &cache,
            &key,
            active_before_waiter.saturating_add(1),
            Duration::from_secs(1),
        );
        waiter_token.cancel();
        assert!(matches!(
            waiter.join().expect("waiter thread"),
            Err(CacheError::Cancelled)
        ));
        let flight = cache
            .inner
            .lock()
            .expect("cache inner")
            .in_flight
            .get(&key)
            .cloned()
            .expect("reserved owner flight");
        assert!(!flight.token.is_cancelled());
        assert_eq!(
            flight
                .consumers
                .lock()
                .expect("flight consumers")
                .active
                .len(),
            1
        );

        gate.release.wait();
        let owner = owner.join().expect("owner thread").expect("owner result");
        assert_eq!(owner.value().0, 8);
        assert_eq!(builds.load(Ordering::Acquire), 1);
    }
}
