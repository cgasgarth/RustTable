use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

use rusttable_color::ColorEncoding;
use rusttable_image::{ImageDimensions, PixelFormat, Roi};
use rusttable_pixelpipe::{
    Cache, CacheConfig, CacheError, CacheKey, CachePrecision, CacheQuality, CacheScope,
    CancellationToken, ColorIdentity, CreationCost, ImplementationIdentity, NodeBoundary,
    OutputIdentity, PipelineGeneration, PipelinePurpose, PipelineSnapshotIdentity, SourceIdentity,
    ValueDescriptor, ValueKind,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct TestValue(u64);

impl rusttable_pixelpipe::CacheValue for TestValue {
    fn descriptor(&self) -> ValueDescriptor {
        ValueDescriptor::new(ValueKind::Analysis, self.0, 0, CreationCost::Normal, true)
    }

    fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

fn key(seed: u8) -> CacheKey {
    let dimensions = ImageDimensions::new(4, 4).expect("dimensions");
    let identity = ImplementationIdentity::new("rusttable.test", 1, "build").expect("identity");
    CacheKey::builder()
        .source(SourceIdentity::new([seed; 32]))
        .source_descriptor([seed, 1, 2])
        .snapshot(PipelineSnapshotIdentity::new([seed; 32]))
        .generation(PipelineGeneration::new(u64::from(seed.max(1))).expect("generation"))
        .purpose(PipelinePurpose::Preview)
        .quality(CacheQuality::Normal)
        .precision(CachePrecision::F32)
        .node(NodeBoundary::whole(identity))
        .output(OutputIdentity::new(
            dimensions,
            Roi::full(dimensions),
            PixelFormat::rgba8(),
            ColorIdentity::new(ColorEncoding::SrgbD65, 1).expect("color"),
            [seed; 32],
        ))
        .parameters(1, [seed, 9])
        .build()
        .expect("complete key")
}

#[test]
fn canonical_key_equality_is_authoritative_and_diagnostic_hash_is_stable() {
    let first = key(1);
    let same = key(1);
    let changed_source = key(2);
    assert_eq!(first, same);
    assert_eq!(first.canonical_bytes(), same.canonical_bytes());
    assert_eq!(first.diagnostic_sha256(), same.diagnostic_sha256());
    assert_ne!(first, changed_source);
    assert_ne!(
        first.diagnostic_sha256(),
        changed_source.diagnostic_sha256()
    );
}

#[test]
fn cache_promotes_hits_and_keeps_leases_pinned_until_drop() {
    let cache = Cache::new(CacheConfig::new(32));
    let token = CancellationToken::new();
    let first = cache
        .get_or_build::<TestValue, _>(key(1), &token, |_| Ok(TestValue(8)))
        .expect("publish");
    assert!(first.is_cached());
    assert_eq!(cache.metrics().pinned_entries, 1);
    drop(first);
    let second = cache.lookup::<TestValue>(&key(1)).expect("lookup");
    assert!(second.is_some());
    assert_eq!(cache.metrics().protected_entries, 1);
    drop(second);
}

#[test]
fn oversize_values_are_direct_leases_and_never_resident() {
    let cache = Cache::new(CacheConfig::new(4));
    let token = CancellationToken::new();
    let lease = cache
        .get_or_build::<TestValue, _>(key(1), &token, |_| Ok(TestValue(8)))
        .expect("direct lease");
    assert!(!lease.is_cached());
    assert_eq!(cache.metrics().entries, 0);
}

#[test]
fn invalidation_removes_matches_and_rejects_stale_publication() {
    let cache = Arc::new(Cache::new(CacheConfig::new(64)));
    let token = CancellationToken::new();
    let lease = cache
        .get_or_build::<TestValue, _>(key(1), &token, |_| Ok(TestValue(1)))
        .expect("publish");
    drop(lease);
    let source = SourceIdentity::new([1; 32]);
    assert_eq!(
        cache
            .invalidate(CacheScope::Source(source))
            .expect("invalidate")
            .removed(),
        1
    );
    assert!(cache.lookup::<TestValue>(&key(1)).expect("miss").is_none());

    let started = Arc::new(AtomicUsize::new(0));
    let build_started = started.clone();
    let build_cache = cache.clone();
    let handle = thread::spawn(move || {
        let token = CancellationToken::new();
        build_cache.get_or_build::<TestValue, _>(key(3), &token, |_| {
            build_started.store(1, Ordering::Release);
            thread::sleep(Duration::from_millis(20));
            Ok(TestValue(3))
        })
    });
    while started.load(Ordering::Acquire) == 0 {
        thread::yield_now();
    }
    cache
        .invalidate(CacheScope::Source(SourceIdentity::new([3; 32])))
        .expect("invalidate in flight");
    assert!(matches!(
        handle.join().expect("worker"),
        Err(CacheError::StalePublication)
    ));
}

#[test]
fn one_single_flight_build_is_shared_and_waiter_cancellation_is_independent() {
    let cache = Arc::new(Cache::new(CacheConfig::new(64)));
    let builds = Arc::new(AtomicUsize::new(0));
    let owner_cache = cache.clone();
    let owner_builds = builds.clone();
    let owner = thread::spawn(move || {
        let token = CancellationToken::new();
        owner_cache.get_or_build::<TestValue, _>(key(4), &token, |_| {
            owner_builds.fetch_add(1, Ordering::AcqRel);
            thread::sleep(Duration::from_millis(25));
            Ok(TestValue(4))
        })
    });
    while builds.load(Ordering::Acquire) == 0 {
        thread::yield_now();
    }
    let waiter_cache = cache.clone();
    let waiter = thread::spawn(move || {
        let token = CancellationToken::new();
        token.cancel();
        waiter_cache.get_or_build::<TestValue, _>(key(4), &token, |_| Ok(TestValue(99)))
    });
    assert!(matches!(
        waiter.join().expect("waiter"),
        Err(CacheError::Cancelled)
    ));
    assert_eq!(owner.join().expect("owner").expect("build").value().0, 4);
    assert_eq!(builds.load(Ordering::Acquire), 1);
}
