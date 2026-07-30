use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use rusttable_color::ColorEncoding;
use rusttable_image::{ImageDimensions, PixelFormat, Roi};
use rusttable_pixelpipe::{
    Cache, CacheConfig, CacheError, CacheKey, CachePrecision, CacheQuality, CancellationDeadline,
    CancellationReason, CancellationScope, CancellationStage, CancellationToken, ColorIdentity,
    CreationCost, GenerationClock, ImplementationIdentity, NodeBoundary, OutputIdentity,
    PipelineGeneration, PipelinePurpose, PipelineSnapshotIdentity, PublicationError,
    PublicationGate, PublicationGeneration, PublicationIdentity, PublicationTarget, RequestId,
    SourceIdentity, TargetIdentity, ValueDescriptor, ValueKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestValue(u8);

impl rusttable_pixelpipe::CacheValue for TestValue {
    fn descriptor(&self) -> ValueDescriptor {
        ValueDescriptor::new(ValueKind::Analysis, 1, 0, CreationCost::Cheap, true)
    }

    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

fn key(generation: u64, source: u8) -> CacheKey {
    let dimensions = ImageDimensions::new(2, 2).expect("dimensions");
    let implementation = ImplementationIdentity::new("rusttable.test", 1, "test").expect("ID");
    CacheKey::builder()
        .source(SourceIdentity::new([source; 32]))
        .source_descriptor([source])
        .snapshot(PipelineSnapshotIdentity::new([source; 32]))
        .generation(PipelineGeneration::new(generation).expect("generation"))
        .purpose(PipelinePurpose::Preview)
        .quality(CacheQuality::Normal)
        .precision(CachePrecision::F32)
        .node(NodeBoundary::whole(implementation))
        .output(OutputIdentity::new(
            dimensions,
            Roi::full(dimensions),
            PixelFormat::rgba8(),
            ColorIdentity::new(ColorEncoding::SrgbD65, 1).expect("color"),
            [source; 32],
        ))
        .parameters(1, [source])
        .build()
        .expect("complete key")
}

#[test]
fn cancellation_is_generation_scoped_and_first_reason_wins() {
    let generation = PipelineGeneration::new(7).expect("generation");
    let root = CancellationScope::root(generation);
    let child = root.child(CancellationStage::Tile);
    let nested = child.child(CancellationStage::Node);
    root.cancel(CancellationReason::EditChanged);
    child.cancel(CancellationReason::DeviceLost);

    assert_eq!(root.token().reason(), Some(CancellationReason::EditChanged));
    assert_eq!(
        child.token().reason(),
        Some(CancellationReason::EditChanged)
    );
    assert_eq!(
        nested.token().reason(),
        Some(CancellationReason::EditChanged)
    );
    let error = child.check().expect_err("cancelled child");
    assert_eq!(error.reason(), CancellationReason::EditChanged);
    assert_eq!(error.stage(), Some(CancellationStage::Tile));
    assert_eq!(child.generation(), generation);
}

#[test]
fn deadlines_are_typed_and_cleanup_hooks_run_once() {
    let generation = PipelineGeneration::new(8).expect("generation");
    let scope = CancellationScope::root(generation)
        .with_deadline(CancellationDeadline::after(Duration::ZERO));
    let cleaned = Arc::new(AtomicUsize::new(0));
    let observed = Arc::new(AtomicBool::new(false));
    let cleaned_hook = cleaned.clone();
    let observed_hook = observed.clone();
    let _registration = scope.register_cleanup(move |reason| {
        cleaned_hook.fetch_add(1, Ordering::AcqRel);
        observed_hook.store(
            reason == CancellationReason::DeadlineExceeded,
            Ordering::Release,
        );
    });

    let error = scope.check().expect_err("expired deadline");
    assert_eq!(error.reason(), CancellationReason::DeadlineExceeded);
    assert_eq!(cleaned.load(Ordering::Acquire), 1);
    assert!(observed.load(Ordering::Acquire));
    scope.cancel(CancellationReason::Shutdown);
    assert_eq!(cleaned.load(Ordering::Acquire), 1);
}

#[test]
fn generation_clock_rejects_overflow_without_wrapping() {
    let clock = GenerationClock::new(PipelineGeneration::new(u64::MAX).expect("generation"));
    assert_eq!(
        clock.next(),
        Err(rusttable_pixelpipe::GenerationClockError::Overflow)
    );
    assert_eq!(clock.current().get(), u64::MAX);
}

#[test]
fn publication_permit_checks_all_identity_and_is_one_shot() {
    let generation = PipelineGeneration::new(9).expect("generation");
    let token = CancellationToken::for_generation(generation);
    let identity = PublicationIdentity::new(
        RequestId::new(1).expect("request"),
        generation,
        PublicationTarget::Cache(TargetIdentity::consumer("cache")),
        SourceIdentity::new([1; 32]),
        PipelineSnapshotIdentity::new([2; 32]),
        PublicationGeneration::new(9),
    );
    let gate = PublicationGate::new(identity, token.clone());
    let mut permit = gate.authorize(identity).expect("permit");
    let cleaned = Arc::new(AtomicUsize::new(0));
    let cleaned_hook = cleaned.clone();
    permit.on_drop(move || {
        cleaned_hook.fetch_add(1, Ordering::AcqRel);
    });
    assert_eq!(permit.commit(identity, || 42).expect("publish"), 42);
    assert_eq!(cleaned.load(Ordering::Acquire), 0);
    assert!(matches!(
        gate.authorize(identity),
        Err(PublicationError::AlreadyIssued)
    ));

    let cancelled_gate = PublicationGate::new(identity, token.clone());
    token.cancel_with_reason(CancellationReason::SupersededGeneration(generation));
    assert!(matches!(
        cancelled_gate.authorize(identity),
        Err(PublicationError::Cancelled(_))
    ));
}

#[test]
fn shared_build_cancels_only_when_every_consumer_leaves() {
    let cache = Arc::new(Cache::new(CacheConfig::new(64)));
    let first = CancellationToken::new();
    let second = CancellationToken::new();
    let started = Arc::new(AtomicBool::new(false));
    let builds = Arc::new(AtomicUsize::new(0));
    let worker_cache = cache.clone();
    let worker_started = started.clone();
    let worker_builds = builds.clone();
    let worker_first = first.clone();
    let worker = thread::spawn(move || {
        worker_cache.get_or_build::<TestValue, _>(key(10, 10), &worker_first, |shared| {
            worker_started.store(true, Ordering::Release);
            worker_builds.fetch_add(1, Ordering::AcqRel);
            while !shared.is_cancelled() {
                thread::sleep(Duration::from_millis(1));
            }
            Err::<TestValue, _>(CacheError::Cancelled)
        })
    });
    while !started.load(Ordering::Acquire) {
        thread::yield_now();
    }
    let waiter_cache = cache.clone();
    let waiter_second = second.clone();
    let waiter = thread::spawn(move || {
        waiter_cache.get_or_build::<TestValue, _>(key(10, 10), &waiter_second, |_| Ok(TestValue(2)))
    });
    first.cancel();
    thread::sleep(Duration::from_millis(5));
    assert!(!second.is_cancelled());
    second.cancel();
    assert!(matches!(
        worker.join().expect("owner"),
        Err(CacheError::Cancelled)
    ));
    assert!(matches!(
        waiter.join().expect("waiter"),
        Err(CacheError::Cancelled)
    ));
    assert_eq!(builds.load(Ordering::Acquire), 1);
}
