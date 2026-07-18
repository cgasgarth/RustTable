use std::fs;
use std::path::{Path, PathBuf};

use rusttable_import::{
    FileSourceSnapshotReader, ImportSourceLimits, ImportSourceLimitsError, SourceReadStage,
    SourceSnapshotError, SourceSnapshotReader,
};

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
fn reads_one_owned_nonempty_snapshot_and_exposes_only_borrowed_bytes() {
    let file = path("valid");
    fs::write(&file, b"source-bytes").expect("fixture writes");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&file, ImportSourceLimits::new(64).unwrap())
        .expect("snapshot reads");
    assert_eq!(snapshot.bytes(), b"source-bytes");
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
