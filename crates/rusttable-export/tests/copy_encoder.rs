use std::fs::{self, File};
use std::io::Read;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_export::copy::{Encoder, Settings, SidecarSettings, SourceDescriptor};
use rusttable_import::{FileSourceSnapshotReader, ImportSourceLimits, SourceSnapshotReader};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn temp_path(label: &str) -> PathBuf {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "rusttable-copy-{}-{id}-{label}",
        std::process::id()
    ))
}

#[test]
fn copy_stream_is_byte_exact_and_hash_verified() {
    let source_path = temp_path("source.jpg");
    let primary_path = temp_path("primary.jpg");
    let bytes = (0_u8..=255).cycle().take(1_000_003).collect::<Vec<_>>();
    fs::write(&source_path, &bytes).expect("source");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&source_path, ImportSourceLimits::new(2_000_000).unwrap())
        .expect("snapshot");
    let descriptor = SourceDescriptor::from_snapshot(&snapshot).expect("descriptor");
    let receipt = Encoder::new(Settings::default().with_chunk_bytes(4096))
        .encode_to_paths(
            &snapshot,
            &descriptor,
            &primary_path,
            None,
            || false,
            |_, _| {},
        )
        .expect("copy");
    assert_eq!(receipt.primary_bytes, bytes.len() as u64);
    assert_eq!(fs::read(&primary_path).expect("primary"), bytes);
    assert_eq!(receipt.original_extension, "jpg");
    assert!(
        receipt
            .manifest
            .bytes()
            .windows(b"source_sha256=".len())
            .any(|window| window == b"source_sha256=")
    );
    let _ = fs::remove_file(source_path);
    let _ = fs::remove_file(primary_path);
}

#[test]
fn copy_sidecar_is_separate_and_cancel_cleans_staging() {
    let source_path = temp_path("source.tif");
    let primary_path = temp_path("primary.tif");
    let sidecar_path = temp_path("primary.xmp");
    fs::write(&source_path, b"immutable source bytes").expect("source");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&source_path, ImportSourceLimits::new(1024).unwrap())
        .expect("snapshot");
    let descriptor = SourceDescriptor::from_snapshot(&snapshot).expect("descriptor");
    let settings = Settings::default()
        .with_sidecar(SidecarSettings::new(7, [4; 32]).with_history(b"ordered-history"));
    let receipt = Encoder::new(settings)
        .encode_to_paths(
            &snapshot,
            &descriptor,
            &primary_path,
            Some(&sidecar_path),
            || false,
            |_, _| {},
        )
        .expect("copy with sidecar");
    assert!(receipt.sidecar_sha256.is_some());
    let mut sidecar = String::new();
    File::open(&sidecar_path)
        .unwrap()
        .read_to_string(&mut sidecar)
        .unwrap();
    assert!(sidecar.contains("editRevision=\"7\""));
    assert!(sidecar.contains("requestHash="));

    let cancelled_primary = temp_path("cancelled.tif");
    let cancelled_sidecar = temp_path("cancelled.xmp");
    let result = Encoder::new(Settings::default()).encode_to_paths(
        &snapshot,
        &descriptor,
        &cancelled_primary,
        Some(&cancelled_sidecar),
        || true,
        |_, _| {},
    );
    assert!(matches!(
        result,
        Err(rusttable_export::copy::Error::Cancelled)
    ));
    assert!(!cancelled_primary.exists());
    assert!(!cancelled_sidecar.exists());
    for path in [source_path, primary_path, sidecar_path] {
        let _ = fs::remove_file(path);
    }
}

#[test]
fn copy_detects_source_drift_after_snapshot() {
    let source_path = temp_path("drift.png");
    let primary_path = temp_path("drift-output.png");
    fs::write(&source_path, b"before").expect("source");
    let snapshot = FileSourceSnapshotReader
        .read_snapshot(&source_path, ImportSourceLimits::new(1024).unwrap())
        .expect("snapshot");
    let descriptor = SourceDescriptor::from_snapshot(&snapshot).expect("descriptor");
    fs::write(&source_path, b"after").expect("drift");
    let result = Encoder::new(Settings::default()).encode_to_paths(
        &snapshot,
        &descriptor,
        &primary_path,
        None,
        || false,
        |_, _| {},
    );
    assert!(matches!(
        result,
        Err(rusttable_export::copy::Error::SourceRead(_)
            | rusttable_export::copy::Error::SourceIdentityMismatch,)
    ));
    assert!(!primary_path.exists());
    let _ = fs::remove_file(source_path);
}
