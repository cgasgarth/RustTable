use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn reference_process_boundary_stays_in_testkit() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in rust_sources(&root) {
        if path.starts_with(root.join("crates/rusttable-testkit"))
            || path.starts_with(root.join("crates/rusttable-parity"))
            || path.starts_with(root.join("crates/xtask"))
        {
            continue;
        }
        let source = fs::read_to_string(&path).expect("Rust source should be readable");
        assert!(
            !source.contains("darktable-cli"),
            "reference executable leaked into {}",
            path.display()
        );
        assert!(
            !source.contains("ReferenceRunner"),
            "reference runner leaked into {}",
            path.display()
        );
        let identity_only = source.replace("rusttable_testkit::reference", "");
        assert!(
            !identity_only.contains("rusttable_testkit::reference"),
            "reference dependency leaked into {}",
            path.display()
        );
    }
    for path in cargo_manifests(&root) {
        if path == root.join("crates/rusttable-testkit/Cargo.toml")
            || path == root.join("Cargo.toml")
            || path == root.join("crates/rusttable-parity/Cargo.toml")
            || path == root.join("crates/xtask/Cargo.toml")
        {
            continue;
        }
        let manifest = fs::read_to_string(&path).expect("Cargo manifest should be readable");
        assert!(
            !manifest.contains("rusttable-testkit"),
            "product dependency leaked into {}",
            path.display()
        );
    }
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect(root, "rs", &mut paths);
    paths
}

fn cargo_manifests(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    collect(root, "toml", &mut paths);
    paths
        .into_iter()
        .filter(|path| path.file_name().is_some_and(|name| name == "Cargo.toml"))
        .collect()
}

fn collect(path: &Path, extension: &str, paths: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .file_name()
            .is_some_and(|name| name == "target" || name == ".git")
        {
            continue;
        }
        if path.is_dir() {
            collect(&path, extension, paths);
        } else if path.extension().is_some_and(|value| value == extension) {
            paths.push(path);
        }
    }
}
