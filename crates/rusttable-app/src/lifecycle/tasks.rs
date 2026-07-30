use std::{
    future::Future,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::{
    sync::{Mutex as AsyncMutex, Notify, Semaphore},
    task::JoinSet,
    time::{self, Instant},
};

/// Why a service-owned task group was cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationReason {
    /// The application is shutting down.
    Shutdown,
    /// A parent service failed while starting.
    StartupFailure,
    /// A caller explicitly cancelled the operation.
    Requested,
}

struct CancellationNode {
    cancelled: AtomicBool,
    reason: Mutex<Option<CancellationReason>>,
    children: Mutex<Vec<Weak<CancellationNode>>>,
    notify: Notify,
}

/// A hierarchical, cancellation-safe token owned by `RustTable`.
#[derive(Clone)]
pub struct CancellationToken {
    node: Arc<CancellationNode>,
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    /// Creates a new root token.
    #[must_use]
    pub fn new() -> Self {
        Self {
            node: Arc::new(CancellationNode {
                cancelled: AtomicBool::new(false),
                reason: Mutex::new(None),
                children: Mutex::new(Vec::new()),
                notify: Notify::new(),
            }),
        }
    }

    /// Creates a child which is cancelled when this token is cancelled.
    #[must_use]
    pub fn child_token(&self) -> Self {
        let child = Self::new();
        {
            let mut children = self
                .node
                .children
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            children.push(Arc::downgrade(&child.node));
        }
        if let Some(reason) = self.reason() {
            child.cancel(reason);
        }
        child
    }

    /// Cancels this token and all currently live descendants.
    pub fn cancel(&self, reason: CancellationReason) {
        if self.node.cancelled.swap(true, Ordering::AcqRel) {
            return;
        }
        *self
            .node
            .reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(reason);
        self.node.notify.notify_waiters();
        let children = self
            .node
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for child in children {
            Self { node: child }.cancel(reason);
        }
    }

    /// Returns whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.node.cancelled.load(Ordering::Acquire)
    }

    /// Returns the first cancellation reason, if any.
    #[must_use]
    pub fn reason(&self) -> Option<CancellationReason> {
        *self
            .node
            .reason
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Waits until this token is cancelled.
    pub async fn cancelled(&self) {
        let notified = self.node.notify.notified();
        if !self.is_cancelled() {
            notified.await;
        }
    }
}

/// Stable identity for a service-owned task group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskGroupId(String);

impl TaskGroupId {
    pub(crate) fn new(service: &str) -> Self {
        Self(format!("service:{service}"))
    }

    /// Returns the stable group identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identity returned when a task is accepted by a group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskReceipt {
    group: TaskGroupId,
    sequence: u64,
}

impl TaskReceipt {
    /// Returns the task group that owns this task.
    #[must_use]
    pub fn group(&self) -> &TaskGroupId {
        &self.group
    }

    /// Returns the monotonically increasing task sequence within the group.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }
}

/// Result of draining a service-owned task group.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskDrainReceipt {
    /// Number of tasks that completed normally or by cancellation.
    pub completed: usize,
    /// Number of tasks that were aborted after the deadline.
    pub forced_aborts: usize,
    /// Number of task panics observed while joining.
    pub panics: usize,
    /// Number of tasks still owned after the bounded drain.
    pub leaked: usize,
}

/// A service-owned task group backed by stable Tokio primitives.
#[derive(Clone)]
pub struct ServiceTaskGroup {
    id: TaskGroupId,
    token: CancellationToken,
    accepting: Arc<AtomicBool>,
    next_sequence: Arc<AtomicU64>,
    tasks: Arc<AsyncMutex<JoinSet<()>>>,
    blocking_slots: Arc<Semaphore>,
}

impl ServiceTaskGroup {
    pub(crate) fn new(service: &str, token: CancellationToken, blocking_limit: usize) -> Self {
        Self {
            id: TaskGroupId::new(service),
            token,
            accepting: Arc::new(AtomicBool::new(true)),
            next_sequence: Arc::new(AtomicU64::new(1)),
            tasks: Arc::new(AsyncMutex::new(JoinSet::new())),
            blocking_slots: Arc::new(Semaphore::new(blocking_limit.max(1))),
        }
    }

    /// Returns this group's cancellation token.
    #[must_use]
    pub fn cancellation(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Spawns a task owned by this group.
    ///
    /// # Errors
    ///
    /// Returns [`TaskGroupError::Stopped`] after shutdown begins.
    ///
    /// # Panics
    ///
    /// Does not panic during normal operation; task panics are reported by the
    /// group's drain receipt.
    pub async fn spawn<F>(&self, task: F) -> Result<TaskReceipt, TaskGroupError>
    where
        F: Future<Output = ()> + Send + 'static,
    {
        if !self.accepting.load(Ordering::Acquire) {
            return Err(TaskGroupError::Stopped);
        }
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed);
        let mut tasks = self.tasks.lock().await;
        if !self.accepting.load(Ordering::Acquire) {
            return Err(TaskGroupError::Stopped);
        }
        tasks.spawn(task);
        Ok(TaskReceipt {
            group: self.id.clone(),
            sequence,
        })
    }

    /// Spawns bounded blocking work without running it on a Tokio worker thread.
    ///
    /// # Errors
    ///
    /// Returns [`TaskGroupError::Stopped`] after shutdown begins.
    ///
    /// # Panics
    ///
    /// Does not panic during normal operation; task panics are reported by the
    /// group's drain receipt.
    pub async fn spawn_blocking<F>(&self, task: F) -> Result<TaskReceipt, TaskGroupError>
    where
        F: FnOnce() + Send + 'static,
    {
        let slots = Arc::clone(&self.blocking_slots);
        self.spawn(async move {
            let Ok(_slot) = slots.acquire_owned().await else {
                return;
            };
            let _ = tokio::task::spawn_blocking(task).await;
        })
        .await
    }

    /// Stops accepting tasks and drains owned tasks within the deadline.
    pub async fn shutdown(&self, deadline: Duration) -> TaskDrainReceipt {
        self.accepting.store(false, Ordering::Release);
        self.token.cancel(CancellationReason::Shutdown);
        let deadline = Instant::now() + deadline;
        let mut receipt = TaskDrainReceipt::default();

        loop {
            let joined = {
                let mut tasks = self.tasks.lock().await;
                if tasks.is_empty() {
                    break;
                }
                time::timeout_at(deadline, tasks.join_next()).await
            };
            match joined {
                Ok(Some(Ok(()))) => receipt.completed = receipt.completed.saturating_add(1),
                Ok(Some(Err(error))) => {
                    receipt.completed = receipt.completed.saturating_add(1);
                    if error.is_panic() {
                        receipt.panics = receipt.panics.saturating_add(1);
                    }
                }
                Ok(None) | Err(_) => break,
            }
        }

        let mut tasks = self.tasks.lock().await;
        if !tasks.is_empty() {
            receipt.forced_aborts = tasks.len();
            tasks.abort_all();
            while let Some(result) = tasks.join_next().await {
                receipt.completed = receipt.completed.saturating_add(1);
                if result.is_err() && result.is_err_and(|error| error.is_panic()) {
                    receipt.panics = receipt.panics.saturating_add(1);
                }
            }
        }
        receipt.leaked = tasks.len();
        receipt
    }

    /// Returns the number of currently owned tasks.
    pub async fn task_count(&self) -> usize {
        self.tasks.lock().await.len()
    }
}

/// Failure to submit work to a service-owned task group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskGroupError {
    /// The group has begun shutdown and rejects new work.
    Stopped,
}
