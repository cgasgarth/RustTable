use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_export::CollisionPolicy;
use rusttable_export::destinations::disk::{
    BundleMember, DiskDestination, DiskError, DiskSettings,
};
use sha2::{Digest, Sha256};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn root(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusttable-disk-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("root");
    path
}

#[test]
fn commits_exact_bytes_and_replays_idempotently() {
    let root = root("commit");
    let destination = DiskDestination::new(
        &root,
        DiskSettings {
            collision: CollisionPolicy::Fail,
            ..DiskSettings::default()
        },
    )
    .expect("destination");
    let bytes = b"exact staged bytes";
    let hash = Sha256::digest(bytes).into();
    let first = destination
        .commit("nested/photo.exr", bytes, hash, "job-1")
        .expect("commit");
    assert_eq!(
        fs::read(root.join("nested/photo.exr")).expect("read"),
        bytes
    );
    let replay = destination
        .commit("nested/photo.exr", bytes, hash, "job-1")
        .expect("replay");
    assert!(!first.idempotency_replayed);
    assert!(replay.idempotency_replayed);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn skip_if_same_reuses_existing_bytes_and_unique_suffix_preserves_collisions() {
    let root = root("collision");
    let bytes = b"existing";
    let hash = Sha256::digest(bytes).into();
    fs::write(root.join("photo.exr"), bytes).expect("existing file");
    let skip = DiskDestination::new(
        &root,
        DiskSettings {
            collision: CollisionPolicy::SkipIfSame,
            ..DiskSettings::default()
        },
    )
    .expect("skip destination");
    let receipt = skip
        .commit("photo.exr", bytes, hash, "skip-1")
        .expect("same bytes are reusable");
    assert!(receipt.idempotency_replayed);
    assert_eq!(fs::read(root.join("photo.exr")).expect("read"), bytes);

    let unique = DiskDestination::new(
        &root,
        DiskSettings {
            collision: CollisionPolicy::UniqueSuffix,
            ..DiskSettings::default()
        },
    )
    .expect("unique destination");
    let unique_receipt = unique
        .commit(
            "photo.exr",
            b"new",
            Sha256::digest(b"new").into(),
            "unique-1",
        )
        .expect("unique suffix");
    assert_eq!(unique_receipt.logical_target, "photo.exr");
    assert_eq!(fs::read(root.join("photo-01.exr")).expect("suffix"), b"new");
    assert_eq!(fs::read(root.join("photo.exr")).expect("original"), bytes);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_traversal_and_child_symlinks_and_never_replaces_on_fail() {
    let root = root("safety");
    let destination = DiskDestination::new(&root, DiskSettings::default()).expect("destination");
    assert!(matches!(
        destination.commit("../escape", b"bad", [0; 32], "job"),
        Err(DiskError::InvalidTarget(_))
    ));
    let child = root.join("child");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&root, &child).expect("symlink");
    #[cfg(unix)]
    assert!(matches!(
        destination.commit("child/file.exr", b"bad", [0; 32], "job"),
        Err(DiskError::ChildSymlink(_))
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn atomically_promotes_a_complete_bundle_and_allocates_unique_suffixes() {
    let root = root("bundle");
    let destination = DiskDestination::new(
        &root,
        DiskSettings {
            collision: CollisionPolicy::UniqueSuffix,
            ..DiskSettings::default()
        },
    )
    .expect("destination");
    let members = vec![
        BundleMember::new(
            "primary.exr",
            b"exr".to_vec(),
            Sha256::digest(b"exr").into(),
        )
        .expect("member"),
        BundleMember::new(
            "primary.xmp",
            b"xmp".to_vec(),
            Sha256::digest(b"xmp").into(),
        )
        .expect("member"),
    ];
    destination
        .commit_bundle("photo", &members, "bundle-1")
        .expect("bundle");
    assert_eq!(
        fs::read(root.join("photo/primary.exr")).expect("primary"),
        b"exr"
    );
    assert_eq!(
        fs::read(root.join("photo/primary.xmp")).expect("sidecar"),
        b"xmp"
    );
    assert!(!root.join("photo/.owner").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bundle_unique_suffix_and_replay_are_complete_operations() {
    let root = root("bundle-collision");
    let destination = DiskDestination::new(
        &root,
        DiskSettings {
            collision: CollisionPolicy::UniqueSuffix,
            ..DiskSettings::default()
        },
    )
    .expect("destination");
    fs::create_dir(root.join("photo")).expect("existing target");
    let members = vec![
        BundleMember::new(
            "primary.exr",
            b"exr".to_vec(),
            Sha256::digest(b"exr").into(),
        )
        .expect("member"),
    ];
    destination
        .commit_bundle("photo", &members, "bundle-2")
        .expect("unique bundle");
    assert_eq!(
        fs::read(root.join("photo-01/primary.exr")).expect("suffix member"),
        b"exr"
    );
    assert!(
        destination
            .commit_bundle("photo-01", &members, "bundle-2")
            .expect("bundle replay")
            .idempotency_replayed
    );
    let _ = fs::remove_dir_all(root);
}
