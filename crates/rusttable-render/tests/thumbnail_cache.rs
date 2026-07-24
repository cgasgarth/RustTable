use std::fs;
use std::sync::{Arc, Barrier};

use rusttable_core::{
    AssetId, ContentHash, Edit, EditId, FiniteF64, Operation, OperationId, OperationKey,
    ParameterName, ParameterValue, PhotoId, Revision,
};
use rusttable_image::{
    CancellationToken, ColorEncoding, DecodedImage, ImageDimensions, Orientation,
};
use rusttable_render::{
    CacheChangeEvent, CacheInvalidationReport, CacheLifecycle, CacheLimits, CacheResolutionSource,
    CacheStore, CacheTime, MipmapLevel, PrefetchCompletion, PrefetchPriority, PrefetchRequest,
    PrefetchScheduler, ThumbnailGenerator, ThumbnailKey, ThumbnailMemoryCache, ThumbnailProvenance,
    ThumbnailRequest, ThumbnailSize, thumbnail_edit_operations_identity,
};

fn key(edit_revision: u64, provenance: ThumbnailProvenance) -> ThumbnailKey {
    key_for_edit(EditId::new(3).expect("edit"), edit_revision, provenance)
}

fn key_for_edit(
    edit_id: EditId,
    edit_revision: u64,
    provenance: ThumbnailProvenance,
) -> ThumbnailKey {
    key_for_edit_identity(edit_id, edit_revision, [15; 32], provenance)
}

fn key_for_edit_identity(
    edit_id: EditId,
    edit_revision: u64,
    edit_operations_identity: [u8; 32],
    provenance: ThumbnailProvenance,
) -> ThumbnailKey {
    let size = ThumbnailSize::fit(128, 96).expect("valid size");
    let request = ThumbnailRequest::new(MipmapLevel::new(2).expect("valid level"), size)
        .with_orientation(Orientation::Rotate90)
        .with_provenance(provenance);
    ThumbnailKey::new(
        ContentHash::Sha256([7; 32]),
        PhotoId::new(1).expect("photo"),
        AssetId::new(2).expect("asset"),
        edit_id,
        Revision::from_u64(4),
        Revision::from_u64(edit_revision),
        edit_operations_identity,
        10,
        11,
        [12; 32],
        13,
        [14; 32],
        request,
    )
}

#[test]
fn exact_edit_operation_content_changes_the_key_and_digest() {
    let left_edit = edit_with_scalar(1.0);
    let right_edit = edit_with_scalar(2.0);
    let left_identity = thumbnail_edit_operations_identity(&left_edit);
    let right_identity = thumbnail_edit_operations_identity(&right_edit);
    let left = key_for_edit_identity(
        left_edit.id(),
        left_edit.revision().get(),
        left_identity,
        ThumbnailProvenance::PipelineRender,
    );
    let right = key_for_edit_identity(
        right_edit.id(),
        right_edit.revision().get(),
        right_identity,
        ThumbnailProvenance::PipelineRender,
    );

    assert_ne!(left_identity, right_identity);
    assert_ne!(left.canonical_bytes(), right.canonical_bytes());
    assert_ne!(left.digest(), right.digest());
}

fn edit_with_scalar(value: f64) -> Edit {
    Edit::from_parts(
        EditId::new(3).expect("edit"),
        PhotoId::new(1).expect("photo"),
        Revision::from_u64(4),
        Revision::from_u64(5),
        [Operation::new(
            OperationId::new(8).expect("operation"),
            OperationKey::new("rusttable.exposure").expect("operation key"),
            true,
            [(
                ParameterName::new("exposure").expect("parameter"),
                ParameterValue::Scalar(FiniteF64::new(value).expect("finite")),
            )],
        )
        .expect("operation")],
    )
    .expect("edit")
}

fn image() -> DecodedImage {
    DecodedImage::new_with_color_encoding(
        ImageDimensions::new(2, 2).expect("dimensions"),
        vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        ColorEncoding::Srgb,
    )
    .expect("image")
}

fn solid_image(width: u32, height: u32, value: u8) -> DecodedImage {
    let dimensions = ImageDimensions::new(width, height).expect("dimensions");
    let bytes = dimensions.decoded_byte_count().expect("decoded bytes");
    DecodedImage::new_with_color_encoding(
        dimensions,
        vec![value; usize::try_from(bytes).expect("image length")],
        ColorEncoding::Srgb,
    )
    .expect("image")
}

fn entry_file_bytes(directory: &std::path::Path, key: ThumbnailKey) -> u64 {
    fs::metadata(directory.join(format!("{}.mip", key.digest_hex())))
        .expect("entry metadata")
        .len()
}

fn mip_file_count(directory: &std::path::Path) -> usize {
    fs::read_dir(directory)
        .expect("cache directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "mip")
        })
        .count()
}

fn mip_file_bytes(directory: &std::path::Path) -> u64 {
    fs::read_dir(directory)
        .expect("cache directory")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "mip")
        })
        .map(|entry| entry.metadata().expect("entry metadata").len())
        .sum()
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
fn reconciliation_is_header_only_and_exact_get_removes_corrupt_payload() {
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
    let (mut store, report) = CacheStore::open(&directory, limits()).expect("reopen");
    assert_eq!(store.len(), 1);
    assert_eq!(report.valid_entries, 1);
    assert_eq!(report.removed_corrupt, 0);
    assert!(
        store
            .get(cache_key, CacheTime::from_seconds(21))
            .expect("validated get")
            .is_none()
    );
    assert!(store.is_empty());
    assert!(!path.exists());
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn same_root_reuses_initialized_index_until_the_last_store_is_dropped() {
    let directory = tempfile_dir("shared-root-reconciliation");
    let first_key = key(5, ThumbnailProvenance::PipelineRender);
    let second_key = key(6, ThumbnailProvenance::PipelineRender);
    let (mut first, _) = CacheStore::open(&directory, limits()).expect("open first");
    first
        .put(first_key, &image(), CacheTime::from_seconds(20))
        .expect("put first");
    let lifetime_anchor = first.clone();
    let unrelated = directory.join("unrelated.mip");
    fs::write(&unrelated, b"not a cache header").expect("corrupt unrelated entry");

    let (mut second, report) = CacheStore::open(&directory, limits()).expect("open second");
    assert_eq!(report.valid_entries, 1);
    assert_eq!(report.removed_corrupt, 0);
    assert!(unrelated.exists());
    second
        .put(second_key, &image(), CacheTime::from_seconds(21))
        .expect("put second");
    assert!(
        first
            .get(first_key, CacheTime::from_seconds(22))
            .expect("get first")
            .is_some()
    );
    assert!(unrelated.exists());

    drop(first);
    drop(second);
    let (same_lifetime, report) =
        CacheStore::open(&directory, limits()).expect("open while anchor remains");
    assert_eq!(report.valid_entries, 2);
    assert_eq!(report.removed_corrupt, 0);
    assert!(unrelated.exists());
    drop(same_lifetime);
    drop(lifetime_anchor);

    let (reconciled, report) =
        CacheStore::open(&directory, limits()).expect("open new coordinator");
    assert_eq!(reconciled.len(), 2);
    assert_eq!(report.valid_entries, 2);
    assert_eq!(report.removed_corrupt, 1);
    assert!(!unrelated.exists());
    drop(reconciled);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn same_root_rejects_different_limits_while_the_coordinator_is_alive() {
    let directory = tempfile_dir("shared-root-limits");
    let first_limits = limits();
    let requested_limits = CacheLimits::new(32 * 1024, 8 * 1024).expect("other limits");
    let (store, _) = CacheStore::open(&directory, first_limits).expect("open first");

    let error = CacheStore::open(&directory, requested_limits).expect_err("limits mismatch");
    assert_eq!(
        error,
        rusttable_render::CacheError::LimitsMismatch {
            existing: first_limits,
            requested: requested_limits,
        }
    );

    drop(store);
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
fn recent_owned_staged_write_is_not_deleted_by_reconciliation() {
    let directory = tempfile_dir("active-temporary");
    let cache_key = key(5, ThumbnailProvenance::PipelineRender);
    let path = directory.join(format!(
        ".{}.{}.{}.tmp",
        cache_key.digest_hex(),
        std::process::id(),
        999
    ));
    fs::write(&path, b"in flight").expect("temporary");
    let (store, report) = CacheStore::open(&directory, limits()).expect("open");
    assert!(store.is_empty());
    assert_eq!(report.removed_temporary, 0);
    assert!(path.exists());
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn oversized_entries_are_bounded_and_removed_on_open_and_get() {
    let open_directory = tempfile_dir("oversized-open");
    let bounded_limits = CacheLimits::new(2_048, 1_024).expect("limits");
    let oversized = open_directory.join("oversized.mip");
    fs::write(&oversized, vec![0; 1_025]).expect("oversized entry");
    let (store, report) = CacheStore::open(&open_directory, bounded_limits).expect("open");
    assert!(store.is_empty());
    assert_eq!(report.removed_corrupt, 1);
    assert!(!oversized.exists());
    fs::remove_dir_all(open_directory).expect("cleanup open");

    let get_directory = tempfile_dir("oversized-get");
    let cache_key = key(5, ThumbnailProvenance::PipelineRender);
    let (mut store, _) = CacheStore::open(&get_directory, bounded_limits).expect("open");
    store
        .put(cache_key, &image(), CacheTime::from_seconds(20))
        .expect("put");
    let path = fs::read_dir(&get_directory)
        .expect("directory")
        .next()
        .expect("entry")
        .expect("entry")
        .path();
    std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("open entry")
        .set_len(1_025)
        .expect("grow entry");
    assert!(
        store
            .get(cache_key, CacheTime::from_seconds(21))
            .expect("bounded get")
            .is_none()
    );
    assert!(!path.exists());
    fs::remove_dir_all(get_directory).expect("cleanup get");
}

#[test]
fn durable_cache_rejects_noncanonical_image_descriptors() {
    let directory = tempfile_dir("descriptor-invariant");
    let cache_key = key(5, ThumbnailProvenance::PipelineRender);
    let (mut store, _) = CacheStore::open(&directory, limits()).expect("open");
    let dimensions = ImageDimensions::new(2, 2).expect("dimensions");
    let linear =
        DecodedImage::new_with_color_encoding(dimensions, vec![1; 16], ColorEncoding::LinearSrgb)
            .expect("linear image");
    let oriented = DecodedImage::new_with_source_orientation(
        dimensions,
        vec![1; 16],
        ColorEncoding::Srgb,
        Orientation::Rotate90,
    )
    .expect("oriented image");
    assert!(matches!(
        store.put(cache_key, &linear, CacheTime::from_seconds(20)),
        Err(rusttable_render::CacheError::Image)
    ));
    assert!(matches!(
        store.put(cache_key, &oriented, CacheTime::from_seconds(20)),
        Err(rusttable_render::CacheError::Image)
    ));
    assert!(store.is_empty());
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn larger_same_key_replacement_evicts_other_lru_entries_without_exceeding_budget() {
    let sizing_directory = tempfile_dir("replacement-sizing");
    let target = key(5, ThumbnailProvenance::PipelineRender);
    let other = key(6, ThumbnailProvenance::PipelineRender);
    let small = solid_image(2, 2, 1);
    let large = solid_image(4, 4, 2);
    let (mut sizing_store, _) = CacheStore::open(
        &sizing_directory,
        CacheLimits::new(64 * 1024, 32 * 1024).unwrap(),
    )
    .expect("open sizing cache");
    sizing_store
        .put(target, &small, CacheTime::from_seconds(20))
        .expect("size target");
    sizing_store
        .put(other, &small, CacheTime::from_seconds(21))
        .expect("size other");
    let target_bytes = entry_file_bytes(&sizing_directory, target);
    let other_bytes = entry_file_bytes(&sizing_directory, other);
    let replacement_bytes = target_bytes
        .checked_sub(u64::try_from(small.pixels().len()).expect("small length"))
        .and_then(|header| {
            header.checked_add(u64::try_from(large.pixels().len()).expect("large length"))
        })
        .expect("replacement size");
    drop(sizing_store);
    fs::remove_dir_all(sizing_directory).expect("cleanup sizing");

    let directory = tempfile_dir("replacement-budget");
    let budget = target_bytes + other_bytes;
    let limits = CacheLimits::new(budget, replacement_bytes).expect("replacement limits");
    let (mut store, _) = CacheStore::open(&directory, limits).expect("open");
    store
        .put(target, &small, CacheTime::from_seconds(20))
        .expect("put target");
    store
        .put(other, &small, CacheTime::from_seconds(21))
        .expect("put other");

    store
        .put(target, &large, CacheTime::from_seconds(22))
        .expect("replace target");

    assert!(store.keys().any(|key| key == target));
    assert!(store.keys().all(|key| key != other));
    assert_eq!(entry_file_bytes(&directory, target), replacement_bytes);
    assert!(mip_file_bytes(&directory) <= budget);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn concurrent_store_instances_share_one_root_budget_and_index() {
    let sizing_directory = tempfile_dir("shared-root-sizing");
    let first = key(5, ThumbnailProvenance::PipelineRender);
    let second = key(6, ThumbnailProvenance::PipelineRender);
    let value = solid_image(2, 2, 3);
    let (mut sizing_store, _) = CacheStore::open(
        &sizing_directory,
        CacheLimits::new(64 * 1024, 32 * 1024).unwrap(),
    )
    .expect("open sizing cache");
    sizing_store
        .put(first, &value, CacheTime::from_seconds(20))
        .expect("size entry");
    let entry_bytes = entry_file_bytes(&sizing_directory, first);
    drop(sizing_store);
    fs::remove_dir_all(sizing_directory).expect("cleanup sizing");

    let directory = tempfile_dir("shared-root-budget");
    let limits = CacheLimits::new(entry_bytes, entry_bytes).expect("single-entry budget");
    let (mut first_store, _) = CacheStore::open(&directory, limits).expect("open first");
    let (mut second_store, _) = CacheStore::open(&directory, limits).expect("open second");
    let barrier = Arc::new(Barrier::new(3));
    let first_barrier = Arc::clone(&barrier);
    let first_image = value.clone();
    let first_put = std::thread::spawn(move || {
        first_barrier.wait();
        let result = first_store.put(first, &first_image, CacheTime::from_seconds(20));
        (first_store, result)
    });
    let second_barrier = Arc::clone(&barrier);
    let second_put = std::thread::spawn(move || {
        second_barrier.wait();
        let result = second_store.put(second, &value, CacheTime::from_seconds(21));
        (second_store, result)
    });
    barrier.wait();

    let (first_store, first_result) = first_put.join().expect("first put");
    let (second_store, second_result) = second_put.join().expect("second put");
    first_result.expect("first publication");
    second_result.expect("second publication");
    assert_eq!(first_store.len(), 1);
    assert_eq!(second_store.len(), 1);
    assert_eq!(mip_file_count(&directory), 1);
    assert!(mip_file_bytes(&directory) <= entry_bytes);
    let (reopened, report) = CacheStore::open(&directory, limits).expect("reopen");
    assert_eq!(report.valid_entries, 1);
    assert_eq!(reopened.len(), 1);
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
            edit_id: first.edit_id(),
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
fn lifecycle_invalidation_matches_edit_id_and_revision_not_revision_alone() {
    let directory = tempfile_dir("edit-identity");
    let first = key_for_edit(
        EditId::new(3).expect("edit"),
        5,
        ThumbnailProvenance::PipelineRender,
    );
    let same_revision_other_edit = key_for_edit(
        EditId::new(4).expect("edit"),
        5,
        ThumbnailProvenance::PipelineRender,
    );
    {
        let (mut store, _) = CacheStore::open(&directory, limits()).expect("open");
        store
            .put(first, &image(), CacheTime::from_seconds(20))
            .expect("put first");
        store
            .put(
                same_revision_other_edit,
                &image(),
                CacheTime::from_seconds(20),
            )
            .expect("put second");
    }
    let (store, _) = CacheStore::open(&directory, limits()).expect("reopen");
    let mut lifecycle = CacheLifecycle::new(store);
    lifecycle
        .apply(CacheChangeEvent::EditChanged {
            photo_id: first.photo_id(),
            edit_id: first.edit_id(),
            edit_revision: first.edit_revision(),
        })
        .expect("invalidate identity");
    assert!(lifecycle.store().keys().all(|key| key != first));
    assert!(
        lifecycle
            .store()
            .keys()
            .any(|key| key == same_revision_other_edit)
    );
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn resident_then_reopened_resolution_distinguishes_memory_from_durable_hits() {
    let directory = tempfile_dir("resident-reopen");
    let cache_key = key(5, ThumbnailProvenance::PipelineRender);
    let memory = ThumbnailMemoryCache::new(64 * 1024);
    let (store, _) = CacheStore::open(&directory, limits()).expect("open");
    let mut lifecycle = CacheLifecycle::with_memory(store, memory);
    let mut produced = 0;

    let first = lifecycle
        .resolve_with(cache_key, CacheTime::from_seconds(20), || {
            produced += 1;
            Ok::<_, ()>(image())
        })
        .expect("produce");
    assert_eq!(first.source(), CacheResolutionSource::Produced);
    assert_eq!(first.image(), &image());

    let resident = lifecycle
        .resolve_with(
            cache_key,
            CacheTime::from_seconds(21),
            || -> Result<_, ()> { panic!("resident hit must not invoke producer") },
        )
        .expect("resident hit");
    assert_eq!(resident.source(), CacheResolutionSource::Memory);
    assert_eq!(resident.created_at(), CacheTime::from_seconds(20));
    assert_eq!(produced, 1);
    drop(lifecycle);

    let (store, report) = CacheStore::open(&directory, limits()).expect("reopen");
    assert_eq!(report.valid_entries, 1);
    let mut reopened = CacheLifecycle::with_memory(store, ThumbnailMemoryCache::new(64 * 1024));
    let durable = reopened
        .resolve_with(
            cache_key,
            CacheTime::from_seconds(22),
            || -> Result<_, ()> { panic!("durable hit must not invoke producer") },
        )
        .expect("durable hit");
    assert_eq!(durable.source(), CacheResolutionSource::Durable);
    assert_eq!(durable.image().dimensions(), image().dimensions());
    assert_eq!(durable.image().pixels(), image().pixels());
    let promoted = reopened
        .resolve_with(
            cache_key,
            CacheTime::from_seconds(23),
            || -> Result<_, ()> { panic!("promoted durable hit must remain resident") },
        )
        .expect("promoted resident hit");
    assert_eq!(promoted.source(), CacheResolutionSource::Memory);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn resident_cost_eviction_falls_back_to_durable_storage() {
    let directory = tempfile_dir("resident-cost");
    let first = key(5, ThumbnailProvenance::PipelineRender);
    let second = key(6, ThumbnailProvenance::PipelineRender);
    let (store, _) = CacheStore::open(&directory, limits()).expect("open");
    let mut lifecycle = CacheLifecycle::with_memory(store, ThumbnailMemoryCache::new(50 * 1024));

    assert_eq!(
        lifecycle
            .resolve_with(first, CacheTime::from_seconds(20), || Ok::<_, ()>(image()))
            .expect("first")
            .source(),
        CacheResolutionSource::Produced
    );
    assert_eq!(
        lifecycle
            .resolve_with(second, CacheTime::from_seconds(21), || Ok::<_, ()>(image()))
            .expect("second")
            .source(),
        CacheResolutionSource::Produced
    );
    let durable = lifecycle
        .resolve_with(first, CacheTime::from_seconds(22), || -> Result<_, ()> {
            panic!("evicted resident entry must remain durable")
        })
        .expect("durable fallback");
    assert_eq!(durable.source(), CacheResolutionSource::Durable);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn lifecycle_invalidates_memory_only_keys_without_disturbing_other_residents() {
    let directory = tempfile_dir("resident-invalidation");
    let first = key(5, ThumbnailProvenance::PipelineRender);
    let second = key(6, ThumbnailProvenance::PipelineRender);
    let (store, _) = CacheStore::open(&directory, limits()).expect("open");
    let mut lifecycle = CacheLifecycle::with_memory(store, ThumbnailMemoryCache::new(128 * 1024));
    lifecycle
        .resolve_with(first, CacheTime::from_seconds(20), || Ok::<_, ()>(image()))
        .expect("first");
    lifecycle
        .resolve_with(second, CacheTime::from_seconds(21), || Ok::<_, ()>(image()))
        .expect("second");
    assert!(
        lifecycle
            .store_mut()
            .invalidate(first)
            .expect("remove durable first")
    );

    let invalidation = lifecycle
        .apply(CacheChangeEvent::EditChanged {
            photo_id: first.photo_id(),
            edit_id: first.edit_id(),
            edit_revision: first.edit_revision(),
        })
        .expect("invalidate resident first");
    assert_eq!(
        invalidation,
        CacheInvalidationReport {
            matched: 1,
            removed: 1,
            protected: 0,
        }
    );
    assert_eq!(
        lifecycle
            .resolve_with(first, CacheTime::from_seconds(22), || Ok::<_, ()>(image()))
            .expect("reproduce invalidated first")
            .source(),
        CacheResolutionSource::Produced
    );
    assert_eq!(
        lifecycle
            .resolve_with(second, CacheTime::from_seconds(23), || -> Result<_, ()> {
                panic!("unaffected resident must remain cached")
            })
            .expect("unaffected resident")
            .source(),
        CacheResolutionSource::Memory
    );
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn a_failed_resident_producer_can_be_retried() {
    let directory = tempfile_dir("resident-retry");
    let cache_key = key(5, ThumbnailProvenance::PipelineRender);
    let (store, _) = CacheStore::open(&directory, limits()).expect("open");
    let mut lifecycle = CacheLifecycle::with_memory(store, ThumbnailMemoryCache::new(64 * 1024));

    assert!(
        lifecycle
            .resolve_with(cache_key, CacheTime::from_seconds(20), || {
                Err::<DecodedImage, _>("failed")
            })
            .is_err()
    );
    assert_eq!(
        lifecycle
            .resolve_with(cache_key, CacheTime::from_seconds(21), || {
                Ok::<_, &'static str>(image())
            })
            .expect("retry")
            .source(),
        CacheResolutionSource::Produced
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
fn expired_leased_entry_is_deferred_until_release() {
    let directory = tempfile_dir("expired-lease");
    let cache_key = key(5, ThumbnailProvenance::PipelineRender);
    let limits = CacheLimits::new(16 * 1024, 8 * 1024)
        .expect("limits")
        .with_max_age(1);
    let (mut store, _) = CacheStore::open(&directory, limits).expect("open");
    store
        .put(cache_key, &image(), CacheTime::from_seconds(20))
        .expect("put");
    let lease = store.lease(cache_key).expect("lease").expect("entry");
    assert_eq!(store.evict(CacheTime::from_seconds(22)).expect("evict"), 0);
    assert_eq!(store.len(), 1);
    store.release_lease(lease).expect("release");
    assert_eq!(store.evict(CacheTime::from_seconds(22)).expect("evict"), 1);
    assert!(store.is_empty());
    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn expired_pinned_entry_is_deferred_until_unpin() {
    let directory = tempfile_dir("expired-pin");
    let cache_key = key(5, ThumbnailProvenance::PipelineRender);
    let limits = CacheLimits::new(16 * 1024, 8 * 1024)
        .expect("limits")
        .with_max_age(1);
    let (mut store, _) = CacheStore::open(&directory, limits).expect("open");
    store
        .put(cache_key, &image(), CacheTime::from_seconds(20))
        .expect("put");
    let pin = store.pin(cache_key).expect("pin").expect("entry");
    assert_eq!(store.evict(CacheTime::from_seconds(22)).expect("evict"), 0);
    assert_eq!(store.len(), 1);
    store.unpin(pin).expect("unpin");
    assert_eq!(store.evict(CacheTime::from_seconds(22)).expect("evict"), 1);
    assert!(store.is_empty());
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
