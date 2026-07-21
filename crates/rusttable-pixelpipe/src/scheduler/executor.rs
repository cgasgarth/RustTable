#![allow(clippy::missing_errors_doc)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::{RunningTask, TaskFailure, TaskId, WorkUnitBoundary};

/// Narrow adapter owned by #180. Its implementation may dispatch onto the
/// dedicated bounded Rayon pool; this crate supplies no executor or runtime.
pub trait CpuWorkerPoolBoundary: Send + Sync {
    fn worker_limit(&self) -> u16;
    fn dispatch(&self, task: RunningTask) -> Result<(), TaskFailure>;
}

/// A work-unit callback is deliberately run only at a scheduler boundary. A
/// panic becomes a task failure and cannot unwind through pool coordination.
pub fn isolate_work_unit<F>(work: F) -> Result<(), TaskFailure>
where
    F: FnOnce() -> Result<(), ()>,
{
    match catch_unwind(AssertUnwindSafe(work)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(())) => Err(TaskFailure::WorkUnitFailed),
        Err(_) => Err(TaskFailure::PanicIsolated),
    }
}

/// The adapter-facing cancellation check. #272 can map its scope/token to
/// this boundary without making scheduler policy depend on an async runtime.
pub trait WorkUnitCancellationBoundary {
    fn check(&mut self, task: TaskId) -> WorkUnitBoundary;
}
