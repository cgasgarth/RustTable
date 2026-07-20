use std::fs;
use std::path::Path;

use rusttable_parity::{parse_operation_manifest, validate_operation_manifest};

const PINNED_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";

#[test]
fn committed_operation_manifest_is_valid_and_pinned() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../architecture/darktable-operations.toml");
    let source = fs::read_to_string(&path).expect("read committed operation manifest");
    let manifest = parse_operation_manifest(&source).expect("parse committed operation manifest");
    validate_operation_manifest(&manifest).expect("validate committed operation manifest");
    assert_eq!(manifest.reference.source_commit, PINNED_COMMIT);
    assert!(!manifest.operations.is_empty());
}
