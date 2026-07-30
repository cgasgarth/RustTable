#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::missing_fields_in_debug
)]

use std::collections::BTreeMap;

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use crate::PipelineGeneration;

const MAX_SECONDARY_REASONS: usize = 4;

/// The typed reasons that may make a pixelpipe request obsolete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancellationReason {
    SupersededGeneration(PipelineGeneration),
    UserRequested,
    SelectionChanged,
    EditChanged,
    SourceChanged,
    DeadlineExceeded,
    MemoryPressure,
    Shutdown,
    DeviceLost,
    ParentFailed,
    NoConsumers,
}

impl CancellationReason {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::SupersededGeneration(_) => "superseded-generation",
            Self::UserRequested => "user-requested",
            Self::SelectionChanged => "selection-changed",
            Self::EditChanged => "edit-changed",
            Self::SourceChanged => "source-changed",
            Self::DeadlineExceeded => "deadline-exceeded",
            Self::MemoryPressure => "memory-pressure",
            Self::Shutdown => "shutdown",
            Self::DeviceLost => "device-lost",
            Self::ParentFailed => "parent-failed",
            Self::NoConsumers => "no-consumers",
        }
    }
}

/// A monotonic deadline. Wall-clock changes cannot extend or shorten it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancellationDeadline(Instant);

impl CancellationDeadline {
    #[must_use]
    pub fn after(duration: Duration) -> Self {
        Self(Instant::now() + duration)
    }

    #[must_use]
    pub const fn at(deadline: Instant) -> Self {
        Self(deadline)
    }

    #[must_use]
    pub const fn instant(self) -> Instant {
        self.0
    }

    #[must_use]
    pub fn expired(self) -> bool {
        Instant::now() >= self.0
    }

    #[must_use]
    pub fn remaining(self) -> Duration {
        self.0.saturating_duration_since(Instant::now())
    }
}

/// A bounded cancellation failure that retains the first reason and stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancellationError {
    reason: CancellationReason,
    stage: Option<CancellationStage>,
}

impl CancellationError {
    #[must_use]
    pub const fn reason(self) -> CancellationReason {
        self.reason
    }

    #[must_use]
    pub const fn stage(self) -> Option<CancellationStage> {
        self.stage
    }
}

impl fmt::Display for CancellationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "pixelpipe cancelled: {}", self.reason.tag())?;
        if let Some(stage) = self.stage {
            write!(formatter, " during {stage:?}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CancellationError {}

/// A named boundary where a cancellation check is mandatory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancellationStage {
    SourceDecode,
    Preparation,
    Node,
    Allocation,
    Tile,
    Transfer,
    Analysis,
    CacheBuild,
    CachePromotion,
    Publication,
    ResourceCleanup,
    GpuRetirement,
}

struct State {
    reason: Option<CancellationReason>,
    secondary: Vec<CancellationReason>,
    hooks: BTreeMap<u64, Box<dyn FnOnce(CancellationReason) + Send + 'static>>,
}

struct Node {
    generation: PipelineGeneration,
    state: Mutex<State>,
    wake: Condvar,
    next_hook: AtomicU64,
    parent_registration: Mutex<Option<CleanupRegistration>>,
}

/// A runtime-independent, thread-safe cancellation token.
#[derive(Clone)]
pub struct CancellationToken(Arc<Node>);

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("generation", &self.generation())
            .field("reason", &self.reason())
            .finish()
    }
}

impl CancellationToken {
    #[must_use]
    pub fn new() -> Self {
        Self::for_generation(PipelineGeneration::new(1).expect("one is nonzero"))
    }

    #[must_use]
    pub fn for_generation(generation: PipelineGeneration) -> Self {
        Self(Arc::new(Node {
            generation,
            state: Mutex::new(State {
                reason: None,
                secondary: Vec::new(),
                hooks: BTreeMap::new(),
            }),
            wake: Condvar::new(),
            next_hook: AtomicU64::new(1),
            parent_registration: Mutex::new(None),
        }))
    }

    #[must_use]
    pub fn generation(&self) -> PipelineGeneration {
        self.0.generation
    }

    /// Compatibility shorthand for an explicit user cancellation.
    pub fn cancel(&self) {
        self.cancel_with_reason(CancellationReason::UserRequested);
    }

    pub fn cancel_with_reason(&self, reason: CancellationReason) {
        let hooks = {
            let Ok(mut state) = self.0.state.lock() else {
                return;
            };
            if state.reason.is_some() {
                if state.secondary.len() < MAX_SECONDARY_REASONS
                    && !state.secondary.contains(&reason)
                {
                    state.secondary.push(reason);
                }
                return;
            }
            state.reason = Some(reason);
            std::mem::take(&mut state.hooks)
        };
        self.0.wake.notify_all();
        for hook in hooks.into_values() {
            hook(reason);
        }
    }

    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.reason().is_some()
    }

    #[must_use]
    pub fn reason(&self) -> Option<CancellationReason> {
        self.0.state.lock().ok().and_then(|state| state.reason)
    }

    #[must_use]
    pub fn secondary_reasons(&self) -> Vec<CancellationReason> {
        self.0
            .state
            .lock()
            .map(|state| state.secondary.clone())
            .unwrap_or_default()
    }

    /// Converts an observed cancellation into a typed stage error.
    pub fn check(&self, stage: CancellationStage) -> Result<(), CancellationError> {
        if self.is_cancelled() {
            return Err(CancellationError {
                reason: self.reason().expect("checked cancellation reason"),
                stage: Some(stage),
            });
        }
        Ok(())
    }

    /// Waits for cancellation without depending on an async runtime.
    #[must_use]
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let Ok(state) = self.0.state.lock() else {
            return true;
        };
        if state.reason.is_some() {
            return true;
        }
        let Ok((state, _)) = self.0.wake.wait_timeout(state, timeout) else {
            return true;
        };
        state.reason.is_some()
    }

    /// Registers a one-shot resource cleanup callback.
    pub fn register_cleanup<F>(&self, callback: F) -> CleanupRegistration
    where
        F: FnOnce(CancellationReason) + Send + 'static,
    {
        let id = self.0.next_hook.fetch_add(1, Ordering::Relaxed);
        let mut callback = Some(callback);
        let immediate = self.0.state.lock().ok().and_then(|mut state| {
            if let Some(reason) = state.reason {
                Some(reason)
            } else {
                state.hooks.insert(
                    id,
                    Box::new(callback.take().expect("cleanup callback is present")),
                );
                None
            }
        });
        if let Some(reason) = immediate {
            callback.take().expect("cleanup callback is present")(reason);
        }
        CleanupRegistration {
            node: Arc::downgrade(&self.0),
            id,
        }
    }

    #[must_use]
    pub fn child(&self, generation: PipelineGeneration) -> Self {
        let child = Self::for_generation(generation);
        let parent = self.clone();
        let child_clone = child.clone();
        let registration = parent.register_cleanup(move |reason| {
            child_clone.cancel_with_reason(reason);
        });
        if let Ok(mut slot) = child.0.parent_registration.lock() {
            *slot = Some(registration);
        }
        child
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// A token plus the immutable generation and named execution scope.
#[derive(Debug, Clone)]
pub struct CancellationScope {
    token: CancellationToken,
    stage: CancellationStage,
    deadline: Option<CancellationDeadline>,
}

impl CancellationScope {
    #[must_use]
    pub fn root(generation: PipelineGeneration) -> Self {
        Self {
            token: CancellationToken::for_generation(generation),
            stage: CancellationStage::Preparation,
            deadline: None,
        }
    }

    /// Creates an execution scope around a cache flight's shared token.
    ///
    /// Consumer-specific deadlines are intentionally not inherited: the
    /// shared build remains useful while any registered consumer is active.
    pub(crate) fn from_shared_token(token: CancellationToken) -> Self {
        Self {
            token,
            stage: CancellationStage::Preparation,
            deadline: None,
        }
    }

    #[must_use]
    pub fn with_deadline(mut self, deadline: CancellationDeadline) -> Self {
        self.deadline = Some(deadline);
        self
    }

    #[must_use]
    pub fn child(&self, stage: CancellationStage) -> Self {
        Self {
            token: self.token.child(self.generation()),
            stage,
            deadline: self.deadline,
        }
    }

    #[must_use]
    pub const fn token(&self) -> &CancellationToken {
        &self.token
    }

    #[must_use]
    pub fn generation(&self) -> PipelineGeneration {
        self.token.generation()
    }

    #[must_use]
    pub const fn stage(&self) -> CancellationStage {
        self.stage
    }

    #[must_use]
    pub const fn deadline(&self) -> Option<CancellationDeadline> {
        self.deadline
    }

    pub fn cancel(&self, reason: CancellationReason) {
        self.token.cancel_with_reason(reason);
    }

    pub fn check(&self) -> Result<(), CancellationError> {
        if self.deadline.is_some_and(CancellationDeadline::expired) {
            self.token
                .cancel_with_reason(CancellationReason::DeadlineExceeded);
        }
        self.token.check(self.stage)
    }

    pub fn register_cleanup<F>(&self, callback: F) -> CleanupRegistration
    where
        F: FnOnce(CancellationReason) + Send + 'static,
    {
        self.token.register_cleanup(callback)
    }
}

/// RAII registration for a cancellation cleanup hook.
pub struct CleanupRegistration {
    node: Weak<Node>,
    id: u64,
}

impl fmt::Debug for CleanupRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CleanupRegistration")
            .field("id", &self.id)
            .finish()
    }
}

impl Drop for CleanupRegistration {
    fn drop(&mut self) {
        if let Some(node) = self.node.upgrade()
            && let Ok(mut state) = node.state.lock()
        {
            state.hooks.remove(&self.id);
        }
    }
}

/// The checked, non-wrapping source of request generations.
#[derive(Debug)]
pub struct GenerationClock(AtomicU64);

impl GenerationClock {
    #[must_use]
    pub const fn new(initial: PipelineGeneration) -> Self {
        Self(AtomicU64::new(initial.get()))
    }

    pub fn next(&self) -> Result<PipelineGeneration, GenerationClockError> {
        let current = self.0.load(Ordering::Acquire);
        let next = current
            .checked_add(1)
            .ok_or(GenerationClockError::Overflow)?;
        self.0
            .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| PipelineGeneration::new(next).expect("checked nonzero generation"))
            .map_err(|_| GenerationClockError::ConcurrentAdvance)
    }

    #[must_use]
    pub fn current(&self) -> PipelineGeneration {
        PipelineGeneration::new(self.0.load(Ordering::Acquire)).expect("clock is nonzero")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerationClockError {
    Overflow,
    ConcurrentAdvance,
}

impl fmt::Display for GenerationClockError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Overflow => "pipeline generation overflowed",
            Self::ConcurrentAdvance => "pipeline generation advanced concurrently",
        })
    }
}

impl std::error::Error for GenerationClockError {}
