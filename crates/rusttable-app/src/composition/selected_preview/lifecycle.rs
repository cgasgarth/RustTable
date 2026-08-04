use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use rusttable_core::{EditId, PhotoId, Revision};
use rusttable_pixelpipe::{CancellationReason, CancellationScope, PipelineGeneration};

type PreviewWork = Box<dyn FnOnce() + Send + 'static>;

struct PreviewWorkState {
    pending: Option<PreviewWork>,
    stopped: bool,
}

struct PreviewWorker {
    queue: Arc<(Mutex<PreviewWorkState>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl PreviewWorker {
    fn new() -> Self {
        let queue = Arc::new((
            Mutex::new(PreviewWorkState {
                pending: None,
                stopped: false,
            }),
            Condvar::new(),
        ));
        let weak_queue = Arc::downgrade(&queue);
        let thread = thread::Builder::new()
            .name("rusttable-preview".to_owned())
            .spawn(move || {
                loop {
                    let Some(queue) = weak_queue.upgrade() else {
                        return;
                    };
                    let work = {
                        let (state_lock, wake) = &*queue;
                        let mut state = state_lock.lock().expect("preview worker state");
                        while state.pending.is_none() && !state.stopped {
                            state = wake.wait(state).expect("preview worker wake");
                        }
                        if state.stopped {
                            return;
                        }
                        state.pending.take().expect("preview work is pending")
                    };
                    work();
                }
            })
            .ok();
        Self { queue, thread }
    }

    fn submit(&self, work: PreviewWork) -> bool {
        let (state_lock, wake) = &*self.queue;
        let mut state = state_lock.lock().expect("preview worker state");
        if state.stopped || self.thread.is_none() {
            return false;
        }
        state.pending = Some(work);
        drop(state);
        wake.notify_one();
        true
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        let (state_lock, wake) = &*self.queue;
        let mut state = state_lock.lock().expect("preview worker state");
        state.stopped = true;
        state.pending = None;
        wake.notify_one();
        drop(state);
        let _ = self.thread.take();
    }
}

/// Monotonic identity for one selected-photo preview request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreviewSelectionToken {
    generation: u64,
    photo_id: PhotoId,
    edit_id: EditId,
    edit_revision: Revision,
}

/// Tracks which asynchronous preview result is still allowed to update the UI.
pub struct PreviewLifecycle {
    next_generation: u64,
    active: Option<PreviewSelectionToken>,
    active_cancellation: Option<CancellationScope>,
    worker: PreviewWorker,
}

impl Default for PreviewLifecycle {
    fn default() -> Self {
        Self {
            next_generation: 0,
            active: None,
            active_cancellation: None,
            worker: PreviewWorker::new(),
        }
    }
}

impl PreviewLifecycle {
    pub fn begin(
        &mut self,
        photo_id: PhotoId,
        edit_id: EditId,
        edit_revision: Revision,
    ) -> PreviewSelectionToken {
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("preview generation must not wrap");
        let generation = PipelineGeneration::new(self.next_generation)
            .expect("preview generation starts at one");
        self.invalidate(CancellationReason::SupersededGeneration(generation));
        let token = PreviewSelectionToken {
            generation: self.next_generation,
            photo_id,
            edit_id,
            edit_revision,
        };
        self.active = Some(token);
        self.active_cancellation = Some(CancellationScope::root(generation));
        token
    }

    pub fn invalidate(&mut self, reason: CancellationReason) {
        self.active = None;
        if let Some(cancellation) = self.active_cancellation.take() {
            cancellation.cancel(reason);
        }
    }

    #[must_use]
    pub fn is_current(&self, token: PreviewSelectionToken) -> bool {
        self.active.is_some_and(|active| {
            active.generation == token.generation
                && active.photo_id == token.photo_id
                && active.edit_id == token.edit_id
                && active.edit_revision == token.edit_revision
        })
    }

    #[must_use]
    pub fn cancellation_scope(&self, token: PreviewSelectionToken) -> Option<CancellationScope> {
        if self.is_current(token) {
            self.active_cancellation.clone()
        } else {
            None
        }
    }

    pub fn submit_work(&self, work: impl FnOnce() + Send + 'static) -> bool {
        self.worker.submit(Box::new(work))
    }
}

impl Drop for PreviewLifecycle {
    fn drop(&mut self) {
        self.invalidate(CancellationReason::SelectionChanged);
    }
}

impl PreviewSelectionToken {
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }

    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    pub const fn edit_id(self) -> EditId {
        self.edit_id
    }

    pub const fn edit_revision(self) -> Revision {
        self.edit_revision
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::{EditId, PhotoId, Revision};
    use rusttable_pixelpipe::{CancellationReason, PipelineGeneration};

    use super::PreviewLifecycle;

    fn photo_id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero test photo ID")
    }

    #[test]
    fn only_the_latest_selection_token_remains_current() {
        let mut lifecycle = PreviewLifecycle::default();
        let first = lifecycle.begin(photo_id(1), EditId::new(2).unwrap(), Revision::from_u64(1));
        let second = lifecycle.begin(photo_id(2), EditId::new(3).unwrap(), Revision::from_u64(1));

        assert!(!lifecycle.is_current(first));
        assert!(lifecycle.is_current(second));
    }

    #[test]
    fn reselection_gets_a_new_generation_for_stale_result_protection() {
        let mut lifecycle = PreviewLifecycle::default();
        let first = lifecycle.begin(photo_id(1), EditId::new(2).unwrap(), Revision::from_u64(1));
        let first_cancellation = lifecycle
            .cancellation_scope(first)
            .expect("first cancellation scope");
        let second = lifecycle.begin(photo_id(1), EditId::new(2).unwrap(), Revision::from_u64(2));

        assert_ne!(first, second);
        assert!(!lifecycle.is_current(first));
        assert!(lifecycle.is_current(second));
        let error = first_cancellation
            .check()
            .expect_err("superseded scope is cancelled");
        assert_eq!(
            error.reason(),
            CancellationReason::SupersededGeneration(
                PipelineGeneration::new(second.generation()).expect("generation")
            )
        );
        lifecycle
            .cancellation_scope(second)
            .expect("current cancellation scope")
            .check()
            .expect("current scope remains live");
    }

    #[test]
    fn invalidation_cancels_the_scope_and_revokes_the_active_token() {
        let mut lifecycle = PreviewLifecycle::default();
        let token = lifecycle.begin(photo_id(1), EditId::new(2).unwrap(), Revision::from_u64(1));
        let cancellation = lifecycle
            .cancellation_scope(token)
            .expect("active cancellation scope");

        lifecycle.invalidate(CancellationReason::SelectionChanged);

        assert!(!lifecycle.is_current(token));
        assert!(lifecycle.cancellation_scope(token).is_none());
        assert_eq!(
            cancellation
                .check()
                .expect_err("invalidated scope is cancelled")
                .reason(),
            CancellationReason::SelectionChanged
        );
    }

    #[test]
    fn preview_worker_runs_one_active_and_only_the_latest_pending_request() {
        use std::sync::mpsc;
        use std::sync::{Arc, Barrier, Mutex};

        let lifecycle = PreviewLifecycle::default();
        let started = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let order = Arc::new(Mutex::new(Vec::new()));
        let (finished_sender, finished_receiver) = mpsc::channel();

        let first_started = Arc::clone(&started);
        let first_release = Arc::clone(&release);
        let first_order = Arc::clone(&order);
        assert!(lifecycle.submit_work(move || {
            first_order.lock().expect("order lock").push(1_u8);
            first_started.wait();
            first_release.wait();
        }));
        started.wait();

        let second_order = Arc::clone(&order);
        assert!(lifecycle.submit_work(move || {
            second_order.lock().expect("order lock").push(2_u8);
        }));
        let third_order = Arc::clone(&order);
        assert!(lifecycle.submit_work(move || {
            third_order.lock().expect("order lock").push(3_u8);
            finished_sender.send(()).expect("test receiver");
        }));
        release.wait();

        finished_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("latest pending preview completes");
        assert_eq!(*order.lock().expect("order lock"), vec![1, 3]);
    }
}
