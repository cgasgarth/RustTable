use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use rusttable_import::{
    FileSourceSnapshotReader, ImportSourceLimits, ImportSourceLimitsError, SourceReadStage,
    SourceSnapshotError, SourceSnapshotReader, StableCopyError, StableCopyOptions,
};
use sha2::{Digest, Sha256};

fn path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusttable-source-snapshot-{name}-{}",
        std::process::id()
    ))
}

#[test]
fn limits_are_finite_and_reserve_the_extra_byte() {
    assert_eq!(
        ImportSourceLimits::new(0),
        Err(ImportSourceLimitsError::ZeroLimit)
    );
    assert!(matches!(
        ImportSourceLimits::new(u64::MAX),
        Err(ImportSourceLimitsError::NotRepresentable | ImportSourceLimitsError::MaxPlusOneOverflow)
    ));
    assert_eq!(ImportSourceLimits::new(64).unwrap().max_source_bytes(), 64);
}

#[test]
fn reads_one_nonempty_snapshot_through_explicit_bounded_materialization() {
    let file = path("valid");
    fs::write(&file, b"source-bytes").expect("fixture writes");
    let limits = ImportSourceLimits::new(64).unwrap();
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&file, limits)
        .expect("snapshot reads");
    assert_eq!(
        snapshot.materialize(limits).expect("source materializes"),
        b"source-bytes"
    );
    assert_eq!(snapshot.byte_length().get(), 12);
    fs::remove_file(file).expect("fixture removes");
}

#[test]
fn rejects_empty_and_over_limit_sources_without_reopening() {
    let empty = path("empty");
    fs::write(&empty, []).expect("fixture writes");
    assert_eq!(
        FileSourceSnapshotReader.read_snapshot(&empty, ImportSourceLimits::new(64).unwrap()),
        Err(SourceSnapshotError::EmptySource)
    );
    fs::remove_file(empty).expect("fixture removes");

    let large = path("large");
    fs::write(&large, b"12345").expect("fixture writes");
    assert!(matches!(
        FileSourceSnapshotReader.read_snapshot(&large, ImportSourceLimits::new(4).unwrap()),
        Err(SourceSnapshotError::SourceTooLarge {
            limit: 4,
            actual: 5,
            ..
        })
    ));
    fs::remove_file(large).expect("fixture removes");
}

#[test]
fn rejects_missing_and_nonregular_sources_with_closed_errors() {
    let missing = path("missing");
    assert!(matches!(
        FileSourceSnapshotReader.read_snapshot(&missing, ImportSourceLimits::new(64).unwrap()),
        Err(SourceSnapshotError::Io {
            stage: SourceReadStage::Open,
            ..
        })
    ));

    let directory = path("directory");
    fs::create_dir(&directory).expect("directory creates");
    assert_eq!(
        FileSourceSnapshotReader
            .read_snapshot(Path::new(&directory), ImportSourceLimits::new(64).unwrap()),
        Err(SourceSnapshotError::NotRegularFile {
            path: directory.clone()
        })
    );
    fs::remove_dir(directory).expect("directory removes");
}

#[test]
fn snapshot_reader_is_object_safe() {
    let reader: Box<dyn SourceSnapshotReader> = Box::new(FileSourceSnapshotReader);
    assert!(
        reader
            .read_snapshot(
                Path::new("not-present"),
                ImportSourceLimits::new(64).unwrap()
            )
            .is_err()
    );
}

#[test]
fn random_access_and_sequential_readers_are_independent_and_bounded() {
    let file = path("access");
    fs::write(&file, b"0123456789").expect("fixture writes");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&file, ImportSourceLimits::new(64).unwrap())
        .expect("snapshot reads");

    let mut random = [0; 3];
    snapshot.read_exact_at(4, &mut random).expect("random read");
    assert_eq!(&random, b"456");

    let mut first = snapshot.open_reader(6).expect("first reader creates");
    let mut second = snapshot.open_reader(6).expect("second reader creates");
    let mut first_bytes = [0; 2];
    let mut second_bytes = [0; 2];
    first
        .read_exact_checked(&mut first_bytes)
        .expect("first reader reads");
    second
        .read_exact_checked(&mut second_bytes)
        .expect("second reader reads");
    assert_eq!(&first_bytes, b"01");
    assert_eq!(&second_bytes, b"01");
    assert_eq!(first.position(), 2);
    assert_eq!(first.remaining(), 4);
    assert_eq!(second.position(), 2);

    let mut too_much = [0; 5];
    assert!(matches!(
        first.read_exact_checked(&mut too_much),
        Err(
            rusttable_import::SourceSnapshotReadError::ReaderBudgetExceeded {
                requested: 5,
                remaining: 4,
            }
        )
    ));
    fs::remove_file(file).expect("fixture removes");
}

#[test]
fn checked_ranges_reject_overflow_and_reader_limits() {
    let file = path("bounds");
    fs::write(&file, b"0123456789").expect("fixture writes");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&file, ImportSourceLimits::new(64).unwrap())
        .expect("snapshot reads");

    let mut bytes = [0; 2];
    assert!(matches!(
        snapshot.read_exact_at(u64::MAX, &mut bytes),
        Err(rusttable_import::SourceSnapshotReadError::OffsetOverflow { .. })
    ));
    assert!(matches!(
        snapshot.open_reader(11),
        Err(
            rusttable_import::SourceSnapshotReadError::ReaderLimitExceedsSource {
                limit: 11,
                source_length: 10,
            }
        )
    ));
    fs::remove_file(file).expect("fixture removes");
}

#[test]
fn reads_fail_after_the_opened_source_changes() {
    let file = path("changed");
    fs::write(&file, b"original").expect("fixture writes");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&file, ImportSourceLimits::new(64).unwrap())
        .expect("snapshot reads");
    fs::write(&file, b"changed!").expect("source changes");

    let mut bytes = [0; 4];
    assert!(matches!(
        snapshot.read_exact_at(0, &mut bytes),
        Err(rusttable_import::SourceSnapshotReadError::SourceChanged { .. })
    ));
    fs::remove_file(file).expect("fixture removes");
}

#[test]
fn reads_fail_after_same_length_rewrite_with_preserved_mtime() {
    let file = path("same-length-rewrite");
    fs::write(&file, b"original").expect("fixture writes");
    let limits = ImportSourceLimits::new(64).expect("limits create");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&file, limits)
        .expect("snapshot reads");
    let original_modified = fs::metadata(&file)
        .expect("fixture metadata reads")
        .modified()
        .expect("fixture modified time reads");

    fs::write(&file, b"changed!").expect("same-length source rewrites");
    fs::File::open(&file)
        .expect("fixture opens for time restore")
        .set_times(std::fs::FileTimes::new().set_modified(original_modified))
        .expect("fixture modified time restores");

    assert!(matches!(
        snapshot.materialize(limits),
        Err(rusttable_import::SourceSnapshotReadError::SourceChanged { .. })
    ));
    fs::remove_file(file).expect("fixture removes");
}

#[test]
fn opened_handle_survives_path_replacement_until_path_revalidation() {
    let file = path("replacement");
    let replacement = path("replacement-new");
    fs::write(&file, b"original").expect("fixture writes");
    let limits = ImportSourceLimits::new(64).unwrap();
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&file, limits)
        .expect("snapshot reads");
    fs::rename(&file, &replacement).expect("source moves");
    fs::write(&file, b"replaced").expect("replacement writes");

    let mut bytes = [0; 8];
    snapshot
        .read_exact_at(0, &mut bytes)
        .expect("opened handle remains stable");
    assert_eq!(&bytes, b"original");
    assert!(matches!(
        FileSourceSnapshotReader.revalidate(&snapshot, limits),
        Err(SourceSnapshotError::SourceChanged { .. })
    ));
    fs::remove_file(file).expect("replacement removes");
    fs::remove_file(replacement).expect("moved source removes");
}

#[test]
fn materialization_requires_an_explicit_byte_limit() {
    let file = path("materialize-limit");
    fs::write(&file, b"0123456789").expect("fixture writes");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&file, ImportSourceLimits::new(64).unwrap())
        .expect("snapshot reads");

    assert!(matches!(
        snapshot.materialize(ImportSourceLimits::new(4).unwrap()),
        Err(
            rusttable_import::SourceSnapshotReadError::MaterializationLimitExceeded {
                limit: 4,
                source_length: 10,
            }
        )
    ));
    fs::remove_file(file).expect("fixture removes");
}

#[test]
fn stable_copy_publishes_one_bounded_verified_snapshot_and_private_receipt() {
    let file = path("stable-copy-source");
    let cache = path("stable-copy-cache");
    let bytes = b"stable source bytes";
    fs::write(&file, bytes).expect("fixture writes");
    fs::create_dir(&cache).expect("cache creates");
    let limits = ImportSourceLimits::new(64).unwrap();
    let options = StableCopyOptions::new(&cache, limits)
        .with_chunk_bytes(3)
        .expect("chunk size is valid");

    let result = FileSourceSnapshotReader
        .read_stable_copy(&file, &options)
        .expect("stable copy publishes");
    assert!(
        result
            .receipt()
            .source_alias
            .starts_with("rusttable-source-snapshot-stable-copy-source-")
    );
    assert_eq!(
        result.receipt().hash_status,
        rusttable_import::SourceHashStatus::Verified
    );
    assert_eq!(
        result.receipt().identity_class,
        rusttable_import::SourceIdentityClass::FileId
    );
    assert_eq!(
        result.receipt().source_length,
        u64::try_from(bytes.len()).unwrap()
    );
    assert_eq!(
        result.receipt().copy_length,
        u64::try_from(bytes.len()).unwrap()
    );
    assert_eq!(
        result.receipt().bytes_read,
        u64::try_from(bytes.len()).unwrap()
    );
    assert!(!format!("{:?}", result.receipt()).contains(cache.to_str().unwrap()));
    assert_eq!(
        result
            .snapshot()
            .materialize(limits)
            .expect("published snapshot reads"),
        bytes
    );
    assert_eq!(fs::read_dir(&cache).unwrap().count(), 1);

    fs::remove_dir_all(cache).expect("cache removes");
    fs::remove_file(file).expect("fixture removes");
}

#[test]
fn stable_copy_reuses_matching_cache_and_rejects_a_collision() {
    let file = path("stable-copy-collision-source");
    let cache = path("stable-copy-collision-cache");
    let bytes = b"collision source";
    fs::write(&file, bytes).expect("fixture writes");
    fs::create_dir(&cache).expect("cache creates");
    let limits = ImportSourceLimits::new(64).unwrap();
    let options = StableCopyOptions::new(&cache, limits);

    let first = FileSourceSnapshotReader
        .read_stable_copy(&file, &options)
        .expect("first copy publishes");
    let second = FileSourceSnapshotReader
        .read_stable_copy(&file, &options)
        .expect("matching copy reuses");
    assert_eq!(
        first.snapshot().content_sha256(),
        second.snapshot().content_sha256()
    );

    let mut digest = Sha256::new();
    digest.update(bytes);
    let digest: [u8; 32] = digest.finalize().into();
    let mut digest_hex = String::with_capacity(64);
    for byte in digest {
        write!(&mut digest_hex, "{byte:02x}").expect("writing to a string cannot fail");
    }
    let destination = cache.join(format!("rusttable-source-{digest_hex}.bin"));
    fs::write(&destination, b"wrong content").expect("collision writes");
    assert!(matches!(
        FileSourceSnapshotReader.read_stable_copy(&file, &options),
        Err(StableCopyError::CacheCollision { .. })
    ));

    fs::remove_dir_all(cache).expect("cache removes");
    fs::remove_file(file).expect("fixture removes");
}

#[test]
fn darktable_source_accounting_assigns_each_issue_anchor() {
    let table = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../architecture/source-snapshot-accounting.toml");
    let contents = fs::read_to_string(table).expect("source accounting reads");
    for anchor in [
        "src/common/image.h",
        "src/common/film.h",
        "src/imageio/imageio_common.h",
        "src/common/mipmap_cache.h",
        "src/common/exif.cc",
    ] {
        assert!(contents.contains(anchor), "missing source anchor {anchor}");
    }
    assert_eq!(contents.matches("status = \"replaced\"").count(), 5);
    assert!(!contents.contains("status = \"unmapped\""));
}
