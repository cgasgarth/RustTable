use std::future::Future;

use iced::task::{Handle, Task};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskGeneration(u64);

impl TaskGeneration {
    #[must_use]
    pub const fn zero() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.saturating_add(1))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskResult<T> {
    Progress {
        generation: TaskGeneration,
        value: T,
    },
    Completed {
        generation: TaskGeneration,
        value: T,
    },
    Cancelled {
        generation: TaskGeneration,
    },
}

#[derive(Debug)]
pub struct GenerationTask<T> {
    generation: TaskGeneration,
    handle: Handle,
    task: Task<TaskResult<T>>,
}

impl<T> GenerationTask<T> {
    #[must_use]
    pub fn new(generation: TaskGeneration, task: Task<TaskResult<T>>, handle: Handle) -> Self {
        Self {
            generation,
            handle,
            task,
        }
    }

    #[must_use]
    pub const fn generation(&self) -> TaskGeneration {
        self.generation
    }

    pub fn abort(&self) {
        self.handle.abort();
    }

    pub fn into_task(self) -> Task<TaskResult<T>> {
        self.task
    }
}

/// Creates an abortable UI-owned task whose output always carries its generation.
///
/// Service-owned jobs must not use this helper: their lifecycle remains outside the UI.
pub fn abortable_generation_task<T, F>(generation: TaskGeneration, future: F) -> GenerationTask<T>
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    let task = Task::perform(future, move |value| TaskResult::Completed {
        generation,
        value,
    });
    let (task, handle) = task.abortable();
    GenerationTask::new(generation, task, handle)
}

/// Creates a progress-reporting task through Iced's `Task::sip` bridge.
///
/// The caller can pass the returned task to `Task::abortable`; generation checks
/// remain the authoritative stale-result guard when a stream finishes late.
pub fn progress_generation_task<S, T>(generation: TaskGeneration, sipper: S) -> Task<TaskResult<T>>
where
    S: iced::task::Sipper<T, T> + Send + 'static,
    T: Send + 'static,
{
    Task::sip(
        sipper,
        move |value| TaskResult::Progress { generation, value },
        move |value| TaskResult::Completed { generation, value },
    )
}

#[cfg(test)]
mod tests {
    use super::{TaskGeneration, TaskResult, abortable_generation_task};

    #[test]
    fn generations_are_monotonic_and_task_output_is_tagged() {
        let generation = TaskGeneration::new(4);
        assert_eq!(generation.next().value(), 5);

        let task = abortable_generation_task(generation, async { 9_u8 });
        assert_eq!(task.generation(), generation);
        assert_eq!(
            TaskResult::Completed {
                generation,
                value: 9_u8,
            },
            TaskResult::Completed {
                generation,
                value: 9_u8,
            }
        );
    }
}
