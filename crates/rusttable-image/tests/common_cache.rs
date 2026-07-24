use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread;

use rusttable_image::{
    CacheAllocation, CacheCallbacks, CacheError, CacheLease, CacheLeaseError, CacheMode,
    CachePoison, CacheRemoveResult, ConcurrentCache,
};

#[derive(Clone, Debug)]
struct RecordingCallbacks {
    allocations: Arc<AtomicUsize>,
    cleanups: Arc<Mutex<Vec<u64>>>,
    cost: usize,
}

impl RecordingCallbacks {
    fn new(cost: usize) -> Self {
        Self {
            allocations: Arc::new(AtomicUsize::new(0)),
            cleanups: Arc::new(Mutex::new(Vec::new())),
            cost,
        }
    }
}

impl CacheCallbacks<u64, u64> for RecordingCallbacks {
    type Error = InfallibleAllocation;

    fn allocate(&self, key: &u64) -> Result<CacheAllocation<u64>, Self::Error> {
        self.allocations.fetch_add(1, Ordering::SeqCst);
        Ok(CacheAllocation::with_cost(*key, self.cost))
    }

    fn cleanup(&self, key: &u64, allocation: CacheAllocation<u64>) {
        assert_eq!(*key, allocation.into_value());
        self.cleanups
            .lock()
            .expect("cleanup recording lock")
            .push(*key);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InfallibleAllocation;

impl Display for InfallibleAllocation {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("infallible allocation")
    }
}

impl Error for InfallibleAllocation {}

fn expect_write<K, V, C>(lease: CacheLease<K, V, C>) -> rusttable_image::CacheWriteLease<K, V, C>
where
    K: Eq + std::hash::Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    match lease {
        CacheLease::Write(write) => write,
        CacheLease::Read(_) => panic!("expected a write lease"),
    }
}

fn expect_read<K, V, C>(lease: CacheLease<K, V, C>) -> rusttable_image::CacheReadLease<K, V, C>
where
    K: Eq + std::hash::Hash + Clone + Send + Sync + 'static,
    V: Send + Sync + 'static,
    C: CacheCallbacks<K, V>,
{
    match lease {
        CacheLease::Read(read) => read,
        CacheLease::Write(_) => panic!("expected a read lease"),
    }
}

#[test]
fn default_entries_use_cost_one_and_honor_the_requested_miss_mode() {
    let cache = ConcurrentCache::<u64, usize>::new(8);

    let read = expect_read(
        cache
            .acquire(7, CacheMode::Read)
            .expect("default read miss"),
    );
    assert!(read.was_created());
    assert_eq!(read.cost(), 1);
    assert_eq!(*read.read().expect("read default value"), 0);
    drop(read);

    let accounting = cache.accounting().expect("cache accounting");
    assert_eq!(accounting.entries, 1);
    assert_eq!(accounting.cost, 1);
    assert_eq!(accounting.cost_quota, 8);
    assert_eq!(cache.set_cost_quota(13).expect("update quota"), 8);
    assert_eq!(
        cache.accounting().expect("updated accounting").cost_quota,
        13
    );
}

#[test]
fn callback_read_miss_is_write_held_then_demotes_without_a_gap() {
    let callbacks = RecordingCallbacks::new(5);
    let cache = ConcurrentCache::with_callbacks(20, callbacks.clone());

    let write = expect_write(
        cache
            .acquire(11, CacheMode::Read)
            .expect("callback read miss"),
    );
    assert!(write.was_created());
    assert_eq!(write.cost(), 5);
    write
        .with_value_mut(|value| *value = 99)
        .expect("initialize callback value");

    let read = write.demote().expect("atomic write-to-read demotion");
    assert_eq!(
        read.with_value(|value| *value).expect("read demoted value"),
        99
    );
    assert!(
        cache
            .try_acquire(&11, CacheMode::Write)
            .expect("nonblocking write attempt")
            .is_none()
    );
    drop(read);

    let hit = expect_read(
        cache
            .acquire(11, CacheMode::Read)
            .expect("existing read hit"),
    );
    assert!(!hit.was_created());
    assert_eq!(*hit.read().expect("read hit"), 99);
    assert_eq!(callbacks.allocations.load(Ordering::SeqCst), 1);
}

#[test]
fn contains_and_failed_testget_do_not_touch_mru_but_success_does() {
    let cache = ConcurrentCache::<u64, usize>::new(10);
    let oldest_held = expect_write(
        cache
            .acquire(1, CacheMode::Write)
            .expect("first write miss"),
    );
    drop(cache.acquire(2, CacheMode::Read).expect("second read miss"));
    assert_eq!(cache.keys().expect("initial MRU order"), vec![1, 2]);

    assert!(cache.contains(&1).expect("contains oldest"));
    assert_eq!(cache.keys().expect("contains preserves order"), vec![1, 2]);
    assert!(
        cache
            .try_acquire(&1, CacheMode::Read)
            .expect("busy testget")
            .is_none()
    );
    assert_eq!(
        cache.keys().expect("failed testget preserves order"),
        vec![1, 2]
    );

    drop(oldest_held);
    drop(
        cache
            .try_acquire(&1, CacheMode::Read)
            .expect("available testget")
            .expect("published oldest"),
    );
    assert_eq!(
        cache.keys().expect("successful testget moves MRU"),
        vec![2, 1]
    );
}

#[test]
fn read_leases_access_the_value_concurrently_and_writer_blocks() {
    let cache = Arc::new(ConcurrentCache::<u64, usize>::new(10));
    drop(cache.acquire(1, CacheMode::Read).expect("create entry"));
    let first = expect_read(cache.acquire(1, CacheMode::Read).expect("first reader"));
    let second = expect_read(cache.acquire(1, CacheMode::Read).expect("second reader"));
    let inside = Arc::new(Barrier::new(3));

    let first_inside = Arc::clone(&inside);
    let first_thread = thread::spawn(move || {
        first
            .with_value(|value| {
                first_inside.wait();
                *value
            })
            .expect("first concurrent read")
    });
    let second_inside = Arc::clone(&inside);
    let second_thread = thread::spawn(move || {
        second
            .with_value(|value| {
                second_inside.wait();
                *value
            })
            .expect("second concurrent read")
    });
    inside.wait();

    let writer = expect_write(
        cache
            .acquire(1, CacheMode::Write)
            .expect("writer after readers"),
    );
    assert_eq!(first_thread.join().expect("first reader thread"), 0);
    assert_eq!(second_thread.join().expect("second reader thread"), 0);
    writer
        .with_value_mut(|value| *value = 3)
        .expect("exclusive mutation");
}

#[test]
fn blocking_get_waits_until_the_requested_entry_mode_is_available() {
    let cache = Arc::new(ConcurrentCache::<u64, usize>::new(10));
    let write = expect_write(cache.acquire(5, CacheMode::Write).expect("exclusive entry"));
    let (attempted_tx, attempted_rx) = mpsc::channel();
    let (acquired_tx, acquired_rx) = mpsc::channel();
    let reader_cache = Arc::clone(&cache);
    let reader = thread::spawn(move || {
        attempted_tx.send(()).expect("signal blocking get");
        let lease = reader_cache
            .acquire(5, CacheMode::Read)
            .expect("blocking reader");
        acquired_tx
            .send(lease.mode())
            .expect("signal acquired reader");
    });
    attempted_rx.recv().expect("reader attempted acquisition");
    assert!(matches!(
        acquired_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));

    drop(write);
    assert_eq!(
        acquired_rx.recv().expect("reader acquired after release"),
        CacheMode::Read
    );
    reader.join().expect("blocking reader thread");
}

#[derive(Debug)]
struct BlockingCallbacks {
    allocations: Arc<AtomicUsize>,
    started: Mutex<Option<mpsc::Sender<()>>>,
    proceed: Mutex<mpsc::Receiver<()>>,
}

impl CacheCallbacks<u64, u64> for BlockingCallbacks {
    type Error = InfallibleAllocation;

    fn allocate(&self, key: &u64) -> Result<CacheAllocation<u64>, Self::Error> {
        let allocation = self.allocations.fetch_add(1, Ordering::SeqCst);
        if allocation == 0 {
            if let Some(started) = self.started.lock().expect("started lock").take() {
                started.send(()).expect("signal allocator start");
            }
            self.proceed
                .lock()
                .expect("proceed lock")
                .recv()
                .expect("release allocator");
        }
        Ok(CacheAllocation::with_cost(*key, 2))
    }
}

#[test]
fn same_key_misses_allocate_once_before_publication() {
    let (started_tx, started_rx) = mpsc::channel();
    let (proceed_tx, proceed_rx) = mpsc::channel();
    let allocations = Arc::new(AtomicUsize::new(0));
    let cache = Arc::new(ConcurrentCache::with_callbacks(
        100,
        BlockingCallbacks {
            allocations: Arc::clone(&allocations),
            started: Mutex::new(Some(started_tx)),
            proceed: Mutex::new(proceed_rx),
        },
    ));
    let launch = Arc::new(Barrier::new(9));
    let mut workers = Vec::new();

    for _ in 0..8 {
        let cache = Arc::clone(&cache);
        let launch = Arc::clone(&launch);
        workers.push(thread::spawn(move || {
            launch.wait();
            let lease = cache.acquire(41, CacheMode::Read).expect("same-key get");
            let mode = lease.mode();
            let value = lease.with_value(|value| *value).expect("same-key value");
            (mode, value)
        }));
    }
    launch.wait();
    started_rx.recv().expect("allocator began");

    assert!(
        !cache
            .contains(&41)
            .expect("not published during allocation")
    );
    assert_eq!(cache.accounting().expect("pending accounting").pending, 1);
    assert!(
        cache
            .try_acquire(&41, CacheMode::Read)
            .expect("pending testget")
            .is_none()
    );
    assert!(
        cache
            .try_acquire(&99, CacheMode::Read)
            .expect("missing testget")
            .is_none()
    );
    assert_eq!(allocations.load(Ordering::SeqCst), 1);
    proceed_tx.send(()).expect("allow allocation");

    let results: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().expect("same-key worker"))
        .collect();
    assert!(results.iter().all(|(_, value)| *value == 41));
    assert_eq!(
        results
            .iter()
            .filter(|(mode, _)| *mode == CacheMode::Write)
            .count(),
        1
    );
    assert_eq!(allocations.load(Ordering::SeqCst), 1);
    assert_eq!(cache.accounting().expect("published accounting").pending, 0);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Rejected;

impl Display for Rejected {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str("rejected")
    }
}

impl Error for Rejected {}

#[derive(Debug)]
struct RejectingCallbacks;

impl CacheCallbacks<u64, u64> for RejectingCallbacks {
    type Error = Rejected;

    fn allocate(&self, _key: &u64) -> Result<CacheAllocation<u64>, Self::Error> {
        Err(Rejected)
    }
}

#[derive(Debug)]
struct PanickingCallbacks;

impl CacheCallbacks<u64, u64> for PanickingCallbacks {
    type Error = Rejected;

    fn allocate(&self, _key: &u64) -> Result<CacheAllocation<u64>, Self::Error> {
        panic!("allocator panic")
    }
}

#[test]
fn allocation_error_and_panic_are_typed_and_clear_the_pending_key() {
    let rejected = ConcurrentCache::with_callbacks(10, RejectingCallbacks);
    assert!(matches!(
        rejected.acquire(1, CacheMode::Read),
        Err(CacheError::Allocation(Rejected))
    ));
    assert!(!rejected.contains(&1).expect("rejected key absent"));
    assert_eq!(
        rejected.accounting().expect("rejected accounting").pending,
        0
    );

    let panicking = ConcurrentCache::with_callbacks(10, PanickingCallbacks);
    assert!(matches!(
        panicking.acquire(1, CacheMode::Read),
        Err(CacheError::AllocationPanicked)
    ));
    assert!(!panicking.contains(&1).expect("panicked key absent"));
    assert_eq!(
        panicking.accounting().expect("panicked accounting").pending,
        0
    );
}

#[derive(Debug)]
struct OverflowCallbacks {
    cleanups: Arc<Mutex<Vec<u64>>>,
}

impl CacheCallbacks<u64, u64> for OverflowCallbacks {
    type Error = InfallibleAllocation;

    fn allocate(&self, key: &u64) -> Result<CacheAllocation<u64>, Self::Error> {
        let cost = if *key == 1 { usize::MAX } else { 1 };
        Ok(CacheAllocation::with_cost(*key, cost))
    }

    fn cleanup(&self, key: &u64, allocation: CacheAllocation<u64>) {
        assert_eq!(*key, allocation.into_value());
        self.cleanups
            .lock()
            .expect("overflow cleanup lock")
            .push(*key);
    }
}

#[test]
fn cost_overflow_is_typed_and_unpublished_allocation_is_cleaned_once() {
    let cleanups = Arc::new(Mutex::new(Vec::new()));
    let cache = ConcurrentCache::with_callbacks(
        usize::MAX,
        OverflowCallbacks {
            cleanups: Arc::clone(&cleanups),
        },
    );
    let maximum = cache.acquire(1, CacheMode::Write).expect("maximum cost");

    assert!(matches!(
        cache.acquire(2, CacheMode::Read),
        Err(CacheError::CostOverflow {
            current: usize::MAX,
            added: 1
        })
    ));
    assert!(!cache.contains(&2).expect("overflow key absent"));
    assert_eq!(*cleanups.lock().expect("overflow cleanups"), vec![2]);
    drop(maximum);
    drop(cache);
    assert_eq!(*cleanups.lock().expect("drop cleanups"), vec![2, 1]);
}

#[test]
fn gc_is_oldest_first_strictly_below_target_and_skips_locked_entries() {
    let callbacks = RecordingCallbacks::new(4);
    let cache = ConcurrentCache::with_callbacks(10, callbacks.clone());
    let oldest_locked = cache.acquire(1, CacheMode::Write).expect("locked oldest");
    drop(cache.acquire(2, CacheMode::Write).expect("second entry"));
    drop(cache.acquire(3, CacheMode::Write).expect("third entry"));
    assert_eq!(cache.keys().expect("pre-GC order"), vec![1, 2, 3]);

    let report = cache.gc(0.5).expect("best-effort GC");
    assert_eq!(report.cost_before, 12);
    assert_eq!(report.cost_after, 4);
    assert_eq!(report.removed_entries, 2);
    assert_eq!(report.reclaimed_cost, 8);
    assert_eq!(report.skipped_locked, 1);
    assert_eq!(cache.keys().expect("post-GC order"), vec![1]);
    assert_eq!(*callbacks.cleanups.lock().expect("GC cleanups"), vec![2, 3]);
    drop(oldest_locked);
}

#[test]
fn automatic_gc_runs_only_for_an_over_eighty_percent_allocating_miss() {
    let callbacks = RecordingCallbacks::new(4);
    let cache = ConcurrentCache::with_callbacks(10, callbacks.clone());
    drop(cache.acquire(1, CacheMode::Write).expect("first entry"));
    drop(
        cache
            .acquire(2, CacheMode::Write)
            .expect("exact 80 percent"),
    );
    assert_eq!(cache.accounting().expect("80 percent accounting").cost, 8);

    drop(
        cache
            .acquire(3, CacheMode::Write)
            .expect("miss at exactly 80 percent"),
    );
    assert_eq!(cache.accounting().expect("soft quota accounting").cost, 12);
    assert!(
        callbacks
            .cleanups
            .lock()
            .expect("no exact-80 GC")
            .is_empty()
    );

    drop(cache.acquire(3, CacheMode::Read).expect("over-quota hit"));
    assert!(
        callbacks
            .cleanups
            .lock()
            .expect("hit does not collect")
            .is_empty()
    );

    drop(
        cache
            .acquire(4, CacheMode::Write)
            .expect("over-80 allocating miss"),
    );
    assert_eq!(
        *callbacks.cleanups.lock().expect("automatic GC order"),
        vec![1, 2]
    );
    assert_eq!(cache.keys().expect("post automatic-GC keys"), vec![3, 4]);
    assert_eq!(
        cache
            .accounting()
            .expect("post automatic-GC accounting")
            .cost,
        8
    );
}

#[test]
fn automatic_gc_keeps_soft_quota_when_every_candidate_is_leased() {
    let callbacks = RecordingCallbacks::new(4);
    let cache = ConcurrentCache::with_callbacks(10, callbacks.clone());
    let first = cache.acquire(1, CacheMode::Write).expect("first locked");
    let second = cache.acquire(2, CacheMode::Write).expect("second locked");
    let third = cache.acquire(3, CacheMode::Write).expect("third locked");
    let fourth = cache
        .acquire(4, CacheMode::Write)
        .expect("soft-quota overage");

    let accounting = cache.accounting().expect("over-quota accounting");
    assert_eq!(accounting.entries, 4);
    assert_eq!(accounting.cost, 16);
    assert!(accounting.cost > accounting.cost_quota);
    assert!(
        callbacks
            .cleanups
            .lock()
            .expect("locked GC skips")
            .is_empty()
    );

    drop((first, second, third, fourth));
}

#[test]
fn remove_waits_for_demoted_reader_and_reports_missing_distinctly() {
    let callbacks = RecordingCallbacks::new(1);
    let cache = Arc::new(ConcurrentCache::with_callbacks(10, callbacks.clone()));
    let write = expect_write(
        cache
            .acquire(8, CacheMode::Read)
            .expect("callback-created write"),
    );
    let read = write.demote().expect("protected demotion");

    let gc = cache.gc(0.0).expect("GC skips demoted reader");
    assert_eq!(gc.removed_entries, 0);
    assert_eq!(gc.skipped_locked, 1);

    let (attempted_tx, attempted_rx) = mpsc::channel();
    let (removed_tx, removed_rx) = mpsc::channel();
    let remover_cache = Arc::clone(&cache);
    let remover = thread::spawn(move || {
        attempted_tx.send(()).expect("signal remove attempt");
        let result = remover_cache.remove(&8).expect("blocking remove");
        removed_tx.send(result).expect("send remove result");
    });
    attempted_rx.recv().expect("remover started");
    assert!(matches!(
        removed_rx.try_recv(),
        Err(mpsc::TryRecvError::Empty)
    ));

    drop(read);
    assert_eq!(
        removed_rx.recv().expect("remove completed"),
        CacheRemoveResult::Removed { cost: 1 }
    );
    remover.join().expect("remover thread");
    assert_eq!(
        cache.remove(&8).expect("second remove"),
        CacheRemoveResult::Missing
    );
    assert_eq!(*callbacks.cleanups.lock().expect("remove cleanup"), vec![8]);
}

#[test]
fn cache_drop_waits_for_owned_lease_and_cleans_exactly_once() {
    let callbacks = RecordingCallbacks::new(1);
    let cleanups = Arc::clone(&callbacks.cleanups);
    let cache = ConcurrentCache::with_callbacks(10, callbacks);
    let lease = cache.acquire(17, CacheMode::Write).expect("owned lease");

    drop(cache);
    assert!(
        cleanups
            .lock()
            .expect("cleanup before lease drop")
            .is_empty()
    );
    drop(lease);
    assert_eq!(
        *cleanups.lock().expect("cleanup after lease drop"),
        vec![17]
    );
}

#[test]
fn writer_panic_poison_is_reported_without_unsafe_recovery() {
    let cache = ConcurrentCache::<u64, usize>::new(4);
    let write = expect_write(cache.acquire(1, CacheMode::Write).expect("write lease"));

    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = write.with_value_mut(|_| panic!("poison cached value"));
    }));
    assert!(panic.is_err());
    assert_eq!(
        write.with_value(|value| *value),
        Err(CacheLeaseError::Poisoned(CachePoison::Value))
    );
    drop(write);
    assert!(matches!(
        cache.remove(&1),
        Err(CacheError::Poisoned(CachePoison::Value))
    ));
    assert!(!cache.contains(&1).expect("poisoned entry was removed"));
}

#[test]
fn invalid_gc_ratios_are_typed() {
    let cache = ConcurrentCache::<u64, usize>::new(4);
    assert!(matches!(
        cache.gc(f64::NAN),
        Err(CacheError::InvalidFillRatio { ratio }) if ratio.is_nan()
    ));
    assert!(matches!(
        cache.gc(-0.1),
        Err(CacheError::InvalidFillRatio { ratio }) if ratio.is_finite() && ratio.is_sign_negative()
    ));
}
