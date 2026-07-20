use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use super::storage::AlignedStorage;
use super::views::{BufferRead, BufferWrite, HostImageView};
use super::{
    AcquireOptions, AllocationClass, BufferRequest, HostPoolError, InitializationPolicy,
    LeaseState, PoolAccounting, PoolBudgets, PoolEvent, PriorityClass, ReturnReceipt,
    ShutdownReport, checked_bucket_request, within_waste_limit,
};
use crate::ImageDescriptor;

#[derive(Debug)]
struct PoolShared {
    state: Mutex<PoolState>,
    wake: Condvar,
}

#[derive(Debug)]
struct PoolState {
    budgets: PoolBudgets,
    allocated_bytes: usize,
    idle_bytes: usize,
    outstanding_bytes: usize,
    outstanding: BTreeSet<u64>,
    idle: Vec<IdleAllocation>,
    class_bytes: BTreeMap<AllocationClass, usize>,
    events: VecDeque<PoolEvent>,
    waiters: BTreeMap<u64, PriorityClass>,
    next_id: u64,
    next_ticket: u64,
    reuse_count: u64,
    eviction_count: u64,
    poison_count: u64,
    failure_count: u64,
    wait_count: u64,
    shutting_down: bool,
}

#[derive(Debug)]
struct IdleAllocation {
    class: AllocationClass,
    requested_bytes: usize,
    capacity: usize,
    returned_at: u64,
    storage: AlignedStorage,
}

#[derive(Debug)]
struct LeaseInner {
    id: u64,
    class: AllocationClass,
    requested_bytes: usize,
    capacity: usize,
    pooled: bool,
    state: LeaseState,
    storage: Option<AlignedStorage>,
}

/// A cloneable owner of aligned host allocations and their bounded leases.
#[derive(Debug, Clone)]
pub struct HostBufferPool {
    shared: Arc<PoolShared>,
}

impl HostBufferPool {
    #[must_use]
    pub fn new(budgets: PoolBudgets) -> Self {
        Self {
            shared: Arc::new(PoolShared {
                state: Mutex::new(PoolState::new(budgets)),
                wake: Condvar::new(),
            }),
        }
    }

    /// Attempts an acquire without registering a waiter or blocking.
    ///
    /// # Errors
    ///
    /// Returns a typed request, budget, transition, or availability error.
    pub fn try_acquire(&self, request: BufferRequest) -> Result<BufferLease, HostPoolError> {
        let mut state = lock_state(&self.shared.state);
        let lease = grant(&mut state, request)?;
        Ok(BufferLease {
            pool: Arc::clone(&self.shared),
            inner: Some(lease),
        })
    }

    /// Acquires with FIFO ordering inside the request's priority class.
    ///
    /// # Errors
    ///
    /// Returns when the request is invalid, cancelled, timed out, or the pool
    /// is shut down.
    ///
    /// # Panics
    ///
    /// Panics only if the pool mutex is poisoned by an unrelated panic.
    pub fn acquire(
        &self,
        request: BufferRequest,
        options: &AcquireOptions,
    ) -> Result<BufferLease, HostPoolError> {
        let priority = options.priority().unwrap_or(request.priority());
        let deadline = options.timeout().map(|timeout| Instant::now() + timeout);
        let mut state = lock_state(&self.shared.state);
        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.saturating_add(1);
        state.waiters.insert(ticket, priority);
        state.wait_count = state.wait_count.saturating_add(1);

        loop {
            if state.shutting_down {
                state.waiters.remove(&ticket);
                self.shared.wake.notify_all();
                return Err(HostPoolError::Shutdown);
            }
            if options
                .cancellation()
                .is_some_and(super::CancellationToken::is_cancelled)
            {
                state.waiters.remove(&ticket);
                self.shared.wake.notify_all();
                return Err(HostPoolError::Cancelled);
            }
            let earlier = state
                .waiters
                .iter()
                .any(|(&other, &other_priority)| other < ticket && other_priority == priority);
            if !earlier {
                match grant(&mut state, request) {
                    Ok(lease) => {
                        state.waiters.remove(&ticket);
                        self.shared.wake.notify_all();
                        return Ok(BufferLease {
                            pool: Arc::clone(&self.shared),
                            inner: Some(lease),
                        });
                    }
                    Err(error @ HostPoolError::BudgetExceeded) => {
                        state.waiters.remove(&ticket);
                        self.shared.wake.notify_all();
                        return Err(error);
                    }
                    Err(error @ HostPoolError::EntryLimitExceeded)
                        if state.budgets.max_entries() == 0 =>
                    {
                        state.waiters.remove(&ticket);
                        self.shared.wake.notify_all();
                        return Err(error);
                    }
                    Err(HostPoolError::Shutdown) => {
                        state.waiters.remove(&ticket);
                        self.shared.wake.notify_all();
                        return Err(HostPoolError::Shutdown);
                    }
                    Err(
                        HostPoolError::InvalidRequest
                        | HostPoolError::InvalidAlignment { .. }
                        | HostPoolError::DescriptorMismatch
                        | HostPoolError::UnsupportedAlignment { .. }
                        | HostPoolError::ArithmeticOverflow,
                    ) => {
                        state.waiters.remove(&ticket);
                        self.shared.wake.notify_all();
                        return grant(&mut state, request).map(|lease| BufferLease {
                            pool: Arc::clone(&self.shared),
                            inner: Some(lease),
                        });
                    }
                    Err(_) => {}
                }
            }

            let mut wait_for = deadline.map(|end| end.saturating_duration_since(Instant::now()));
            if options.cancellation().is_some() {
                wait_for = Some(
                    wait_for
                        .unwrap_or(Duration::from_millis(10))
                        .min(Duration::from_millis(10)),
                );
            }
            if wait_for == Some(Duration::ZERO) {
                state.waiters.remove(&ticket);
                self.shared.wake.notify_all();
                return Err(HostPoolError::DeadlineExceeded);
            }
            state = if let Some(wait_for) = wait_for {
                let (guard, _result) = self
                    .shared
                    .wake
                    .wait_timeout(state, wait_for)
                    .expect("host pool mutex is not poisoned");
                guard
            } else {
                self.shared
                    .wake
                    .wait(state)
                    .expect("host pool mutex is not poisoned")
            };
        }
    }

    /// Replaces hard limits and immediately evicts idle entries that no longer fit.
    ///
    /// # Errors
    ///
    /// Returns an accounting error if an internal capacity invariant is broken.
    pub fn update_budgets(&self, budgets: PoolBudgets) -> Result<PoolAccounting, HostPoolError> {
        let mut state = lock_state(&self.shared.state);
        state.budgets = budgets;
        state.events.push_back(PoolEvent::BudgetUpdated);
        evict_to_limits(&mut state);
        self.shared.wake.notify_all();
        state.accounting()
    }

    #[must_use]
    pub fn budgets(&self) -> PoolBudgets {
        lock_state(&self.shared.state).budgets
    }

    #[must_use]
    pub fn accounting(&self) -> PoolAccounting {
        lock_state(&self.shared.state)
            .accounting()
            .unwrap_or_default()
    }

    /// Removes health and pressure events without invoking a global callback.
    #[must_use]
    pub fn drain_events(&self) -> Vec<PoolEvent> {
        lock_state(&self.shared.state).events.drain(..).collect()
    }

    /// Stops new work, frees idle storage, and waits up to `timeout` for owners.
    ///
    /// # Panics
    ///
    /// Panics only if the pool mutex is poisoned by an unrelated panic.
    #[must_use]
    pub fn shutdown(&self, timeout: Duration) -> ShutdownReport {
        let deadline = Instant::now() + timeout;
        let mut state = lock_state(&self.shared.state);
        state.shutting_down = true;
        let idle_entries_freed = state.idle.len();
        let idle_bytes_freed = state.idle_bytes;
        let idle_entries = std::mem::take(&mut state.idle);
        for idle in idle_entries {
            subtract_class_bytes(&mut state.class_bytes, idle.class, idle.capacity);
        }
        state.allocated_bytes = state.allocated_bytes.saturating_sub(idle_bytes_freed);
        state.idle_bytes = 0;
        self.shared.wake.notify_all();
        while !state.outstanding.is_empty() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining == Duration::ZERO {
                break;
            }
            state = self
                .shared
                .wake
                .wait_timeout(state, remaining)
                .expect("host pool mutex is not poisoned")
                .0;
        }
        ShutdownReport {
            outstanding_entries: state.outstanding.len(),
            outstanding_bytes: state.outstanding_bytes,
            idle_entries_freed,
            idle_bytes_freed,
            timed_out: !state.outstanding.is_empty(),
        }
    }
}

impl PoolState {
    fn new(budgets: PoolBudgets) -> Self {
        Self {
            budgets,
            allocated_bytes: 0,
            idle_bytes: 0,
            outstanding_bytes: 0,
            outstanding: BTreeSet::new(),
            idle: Vec::new(),
            class_bytes: BTreeMap::new(),
            events: VecDeque::new(),
            waiters: BTreeMap::new(),
            next_id: 1,
            next_ticket: 1,
            reuse_count: 0,
            eviction_count: 0,
            poison_count: 0,
            failure_count: 0,
            wait_count: 0,
            shutting_down: false,
        }
    }

    fn accounting(&self) -> Result<PoolAccounting, HostPoolError> {
        if self.allocated_bytes != self.idle_bytes.saturating_add(self.outstanding_bytes)
            || self.outstanding.len() > self.budgets.max_entries() && self.idle.is_empty()
        {
            return Err(HostPoolError::AccountingCorruption);
        }
        Ok(PoolAccounting {
            allocated_bytes: self.allocated_bytes,
            idle_bytes: self.idle_bytes,
            outstanding_bytes: self.outstanding_bytes,
            entries: self.idle.len().saturating_add(self.outstanding.len()),
            outstanding_entries: self.outstanding.len(),
            reuse_count: self.reuse_count,
            eviction_count: self.eviction_count,
            poison_count: self.poison_count,
            failure_count: self.failure_count,
            wait_count: self.wait_count,
        })
    }
}

fn grant(state: &mut PoolState, request: BufferRequest) -> Result<LeaseInner, HostPoolError> {
    validate_request(state, request)?;
    let class = request.class();
    let (capacity, pooled, mut storage, fresh) = take_or_allocate(state, request)?;
    if storage.capacity() != capacity {
        return Err(HostPoolError::AccountingCorruption);
    }
    let id = state.next_id;
    state.next_id = state.next_id.saturating_add(1);
    if fresh {
        state.allocated_bytes = state.allocated_bytes.saturating_add(capacity);
    }
    state.outstanding_bytes = state.outstanding_bytes.saturating_add(capacity);
    state.outstanding.insert(id);
    if fresh {
        *state.class_bytes.entry(class).or_default() = state
            .class_bytes
            .get(&class)
            .copied()
            .unwrap_or(0)
            .saturating_add(capacity);
    }
    let initialized = match request.initialization() {
        InitializationPolicy::Zeroed => {
            storage.zero();
            true
        }
        InitializationPolicy::Uninitialized | InitializationPolicy::Callback => false,
    };
    Ok(LeaseInner {
        id,
        class,
        requested_bytes: request.bytes(),
        capacity,
        pooled,
        state: if initialized {
            LeaseState::ExclusiveInitialized
        } else {
            LeaseState::ExclusiveUninitialized
        },
        storage: Some(storage),
    })
}

fn validate_request(state: &PoolState, request: BufferRequest) -> Result<(), HostPoolError> {
    if state.shutting_down {
        return Err(HostPoolError::Shutdown);
    }
    if request.bytes() > state.budgets.per_request_bytes()
        || state.budgets.per_request_entries() < 1
    {
        return Err(HostPoolError::BudgetExceeded);
    }
    if state.outstanding.len().saturating_add(state.idle.len()) >= state.budgets.max_entries() {
        return Err(HostPoolError::EntryLimitExceeded);
    }
    Ok(())
}

fn take_or_allocate(
    state: &mut PoolState,
    request: BufferRequest,
) -> Result<(usize, bool, AlignedStorage, bool), HostPoolError> {
    let class = request.class();
    let idle_index = state
        .idle
        .iter()
        .enumerate()
        .filter(|(_, idle)| {
            idle.class == class
                && idle.capacity >= request.bytes()
                && within_waste_limit(idle.capacity, request.bytes())
        })
        .min_by_key(|(_, idle)| idle.capacity)
        .map(|(index, _)| index);
    if let Some(index) = idle_index {
        let idle = state.idle.swap_remove(index);
        state.idle_bytes = state.idle_bytes.saturating_sub(idle.capacity);
        state.reuse_count = state.reuse_count.saturating_add(1);
        return Ok((idle.capacity, true, idle.storage, false));
    }
    let bucket = checked_bucket_request(request.bytes())?;
    let pooled = within_waste_limit(bucket, request.bytes());
    let requested_capacity = if pooled { bucket } else { request.bytes() };
    let capacity = round_up(requested_capacity, class.alignment().bytes())?;
    check_capacity(state, request, capacity)?;
    let storage = match AlignedStorage::allocate(class.alignment(), capacity) {
        Ok(storage) => storage,
        Err(error) => {
            state.failure_count = state.failure_count.saturating_add(1);
            state.events.push_back(PoolEvent::AllocationFailed);
            return Err(error);
        }
    };
    Ok((capacity, pooled, storage, true))
}

fn check_capacity(
    state: &PoolState,
    request: BufferRequest,
    capacity: usize,
) -> Result<(), HostPoolError> {
    let class_bytes = state
        .class_bytes
        .get(&request.class())
        .copied()
        .unwrap_or(0);
    if capacity > state.budgets.per_class_bytes()
        || class_bytes.saturating_add(capacity) > state.budgets.per_class_bytes()
        || state.allocated_bytes.saturating_add(capacity) > state.budgets.total_allocated_bytes()
    {
        return Err(
            if request.bytes() > state.budgets.total_allocated_bytes()
                || request.bytes() > state.budgets.per_class_bytes()
            {
                HostPoolError::BudgetExceeded
            } else {
                HostPoolError::WouldBlock
            },
        );
    }
    Ok(())
}

fn round_up(value: usize, alignment: usize) -> Result<usize, HostPoolError> {
    value
        .checked_add(alignment.saturating_sub(1))
        .map(|value| value / alignment * alignment)
        .ok_or(HostPoolError::ArithmeticOverflow)
}

fn return_inner(shared: &Arc<PoolShared>, mut inner: LeaseInner) -> ReturnReceipt {
    let mut state = lock_state(&shared.state);
    let poisoned = inner.state == LeaseState::Poisoned;
    let Some(storage) = inner.storage.take() else {
        return ReturnReceipt {
            lease_id: inner.id,
            pooled: false,
            capacity: inner.capacity,
            poisoned: true,
        };
    };
    state.outstanding.remove(&inner.id);
    state.outstanding_bytes = state.outstanding_bytes.saturating_sub(inner.capacity);
    let retain = inner.pooled
        && !poisoned
        && !state.shutting_down
        && state.idle_bytes.saturating_add(inner.capacity) <= state.budgets.idle_bytes()
        && state.allocated_bytes <= state.budgets.total_allocated_bytes();
    if retain {
        state.idle_bytes = state.idle_bytes.saturating_add(inner.capacity);
        let returned_at = state.next_id;
        state.idle.push(IdleAllocation {
            class: inner.class,
            requested_bytes: inner.requested_bytes,
            capacity: inner.capacity,
            returned_at,
            storage,
        });
        state.next_id = state.next_id.saturating_add(1);
    } else {
        state.allocated_bytes = state.allocated_bytes.saturating_sub(inner.capacity);
        subtract_class_bytes(&mut state.class_bytes, inner.class, inner.capacity);
        if poisoned {
            state.poison_count = state.poison_count.saturating_add(1);
            state.events.push_back(PoolEvent::Poisoned {
                bytes: inner.capacity,
            });
        } else if inner.pooled {
            state.eviction_count = state.eviction_count.saturating_add(1);
            state.events.push_back(PoolEvent::Evicted {
                bytes: inner.capacity,
            });
        }
    }
    evict_to_limits(&mut state);
    let allocated_bytes = state.allocated_bytes;
    let total_bytes = state.budgets.total_allocated_bytes();
    state.events.push_back(PoolEvent::Pressure {
        allocated_bytes,
        total_bytes,
    });
    shared.wake.notify_all();
    ReturnReceipt {
        lease_id: inner.id,
        pooled: retain,
        capacity: inner.capacity,
        poisoned,
    }
}

fn evict_to_limits(state: &mut PoolState) {
    let idle_limit = state.budgets.idle_bytes().min(
        state
            .budgets
            .total_allocated_bytes()
            .saturating_sub(state.outstanding_bytes),
    );
    while state.idle_bytes > idle_limit
        || state.idle.len().saturating_add(state.outstanding.len()) > state.budgets.max_entries()
    {
        let Some(index) = state
            .idle
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| {
                (
                    left.capacity.saturating_sub(left.requested_bytes),
                    right.returned_at,
                )
                    .cmp(&(
                        right.capacity.saturating_sub(right.requested_bytes),
                        left.returned_at,
                    ))
            })
            .map(|(index, _)| index)
        else {
            break;
        };
        let idle = state.idle.swap_remove(index);
        state.idle_bytes = state.idle_bytes.saturating_sub(idle.capacity);
        state.allocated_bytes = state.allocated_bytes.saturating_sub(idle.capacity);
        subtract_class_bytes(&mut state.class_bytes, idle.class, idle.capacity);
        state.eviction_count = state.eviction_count.saturating_add(1);
        state.events.push_back(PoolEvent::Evicted {
            bytes: idle.capacity,
        });
    }
}

fn subtract_class_bytes(
    class_bytes: &mut BTreeMap<AllocationClass, usize>,
    class: AllocationClass,
    amount: usize,
) {
    if let Some(current) = class_bytes.get_mut(&class) {
        *current = current.saturating_sub(amount);
        if *current == 0 {
            class_bytes.remove(&class);
        }
    }
}

fn lock_state(state: &Mutex<PoolState>) -> MutexGuard<'_, PoolState> {
    state.lock().expect("host pool mutex is not poisoned")
}

/// An exclusive lease. Its destructor returns valid storage through the pool;
/// poisoned and direct allocations are freed instead of being reused.
#[derive(Debug)]
pub struct BufferLease {
    pool: Arc<PoolShared>,
    inner: Option<LeaseInner>,
}

impl BufferLease {
    #[must_use]
    pub const fn state(&self) -> LeaseState {
        match self.inner.as_ref() {
            Some(inner) => inner.state,
            None => LeaseState::Returning,
        }
    }

    #[must_use]
    pub fn class(&self) -> Option<AllocationClass> {
        self.inner.as_ref().map(|inner| inner.class)
    }

    #[must_use]
    pub fn requested_bytes(&self) -> Option<usize> {
        self.inner.as_ref().map(|inner| inner.requested_bytes)
    }

    #[must_use]
    pub fn capacity(&self) -> Option<usize> {
        self.inner.as_ref().map(|inner| inner.capacity)
    }

    /// Zeroes all storage and publishes it as initialized.
    ///
    /// # Errors
    ///
    /// Returns an error when the lease is not exclusively uninitialized.
    pub fn initialize_zeroed(&mut self) -> Result<(), HostPoolError> {
        let inner = self.inner.as_mut().ok_or(HostPoolError::AlreadyReturned)?;
        if inner.state != LeaseState::ExclusiveUninitialized {
            return Err(HostPoolError::InvalidTransition { state: inner.state });
        }
        inner
            .storage
            .as_mut()
            .ok_or(HostPoolError::AlreadyReturned)?
            .zero();
        inner.state = LeaseState::ExclusiveInitialized;
        Ok(())
    }

    /// Runs a checked initializer and publishes only after it succeeds.
    ///
    /// # Errors
    ///
    /// Returns the callback error or a panic/transition error and poisons the lease.
    pub fn initialize_with<F>(&mut self, callback: F) -> Result<(), HostPoolError>
    where
        F: FnOnce(&mut BufferWrite<'_>) -> Result<(), HostPoolError> + std::panic::UnwindSafe,
    {
        let inner = self.inner.as_mut().ok_or(HostPoolError::AlreadyReturned)?;
        if inner.state != LeaseState::ExclusiveUninitialized {
            return Err(HostPoolError::InvalidTransition { state: inner.state });
        }
        let storage = inner
            .storage
            .as_mut()
            .ok_or(HostPoolError::AlreadyReturned)?;
        let mut view = BufferWrite {
            storage,
            length: inner.requested_bytes,
        };
        match catch_unwind(AssertUnwindSafe(|| callback(&mut view))) {
            Ok(Ok(())) => inner.state = LeaseState::ExclusiveInitialized,
            Ok(Err(error)) => {
                inner.state = LeaseState::Poisoned;
                return Err(error);
            }
            Err(_) => {
                inner.state = LeaseState::Poisoned;
                return Err(HostPoolError::CallbackPanicked);
            }
        }
        Ok(())
    }

    /// Publishes bytes written through [`Self::write_view`].
    ///
    /// # Errors
    ///
    /// Returns an error when the lease is not exclusively uninitialized.
    pub fn mark_initialized(&mut self) -> Result<(), HostPoolError> {
        let inner = self.inner.as_mut().ok_or(HostPoolError::AlreadyReturned)?;
        if inner.state != LeaseState::ExclusiveUninitialized {
            return Err(HostPoolError::InvalidTransition { state: inner.state });
        }
        inner.state = LeaseState::ExclusiveInitialized;
        Ok(())
    }

    /// Borrows the exclusive write view for the requested byte length.
    ///
    /// # Errors
    ///
    /// Returns an error for immutable, poisoned, or returned leases.
    pub fn write_view(&mut self) -> Result<BufferWrite<'_>, HostPoolError> {
        let inner = self.inner.as_mut().ok_or(HostPoolError::AlreadyReturned)?;
        if !matches!(
            inner.state,
            LeaseState::ExclusiveUninitialized | LeaseState::ExclusiveInitialized
        ) {
            return Err(if inner.state == LeaseState::Poisoned {
                HostPoolError::Poisoned
            } else {
                HostPoolError::InvalidTransition { state: inner.state }
            });
        }
        let storage = inner
            .storage
            .as_mut()
            .ok_or(HostPoolError::AlreadyReturned)?;
        Ok(BufferWrite {
            storage,
            length: inner.requested_bytes,
        })
    }

    /// Borrows an immutable view after initialization is published.
    ///
    /// # Errors
    ///
    /// Returns an error before publication or after return/poison.
    pub fn read_view(&self) -> Result<BufferRead<'_>, HostPoolError> {
        let inner = self.inner.as_ref().ok_or(HostPoolError::AlreadyReturned)?;
        if !matches!(inner.state, LeaseState::ExclusiveInitialized) {
            return Err(if inner.state == LeaseState::Poisoned {
                HostPoolError::Poisoned
            } else {
                HostPoolError::InvalidTransition { state: inner.state }
            });
        }
        Ok(BufferRead {
            storage: inner
                .storage
                .as_ref()
                .ok_or(HostPoolError::AlreadyReturned)?,
            length: inner.requested_bytes,
        })
    }

    /// Checks the complete image format and byte-length contract.
    ///
    /// # Errors
    ///
    /// Returns [`HostPoolError::DescriptorMismatch`] for a different class or extent.
    pub fn validate_descriptor(&self, descriptor: &ImageDescriptor) -> Result<(), HostPoolError> {
        let inner = self.inner.as_ref().ok_or(HostPoolError::AlreadyReturned)?;
        if descriptor.format() != inner.class.format()
            || descriptor.byte_length() != inner.requested_bytes
        {
            return Err(HostPoolError::DescriptorMismatch);
        }
        Ok(())
    }

    /// Creates a descriptor-backed immutable host view.
    ///
    /// # Errors
    ///
    /// Returns a descriptor or lease-state error.
    pub fn image_view<'descriptor, 'bytes>(
        &'bytes self,
        descriptor: &'descriptor ImageDescriptor,
    ) -> Result<HostImageView<'descriptor, 'bytes>, HostPoolError> {
        self.validate_descriptor(descriptor)?;
        Ok(HostImageView {
            descriptor,
            bytes: self.read_view()?,
        })
    }

    /// Marks storage unusable and ensures it is freed on return.
    ///
    /// # Errors
    ///
    /// Returns an error when the lease was already returned.
    pub fn poison(&mut self) -> Result<(), HostPoolError> {
        let inner = self.inner.as_mut().ok_or(HostPoolError::AlreadyReturned)?;
        if inner.state == LeaseState::Returning {
            return Err(HostPoolError::AlreadyReturned);
        }
        if let Some(storage) = inner.storage.as_mut() {
            storage.zero();
        }
        inner.state = LeaseState::Poisoned;
        Ok(())
    }

    /// Returns the lease exactly once through the pool accounting boundary.
    ///
    /// # Errors
    ///
    /// Returns [`HostPoolError::AlreadyReturned`] after the first return.
    pub fn return_to_pool(&mut self) -> Result<ReturnReceipt, HostPoolError> {
        let mut inner = self.inner.take().ok_or(HostPoolError::AlreadyReturned)?;
        if inner.state == LeaseState::Returning {
            return Err(HostPoolError::AlreadyReturned);
        }
        if inner.state != LeaseState::Poisoned {
            inner.state = LeaseState::Returning;
        }
        Ok(return_inner(&self.pool, inner))
    }

    /// Freezes an initialized exclusive lease into immutable ownership.
    ///
    /// # Errors
    ///
    /// Returns the original lease unless it is exclusively initialized.
    pub fn freeze(mut self) -> Result<SharedBufferLease, Self> {
        let Some(mut inner) = self.inner.take() else {
            return Err(self);
        };
        if inner.state != LeaseState::ExclusiveInitialized {
            self.inner = Some(inner);
            return Err(self);
        }
        inner.state = LeaseState::SharedImmutable;
        Ok(SharedBufferLease {
            pool: Arc::clone(&self.pool),
            inner: Some(inner),
        })
    }
}

impl Drop for BufferLease {
    fn drop(&mut self) {
        if let Some(mut inner) = self.inner.take() {
            inner.state = if inner.state == LeaseState::Poisoned {
                LeaseState::Poisoned
            } else {
                LeaseState::Returning
            };
            let _ = return_inner(&self.pool, inner);
        }
    }
}

/// An immutable lease produced by freezing an initialized exclusive lease.
#[derive(Debug)]
pub struct SharedBufferLease {
    pool: Arc<PoolShared>,
    inner: Option<LeaseInner>,
}

impl SharedBufferLease {
    #[must_use]
    pub const fn state(&self) -> LeaseState {
        LeaseState::SharedImmutable
    }

    /// Borrows immutable bytes from the frozen lease.
    ///
    /// # Errors
    ///
    /// Returns an error after return or if the lease state is invalid.
    pub fn read_view(&self) -> Result<BufferRead<'_>, HostPoolError> {
        let inner = self.inner.as_ref().ok_or(HostPoolError::AlreadyReturned)?;
        if inner.state != LeaseState::SharedImmutable {
            return Err(HostPoolError::InvalidTransition { state: inner.state });
        }
        Ok(BufferRead {
            storage: inner
                .storage
                .as_ref()
                .ok_or(HostPoolError::AlreadyReturned)?,
            length: inner.requested_bytes,
        })
    }

    /// Checks the complete image format and byte-length contract.
    ///
    /// # Errors
    ///
    /// Returns [`HostPoolError::DescriptorMismatch`] for a different class or extent.
    pub fn validate_descriptor(&self, descriptor: &ImageDescriptor) -> Result<(), HostPoolError> {
        let inner = self.inner.as_ref().ok_or(HostPoolError::AlreadyReturned)?;
        if descriptor.format() != inner.class.format()
            || descriptor.byte_length() != inner.requested_bytes
        {
            return Err(HostPoolError::DescriptorMismatch);
        }
        Ok(())
    }

    /// Creates a descriptor-backed immutable host view.
    ///
    /// # Errors
    ///
    /// Returns a descriptor or lease-state error.
    pub fn image_view<'descriptor, 'bytes>(
        &'bytes self,
        descriptor: &'descriptor ImageDescriptor,
    ) -> Result<HostImageView<'descriptor, 'bytes>, HostPoolError> {
        self.validate_descriptor(descriptor)?;
        Ok(HostImageView {
            descriptor,
            bytes: self.read_view()?,
        })
    }

    /// Returns the shared lease exactly once.
    ///
    /// # Errors
    ///
    /// Returns [`HostPoolError::AlreadyReturned`] after the first return.
    pub fn return_to_pool(&mut self) -> Result<ReturnReceipt, HostPoolError> {
        let mut inner = self.inner.take().ok_or(HostPoolError::AlreadyReturned)?;
        inner.state = LeaseState::Returning;
        Ok(return_inner(&self.pool, inner))
    }
}

impl Drop for SharedBufferLease {
    fn drop(&mut self) {
        if let Some(mut inner) = self.inner.take() {
            inner.state = LeaseState::Returning;
            let _ = return_inner(&self.pool, inner);
        }
    }
}
