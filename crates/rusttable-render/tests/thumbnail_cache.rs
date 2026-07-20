use std::fs;

use rusttable_core::{AssetId, ContentHash, EditId, PhotoId, Revision};
use rusttable_image::{CancellationToken, DecodedImage, ImageDimensions, Orientation};
use rusttable_render::{
    CacheChangeEvent, CacheInvalidationReport, CacheLifecycle, CacheLimits, CacheStore, CacheTime,
    MipmapLevel, PrefetchCompletion, PrefetchPriority, PrefetchRequest, PrefetchScheduler,
    ThumbnailGenerator, ThumbnailKey, ThumbnailProvenance, ThumbnailRequest, ThumbnailSize,
};

fn key(edit_revision: u64, provenance: ThumbnailProvenance) -> ThumbnailKey {
    let size = ThumbnailSize::fit(128, 96).expect("valid size");
    let request = ThumbnailRequest::new(MipmapLevel::new(2).expect("valid level"), size)
        .with_orientation(Orientation::Rotate90)
        .with_provenance(provenance);
    ThumbnailKey::new(
        ContentHash::Sha256([7; 32]),
        PhotoId::new(1).expect("photo"),
        AssetId::new(2).expect("asset"),
        EditId::new(3).expect("edit"),
        Revision::from_u64(4),
        Revision::from_u64(edit_revision),
        10,
        11,
        [12; 32],
        13,
        [14; 32],
        request,
    )
}

fn image() -> DecodedImage {
    DecodedImage::new(
        ImageDimensions::new(2, 2).expect("dimensions"),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
    )
    .expect("image")
}

fn limits() -> CacheLimits {
    CacheLimits::new(16 * 1024, 8 * 1024).expect("limits")
}

#[test]
fn equal_inputs_have_equal_canonical_keys_and_digest() {
    let left = key(5, ThumbnailProvenance::PipelineRender);
    let right = key(5, ThumbnailProvenance::PipelineRender);
    assert_eq!(left.canonical_bytes(), right.canonical_bytes());
    assert_eq!(left.digest(), right.digest());
    assert_ne!(
        left.digest(),
        key(6, ThumbnailProvenance::PipelineRender).digest()
    );
    assert_ne!(
        left.digest(),
        key(5, ThumbnailProvenance::EmbeddedPreview).digest()
    );
}

#[test]
fn store_round_trips_header_dimensions_pixels_and_key() {
    let directory = tempfile_dir("round-trip");
    let (mut store, report) = CacheStore::open(&directory, limits()).expect("open");
    assert_eq!(report.valid_entries, 0);
    let cache_key = key(5, ThumbnailProvenance::RawFallback);
    store
        .put(cache_key, &image(), CacheTime::from_seconds(20))
        .expect("put");
    let entry = store
        .get(cache_key, CacheTime::from_seconds(21))
        .expect("get")
        .expect("entry");
    assert_eq!(entry.key(), cache_key);
    assert_eq!(entry.created_at(), CacheTime::from_seconds(20));
    assert_eq!(
        entry.image().dimensions(),
        ImageDimensions::new(2, 2).expect("dimensions")
    );
    assert_eq!(entry.image().pixels(), image().pixels());
    assert_eq!(fs::read_dir(&directory).expect("directory").count(), 1);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn corrupt_payload_is_rejected_and_removed_on_reopen() {
    let directory = tempfile_dir("corrupt");
    let cache_key = key(5, ThumbnailProvenance::EmbeddedPreview);
    {
        let (mut store, _) = CacheStore::open(&directory, limits()).expect("open");
        store
            .put(cache_key, &image(), CacheTime::from_seconds(20))
            .expect("put");
    }
    let path = fs::read_dir(&directory)
        .expect("directory")
        .next()
        .expect("entry")
        .expect("entry")
        .path();
    let mut bytes = fs::read(&path).expect("bytes");
    *bytes.last_mut().expect("payload") ^= 0xff;
    fs::write(&path, bytes).expect("corrupt");
    let (store, report) = CacheStore::open(&directory, limits()).expect("reopen");
    assert!(store.is_empty());
    assert_eq!(report.removed_corrupt, 1);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn interrupted_staged_write_is_cleaned_without_becoming_an_entry() {
    let directory = tempfile_dir("temporary");
    fs::write(directory.join(".orphan.tmp"), b"partial").expect("temporary");
    let (store, report) = CacheStore::open(&directory, limits()).expect("open");
    assert!(store.is_empty());
    assert_eq!(report.removed_temporary, 1);
    assert!(!directory.join(".orphan.tmp").exists());
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn generator_is_deterministic_bounded_and_orientation_aware() {
    let source = DecodedImage::new(
        ImageDimensions::new(4, 2).expect("dimensions"),
        (0..32).collect(),
    )
    .expect("image");
    let request =
        ThumbnailRequest::new(MipmapLevel::zero(), ThumbnailSize::fit(2, 2).expect("size"))
            .with_orientation(Orientation::Rotate90);
    let cancellation = CancellationToken::new();
    let left =
        ThumbnailGenerator::generate(&source, request, 1024, &cancellation).expect("thumbnail");
    let right =
        ThumbnailGenerator::generate(&source, request, 1024, &cancellation).expect("thumbnail");
    assert_eq!(left, right);
    assert_eq!(
        left.dimensions(),
        ImageDimensions::new(1, 2).expect("oriented fit")
    );
    assert_eq!(left.pixels()[3], 13);
    assert!(ThumbnailGenerator::generate(&source, request, 1, &cancellation).is_err());
}

#[test]
fn generator_honors_cancellation_before_publishing() {
    let source = image();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let request = ThumbnailRequest::new(
        MipmapLevel::zero(),
        ThumbnailSize::exact(2, 2).expect("size"),
    );
    assert!(matches!(
        ThumbnailGenerator::generate(&source, request, 1024, &cancellation),
        Err(rusttable_render::ThumbnailError::Cancelled)
    ));
}

#[test]
fn lifecycle_rebuilds_keys_and_invalidates_only_the_affected_edit() {
    let directory = tempfile_dir("lifecycle");
    let first = key(5, ThumbnailProvenance::PipelineRender);
    let second = key(6, ThumbnailProvenance::PipelineRender);
    {
        let (mut store, _) = CacheStore::open(&directory, limits()).expect("open");
        store
            .put(first, &image(), CacheTime::from_seconds(20))
            .expect("put");
        store
            .put(second, &image(), CacheTime::from_seconds(20))
            .expect("put");
    }
    let (store, report) = CacheStore::open(&directory, limits()).expect("reopen");
    assert_eq!(report.valid_entries, 2);
    let mut lifecycle = CacheLifecycle::new(store);
    let invalidation = lifecycle
        .apply(CacheChangeEvent::EditChanged {
            photo_id: first.photo_id(),
            edit_revision: first.edit_revision(),
        })
        .expect("invalidate");
    assert_eq!(
        invalidation,
        CacheInvalidationReport {
            matched: 1,
            removed: 1,
            protected: 0
        }
    );
    assert!(lifecycle.store().keys().all(|cache_key| cache_key != first));
    assert!(
        lifecycle
            .store()
            .keys()
            .any(|cache_key| cache_key == second)
    );
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn pinned_entries_are_not_evicted_or_invalidated() {
    let directory = tempfile_dir("pin");
    let first = key(5, ThumbnailProvenance::PipelineRender);
    let second = key(6, ThumbnailProvenance::PipelineRender);
    let limits = CacheLimits::new(400, 400).expect("limits");
    let (mut store, _) = CacheStore::open(&directory, limits).expect("open");
    store
        .put(first, &image(), CacheTime::from_seconds(20))
        .expect("put");
    let pin = store.pin(first).expect("pin").expect("entry");
    assert!(matches!(
        store.put(second, &image(), CacheTime::from_seconds(21)),
        Err(rusttable_render::CacheError::CacheFull { .. })
    ));
    assert!(matches!(
        store.invalidate(first),
        Err(rusttable_render::CacheError::Protected)
    ));
    store.unpin(pin).expect("unpin");
    assert!(store.invalidate(first).expect("invalidate"));
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn prefetch_is_bounded_prioritized_and_rejects_stale_results() {
    let first = key(5, ThumbnailProvenance::PipelineRender);
    let second = key(6, ThumbnailProvenance::PipelineRender);
    let mut scheduler = PrefetchScheduler::new(2, 1).expect("scheduler");
    let (maintenance, _) = scheduler
        .submit(PrefetchRequest {
            key: first,
            priority: PrefetchPriority::Maintenance,
        })
        .expect("submit");
    let (_, cancellation) = scheduler
        .submit(PrefetchRequest {
            key: second,
            priority: PrefetchPriority::Visible,
        })
        .expect("submit");
    assert!(
        scheduler
            .submit(PrefetchRequest {
                key: first,
                priority: PrefetchPriority::NearVisible
            })
            .is_err()
    );
    let visible = scheduler.next().expect("visible");
    assert_eq!(visible.request().priority, PrefetchPriority::Visible);
    scheduler.invalidate_generation().expect("generation");
    assert_eq!(scheduler.complete(visible), PrefetchCompletion::Stale);
    assert!(scheduler.cancel(maintenance));
    cancellation.cancel();
    assert!(scheduler.next().is_none());
}

fn tempfile_dir(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("rusttable-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("temporary directory");
    path
}
