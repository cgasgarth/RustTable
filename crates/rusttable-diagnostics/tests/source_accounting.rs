use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

const SOURCE_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";

#[derive(Debug, Deserialize)]
struct Receipt {
    schema: String,
    issue: u16,
    parent_issue: u16,
    source_commit: String,
    source_map_issue: u16,
    source_map_schema: String,
    source_map_status: String,
    rust_owners: Vec<String>,
    preserve: Vec<String>,
    redesign: Vec<String>,
    excluded: Vec<String>,
    anchor: Vec<Anchor>,
}

#[derive(Debug, Deserialize)]
struct Anchor {
    path: String,
    role: String,
    symbols: Vec<String>,
    owner: String,
    responsibility: String,
}

#[test]
fn diagnostics_source_receipt_matches_current_contract() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../architecture/issue-178-diagnostics-source-receipt.toml");
    let source = std::fs::read_to_string(path).expect("diagnostics source receipt");
    let receipt: Receipt = toml::from_str(&source).expect("valid diagnostics source receipt");

    assert_eq!(receipt.schema, "rusttable.source-map.receipt.v1");
    assert_eq!(receipt.issue, 178);
    assert_eq!(receipt.parent_issue, 158);
    assert_eq!(receipt.source_commit, SOURCE_COMMIT);
    assert_eq!(receipt.source_map_issue, 555);
    assert_eq!(receipt.source_map_schema, "rusttable.source-map.v1");
    assert_eq!(
        receipt.source_map_status,
        "candidate-validated-pending-555-acceptance"
    );
    assert!(
        receipt
            .rust_owners
            .iter()
            .any(|owner| owner == "crates/rusttable-diagnostics")
    );
    assert!(
        receipt
            .rust_owners
            .iter()
            .any(|owner| owner == "crates/rusttable-app")
    );
    assert!(!receipt.preserve.is_empty());
    assert!(!receipt.redesign.is_empty());
    assert!(!receipt.excluded.is_empty());
    assert_eq!(receipt.anchor.len(), 10);
    for anchor in &receipt.anchor {
        assert!(!anchor.path.is_empty());
        assert!(!anchor.role.is_empty());
        assert!(!anchor.symbols.is_empty());
        assert!(!anchor.owner.is_empty());
        assert!(!anchor.responsibility.is_empty());
    }

    if let Some(reference) = reference_clone() {
        verify_against_pinned_tree(&reference, &receipt);
    }
}

fn reference_clone() -> Option<PathBuf> {
    let configured = std::env::var_os("RUSTTABLE_DARKTABLE_REFERENCE").map(PathBuf::from);
    let default = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../Darktable");
    let reference = configured.unwrap_or(default);
    reference.is_dir().then_some(reference)
}

fn verify_against_pinned_tree(reference: &Path, receipt: &Receipt) {
    assert_git_success(
        reference,
        ["cat-file", "-e", SOURCE_COMMIT],
        "pinned darktable commit",
    );
    for anchor in &receipt.anchor {
        let object = format!("{}:{}", SOURCE_COMMIT, anchor.path);
        assert_git_success(
            reference,
            ["cat-file", "-e", object.as_str()],
            "source anchor path",
        );
        let contents = git_output(reference, ["show", object.as_str()]);
        for symbol in &anchor.symbols {
            assert!(
                contents.contains(symbol),
                "{} is absent from pinned {}",
                symbol,
                anchor.path
            );
        }
    }
}

fn assert_git_success<const N: usize>(reference: &Path, args: [&str; N], label: &str) {
    let output = Command::new("git")
        .arg("-C")
        .arg(reference)
        .args(args)
        .output()
        .expect("git available");
    assert!(output.status.success(), "{label}: git command failed");
}

fn git_output<const N: usize>(reference: &Path, args: [&str; N]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(reference)
        .args(args)
        .output()
        .expect("git show");
    assert!(output.status.success(), "git show failed");
    String::from_utf8(output.stdout).expect("pinned source is UTF-8")
}
