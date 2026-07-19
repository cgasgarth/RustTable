use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use rusttable_image_io::{
    HashStatus, PositionedSourceReader, ReadCancellation, SequentialSourceReader, SnapshotPolicy,
    SourceReadError, SourceSnapshot, SourceSnapshotError,
};

fn temporary_directory(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("rusttable-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("temporary directory");
    path
}

#[test]
fn handle_snapshot_supports_positioned_sequential_and_incremental_hash_reads() {
    let directory = temporary_directory("source-handle");
    let path = directory.join("source.bin");
    fs::write(&path, b"rusttable-source").expect("source");
    let policy = SnapshotPolicy::new(1024, 1024).expect("policy");
    let snapshot = SourceSnapshot::open(&path, policy).expect("snapshot");

    let mut positioned = [0_u8; 4];
    snapshot
        .read_exact_at(12, &mut positioned)
        .expect("positioned read");
    assert_eq!(&positioned, b"urce");

    let mut reader = snapshot.sequential_reader().expect("reader");
    let mut sequential = [0_u8; 9];
    reader.read_exact(&mut sequential).expect("sequential read");
    assert_eq!(&sequential, b"rusttable");
    assert_eq!(reader.position(), 9);

    let hash = snapshot.sha256().expect("hash");
    assert!(matches!(
        snapshot
            .receipt_for_alias("fixture-1")
            .unwrap()
            .hash_status(),
        HashStatus::Sha256(_)
    ));
    assert_eq!(
        snapshot.verify_sha256(hash).expect("verification").length(),
        16
    );
    assert!(
        snapshot
            .receipt_for_alias("fixture-1")
            .unwrap()
            .bytes_read()
            <= 1024
    );

    let _ = fs::remove_dir_all(directory);
}

#[test]
fn changed_source_is_rejected_before_read_result_is_accepted() {
    let directory = temporary_directory("source-change");
    let path = directory.join("source.bin");
    fs::write(&path, b"before").expect("source");
    let snapshot =
        SourceSnapshot::open(&path, SnapshotPolicy::new(1024, 1024).unwrap()).expect("snapshot");
    fs::write(&path, b"after-and-longer").expect("replacement");
    let mut bytes = [0_u8; 6];
    assert_eq!(
        snapshot.read_exact_at(0, &mut bytes),
        Err(SourceReadError::SourceChanged)
    );
    assert_eq!(
        snapshot.revalidate(),
        Err(rusttable_image_io::SourceChanged)
    );
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn read_limits_and_bounds_are_checked_without_partial_success() {
    let directory = temporary_directory("source-limits");
    let path = directory.join("source.bin");
    fs::write(&path, b"12345678").expect("source");
    let snapshot =
        SourceSnapshot::open(&path, SnapshotPolicy::new(8, 4).unwrap()).expect("snapshot");
    let mut bytes = [0_u8; 5];
    assert!(matches!(
        snapshot.read_exact_at(0, &mut bytes),
        Err(SourceReadError::ReadLimit { .. })
    ));
    let mut bytes = [0_u8; 2];
    assert!(matches!(
        snapshot.read_exact_at(7, &mut bytes),
        Err(SourceReadError::OutOfBounds { .. })
    ));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn stable_copy_is_bounded_and_removed_when_snapshot_is_released() {
    let directory = temporary_directory("source-copy");
    let path = directory.join("source.bin");
    fs::write(&path, b"stable source").expect("source");
    {
        let policy = SnapshotPolicy::new(1024, 1024)
            .expect("policy")
            .with_stable_copy();
        let snapshot = SourceSnapshot::open(&path, policy).expect("snapshot");
        assert_eq!(snapshot.read_all().expect("copy read"), b"stable source");
        assert_eq!(snapshot.length(), 13);
    }
    let entries = fs::read_dir(&directory)
        .expect("directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path(), path);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn cancellation_is_checked_before_and_after_positioned_reads() {
    struct Cancelled(AtomicBool);
    impl ReadCancellation for Cancelled {
        fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::Relaxed)
        }
    }

    let directory = temporary_directory("source-cancel");
    let path = directory.join("source.bin");
    fs::write(&path, b"cancel me").expect("source");
    let snapshot =
        SourceSnapshot::open(&path, SnapshotPolicy::new(1024, 1024).unwrap()).expect("snapshot");
    let cancelled = Cancelled(AtomicBool::new(true));
    let mut bytes = [0_u8; 1];
    assert_eq!(
        snapshot.read_exact_at_with_cancellation(0, &mut bytes, &cancelled),
        Err(SourceReadError::Cancelled)
    );
    let _ = fs::remove_dir_all(directory);
}

#[cfg(unix)]
#[test]
fn directories_and_symlinks_are_rejected_by_default() {
    use std::os::unix::fs::symlink;

    let directory = temporary_directory("source-policy");
    let file = directory.join("source.bin");
    let link = directory.join("source.link");
    fs::write(&file, b"source").expect("source");
    symlink(&file, &link).expect("symlink");
    assert!(matches!(
        SourceSnapshot::open(&directory, SnapshotPolicy::new(1024, 1024).unwrap()),
        Err(SourceSnapshotError::NotRegularFile)
    ));
    assert!(matches!(
        SourceSnapshot::open(&link, SnapshotPolicy::new(1024, 1024).unwrap()),
        Err(SourceSnapshotError::SymlinkRejected)
    ));
    let _ = fs::remove_dir_all(directory);
}
