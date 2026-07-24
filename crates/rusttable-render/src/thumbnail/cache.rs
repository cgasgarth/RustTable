#![allow(
    clippy::format_collect,
    clippy::manual_let_else,
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    clippy::single_match_else,
    clippy::struct_field_names
)]

use std::collections::{BTreeMap, HashMap};
use std::convert::Infallible;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use rusttable_image::common::cache::{
    CacheAllocation as CommonCacheAllocation, CacheCallbacks as CommonCacheCallbacks,
    CacheLease as CommonCacheLease, CacheMode as CommonCacheMode,
    CacheRemoveResult as CommonCacheRemoveResult, ConcurrentCache as CommonConcurrentCache,
};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions, Orientation};
use sha2::{Digest, Sha256};

use super::lifecycle::CacheChangeEvent;
use crate::{ThumbnailKey, ThumbnailKeyError};

pub const CURRENT_CACHE_SCHEMA: u16 = 2;
const MAGIC: &[u8; 8] = b"RTTHMB02";
const FIXED_HEADER_BYTES: usize = 8 + 2 + 4 + 8 + 4 + 4 + 8 + 32 + 32;
const HEADER_DIGEST_BYTES: usize = 32;
const TEMPORARY_STALE_SECONDS: u64 = 60 * 60;
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);
static DURABLE_COORDINATORS: OnceLock<Mutex<HashMap<PathBuf, Weak<DurableRootCoordinator>>>> =
    OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CacheTime(u64);

impl CacheTime {
    #[must_use]
    pub const fn from_seconds(value: u64) -> Self {
        Self(value)
    }

    pub fn now() -> Result<Self, CacheError> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| Self(value.as_secs()))
            .map_err(|_| CacheError::Clock)
    }

    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheLimits {
    max_total_bytes: u64,
    max_entry_bytes: u64,
    max_age_seconds: Option<u64>,
}

impl CacheLimits {
    pub const fn new(max_total_bytes: u64, max_entry_bytes: u64) -> Result<Self, CacheError> {
        if max_total_bytes == 0 || max_entry_bytes == 0 {
            return Err(CacheError::InvalidLimits);
        }
        Ok(Self {
            max_total_bytes,
            max_entry_bytes,
            max_age_seconds: None,
        })
    }

    #[must_use]
    pub const fn with_max_age(mut self, seconds: u64) -> Self {
        self.max_age_seconds = Some(seconds);
        self
    }
    #[must_use]
    pub const fn max_total_bytes(self) -> u64 {
        self.max_total_bytes
    }
    #[must_use]
    pub const fn max_entry_bytes(self) -> u64 {
        self.max_entry_bytes
    }
    #[must_use]
    pub const fn max_age_seconds(self) -> Option<u64> {
        self.max_age_seconds
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    key: ThumbnailKey,
    image: DecodedImage,
    created_at: CacheTime,
}

impl CacheEntry {
    #[must_use]
    pub const fn key(&self) -> ThumbnailKey {
        self.key
    }
    #[must_use]
    pub const fn image(&self) -> &DecodedImage {
        &self.image
    }
    #[must_use]
    pub const fn created_at(&self) -> CacheTime {
        self.created_at
    }

    #[must_use]
    pub fn into_image(self) -> DecodedImage {
        self.image
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReconciliationReport {
    pub valid_entries: usize,
    pub removed_corrupt: usize,
    pub removed_temporary: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheError {
    Io {
        operation: &'static str,
        path: PathBuf,
        message: String,
    },
    Clock,
    InvalidLimits,
    LimitsMismatch {
        existing: CacheLimits,
        requested: CacheLimits,
    },
    EntryTooLarge {
        bytes: u64,
        limit: u64,
    },
    CacheFull {
        required: u64,
        available: u64,
    },
    Protected,
    CorruptEntry,
    UnsupportedSchema(u16),
    Key(ThumbnailKeyError),
    Image,
    Memory,
    Invalidated,
}

impl fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "thumbnail cache error: {self:?}")
    }
}
impl std::error::Error for CacheError {}

#[derive(Debug, Clone)]
struct CacheMetadata {
    key: ThumbnailKey,
    path: PathBuf,
    bytes: u64,
    created_at: CacheTime,
    last_access: CacheTime,
    revision: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ResidentCacheEntry {
    image: Arc<DecodedImage>,
    created_at: CacheTime,
}

impl ResidentCacheEntry {
    pub(crate) fn new(image: DecodedImage, created_at: CacheTime) -> Self {
        Self {
            image: Arc::new(image),
            created_at,
        }
    }

    pub(crate) fn image(&self) -> Arc<DecodedImage> {
        Arc::clone(&self.image)
    }

    pub(crate) const fn created_at(&self) -> CacheTime {
        self.created_at
    }
}

#[derive(Debug)]
pub(crate) enum ResidentCacheSlot {
    Vacant,
    Ready(ResidentCacheEntry),
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ThumbnailCacheCallbacks;

impl CommonCacheCallbacks<ThumbnailKey, ResidentCacheSlot> for ThumbnailCacheCallbacks {
    type Error = Infallible;

    fn allocate(
        &self,
        key: &ThumbnailKey,
    ) -> Result<CommonCacheAllocation<ResidentCacheSlot>, Self::Error> {
        Ok(CommonCacheAllocation::with_cost(
            ResidentCacheSlot::Vacant,
            resident_cost(*key),
        ))
    }
}

pub(crate) type ThumbnailCacheLease =
    CommonCacheLease<ThumbnailKey, ResidentCacheSlot, ThumbnailCacheCallbacks>;

/// Shared process-resident thumbnail tier. The durable cache remains authoritative.
#[derive(Clone)]
pub struct ThumbnailMemoryCache {
    inner: CommonConcurrentCache<ThumbnailKey, ResidentCacheSlot, ThumbnailCacheCallbacks>,
    coordinator: Arc<ThumbnailCacheCoordinator>,
}

impl fmt::Debug for ThumbnailMemoryCache {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let accounting = self.inner.accounting().ok();
        formatter
            .debug_struct("ThumbnailMemoryCache")
            .field("accounting", &accounting)
            .finish_non_exhaustive()
    }
}

impl ThumbnailMemoryCache {
    #[must_use]
    pub fn new(cost_quota: usize) -> Self {
        Self {
            inner: CommonConcurrentCache::with_callbacks(cost_quota, ThumbnailCacheCallbacks),
            coordinator: Arc::new(ThumbnailCacheCoordinator::default()),
        }
    }

    pub(crate) fn begin_resolution(
        &self,
        key: ThumbnailKey,
    ) -> Result<ThumbnailResolutionGuard, CacheError> {
        ThumbnailResolutionGuard::begin(Arc::clone(&self.coordinator), key)
    }

    pub(crate) fn begin_invalidation(
        &self,
        event: CacheChangeEvent,
    ) -> Result<ThumbnailInvalidationGuard, CacheError> {
        ThumbnailInvalidationGuard::begin(Arc::clone(&self.coordinator), event)
    }

    pub(crate) fn acquire(
        &self,
        key: ThumbnailKey,
        mode: CommonCacheMode,
    ) -> Result<ThumbnailCacheLease, CacheError> {
        self.inner
            .acquire(key, mode)
            .map_err(|_| CacheError::Memory)
    }

    pub(crate) fn keys(&self) -> Result<Vec<ThumbnailKey>, CacheError> {
        self.inner.keys().map_err(|_| CacheError::Memory)
    }

    pub(crate) fn remove(&self, key: &ThumbnailKey) -> Result<bool, CacheError> {
        self.inner
            .remove(key)
            .map(|result| matches!(result, CommonCacheRemoveResult::Removed { .. }))
            .map_err(|_| CacheError::Memory)
    }

    #[cfg(test)]
    pub(crate) fn is_invalidating(&self) -> bool {
        self.coordinator
            .state
            .lock()
            .map_or(true, |state| state.invalidation.is_some())
    }

    #[cfg(test)]
    pub(crate) fn pause_next_commit(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (reached_sender, reached_receiver) = mpsc::channel();
        let (resume_sender, resume_receiver) = mpsc::channel();
        *self
            .coordinator
            .commit_pause
            .lock()
            .expect("commit-pause lock") = Some((reached_sender, resume_receiver));
        (reached_receiver, resume_sender)
    }
}

#[derive(Debug, Default)]
struct ThumbnailCacheCoordinator {
    state: Mutex<ThumbnailCacheCoordinatorState>,
    publication: Mutex<()>,
    changed: Condvar,
    #[cfg(test)]
    commit_pause: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

#[derive(Debug, Default)]
struct ThumbnailCacheCoordinatorState {
    generations: HashMap<ThumbnailKey, u64>,
    invalidation: Option<CacheChangeEvent>,
    active: HashMap<ThumbnailKey, usize>,
}

#[derive(Debug)]
pub(crate) struct ThumbnailResolutionGuard {
    coordinator: Arc<ThumbnailCacheCoordinator>,
    key: ThumbnailKey,
    generation: u64,
}

impl ThumbnailResolutionGuard {
    fn begin(
        coordinator: Arc<ThumbnailCacheCoordinator>,
        key: ThumbnailKey,
    ) -> Result<Self, CacheError> {
        let mut state = coordinator.state.lock().map_err(|_| CacheError::Memory)?;
        while state.invalidation.is_some_and(|event| event.affects(key)) {
            state = coordinator
                .changed
                .wait(state)
                .map_err(|_| CacheError::Memory)?;
        }
        let generation = state.generations.get(&key).copied().unwrap_or(0);
        let active = state.active.entry(key).or_default();
        *active = active.checked_add(1).ok_or(CacheError::Memory)?;
        drop(state);
        Ok(Self {
            coordinator,
            key,
            generation,
        })
    }

    pub(crate) fn commit<T>(
        &self,
        commit: impl FnOnce() -> Result<T, CacheError>,
    ) -> Result<T, CacheError> {
        #[cfg(test)]
        if let Some((reached, resume)) = self
            .coordinator
            .commit_pause
            .lock()
            .map_err(|_| CacheError::Memory)?
            .take()
        {
            reached.send(()).map_err(|_| CacheError::Memory)?;
            resume.recv().map_err(|_| CacheError::Memory)?;
        }
        let _publication = self
            .coordinator
            .publication
            .lock()
            .map_err(|_| CacheError::Memory)?;
        let state = self
            .coordinator
            .state
            .lock()
            .map_err(|_| CacheError::Memory)?;
        if state
            .invalidation
            .is_some_and(|event| event.affects(self.key))
            || state.generations.get(&self.key).copied().unwrap_or(0) != self.generation
        {
            return Err(CacheError::Invalidated);
        }
        drop(state);
        commit()
    }
}

impl Drop for ThumbnailResolutionGuard {
    fn drop(&mut self) {
        let mut state = match self.coordinator.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(active) = state.active.get_mut(&self.key) {
            if *active <= 1 {
                state.active.remove(&self.key);
            } else {
                *active -= 1;
            }
        }
        self.coordinator.changed.notify_all();
    }
}

#[derive(Debug)]
pub(crate) struct ThumbnailInvalidationGuard {
    coordinator: Arc<ThumbnailCacheCoordinator>,
    active_keys: Vec<ThumbnailKey>,
}

impl ThumbnailInvalidationGuard {
    fn begin(
        coordinator: Arc<ThumbnailCacheCoordinator>,
        event: CacheChangeEvent,
    ) -> Result<Self, CacheError> {
        loop {
            let mut state = coordinator.state.lock().map_err(|_| CacheError::Memory)?;
            while state.invalidation.is_some() {
                state = coordinator
                    .changed
                    .wait(state)
                    .map_err(|_| CacheError::Memory)?;
            }
            drop(state);

            let publication = coordinator
                .publication
                .lock()
                .map_err(|_| CacheError::Memory)?;
            let mut state = coordinator.state.lock().map_err(|_| CacheError::Memory)?;
            if state.invalidation.is_some() {
                drop(state);
                drop(publication);
                continue;
            }
            let active_keys = state
                .active
                .keys()
                .copied()
                .filter(|key| event.affects(*key))
                .collect::<Vec<_>>();
            if active_keys
                .iter()
                .any(|key| state.generations.get(key).copied().unwrap_or(0) == u64::MAX)
            {
                return Err(CacheError::Invalidated);
            }
            for key in &active_keys {
                *state.generations.entry(*key).or_default() += 1;
            }
            state.invalidation = Some(event);
            drop(state);
            drop(publication);
            return Ok(Self {
                coordinator,
                active_keys,
            });
        }
    }

    pub(crate) fn active_keys(&self) -> &[ThumbnailKey] {
        &self.active_keys
    }
}

impl Drop for ThumbnailInvalidationGuard {
    fn drop(&mut self) {
        let mut state = match self.coordinator.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.invalidation = None;
        self.coordinator.changed.notify_all();
    }
}

fn resident_cost(key: ThumbnailKey) -> usize {
    let (width, height) = key.request().size().dimensions();
    let pixel_bytes = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(4);
    usize::try_from(pixel_bytes)
        .unwrap_or(usize::MAX)
        .saturating_add(mem::size_of::<ResidentCacheSlot>())
}

#[derive(Debug, Clone)]
pub struct CacheStore {
    root: PathBuf,
    limits: CacheLimits,
    coordinator: Arc<DurableRootCoordinator>,
}

#[derive(Debug)]
struct DurableRootCoordinator {
    limits: CacheLimits,
    state: Mutex<DurableRootState>,
    #[cfg(test)]
    read_pause: Mutex<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>>,
}

#[derive(Debug, Default)]
struct DurableRootState {
    initialized: bool,
    entries: BTreeMap<[u8; 32], CacheMetadata>,
    leases: BTreeMap<[u8; 32], u32>,
    pins: BTreeMap<[u8; 32], u32>,
    in_use: BTreeMap<[u8; 32], u32>,
    next_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheLease {
    digest: [u8; 32],
}

impl CacheLease {
    #[must_use]
    pub const fn digest(self) -> [u8; 32] {
        self.digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePin {
    digest: [u8; 32],
}

impl CachePin {
    #[must_use]
    pub const fn digest(self) -> [u8; 32] {
        self.digest
    }
}

impl CacheStore {
    /// Opens a cache directory and reconstructs its minimal index from headers.
    pub fn open(
        root: impl Into<PathBuf>,
        limits: CacheLimits,
    ) -> Result<(Self, ReconciliationReport), CacheError> {
        let requested_root = root.into();
        fs::create_dir_all(&requested_root)
            .map_err(|error| io_error("create cache directory", &requested_root, error))?;
        let root = fs::canonicalize(&requested_root)
            .map_err(|error| io_error("canonicalize cache directory", &requested_root, error))?;
        let coordinator = durable_coordinator(&root, limits)?;
        let store = Self {
            root,
            limits,
            coordinator: Arc::clone(&coordinator),
        };
        let mut state = coordinator.state.lock().map_err(|_| CacheError::Memory)?;
        let report = if state.initialized {
            ReconciliationReport {
                valid_entries: state.entries.len(),
                ..ReconciliationReport::default()
            }
        } else {
            let report = store.reconcile_locked(&mut state)?;
            state.initialized = true;
            report
        };
        drop(state);
        Ok((store, report))
    }

    #[must_use]
    pub const fn limits(&self) -> CacheLimits {
        self.limits
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.coordinator
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.coordinator
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = ThumbnailKey> {
        self.coordinator
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entries
            .values()
            .map(|entry| entry.key)
            .collect::<Vec<_>>()
            .into_iter()
    }

    /// Refreshes process-shared LRU metadata without reading the durable payload.
    pub fn touch(&mut self, key: ThumbnailKey, now: CacheTime) -> bool {
        let mut state = self
            .coordinator
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(metadata) = state.entries.get_mut(&key.digest()) else {
            return false;
        };
        if metadata.key != key {
            return false;
        }
        metadata.last_access = now;
        true
    }

    /// Reads and independently verifies the header, key, dimensions, length, and payload digest.
    pub fn get(
        &mut self,
        key: ThumbnailKey,
        now: CacheTime,
    ) -> Result<Option<CacheEntry>, CacheError> {
        let coordinator = Arc::clone(&self.coordinator);
        let digest = key.digest();
        let (path, revision) = {
            let mut state = coordinator.state.lock().map_err(|_| CacheError::Memory)?;
            let Some(metadata) = state.entries.get(&digest) else {
                return Ok(None);
            };
            if metadata.key != key {
                return Ok(None);
            }
            let path = metadata.path.clone();
            let revision = metadata.revision;
            let count = state.in_use.entry(digest).or_default();
            *count = count.checked_add(1).ok_or(CacheError::Memory)?;
            (path, revision)
        };

        #[cfg(test)]
        let result = self
            .pause_read_after_reservation()
            .and_then(|()| read_entry(&path, key, self.limits));
        #[cfg(not(test))]
        let result = read_entry(&path, key, self.limits);
        let mut state = coordinator.state.lock().map_err(|_| CacheError::Memory)?;
        decrement(&mut state.in_use, digest)?;
        let is_current = state
            .entries
            .get(&digest)
            .is_some_and(|metadata| metadata.key == key && metadata.revision == revision);
        match result {
            Ok(entry) => {
                if is_current && let Some(metadata) = state.entries.get_mut(&digest) {
                    metadata.last_access = now;
                }
                Ok(Some(entry))
            }
            Err(CacheError::CorruptEntry | CacheError::UnsupportedSchema(_)) => {
                if is_current && !Self::is_protected_locked(&state, digest) {
                    self.remove_digest_locked(&mut state, digest)?;
                }
                Ok(None)
            }
            Err(CacheError::Io {
                operation: "open cache entry",
                ..
            }) if !path.exists() => {
                if is_current && !Self::is_protected_locked(&state, digest) {
                    self.remove_digest_locked(&mut state, digest)?;
                }
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Publishes a complete entry using a same-directory staged file and atomic rename.
    pub fn put(
        &mut self,
        key: ThumbnailKey,
        image: &DecodedImage,
        created_at: CacheTime,
    ) -> Result<(), CacheError> {
        if image.color_encoding() != ColorEncoding::Srgb
            || image.source_orientation() != Orientation::Normal
        {
            return Err(CacheError::Image);
        }
        let payload = image.pixels();
        let key_bytes = key.canonical_bytes();
        let file_bytes = serialized_size(key_bytes.len(), payload.len())?;
        if file_bytes > self.limits.max_entry_bytes {
            return Err(CacheError::EntryTooLarge {
                bytes: file_bytes,
                limit: self.limits.max_entry_bytes,
            });
        }
        let digest = key.digest();
        let final_path = self.path_for(digest);
        let temporary_path = self.temporary_path(key);
        if let Err(error) = write_entry(&temporary_path, key, image, created_at) {
            let _ = fs::remove_file(&temporary_path);
            return Err(error);
        }

        let coordinator = Arc::clone(&self.coordinator);
        let result = (|| {
            let mut state = coordinator.state.lock().map_err(|_| CacheError::Memory)?;
            self.evict_for_replacement_locked(&mut state, digest, file_bytes)?;
            let previous = state.entries.get(&digest).map_or(0, |entry| entry.bytes);
            let occupied_without_target = Self::total_bytes_locked(&state).saturating_sub(previous);
            let available = self
                .limits
                .max_total_bytes
                .saturating_sub(occupied_without_target);
            if file_bytes > available {
                return Err(CacheError::CacheFull {
                    required: file_bytes,
                    available,
                });
            }
            let revision = Self::next_revision_locked(&mut state)?;
            fs::rename(&temporary_path, &final_path)
                .map_err(|error| io_error("publish cache entry", &final_path, error))?;
            state.entries.insert(
                digest,
                CacheMetadata {
                    key,
                    path: final_path,
                    bytes: file_bytes,
                    created_at,
                    last_access: created_at,
                    revision,
                },
            );
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }

    #[cfg(test)]
    fn pause_next_read(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (reached_sender, reached_receiver) = mpsc::channel();
        let (resume_sender, resume_receiver) = mpsc::channel();
        *self.coordinator.read_pause.lock().expect("read-pause lock") =
            Some((reached_sender, resume_receiver));
        (reached_receiver, resume_sender)
    }

    #[cfg(test)]
    fn pause_read_after_reservation(&self) -> Result<(), CacheError> {
        if let Some((reached, resume)) = self
            .coordinator
            .read_pause
            .lock()
            .map_err(|_| CacheError::Memory)?
            .take()
        {
            reached.send(()).map_err(|_| CacheError::Memory)?;
            resume.recv().map_err(|_| CacheError::Memory)?;
        }
        Ok(())
    }

    /// Removes one exact key. A missing key is a successful no-op.
    pub fn invalidate(&mut self, key: ThumbnailKey) -> Result<bool, CacheError> {
        let coordinator = Arc::clone(&self.coordinator);
        let mut state = coordinator.state.lock().map_err(|_| CacheError::Memory)?;
        if Self::is_protected_locked(&state, key.digest()) {
            return Err(CacheError::Protected);
        }
        self.remove_digest_locked(&mut state, key.digest())
    }

    /// Acquires a lease that prevents eviction or invalidation until released.
    pub fn lease(&mut self, key: ThumbnailKey) -> Result<Option<CacheLease>, CacheError> {
        let mut state = self
            .coordinator
            .state
            .lock()
            .map_err(|_| CacheError::Memory)?;
        let digest = key.digest();
        if !state.entries.contains_key(&digest) {
            return Ok(None);
        }
        *state.leases.entry(digest).or_default() += 1;
        Ok(Some(CacheLease { digest }))
    }

    pub fn release_lease(&mut self, lease: CacheLease) -> Result<(), CacheError> {
        let mut state = self
            .coordinator
            .state
            .lock()
            .map_err(|_| CacheError::Memory)?;
        decrement(&mut state.leases, lease.digest)
    }

    /// Acquires a durable-session pin that survives ordinary eviction passes.
    pub fn pin(&mut self, key: ThumbnailKey) -> Result<Option<CachePin>, CacheError> {
        let mut state = self
            .coordinator
            .state
            .lock()
            .map_err(|_| CacheError::Memory)?;
        let digest = key.digest();
        if !state.entries.contains_key(&digest) {
            return Ok(None);
        }
        *state.pins.entry(digest).or_default() += 1;
        Ok(Some(CachePin { digest }))
    }

    pub fn unpin(&mut self, pin: CachePin) -> Result<(), CacheError> {
        let mut state = self
            .coordinator
            .state
            .lock()
            .map_err(|_| CacheError::Memory)?;
        decrement(&mut state.pins, pin.digest)
    }

    /// Removes expired entries and least-recently-used entries until the budget holds.
    pub fn evict(&mut self, now: CacheTime) -> Result<usize, CacheError> {
        let coordinator = Arc::clone(&self.coordinator);
        let mut state = coordinator.state.lock().map_err(|_| CacheError::Memory)?;
        let before = state.entries.len();
        if let Some(age) = self.limits.max_age_seconds {
            let expired = state
                .entries
                .iter()
                .filter_map(|(digest, metadata)| {
                    (!Self::is_protected_locked(&state, *digest)
                        && now.seconds().saturating_sub(metadata.created_at.seconds()) > age)
                        .then_some(*digest)
                })
                .collect::<Vec<_>>();
            for digest in expired {
                self.remove_digest_locked(&mut state, digest)?;
            }
        }
        self.evict_to_fit_locked(&mut state, 0, None)?;
        Ok(before - state.entries.len())
    }

    fn evict_for_replacement_locked(
        &self,
        state: &mut DurableRootState,
        target: [u8; 32],
        replacement_bytes: u64,
    ) -> Result<(), CacheError> {
        let previous = state.entries.get(&target).map_or(0, |entry| entry.bytes);
        self.evict_to_fit_locked(
            state,
            replacement_bytes.saturating_sub(previous),
            Some(target),
        )
    }

    fn evict_to_fit_locked(
        &self,
        state: &mut DurableRootState,
        additional: u64,
        excluded: Option<[u8; 32]>,
    ) -> Result<(), CacheError> {
        while Self::total_bytes_locked(state).saturating_add(additional)
            > self.limits.max_total_bytes
        {
            let Some(digest) = state
                .entries
                .iter()
                .filter(|(digest, _)| {
                    Some(**digest) != excluded && !Self::is_protected_locked(state, **digest)
                })
                .min_by_key(|(digest, metadata)| (metadata.last_access, **digest))
                .map(|(digest, _)| *digest)
            else {
                break;
            };
            self.remove_digest_locked(state, digest)?;
        }
        Ok(())
    }

    fn total_bytes_locked(state: &DurableRootState) -> u64 {
        state.entries.values().map(|entry| entry.bytes).sum()
    }

    fn next_revision_locked(state: &mut DurableRootState) -> Result<u64, CacheError> {
        state.next_revision = state
            .next_revision
            .checked_add(1)
            .ok_or(CacheError::Memory)?;
        Ok(state.next_revision)
    }

    fn is_protected_locked(state: &DurableRootState, digest: [u8; 32]) -> bool {
        state.leases.get(&digest).copied().unwrap_or(0) > 0
            || state.pins.get(&digest).copied().unwrap_or(0) > 0
            || state.in_use.get(&digest).copied().unwrap_or(0) > 0
    }

    fn path_for(&self, digest: [u8; 32]) -> PathBuf {
        self.root.join(format!(
            "{}.mip",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ))
    }

    fn temporary_path(&self, key: ThumbnailKey) -> PathBuf {
        let counter = TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed);
        self.root.join(format!(
            ".{}.{}.{}.tmp",
            key.digest_hex(),
            std::process::id(),
            counter
        ))
    }

    fn remove_digest_locked(
        &self,
        state: &mut DurableRootState,
        digest: [u8; 32],
    ) -> Result<bool, CacheError> {
        let metadata = state.entries.remove(&digest);
        let was_indexed = metadata.is_some();
        let path = metadata.map_or_else(|| self.path_for(digest), |metadata| metadata.path);
        state.leases.remove(&digest);
        state.pins.remove(&digest);
        state.in_use.remove(&digest);
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(was_indexed),
            Err(error) => Err(io_error("remove cache entry", &path, error)),
        }
    }

    fn reconcile_locked(
        &self,
        state: &mut DurableRootState,
    ) -> Result<ReconciliationReport, CacheError> {
        let previous = mem::take(&mut state.entries);
        let mut report = ReconciliationReport::default();
        let directory = fs::read_dir(&self.root)
            .map_err(|error| io_error("read cache directory", &self.root, error))?;
        for item in directory {
            let item =
                item.map_err(|error| io_error("read cache directory entry", &self.root, error))?;
            let path = item.path();
            if path.extension().is_some_and(|extension| extension == "tmp") {
                if temporary_entry_is_stale(&path) {
                    fs::remove_file(&path)
                        .map_err(|error| io_error("remove temporary cache entry", &path, error))?;
                    report.removed_temporary += 1;
                }
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "mip") {
                continue;
            }
            match read_header(&path, self.limits) {
                Ok(header) => {
                    let key = match ThumbnailKey::from_canonical_bytes(&header.key_bytes) {
                        Ok(key) => key,
                        Err(_) => {
                            fs::remove_file(&path).map_err(|error| {
                                io_error("remove invalid cache key", &path, error)
                            })?;
                            report.removed_corrupt += 1;
                            continue;
                        }
                    };
                    let last_access = previous
                        .get(&header.digest)
                        .map_or(header.created_at, |metadata| metadata.last_access);
                    let revision = Self::next_revision_locked(state)?;
                    state.entries.insert(
                        header.digest,
                        CacheMetadata {
                            key,
                            path,
                            bytes: header.file_bytes,
                            created_at: header.created_at,
                            last_access,
                            revision,
                        },
                    );
                    report.valid_entries += 1;
                }
                Err(CacheError::CorruptEntry | CacheError::UnsupportedSchema(_)) => {
                    fs::remove_file(&path)
                        .map_err(|error| io_error("remove corrupt cache entry", &path, error))?;
                    report.removed_corrupt += 1;
                }
                Err(error) => return Err(error),
            }
        }
        self.evict_to_fit_locked(state, 0, None)?;
        Ok(report)
    }
}

fn durable_coordinator(
    root: &Path,
    limits: CacheLimits,
) -> Result<Arc<DurableRootCoordinator>, CacheError> {
    let registry = DURABLE_COORDINATORS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry.lock().map_err(|_| CacheError::Memory)?;
    registry.retain(|_, coordinator| coordinator.strong_count() > 0);
    if let Some(coordinator) = registry.get(root).and_then(Weak::upgrade) {
        if coordinator.limits != limits {
            return Err(CacheError::LimitsMismatch {
                existing: coordinator.limits,
                requested: limits,
            });
        }
        return Ok(coordinator);
    }
    let coordinator = Arc::new(DurableRootCoordinator {
        limits,
        state: Mutex::new(DurableRootState::default()),
        #[cfg(test)]
        read_pause: Mutex::new(None),
    });
    registry.insert(root.to_owned(), Arc::downgrade(&coordinator));
    Ok(coordinator)
}

fn temporary_entry_is_stale(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return true;
    };
    let Some(stem) = name
        .strip_prefix('.')
        .and_then(|name| name.strip_suffix(".tmp"))
    else {
        return true;
    };
    let mut components = stem.rsplitn(3, '.');
    let Some(counter) = components.next() else {
        return true;
    };
    let Some(process) = components.next() else {
        return true;
    };
    let Some(digest) = components.next() else {
        return true;
    };
    if counter.parse::<u64>().is_err()
        || process.parse::<u32>().is_err()
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return true;
    }
    let Ok(modified) = path.metadata().and_then(|metadata| metadata.modified()) else {
        return true;
    };
    SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age.as_secs() >= TEMPORARY_STALE_SECONDS)
}

fn decrement(counts: &mut BTreeMap<[u8; 32], u32>, digest: [u8; 32]) -> Result<(), CacheError> {
    let Some(count) = counts.get_mut(&digest) else {
        return Err(CacheError::Protected);
    };
    if *count == 1 {
        counts.remove(&digest);
    } else {
        *count -= 1;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct Header {
    digest: [u8; 32],
    created_at: CacheTime,
    key_bytes: Vec<u8>,
    width: u32,
    height: u32,
    payload_len: u64,
    payload_digest: [u8; 32],
    file_bytes: u64,
}

fn write_entry(
    path: &Path,
    key: ThumbnailKey,
    image: &DecodedImage,
    created_at: CacheTime,
) -> Result<(), CacheError> {
    let key_bytes = key.canonical_bytes();
    let payload = image.pixels();
    let payload_digest: [u8; 32] = Sha256::digest(payload).into();
    let mut header = Vec::with_capacity(8 + 2 + 4 + 8 + 4 + 4 + 8 + 32 + 32 + key_bytes.len() + 32);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&CURRENT_CACHE_SCHEMA.to_be_bytes());
    header.extend_from_slice(
        &(u32::try_from(key_bytes.len()).map_err(|_| CacheError::CorruptEntry)?).to_be_bytes(),
    );
    header.extend_from_slice(&created_at.seconds().to_be_bytes());
    header.extend_from_slice(&image.dimensions().width().to_be_bytes());
    header.extend_from_slice(&image.dimensions().height().to_be_bytes());
    header.extend_from_slice(
        &(u64::try_from(payload.len()).map_err(|_| CacheError::CorruptEntry)?).to_be_bytes(),
    );
    header.extend_from_slice(&key.digest());
    header.extend_from_slice(&payload_digest);
    header.extend_from_slice(&key_bytes);
    let header_digest: [u8; 32] = Sha256::digest(&header).into();
    header.extend_from_slice(&header_digest);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error("stage cache entry", path, error))?;
    let result = file
        .write_all(&header)
        .and_then(|()| file.write_all(payload))
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error("write cache entry", path, error));
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn read_entry(
    path: &Path,
    key: ThumbnailKey,
    limits: CacheLimits,
) -> Result<CacheEntry, CacheError> {
    let bytes = read_bounded_entry(path, limits)?;
    let file_bytes = u64::try_from(bytes.len()).map_err(|_| CacheError::CorruptEntry)?;
    let header = parse_header(&bytes, file_bytes, limits)?;
    if header.digest != key.digest() || header.key_bytes != key.canonical_bytes() {
        return Err(CacheError::CorruptEntry);
    }
    let payload_start = bytes
        .len()
        .checked_sub(usize::try_from(header.payload_len).map_err(|_| CacheError::CorruptEntry)?)
        .ok_or(CacheError::CorruptEntry)?;
    let payload = &bytes[payload_start..];
    if Sha256::digest(payload).as_slice() != header.payload_digest {
        return Err(CacheError::CorruptEntry);
    }
    let dimensions =
        ImageDimensions::new(header.width, header.height).map_err(|_| CacheError::CorruptEntry)?;
    let image =
        DecodedImage::new_with_color_encoding(dimensions, payload.to_vec(), ColorEncoding::Srgb)
            .map_err(|_| CacheError::Image)?;
    Ok(CacheEntry {
        key,
        image,
        created_at: header.created_at,
    })
}

fn read_header(path: &Path, limits: CacheLimits) -> Result<Header, CacheError> {
    let mut file = File::open(path).map_err(|error| io_error("open cache entry", path, error))?;
    let initial_length = file
        .metadata()
        .map_err(|error| io_error("read cache entry metadata", path, error))?
        .len();
    if initial_length > limits.max_entry_bytes {
        return Err(CacheError::CorruptEntry);
    }

    let mut fixed = [0; FIXED_HEADER_BYTES];
    read_header_exact(&mut file, &mut fixed, path)?;
    let key_len = usize::try_from(u32::from_be_bytes(
        fixed[10..14]
            .try_into()
            .map_err(|_| CacheError::CorruptEntry)?,
    ))
    .map_err(|_| CacheError::CorruptEntry)?;
    let header_bytes = FIXED_HEADER_BYTES
        .checked_add(key_len)
        .and_then(|value| value.checked_add(HEADER_DIGEST_BYTES))
        .ok_or(CacheError::CorruptEntry)?;
    if u64::try_from(header_bytes).map_err(|_| CacheError::CorruptEntry)? > limits.max_entry_bytes {
        return Err(CacheError::CorruptEntry);
    }

    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(header_bytes)
        .map_err(|_| CacheError::CorruptEntry)?;
    bytes.extend_from_slice(&fixed);
    bytes.resize(header_bytes, 0);
    read_header_exact(&mut file, &mut bytes[FIXED_HEADER_BYTES..], path)?;
    let observed_length = file
        .metadata()
        .map_err(|error| io_error("read cache entry metadata", path, error))?
        .len();
    parse_header(&bytes, observed_length, limits)
}

fn read_header_exact(file: &mut File, bytes: &mut [u8], path: &Path) -> Result<(), CacheError> {
    file.read_exact(bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::UnexpectedEof {
            CacheError::CorruptEntry
        } else {
            io_error("read cache entry header", path, error)
        }
    })
}

fn read_bounded_entry(path: &Path, limits: CacheLimits) -> Result<Vec<u8>, CacheError> {
    let file = File::open(path).map_err(|error| io_error("open cache entry", path, error))?;
    let length = file
        .metadata()
        .map_err(|error| io_error("read cache entry metadata", path, error))?
        .len();
    if length > limits.max_entry_bytes {
        return Err(CacheError::CorruptEntry);
    }
    let capacity = usize::try_from(length).map_err(|_| CacheError::CorruptEntry)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| CacheError::CorruptEntry)?;
    (&file)
        .take(limits.max_entry_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read cache entry", path, error))?;
    let observed_length = file
        .metadata()
        .map_err(|error| io_error("read cache entry metadata", path, error))?
        .len();
    let read_length = u64::try_from(bytes.len()).map_err(|_| CacheError::CorruptEntry)?;
    if observed_length > limits.max_entry_bytes || read_length != observed_length {
        return Err(CacheError::CorruptEntry);
    }
    Ok(bytes)
}

fn parse_header(bytes: &[u8], file_bytes: u64, limits: CacheLimits) -> Result<Header, CacheError> {
    if file_bytes > limits.max_entry_bytes {
        return Err(CacheError::CorruptEntry);
    }
    let mut reader = Reader { bytes, offset: 0 };
    if reader.take(8)? != MAGIC {
        return Err(CacheError::CorruptEntry);
    }
    let schema = reader.u16()?;
    if schema != CURRENT_CACHE_SCHEMA {
        return Err(CacheError::UnsupportedSchema(schema));
    }
    let key_len = usize::try_from(reader.u32()?).map_err(|_| CacheError::CorruptEntry)?;
    let created_at = CacheTime::from_seconds(reader.u64()?);
    let width = reader.u32()?;
    let height = reader.u32()?;
    let payload_len = reader.u64()?;
    let digest = reader.array::<32>()?;
    let payload_digest = reader.array::<32>()?;
    let key_bytes = reader.take(key_len)?.to_vec();
    let header_end = reader.offset;
    let stored_header_digest = reader.array::<32>()?;
    if Sha256::digest(&bytes[..header_end]).as_slice() != stored_header_digest {
        return Err(CacheError::CorruptEntry);
    }
    let payload_start = header_end
        .checked_add(HEADER_DIGEST_BYTES)
        .ok_or(CacheError::CorruptEntry)?;
    let expected_file_bytes = u64::try_from(payload_start)
        .map_err(|_| CacheError::CorruptEntry)?
        .checked_add(payload_len)
        .ok_or(CacheError::CorruptEntry)?;
    if expected_file_bytes != file_bytes {
        return Err(CacheError::CorruptEntry);
    }
    Ok(Header {
        digest,
        created_at,
        key_bytes,
        width,
        height,
        payload_len,
        payload_digest,
        file_bytes,
    })
}

fn serialized_size(key_len: usize, payload_len: usize) -> Result<u64, CacheError> {
    u64::try_from(
        8usize
            .checked_add(2)
            .and_then(|value| value.checked_add(4))
            .and_then(|value| {
                value.checked_add(8 + 4 + 4 + 8 + 32 + 32 + key_len + 32 + payload_len)
            })
            .ok_or(CacheError::CorruptEntry)?,
    )
    .map_err(|_| CacheError::CorruptEntry)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], CacheError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(CacheError::CorruptEntry)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(CacheError::CorruptEntry)?;
        self.offset = end;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16, CacheError> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| CacheError::CorruptEntry)?,
        ))
    }
    fn u32(&mut self) -> Result<u32, CacheError> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| CacheError::CorruptEntry)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, CacheError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| CacheError::CorruptEntry)?,
        ))
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], CacheError> {
        self.take(N)?
            .try_into()
            .map_err(|_| CacheError::CorruptEntry)
    }
}

fn io_error(operation: &'static str, path: &Path, error: std::io::Error) -> CacheError {
    CacheError::Io {
        operation,
        path: path.to_owned(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use rusttable_core::{AssetId, ContentHash, EditId, PhotoId, Revision};

    use super::*;
    use crate::{MipmapLevel, ThumbnailRequest, ThumbnailSize};

    #[test]
    fn a_reserved_read_does_not_block_an_unrelated_staged_write() {
        let directory = std::env::temp_dir().join(format!(
            "rusttable-cache-io-lock-{}-{}",
            std::process::id(),
            TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&directory);
        fs::create_dir_all(&directory).expect("temporary directory");
        let limits = CacheLimits::new(64 * 1024, 32 * 1024).expect("limits");
        let first = test_key(5);
        let second = test_key(6);
        let image = test_image();
        let (mut store, _) = CacheStore::open(&directory, limits).expect("open");
        store
            .put(first, &image, CacheTime::from_seconds(20))
            .expect("put first");

        let (read_reserved, resume_read) = store.pause_next_read();
        let mut reader = store.clone();
        let read = std::thread::spawn(move || reader.get(first, CacheTime::from_seconds(21)));
        read_reserved
            .recv_timeout(Duration::from_secs(2))
            .expect("read reservation");
        assert!(matches!(
            store.invalidate(first),
            Err(CacheError::Protected)
        ));

        let mut writer = store.clone();
        let (write_done, write_completed) = mpsc::channel();
        let write = std::thread::spawn(move || {
            let result = writer.put(second, &image, CacheTime::from_seconds(22));
            let _ = write_done.send(());
            result
        });
        let completed_while_read_was_paused =
            write_completed.recv_timeout(Duration::from_secs(2)).is_ok();
        resume_read.send(()).expect("resume read");

        assert!(
            read.join()
                .expect("read thread")
                .expect("read result")
                .is_some()
        );
        write.join().expect("write thread").expect("write result");
        assert!(
            completed_while_read_was_paused,
            "root lock was held during payload read"
        );
        drop(store);
        fs::remove_dir_all(directory).expect("cleanup");
    }

    fn test_key(edit_revision: u64) -> ThumbnailKey {
        ThumbnailKey::new(
            ContentHash::Sha256([7; 32]),
            PhotoId::new(1).expect("photo"),
            AssetId::new(2).expect("asset"),
            EditId::new(3).expect("edit"),
            Revision::from_u64(4),
            Revision::from_u64(edit_revision),
            [15; 32],
            10,
            11,
            [12; 32],
            13,
            [14; 32],
            ThumbnailRequest::new(
                MipmapLevel::new(2).expect("level"),
                ThumbnailSize::fit(128, 96).expect("size"),
            ),
        )
    }

    fn test_image() -> DecodedImage {
        DecodedImage::new_with_color_encoding(
            ImageDimensions::new(2, 2).expect("dimensions"),
            vec![1; 16],
            ColorEncoding::Srgb,
        )
        .expect("image")
    }
}
