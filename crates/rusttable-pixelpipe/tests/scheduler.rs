use std::sync::Arc;
use std::time::{Duration, Instant};

use rusttable_pixelpipe::{
    CancellationToken, CpuPriority, CpuScheduler, PipelineGeneration, PipelineSnapshotIdentity,
    PublicationTargetKind, RequestId, ResourceClaim, SchedulerConfig, SchedulerError,
    SchedulerPublicationTarget, ShutdownMode, TaskFailure, TaskId, TaskSpec, TaskState,
    WorkUnitBoundary,
};

fn config() -> SchedulerConfig {
    SchedulerConfig::new(16, 8, 4, 4, 1_000).expect("valid scheduler config")
}

fn claim(memory: u64, workers: u16) -> ResourceClaim {
    ResourceClaim::new(memory, workers, workers, true, Vec::new()).expect("valid claim")
}

fn task(id: u64, priority: CpuPriority, memory: u64) -> TaskSpec {
    TaskSpec::new(
        TaskId::new(id).expect("task ID"),
        RequestId::new(id).expect("request ID"),
        PipelineSnapshotIdentity::new([u8::try_from(id).expect("fixture ID fits"); 32]),
        PipelineGeneration::new(id).expect("generation"),
        priority,
        2,
        claim(memory, 1),
    )
    .expect("task")
    .with_publication_target(SchedulerPublicationTarget::new(
        PublicationTargetKind::Cache,
        [u8::try_from(id).expect("fixture ID fits"); 32],
    ))
}

#[test]
fn explicit_priorities_are_weighted_and_interactive_wins_admission_selection() {
    assert_eq!(CpuPriority::InteractivePreview.weight(), 16);
    assert_eq!(CpuPriority::VisibleFullView.weight(), 8);
    assert_eq!(CpuPriority::UserExport.weight(), 4);
    assert_eq!(CpuPriority::VisibleThumbnail.weight(), 4);
    assert_eq!(CpuPriority::PrefetchThumbnail.weight(), 2);
    assert_eq!(CpuPriority::BackgroundAnalysis.weight(), 1);
    assert_eq!(CpuPriority::Maintenance.weight(), 1);

    let now = Instant::now();
    let mut scheduler = CpuScheduler::new(config());
    scheduler
        .submit_at(task(1, CpuPriority::Maintenance, 10), now)
        .expect("maintenance admitted");
    scheduler
        .submit_at(task(2, CpuPriority::InteractivePreview, 10), now)
        .expect("preview admitted");
    assert_eq!(
        scheduler
            .start_next_at(now)
            .expect("task starts")
            .task_id()
            .get(),
        2
    );
}

#[test]
fn dag_passes_and_failures_skip_only_dependent_branches() {
    let now = Instant::now();
    let mut scheduler = CpuScheduler::new(config());
    scheduler
        .submit_at(task(1, CpuPriority::UserExport, 10), now)
        .expect("root");
    scheduler
        .submit_at(
            task(2, CpuPriority::UserExport, 10)
                .with_dependencies(vec![TaskId::new(1).expect("dependency")])
                .expect("dependency"),
            now,
        )
        .expect("dependent");
    scheduler
        .submit_at(task(3, CpuPriority::UserExport, 10), now)
        .expect("independent");

    let root = scheduler.start_next_at(now).expect("root starts").task_id();
    scheduler
        .complete_failure(root, now, TaskFailure::WorkUnitFailed)
        .expect("root fails");
    assert_eq!(
        scheduler
            .start_next_at(now)
            .expect("independent starts")
            .task_id()
            .get(),
        3
    );
    assert_eq!(scheduler.start_next_at(now), None);
    assert_eq!(
        scheduler.state(TaskId::new(2).expect("dependent ID")),
        Some(TaskState::Skipped)
    );
}

#[test]
fn cancellation_and_panic_are_isolated_at_work_unit_boundaries() {
    let now = Instant::now();
    let token = CancellationToken::new();
    let mut cancelled =
        task(1, CpuPriority::InteractivePreview, 10).with_cancellation(Arc::new(token.clone()));
    let mut scheduler = CpuScheduler::new(config());
    scheduler
        .submit_at(cancelled.clone(), now)
        .expect("admitted");
    token.cancel();
    assert_eq!(scheduler.start_next_at(now), None);
    assert_eq!(
        scheduler.state(cancelled.task_id()),
        Some(TaskState::Cancelled)
    );

    cancelled = task(2, CpuPriority::InteractivePreview, 10);
    scheduler
        .submit_at(cancelled.clone(), now)
        .expect("admitted");
    let id = scheduler.start_next_at(now).expect("starts").task_id();
    assert_eq!(
        scheduler.run_task(id, now, |_| -> Result<(), ()> { panic!("isolated") }),
        Ok(TaskState::Failed)
    );
    assert_eq!(scheduler.metrics().panics(), 1);
    assert!(
        !scheduler
            .receipts()
            .next()
            .expect("receipt")
            .publication_allowed()
    );
}

#[test]
fn memory_and_worker_limits_are_hard_and_lease_mismatch_fails() {
    let now = Instant::now();
    let mut scheduler = CpuScheduler::new(config());
    scheduler
        .submit_at(task(1, CpuPriority::UserExport, 800), now)
        .expect("admitted");
    assert_eq!(
        scheduler.submit_at(task(2, CpuPriority::UserExport, 200), now),
        Err(SchedulerError::MemoryReservationUnavailable)
    );
    let id = scheduler.start_next_at(now).expect("starts").task_id();
    assert_eq!(
        scheduler.confirm_leases(id, 799, now),
        Err(SchedulerError::LeaseMismatch {
            planned_bytes: 800,
            actual_bytes: 799
        })
    );
    assert_eq!(scheduler.state(id), Some(TaskState::Failed));
    assert_eq!(scheduler.snapshot().active_memory_bytes, 0);
}

#[test]
fn shutdown_rejects_new_admission_and_requests_active_cancellation() {
    let now = Instant::now();
    let mut scheduler = CpuScheduler::new(config());
    scheduler
        .submit_at(task(1, CpuPriority::BackgroundAnalysis, 10), now)
        .expect("admitted");
    let id = scheduler.start_next_at(now).expect("starts").task_id();
    let report = scheduler.shutdown(ShutdownMode::AbortAll, now);
    assert_eq!(report.cancellation_requested_active(), 1);
    assert_eq!(
        scheduler.work_unit_boundary(id).expect("boundary"),
        WorkUnitBoundary::Shutdown
    );
    assert_eq!(
        scheduler.submit_at(task(2, CpuPriority::InteractivePreview, 10), now),
        Err(SchedulerError::PostShutdown)
    );
    assert!(scheduler.is_shutting_down());
}

#[test]
fn receipts_are_bounded_and_wait_times_are_monotonic() {
    let config = config().with_receipt_limit(2);
    let now = Instant::now();
    let later = now + Duration::from_secs(3);
    let mut scheduler = CpuScheduler::new(config);
    for id in 1..=3 {
        scheduler
            .submit_at(task(id, CpuPriority::Maintenance, 10), now)
            .expect("admitted");
        let running = scheduler.start_next_at(later).expect("starts");
        scheduler
            .complete_success(running.task_id(), later)
            .expect("complete");
    }
    assert_eq!(scheduler.receipts().len(), 2);
    assert!(
        scheduler
            .receipts()
            .all(|receipt| receipt.queue_wait() >= Duration::from_secs(3))
    );
}
