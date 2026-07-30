//! Concurrent MRU cache ported from Darktable's `src/common/cache.c` and
//! `src/common/cache.h`.
//!
//! Darktable holds a per-entry read/write lock for the complete duration of a
//! cache lease. Rust's standard-library lock guards borrow their lock, so an
//! owned lease cannot safely contain one together with the `Arc` that owns it.
//! This port therefore keeps the lease lifetime in a small logical
//! `Mutex`/`Condvar` read/write state and protects value access with a private
//! `RwLock`. The logical state excludes writers for the whole lease while the
//! value lock still permits genuinely concurrent readers, without unsafe code.

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::error::Error;
use std::fmt::{self, Debug, Display, Formatter};
use std::hash::Hash;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{
    Arc, Condvar, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError,
};

const AUTOMATIC_GC_FILL_RATIO: f64 = 0.8;

/// Requested or held access mode for a cache entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheMode {
    /// Multiple leases may inspect the value concurrently.
    Read,
    /// One lease may mutate the value exclusively.
    Write,
}

/// Identifies the synchronized cache component whose lock was poisoned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CachePoison {
    /// The cache-wide key, cost, and MRU state.
    State,
    /// One entry's logical read/write lease state.
    EntryLease,
    /// One entry's protected value.
    Value,
}

/// A value and its caller-defined contribution to the cache's soft quota.
#[derive(Debug)]
pub struct CacheAllocation<V> {
    value: V,
    cost: usize,
}

impl<V> CacheAllocation<V> {
    /// Wraps a value with Darktable's default per-entry cost of one.
    #[must_use]
    pub const fn new(value: V) -> Self {
        Self { value, cost: 1 }
    }

    /// Wraps a value with the cost selected by an allocation callback.
    #[must_use]
    pub const fn with_cost(value: V, cost: usize) -> Self {
        Self { value, cost }
    }

    /// Returns the caller-defined cost.
    #[must_use]
    pub const fn cost(&self) -> usize {
        self.cost
    }

    /// Borrows the allocated value.
    #[must_use]
    pub const fn value(&self) -> &V {
        &self.value
    }

    /// Mutably borrows the allocated value.
    #[must_use]
    pub const fn value_mut(&mut self) -> &mut V {
        &mut self.value
    }

    /// Consumes the allocation and returns its value.
    #[must_use]
    pub fn into_value(self) -> V {
        self.value
    }
}

/// Allocation and cleanup behavior for callback-backed cache entries.
///
/// `allocate` is single-flight for each key. Its result is completely
/// initialized before publication. `cleanup` receives the same value and cost
/// exactly once when a published entry is removed, collected, or dropped. It
/// also receives a successfully allocated value that cannot be published
/// because cost accounting overflows.
pub trait CacheCallbacks<K, V>: Send + Sync + 'static {
    /// A caller-defined allocation failure.
    type Error: Send + Sync + 'static;

    /// Allocates the value and selects its cost.
    ///
    /// # Errors
    ///
    /// Returns the callback's typed allocation failure.
    fn allocate(&self, key: &K) -> Result<CacheAllocation<V>, Self::Error>;

    /// Cleans up one allocation. The default implementation simply drops it.
    fn cleanup(&self, _key: &K, _allocation: CacheAllocation<V>) {}
}

/// Callback implementation used by [`ConcurrentCache::new`].
#[derive(Debug)]
pub struct DefaultCacheCallbacks<V> {
    value: PhantomData<fn() -> V>,
}

impl<V> Default for DefaultCacheCallbacks<V> {
    fn default() -> Self {
        Self { value: PhantomData }
    }
}

impl<K, V> CacheCallbacks<K, V> for DefaultCacheCallbacks<V>
where
    K: Send + Sync + 'static,
    V: Default + Send + Sync + 'static,
{
    type Error = Infallible;

    fn allocate(&self, _key: &K) -> Result<CacheAllocation<V>, Self::Error> {
        Ok(CacheAllocation::new(V::default()))
    }
}

/// Failure from a cache-wide operation.
#[derive(Debug)]
pub enum CacheError<E> {
    /// A synchronization primitive was poisoned by a panic.
    Poisoned(CachePoison),
    /// The caller's allocation callback rejected the miss.
    Allocation(E),
    /// The allocation callback panicked. No entry was published.
    AllocationPanicked,
    /// The cleanup callback panicked after being invoked exactly once.
    CleanupPanicked,
    /// Standard-library capacity reservation failed.
    CapacityAllocation {
        /// The state collection that could not reserve capacity.
        resource: &'static str,
    },
    /// Adding a callback-selected cost would overflow `usize`.
    CostOverflow {
        /// Cost already charged to the cache.
        current: usize,
        /// Cost selected for the new entry.
        added: usize,
    },
    /// Removing a recorded cost would underflow cache accounting.
    CostUnderflow {
        /// Cost currently charged to the cache.
        current: usize,
        /// Cost recorded by the entry being removed.
        removed: usize,
    },
    /// A garbage-collection fill ratio was negative or non-finite.
    InvalidFillRatio {
        /// The rejected ratio.
        ratio: f64,
    },
}

impl<E> Display for CacheError<E>
where
    E: Display,
{
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Poisoned(component) => {
                write!(formatter, "cache {component:?} lock was poisoned")
            }
            Self::Allocation(error) => write!(formatter, "cache allocation failed: {error}"),
            Self::AllocationPanicked => formatter.write_str("cache allocation callback panicked"),
            Self::CleanupPanicked => formatter.write_str("cache cleanup callback panicked"),
            Self::CapacityAllocation { resource } => {
                write!(formatter, "cache could not reserve capacity for {resource}")
            }
            Self::CostOverflow { current, added } => {
                write!(
                    formatter,
                    "cache cost overflow while adding {added} to {current}"
                )
            }
            Self::CostUnderflow { current, removed } => {
                write!(
                    formatter,
                    "cache cost underflow while removing {removed} from {current}"
                )
            }
            Self::InvalidFillRatio { ratio } => {
                write!(formatter, "invalid cache fill ratio {ratio}")
            }
        }
    }
}

impl<E> Error for CacheError<E>
where
    E: Error + 'static,
{
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Allocation(error) => Some(error),
            _ => None,
        }
    }
}

/// Failure while accessing or transforming an already-held lease.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheLeaseError {
    /// A synchronized entry component was poisoned by a panic.
    Poisoned(CachePoison),
    /// The entry no longer contains a value, indicating an internal invariant
    /// violation.
    ValueUnavailable,
}

impl Display for CacheLeaseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Poisoned(component) => {
                write!(formatter, "cache lease {component:?} lock was poisoned")
            }
            Self::ValueUnavailable => formatter.write_str("cache lease value is unavailable"),
        }
    }
}

impl Error for CacheLeaseError {}

/// Result of waiting for exclusive removal of a key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheRemoveResult {
    /// The key was present and its allocation was cleaned up.
    Removed {
        /// Cost released by the removed entry.
        cost: usize,
    },
    /// The key was not present when removal could proceed.
    Missing,
}

/// A point-in-time summary of cache usage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheAccounting {
    /// Number of fully initialized, published entries.
    pub entries: usize,
    /// Number of keys currently being allocated or cleaned up.
    pub pending: usize,
    /// Sum of published callback-selected costs.
    pub cost: usize,
    /// Soft quota that triggers automatic best-effort collection.
    pub cost_quota: usize,
}

/// Outcome of one oldest-first best-effort garbage-collection pass.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheGcReport {
    /// Cost before collection.
    pub cost_before: usize,
    /// Cost after collection.
    pub cost_after: usize,
    /// Number of entries removed.
    pub removed_entries: usize,
    /// Sum of removed costs.
    pub reclaimed_cost: usize,
    /// Number of entries skipped because a lease or demotion held them.
    pub skipped_locked: usize,
}

#[derive(Debug)]
struct LeaseState {
    readers: usize,
    writer: bool,
    demoting: bool,
    removed: bool,
}

impl LeaseState {
    const fn held(mode: CacheMode) -> Self {
        match mode {
            CacheMode::Read => Self {
                readers: 1,
                writer: false,
                demoting: false,
                removed: false,
            },
            CacheMode::Write => Self {
                readers: 0,
                writer: true,
                demoting: false,
                removed: false,
            },
        }
    }
}

struct Entry<K, V> {
    key: K,
    cost: usize,
    value: RwLock<Option<CacheAllocation<V>>>,
    lease: Mutex<LeaseState>,
    wake: Condvar,
}

enum EntryAcquireError {
    Removed,
    Poisoned,
}

enum EntryTryAcquire {
    Acquired,
    Unavailable,
    Removed,
}

impl<K, V> Entry<K, V> {
    fn new_held(key: K, allocation: CacheAllocation<V>, mode: CacheMode) -> Self {
        let cost = allocation.cost();
        Self {
            key,
            cost,
            value: RwLock::new(Some(allocation)),
            lease: Mutex::new(LeaseState::held(mode)),
            wake: Condvar::new(),
        }
    }

    fn acquire(&self, mode: CacheMode) -> Result<(), EntryAcquireError> {
        let mut state = self.lease.lock().map_err(|_| EntryAcquireError::Poisoned)?;
        loop {
            if state.removed {
                return Err(EntryAcquireError::Removed);
            }
            let available = match mode {
                CacheMode::Read => !state.writer && !state.demoting,
                CacheMode::Write => state.readers == 0 && !state.writer && !state.demoting,
            };
            if available {
                match mode {
                    CacheMode::Read => {
                        state.readers = state
                            .readers
                            .checked_add(1)
                            .ok_or(EntryAcquireError::Poisoned)?;
                    }
                    CacheMode::Write => state.writer = true,
                }
                return Ok(());
            }
            state = self
                .wake
                .wait(state)
                .map_err(|_| EntryAcquireError::Poisoned)?;
        }
    }

    fn try_acquire(&self, mode: CacheMode) -> Result<EntryTryAcquire, CachePoison> {
        let mut state = match self.lease.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return Ok(EntryTryAcquire::Unavailable),
            Err(TryLockError::Poisoned(_)) => return Err(CachePoison::EntryLease),
        };
        if state.removed {
            return Ok(EntryTryAcquire::Removed);
        }
        let available = match mode {
            CacheMode::Read => !state.writer && !state.demoting,
            CacheMode::Write => state.readers == 0 && !state.writer && !state.demoting,
        };
        if !available {
            return Ok(EntryTryAcquire::Unavailable);
        }
        match mode {
            CacheMode::Read => {
                state.readers = state
                    .readers
                    .checked_add(1)
                    .ok_or(CachePoison::EntryLease)?;
            }
            CacheMode::Write => state.writer = true,
        }
        Ok(EntryTryAcquire::Acquired)
    }

    fn release(&self, mode: CacheMode) {
        let mut state = match self.lease.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        match mode {
            CacheMode::Read => {
                debug_assert!(state.readers > 0);
                state.readers = state.readers.saturating_sub(1);
            }
            CacheMode::Write => {
                debug_assert!(state.writer);
                state.writer = false;
            }
        }
        self.wake.notify_all();
    }

    fn demote(&self) -> Result<(), CacheLeaseError> {
        let mut state = self
            .lease
            .lock()
            .map_err(|_| CacheLeaseError::Poisoned(CachePoison::EntryLease))?;
        if !state.writer || state.removed {
            return Err(CacheLeaseError::ValueUnavailable);
        }
        state.demoting = true;
        state.writer = false;
        state.readers = 1;
        state.demoting = false;
        self.wake.notify_all();
        Ok(())
    }

    fn mark_removed(&self) -> Result<(), CachePoison> {
        let mut state = self.lease.lock().map_err(|_| CachePoison::EntryLease)?;
        state.removed = true;
        self.wake.notify_all();
        Ok(())
    }

    fn mark_removed_recover(&self) -> bool {
        let (mut state, poisoned) = match self.lease.lock() {
            Ok(state) => (state, false),
            Err(error) => (error.into_inner(), true),
        };
        state.removed = true;
        self.wake.notify_all();
        poisoned
    }

    fn take_value_recover(&self) -> (Option<CacheAllocation<V>>, bool) {
        let (mut value, poisoned) = match self.value.write() {
            Ok(value) => (value, false),
            Err(error) => (error.into_inner(), true),
        };
        (value.take(), poisoned)
    }
}

struct CacheState<K, V> {
    entries: HashMap<K, Arc<Entry<K, V>>>,
    pending: HashSet<K>,
    lru: VecDeque<K>,
    cost: usize,
    cost_quota: usize,
}

impl<K, V> CacheState<K, V> {
    fn new(cost_quota: usize) -> Self {
        Self {
            entries: HashMap::new(),
            pending: HashSet::new(),
            lru: VecDeque::new(),
            cost: 0,
            cost_quota,
        }
    }
}

struct CacheInner<K, V, C>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    state: Mutex<CacheState<K, V>>,
    changed: Condvar,
    callbacks: C,
    callback_backed: bool,
}

impl<K, V, C> CacheInner<K, V, C>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    fn cleanup(&self, key: &K, allocation: CacheAllocation<V>) -> Result<(), CacheError<C::Error>> {
        catch_unwind(AssertUnwindSafe(|| {
            self.callbacks.cleanup(key, allocation);
        }))
        .map_err(|_| CacheError::CleanupPanicked)
    }

    fn cleanup_ignoring_panic(&self, key: &K, allocation: CacheAllocation<V>) {
        let _ = catch_unwind(AssertUnwindSafe(|| {
            self.callbacks.cleanup(key, allocation);
        }));
    }
}

impl<K, V, C> Drop for CacheInner<K, V, C>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    fn drop(&mut self) {
        let state = match self.state.get_mut() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        let mut ordered = Vec::with_capacity(state.entries.len());
        while let Some(key) = state.lru.pop_front() {
            if let Some(entry) = state.entries.remove(&key) {
                ordered.push(entry);
            }
        }
        ordered.extend(state.entries.drain().map(|(_, entry)| entry));
        for entry in ordered {
            entry.mark_removed_recover();
            if let (Some(allocation), _) = entry.take_value_recover() {
                self.cleanup_ignoring_panic(&entry.key, allocation);
            }
        }
    }
}

/// Generic concurrent cache with Darktable-compatible MRU and soft-quota
/// behavior.
///
/// Cloning the cache is inexpensive. Leases are owned and keep the cache alive,
/// so they may be returned from APIs or moved between threads without borrowing
/// the `ConcurrentCache` handle.
pub struct ConcurrentCache<K, V, C = DefaultCacheCallbacks<V>>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    inner: Arc<CacheInner<K, V, C>>,
}

/// Return type for an immediate nonblocking cache acquisition.
pub type CacheTryAcquireResult<K, V, C> =
    Result<Option<CacheLease<K, V, C>>, CacheError<<C as CacheCallbacks<K, V>>::Error>>;

impl<K, V, C> Clone for ConcurrentCache<K, V, C>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<K, V> ConcurrentCache<K, V, DefaultCacheCallbacks<V>>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Default + Send + Sync + 'static,
{
    /// Creates a default-backed cache.
    ///
    /// Default-created entries have cost one and a new read miss immediately
    /// returns a read lease, matching Darktable's non-callback allocation path.
    #[must_use]
    pub fn new(cost_quota: usize) -> Self {
        Self::from_parts(cost_quota, DefaultCacheCallbacks::default(), false)
    }
}

impl<K, V, C> ConcurrentCache<K, V, C>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    /// Creates a callback-backed cache.
    ///
    /// A newly allocated entry is returned write-held even when
    /// [`CacheMode::Read`] was requested. This lets the caller finish
    /// initialization and then use [`CacheWriteLease::demote`], matching
    /// Darktable's allocation-callback contract.
    #[must_use]
    pub fn with_callbacks(cost_quota: usize, callbacks: C) -> Self {
        Self::from_parts(cost_quota, callbacks, true)
    }

    fn from_parts(cost_quota: usize, callbacks: C, callback_backed: bool) -> Self {
        Self {
            inner: Arc::new(CacheInner {
                state: Mutex::new(CacheState::new(cost_quota)),
                changed: Condvar::new(),
                callbacks,
                callback_backed,
            }),
        }
    }

    /// Returns whether a fully initialized entry is published for `key`.
    ///
    /// This operation does not alter MRU ordering.
    ///
    /// # Errors
    ///
    /// Returns [`CacheError::Poisoned`] if the cache-wide state was poisoned.
    pub fn contains(&self, key: &K) -> Result<bool, CacheError<C::Error>> {
        let state = self.lock_state()?;
        Ok(state.entries.contains_key(key))
    }

    /// Returns current accounting without altering MRU ordering.
    ///
    /// # Errors
    ///
    /// Returns [`CacheError::Poisoned`] if the cache-wide state was poisoned.
    pub fn accounting(&self) -> Result<CacheAccounting, CacheError<C::Error>> {
        let state = self.lock_state()?;
        Ok(CacheAccounting {
            entries: state.entries.len(),
            pending: state.pending.len(),
            cost: state.cost,
            cost_quota: state.cost_quota,
        })
    }

    /// Updates the cache's soft cost quota and returns the previous quota.
    ///
    /// Existing entries are retained. Call [`Self::gc`] when the update should
    /// immediately reduce resident cost; otherwise the next allocating miss
    /// performs Darktable's normal 80%-fill collection attempt.
    ///
    /// # Errors
    ///
    /// Returns [`CacheError::Poisoned`] if the cache-wide state was poisoned.
    pub fn set_cost_quota(&self, cost_quota: usize) -> Result<usize, CacheError<C::Error>> {
        let mut state = self.lock_state()?;
        let previous = state.cost_quota;
        state.cost_quota = cost_quota;
        Ok(previous)
    }

    /// Snapshots published keys from oldest to most recently used.
    ///
    /// # Errors
    ///
    /// Returns a typed poison or capacity-allocation error.
    pub fn keys(&self) -> Result<Vec<K>, CacheError<C::Error>> {
        let state = self.lock_state()?;
        let mut keys = Vec::new();
        keys.try_reserve_exact(state.lru.len())
            .map_err(|_| CacheError::CapacityAllocation {
                resource: "key snapshot",
            })?;
        keys.extend(state.lru.iter().cloned());
        Ok(keys)
    }

    /// Acquires a read or write lease, allocating a miss exactly once.
    ///
    /// Existing entries block until the requested per-entry mode is available.
    /// A callback-created read miss returns [`CacheLease::Write`] so the caller
    /// can finalize it before demotion. Allocation completes before the entry is
    /// published; concurrent same-key misses wait for that one allocation.
    ///
    /// # Errors
    ///
    /// Returns typed poison, callback allocation, callback panic, cleanup,
    /// capacity-allocation, or cost-accounting errors.
    pub fn acquire(
        &self,
        key: K,
        requested: CacheMode,
    ) -> Result<CacheLease<K, V, C>, CacheError<C::Error>> {
        let mut attempted_gc = false;
        loop {
            let mut state = self.lock_state()?;
            if let Some(entry) = state.entries.get(&key).cloned() {
                drop(state);
                match entry.acquire(requested) {
                    Ok(()) => {}
                    Err(EntryAcquireError::Removed) => continue,
                    Err(EntryAcquireError::Poisoned) => {
                        return Err(CacheError::Poisoned(CachePoison::EntryLease));
                    }
                }
                let Ok(published_state) = self.inner.state.lock() else {
                    entry.release(requested);
                    return Err(CacheError::Poisoned(CachePoison::State));
                };
                state = published_state;
                let still_published = state
                    .entries
                    .get(&key)
                    .is_some_and(|published| Arc::ptr_eq(published, &entry));
                if still_published {
                    touch_mru(&mut state, &key);
                    drop(state);
                    return Ok(self.make_lease(entry, requested, false));
                }
                drop(state);
                entry.release(requested);
                continue;
            }

            if state.pending.contains(&key) {
                drop(
                    self.inner
                        .changed
                        .wait(state)
                        .map_err(|_| CacheError::Poisoned(CachePoison::State))?,
                );
                continue;
            }

            if !attempted_gc && over_automatic_gc_threshold(state.cost, state.cost_quota) {
                drop(state);
                self.gc(AUTOMATIC_GC_FILL_RATIO)?;
                attempted_gc = true;
                continue;
            }

            state
                .pending
                .try_reserve(1)
                .map_err(|_| CacheError::CapacityAllocation {
                    resource: "pending-key set",
                })?;
            state.pending.insert(key.clone());
            drop(state);
            return self.allocate_and_publish(key, requested);
        }
    }

    /// Immediately tries to lease an already-published entry.
    ///
    /// This method never allocates and never waits for either the cache-wide
    /// state or an entry lease. It returns `Ok(None)` when any required lock is
    /// busy, the key is missing, or the key is still being allocated.
    ///
    /// # Errors
    ///
    /// Returns [`CacheError::Poisoned`] if a lock encountered by the attempt was
    /// poisoned.
    pub fn try_acquire(&self, key: &K, requested: CacheMode) -> CacheTryAcquireResult<K, V, C> {
        let mut state = match self.inner.state.try_lock() {
            Ok(state) => state,
            Err(TryLockError::WouldBlock) => return Ok(None),
            Err(TryLockError::Poisoned(_)) => {
                return Err(CacheError::Poisoned(CachePoison::State));
            }
        };
        let Some(entry) = state.entries.get(key).cloned() else {
            return Ok(None);
        };
        match entry.try_acquire(requested).map_err(CacheError::Poisoned)? {
            EntryTryAcquire::Acquired => {
                touch_mru(&mut state, key);
                drop(state);
                Ok(Some(self.make_lease(entry, requested, false)))
            }
            EntryTryAcquire::Unavailable | EntryTryAcquire::Removed => Ok(None),
        }
    }

    /// Waits for exclusive access, removes `key`, and invokes cleanup.
    ///
    /// # Errors
    ///
    /// Returns typed poison, cleanup-panic, or accounting-underflow errors.
    pub fn remove(&self, key: &K) -> Result<CacheRemoveResult, CacheError<C::Error>> {
        loop {
            let state = self.lock_state()?;
            let Some(entry) = state.entries.get(key).cloned() else {
                return Ok(CacheRemoveResult::Missing);
            };
            drop(state);

            match entry.acquire(CacheMode::Write) {
                Ok(()) => {}
                Err(EntryAcquireError::Removed) => continue,
                Err(EntryAcquireError::Poisoned) => {
                    return Err(CacheError::Poisoned(CachePoison::EntryLease));
                }
            }

            let Ok(mut state) = self.inner.state.lock() else {
                entry.release(CacheMode::Write);
                return Err(CacheError::Poisoned(CachePoison::State));
            };
            let still_published = state
                .entries
                .get(key)
                .is_some_and(|published| Arc::ptr_eq(published, &entry));
            if !still_published {
                drop(state);
                entry.release(CacheMode::Write);
                continue;
            }

            if state.pending.try_reserve(1).is_err() {
                drop(state);
                entry.release(CacheMode::Write);
                return Err(CacheError::CapacityAllocation {
                    resource: "removal tombstone",
                });
            }
            if let Err(poison) = entry.mark_removed() {
                drop(state);
                entry.release(CacheMode::Write);
                return Err(CacheError::Poisoned(poison));
            }
            state.pending.insert(key.clone());
            state.entries.remove(key);
            remove_lru_key(&mut state.lru, key);
            let accounting_error = if let Some(cost) = state.cost.checked_sub(entry.cost) {
                state.cost = cost;
                None
            } else {
                let error = CacheError::CostUnderflow {
                    current: state.cost,
                    removed: entry.cost,
                };
                state.cost = 0;
                Some(error)
            };
            drop(state);

            let (allocation, value_poisoned) = entry.take_value_recover();
            entry.release(CacheMode::Write);
            let cleanup_error = allocation
                .map(|allocation| self.inner.cleanup(&entry.key, allocation))
                .transpose()
                .err();
            self.finish_pending(key);
            if let Some(error) = accounting_error {
                return Err(error);
            }
            if let Some(error) = cleanup_error {
                return Err(error);
            }
            if value_poisoned {
                return Err(CacheError::Poisoned(CachePoison::Value));
            }
            return Ok(CacheRemoveResult::Removed { cost: entry.cost });
        }
    }

    /// Removes unlocked entries from oldest to newest until total cost is
    /// strictly below `cost_quota * fill_ratio`, or no more entries can be
    /// removed.
    ///
    /// Collection is best-effort: entry locks are only tried, never waited on.
    /// Read-held, write-held, and demoting entries are skipped.
    ///
    /// # Errors
    ///
    /// Returns typed invalid-ratio, poison, cleanup-panic, capacity-allocation,
    /// or cost-accounting errors.
    pub fn gc(&self, fill_ratio: f64) -> Result<CacheGcReport, CacheError<C::Error>> {
        if !fill_ratio.is_finite() || fill_ratio < 0.0 {
            return Err(CacheError::InvalidFillRatio { ratio: fill_ratio });
        }
        let mut state = self.lock_state()?;
        let cost_before = state.cost;
        let mut removed = Vec::new();
        removed
            .try_reserve(state.entries.len())
            .map_err(|_| CacheError::CapacityAllocation {
                resource: "garbage-collection cleanup list",
            })?;
        let entry_count = state.entries.len();
        state
            .pending
            .try_reserve(entry_count)
            .map_err(|_| CacheError::CapacityAllocation {
                resource: "garbage-collection tombstones",
            })?;
        let mut skipped_locked = 0;
        let mut index = 0;
        let mut deferred_error = None;

        while index < state.lru.len()
            && !strictly_below_ratio(state.cost, state.cost_quota, fill_ratio)
        {
            let Some(key) = state.lru.get(index).cloned() else {
                break;
            };
            let Some(entry) = state.entries.get(&key).cloned() else {
                state.lru.remove(index);
                continue;
            };
            match entry.try_acquire(CacheMode::Write) {
                Ok(EntryTryAcquire::Acquired) => {}
                Ok(EntryTryAcquire::Unavailable | EntryTryAcquire::Removed) => {
                    skipped_locked += 1;
                    index += 1;
                    continue;
                }
                Err(poison) => {
                    deferred_error = Some(CacheError::Poisoned(poison));
                    break;
                }
            }

            let lease_poisoned = entry.mark_removed_recover();
            state.pending.insert(key.clone());
            state.entries.remove(&key);
            state.lru.remove(index);
            if let Some(cost) = state.cost.checked_sub(entry.cost) {
                state.cost = cost;
            } else {
                deferred_error = Some(CacheError::CostUnderflow {
                    current: state.cost,
                    removed: entry.cost,
                });
                state.cost = 0;
            }
            let (allocation, value_poisoned) = entry.take_value_recover();
            entry.release(CacheMode::Write);
            if lease_poisoned && deferred_error.is_none() {
                deferred_error = Some(CacheError::Poisoned(CachePoison::EntryLease));
            }
            if value_poisoned && deferred_error.is_none() {
                deferred_error = Some(CacheError::Poisoned(CachePoison::Value));
            }
            removed.push((entry.key.clone(), allocation));
            if deferred_error.is_some() {
                break;
            }
        }

        let cost_after = state.cost;
        drop(state);
        let removed_entries = removed.len();
        let reclaimed_cost = cost_before.saturating_sub(cost_after);
        for (key, allocation) in removed {
            if let Some(allocation) = allocation
                && let Err(error) = self.inner.cleanup(&key, allocation)
                && deferred_error.is_none()
            {
                deferred_error = Some(error);
            }
            self.finish_pending(&key);
        }
        if let Some(error) = deferred_error {
            return Err(error);
        }
        Ok(CacheGcReport {
            cost_before,
            cost_after,
            removed_entries,
            reclaimed_cost,
            skipped_locked,
        })
    }

    fn lock_state(&self) -> Result<MutexGuard<'_, CacheState<K, V>>, CacheError<C::Error>> {
        self.inner
            .state
            .lock()
            .map_err(|_| CacheError::Poisoned(CachePoison::State))
    }

    fn allocate_and_publish(
        &self,
        key: K,
        requested: CacheMode,
    ) -> Result<CacheLease<K, V, C>, CacheError<C::Error>> {
        let allocation =
            match catch_unwind(AssertUnwindSafe(|| self.inner.callbacks.allocate(&key))) {
                Ok(Ok(allocation)) => allocation,
                Ok(Err(error)) => {
                    self.finish_pending(&key);
                    return Err(CacheError::Allocation(error));
                }
                Err(_) => {
                    self.finish_pending(&key);
                    return Err(CacheError::AllocationPanicked);
                }
            };
        let cost = allocation.cost();
        let held_mode = if self.inner.callback_backed {
            CacheMode::Write
        } else {
            requested
        };

        let mut state = match self.inner.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                drop(poisoned.into_inner());
                self.cleanup_then_finish_pending(&key, allocation)?;
                return Err(CacheError::Poisoned(CachePoison::State));
            }
        };
        if let Err(error) = state.entries.try_reserve(1) {
            let _ = error;
            drop(state);
            self.cleanup_then_finish_pending(&key, allocation)?;
            return Err(CacheError::CapacityAllocation {
                resource: "entry map",
            });
        }
        if let Err(error) = state.lru.try_reserve(1) {
            let _ = error;
            drop(state);
            self.cleanup_then_finish_pending(&key, allocation)?;
            return Err(CacheError::CapacityAllocation {
                resource: "MRU queue",
            });
        }
        let Some(new_cost) = state.cost.checked_add(cost) else {
            let current = state.cost;
            drop(state);
            self.cleanup_then_finish_pending(&key, allocation)?;
            return Err(CacheError::CostOverflow {
                current,
                added: cost,
            });
        };

        state.pending.remove(&key);
        let published_key = key.clone();
        let entry = Arc::new(Entry::new_held(key, allocation, held_mode));
        state
            .entries
            .insert(published_key.clone(), Arc::clone(&entry));
        state.lru.push_back(published_key);
        state.cost = new_cost;
        drop(state);
        self.inner.changed.notify_all();
        Ok(self.make_lease(entry, held_mode, true))
    }

    fn finish_pending(&self, key: &K) {
        let mut state = match self.inner.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.pending.remove(key);
        drop(state);
        self.inner.changed.notify_all();
    }

    fn cleanup_then_finish_pending(
        &self,
        key: &K,
        allocation: CacheAllocation<V>,
    ) -> Result<(), CacheError<C::Error>> {
        let cleanup = self.inner.cleanup(key, allocation);
        self.finish_pending(key);
        cleanup
    }

    fn make_lease(
        &self,
        entry: Arc<Entry<K, V>>,
        mode: CacheMode,
        created: bool,
    ) -> CacheLease<K, V, C> {
        match mode {
            CacheMode::Read => CacheLease::Read(CacheReadLease {
                inner: Arc::clone(&self.inner),
                entry,
                active: true,
                created,
            }),
            CacheMode::Write => CacheLease::Write(CacheWriteLease {
                inner: Arc::clone(&self.inner),
                entry,
                active: true,
                created,
            }),
        }
    }
}

fn over_automatic_gc_threshold(cost: usize, quota: usize) -> bool {
    (cost as u128) * 5 > (quota as u128) * 4
}

#[allow(clippy::cast_precision_loss)]
fn strictly_below_ratio(cost: usize, quota: usize, fill_ratio: f64) -> bool {
    (cost as f64) < (quota as f64) * fill_ratio
}

fn touch_mru<K, V>(state: &mut CacheState<K, V>, key: &K)
where
    K: Eq + Clone,
{
    remove_lru_key(&mut state.lru, key);
    state.lru.push_back(key.clone());
}

fn remove_lru_key<K>(lru: &mut VecDeque<K>, key: &K)
where
    K: Eq,
{
    if let Some(position) = lru.iter().position(|candidate| candidate == key) {
        lru.remove(position);
    }
}

/// Owned read or write lease returned by [`ConcurrentCache::acquire`] and
/// [`ConcurrentCache::try_acquire`].
pub enum CacheLease<K, V, C = DefaultCacheCallbacks<V>>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    /// Shared read access.
    Read(CacheReadLease<K, V, C>),
    /// Exclusive mutable access.
    Write(CacheWriteLease<K, V, C>),
}

impl<K, V, C> CacheLease<K, V, C>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    /// Returns the held mode.
    #[must_use]
    pub const fn mode(&self) -> CacheMode {
        match self {
            Self::Read(_) => CacheMode::Read,
            Self::Write(_) => CacheMode::Write,
        }
    }

    /// Returns whether this lease published a newly allocated entry.
    #[must_use]
    pub const fn was_created(&self) -> bool {
        match self {
            Self::Read(lease) => lease.was_created(),
            Self::Write(lease) => lease.was_created(),
        }
    }

    /// Returns the leased key.
    #[must_use]
    pub fn key(&self) -> &K {
        match self {
            Self::Read(lease) => lease.key(),
            Self::Write(lease) => lease.key(),
        }
    }

    /// Returns the entry's callback-selected cost.
    #[must_use]
    pub fn cost(&self) -> usize {
        match self {
            Self::Read(lease) => lease.cost(),
            Self::Write(lease) => lease.cost(),
        }
    }

    /// Runs a closure with shared access to the value.
    ///
    /// # Errors
    ///
    /// Returns a typed value-lock poison or unavailable-value error.
    pub fn with_value<R>(&self, operation: impl FnOnce(&V) -> R) -> Result<R, CacheLeaseError> {
        match self {
            Self::Read(lease) => lease.with_value(operation),
            Self::Write(lease) => lease.with_value(operation),
        }
    }

    /// Converts a write lease to a read lease atomically. A read lease is
    /// returned unchanged.
    ///
    /// # Errors
    ///
    /// Returns a typed entry-lock poison or unavailable-value error.
    pub fn into_read(self) -> Result<CacheReadLease<K, V, C>, CacheLeaseError> {
        match self {
            Self::Read(lease) => Ok(lease),
            Self::Write(lease) => lease.demote(),
        }
    }
}

/// Shared, owned cache-entry lease.
pub struct CacheReadLease<K, V, C = DefaultCacheCallbacks<V>>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    inner: Arc<CacheInner<K, V, C>>,
    entry: Arc<Entry<K, V>>,
    active: bool,
    created: bool,
}

impl<K, V, C> CacheReadLease<K, V, C>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    /// Returns the leased key.
    #[must_use]
    pub fn key(&self) -> &K {
        &self.entry.key
    }

    /// Returns the callback-selected entry cost.
    #[must_use]
    pub fn cost(&self) -> usize {
        self.entry.cost
    }

    /// Returns whether this lease published a newly allocated entry.
    #[must_use]
    pub const fn was_created(&self) -> bool {
        self.created
    }

    /// Acquires a short-lived standard-library read guard.
    ///
    /// The logical lease already excludes writers for its complete lifetime;
    /// this guard only provides safe access to the owned entry storage.
    ///
    /// # Errors
    ///
    /// Returns a typed value-lock poison or unavailable-value error.
    pub fn read(&self) -> Result<CacheValueRead<'_, V>, CacheLeaseError> {
        CacheValueRead::new(
            self.entry
                .value
                .read()
                .map_err(|_| CacheLeaseError::Poisoned(CachePoison::Value))?,
        )
    }

    /// Runs a closure with shared value access.
    ///
    /// # Errors
    ///
    /// Returns a typed value-lock poison or unavailable-value error.
    pub fn with_value<R>(&self, operation: impl FnOnce(&V) -> R) -> Result<R, CacheLeaseError> {
        let value = self.read()?;
        Ok(operation(&value))
    }
}

impl<K, V, C> Drop for CacheReadLease<K, V, C>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    fn drop(&mut self) {
        if self.active {
            self.entry.release(CacheMode::Read);
            self.active = false;
        }
        let _ = &self.inner;
    }
}

/// Exclusive, owned cache-entry lease.
pub struct CacheWriteLease<K, V, C = DefaultCacheCallbacks<V>>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    inner: Arc<CacheInner<K, V, C>>,
    entry: Arc<Entry<K, V>>,
    active: bool,
    created: bool,
}

impl<K, V, C> CacheWriteLease<K, V, C>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    /// Returns the leased key.
    #[must_use]
    pub fn key(&self) -> &K {
        &self.entry.key
    }

    /// Returns the callback-selected entry cost.
    #[must_use]
    pub fn cost(&self) -> usize {
        self.entry.cost
    }

    /// Returns whether this lease published a newly allocated entry.
    #[must_use]
    pub const fn was_created(&self) -> bool {
        self.created
    }

    /// Acquires a short-lived standard-library read guard.
    ///
    /// # Errors
    ///
    /// Returns a typed value-lock poison or unavailable-value error.
    pub fn read(&self) -> Result<CacheValueRead<'_, V>, CacheLeaseError> {
        CacheValueRead::new(
            self.entry
                .value
                .read()
                .map_err(|_| CacheLeaseError::Poisoned(CachePoison::Value))?,
        )
    }

    /// Acquires a short-lived standard-library write guard.
    ///
    /// # Errors
    ///
    /// Returns a typed value-lock poison or unavailable-value error.
    pub fn write(&self) -> Result<CacheValueWrite<'_, V>, CacheLeaseError> {
        CacheValueWrite::new(
            self.entry
                .value
                .write()
                .map_err(|_| CacheLeaseError::Poisoned(CachePoison::Value))?,
        )
    }

    /// Runs a closure with shared value access.
    ///
    /// # Errors
    ///
    /// Returns a typed value-lock poison or unavailable-value error.
    pub fn with_value<R>(&self, operation: impl FnOnce(&V) -> R) -> Result<R, CacheLeaseError> {
        let value = self.read()?;
        Ok(operation(&value))
    }

    /// Runs a closure with exclusive mutable value access.
    ///
    /// # Errors
    ///
    /// Returns a typed value-lock poison or unavailable-value error.
    pub fn with_value_mut<R>(
        &self,
        operation: impl FnOnce(&mut V) -> R,
    ) -> Result<R, CacheLeaseError> {
        let mut value = self.write()?;
        Ok(operation(&mut value))
    }

    /// Atomically changes this write lease into a read lease.
    ///
    /// Removal and garbage collection cannot observe an unlocked interval
    /// during this transition.
    ///
    /// # Errors
    ///
    /// Returns a typed entry-lock poison or unavailable-value error.
    pub fn demote(mut self) -> Result<CacheReadLease<K, V, C>, CacheLeaseError> {
        self.entry.demote()?;
        self.active = false;
        Ok(CacheReadLease {
            inner: Arc::clone(&self.inner),
            entry: Arc::clone(&self.entry),
            active: true,
            created: self.created,
        })
    }
}

impl<K, V, C> Drop for CacheWriteLease<K, V, C>
where
    K: Eq + Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    fn drop(&mut self) {
        if self.active {
            self.entry.release(CacheMode::Write);
            self.active = false;
        }
    }
}

/// Short-lived shared value guard obtained from an owned cache lease.
pub struct CacheValueRead<'a, V> {
    guard: RwLockReadGuard<'a, Option<CacheAllocation<V>>>,
}

impl<'a, V> CacheValueRead<'a, V> {
    fn new(
        guard: RwLockReadGuard<'a, Option<CacheAllocation<V>>>,
    ) -> Result<Self, CacheLeaseError> {
        if guard.is_none() {
            return Err(CacheLeaseError::ValueUnavailable);
        }
        Ok(Self { guard })
    }
}

impl<V> Deref for CacheValueRead<'_, V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        self.guard
            .as_ref()
            .expect("cache value was verified when the guard was created")
            .value()
    }
}

/// Short-lived exclusive value guard obtained from a write cache lease.
pub struct CacheValueWrite<'a, V> {
    guard: RwLockWriteGuard<'a, Option<CacheAllocation<V>>>,
}

impl<'a, V> CacheValueWrite<'a, V> {
    fn new(
        guard: RwLockWriteGuard<'a, Option<CacheAllocation<V>>>,
    ) -> Result<Self, CacheLeaseError> {
        if guard.is_none() {
            return Err(CacheLeaseError::ValueUnavailable);
        }
        Ok(Self { guard })
    }
}

impl<V> Deref for CacheValueWrite<'_, V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        self.guard
            .as_ref()
            .expect("cache value was verified when the guard was created")
            .value()
    }
}

impl<V> DerefMut for CacheValueWrite<'_, V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.guard
            .as_mut()
            .expect("cache value was verified when the guard was created")
            .value_mut()
    }
}
