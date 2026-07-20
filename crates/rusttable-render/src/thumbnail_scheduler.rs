#![allow(
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    clippy::should_implement_trait
)]

use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap};

use rusttable_image::CancellationToken;

use crate::ThumbnailKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PrefetchPriority {
    Visible,
    Explicit,
    NearVisible,
    Maintenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefetchRequest {
    pub key: ThumbnailKey,
    pub priority: PrefetchPriority,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PrefetchHandle(u64);

impl PrefetchHandle {
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct PrefetchCancellation(CancellationToken);

impl PrefetchCancellation {
    #[must_use]
    pub fn new() -> Self {
        Self(CancellationToken::new())
    }
    pub fn cancel(&self) {
        self.0.cancel();
    }
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.is_cancelled()
    }
}

impl Default for PrefetchCancellation {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct PrefetchJob {
    handle: PrefetchHandle,
    request: PrefetchRequest,
    generation: u64,
    cancellation: PrefetchCancellation,
}

impl PrefetchJob {
    #[must_use]
    pub const fn handle(&self) -> PrefetchHandle {
        self.handle
    }
    #[must_use]
    pub const fn request(&self) -> PrefetchRequest {
        self.request
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn cancellation(&self) -> &PrefetchCancellation {
        &self.cancellation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefetchCompletion {
    Publish,
    Cancelled,
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefetchError {
    InvalidLimits,
    QueueFull,
}

#[derive(Debug)]
struct QueuedJob {
    job: PrefetchJob,
    sequence: u64,
}

impl PartialEq for QueuedJob {
    fn eq(&self, other: &Self) -> bool {
        self.sequence == other.sequence
    }
}
impl Eq for QueuedJob {}
impl PartialOrd for QueuedJob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for QueuedJob {
    fn cmp(&self, other: &Self) -> Ordering {
        priority_rank(other.job.request.priority)
            .cmp(&priority_rank(self.job.request.priority))
            .then_with(|| other.sequence.cmp(&self.sequence))
    }
}

#[derive(Debug)]
pub struct PrefetchScheduler {
    queue: BinaryHeap<QueuedJob>,
    cancelled: BTreeSet<PrefetchHandle>,
    next_handle: u64,
    next_sequence: u64,
    generation: u64,
    max_queued: usize,
    max_active: usize,
    active: usize,
}

impl PrefetchScheduler {
    pub const fn new(max_queued: usize, max_active: usize) -> Result<Self, PrefetchError> {
        if max_queued == 0 || max_active == 0 {
            return Err(PrefetchError::InvalidLimits);
        }
        Ok(Self {
            queue: BinaryHeap::new(),
            cancelled: BTreeSet::new(),
            next_handle: 1,
            next_sequence: 0,
            generation: 0,
            max_queued,
            max_active,
            active: 0,
        })
    }

    pub fn submit(
        &mut self,
        request: PrefetchRequest,
    ) -> Result<(PrefetchHandle, PrefetchCancellation), PrefetchError> {
        if self.queue.len() >= self.max_queued {
            return Err(PrefetchError::QueueFull);
        }
        let handle = PrefetchHandle(self.next_handle);
        self.next_handle = self.next_handle.saturating_add(1).max(1);
        let cancellation = PrefetchCancellation::new();
        self.queue.push(QueuedJob {
            job: PrefetchJob {
                handle,
                request,
                generation: self.generation,
                cancellation: cancellation.clone(),
            },
            sequence: self.next_sequence,
        });
        self.next_sequence = self.next_sequence.wrapping_add(1);
        Ok((handle, cancellation))
    }

    pub fn cancel(&mut self, handle: PrefetchHandle) -> bool {
        self.cancelled.insert(handle)
    }

    pub fn invalidate_generation(&mut self) -> Result<u64, PrefetchError> {
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or(PrefetchError::InvalidLimits)?;
        Ok(self.generation)
    }

    pub fn next(&mut self) -> Option<PrefetchJob> {
        if self.active >= self.max_active {
            return None;
        }
        while let Some(queued) = self.queue.pop() {
            if self.cancelled.remove(&queued.job.handle) || queued.job.cancellation.is_cancelled() {
                continue;
            }
            self.active += 1;
            return Some(queued.job);
        }
        None
    }

    pub fn complete(&mut self, job: PrefetchJob) -> PrefetchCompletion {
        self.active = self.active.saturating_sub(1);
        if job.cancellation.is_cancelled() {
            PrefetchCompletion::Cancelled
        } else if job.generation != self.generation {
            PrefetchCompletion::Stale
        } else {
            PrefetchCompletion::Publish
        }
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub fn queued(&self) -> usize {
        self.queue.len()
    }
    #[must_use]
    pub const fn active(&self) -> usize {
        self.active
    }
}

fn priority_rank(priority: PrefetchPriority) -> u8 {
    match priority {
        PrefetchPriority::Visible => 0,
        PrefetchPriority::Explicit => 1,
        PrefetchPriority::NearVisible => 2,
        PrefetchPriority::Maintenance => 3,
    }
}
