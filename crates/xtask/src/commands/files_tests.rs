use std::collections::BTreeMap;

use icu_casemap::CaseMapper;

use super::{
    ArtifactClass, MagicSignature, Policy, Violation, collision_violations, inspect_manifest_paths,
    inspect_path, inspect_text, is_binary, manifest_escapes_repository, normalized_path,
    validate_mode,
};
use crate::commands::file_source::{EntryKind, SourceEntry, parse_index};

fn policy() -> Policy {
    Policy {
        version: 1,
        max_path_length: 240,
        case_mode: "reject".to_owned(),
        unicode_normalization: "nfc".to_owned(),
        symlinks: "reject".to_owned(),
        governed_roots: vec!["<root>".to_owned(), "fixtures".to_owned()],
        allowed_extensions: vec!["md".to_owned(), "raw".to_owned(), "toml".to_owned()],
        allowed_filenames: vec!["LICENSE".to_owned()],
        empty_filenames: vec![".gitkeep".to_owned()],
        binary_extensions: vec!["raw".to_owned()],
        binary_filenames: Vec::new(),
        executable_paths: vec!["scripts/run.sh".to_owned()],
        manifest_extensions: vec!["toml".to_owned()],
        reserved_windows_names: vec!["CON".to_owned(), "NUL".to_owned()],
        max_path_bytes: 240,
        max_path_utf16: 240,
        max_component_bytes: 255,
        max_component_utf16: 255,
        attributes_rules: Vec::new(),
        artifact_classes: vec![ArtifactClass {
            id: "raw".to_owned(),
            kind: "binary".to_owned(),
            extensions: vec!["raw".to_owned()],
            filenames: Vec::new(),
            paths: Vec::new(),
            max_size_bytes: 1024,
            empty_allowed: false,
        }],
        magic_signatures: vec![MagicSignature {
            extension: "raw".to_owned(),
            offset: 0,
            bytes: String::new(),
        }],
    }
}

#[test]
fn synthetic_index_fixture_preserves_modes_and_paths() {
    let output = b"100644 0123456789012345678901234567890123456789 0\tREADME.md\0\
100755 0123456789012345678901234567890123456789 0\tscripts/run.sh\0";
    let entries = parse_index(output).expect("synthetic index parses");
    assert_eq!(entries[0].mode, 0o100_644);
    assert_eq!(entries[0].path, "README.md");
    assert_eq!(entries[1].mode, 0o100_755);
    assert_eq!(entries[1].path, "scripts/run.sh");
}

#[test]
fn executable_mode_drift_is_reported_as_a_stable_rule() {
    let mut violations = Vec::new();
    validate_mode(
        &SourceEntry {
            mode: 0o100_644,
            object_id: "0123456789012345678901234567890123456789".to_owned(),
            stage: 0,
            path: "scripts/run.sh".to_owned(),
            valid_utf8: true,
            kind: EntryKind::Regular,
        },
        &policy(),
        &mut violations,
    );
    assert_eq!(violations[0].path, "scripts/run.sh");
    assert_eq!(violations[0].rule, "executable-bit");
}

#[test]
fn text_fixture_rules_cover_crlf_bom_invalid_utf8_nul_and_newlines() {
    let violations = inspect_text("fixture.md", b"\xef\xbb\xbfbad\r\n\xff\0\n\n");
    let rules = violations
        .iter()
        .map(|violation| violation.rule.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        rules,
        ["crlf", "bom", "nul", "invalid-utf8", "extra-final-newline"]
    );
}

#[test]
fn lone_cr_is_distinct_from_crlf() {
    assert!(
        inspect_text("fixture.md", b"bad\rtext\n")
            .iter()
            .any(|v| v.rule == "lone-cr")
    );
    assert!(
        !inspect_text("fixture.md", b"good\r\n")
            .iter()
            .any(|v| v.rule == "lone-cr")
    );
}

#[test]
fn binary_magic_mismatch_is_not_silently_accepted() {
    let mut policy = policy();
    policy.magic_signatures = vec![MagicSignature {
        extension: "raw".to_owned(),
        offset: 0,
        bytes: "89504e47".to_owned(),
    }];
    assert!(
        super::inspect_contents("picture.raw", b"not-a-png", &policy)
            .iter()
            .any(|v| v.rule == "binary-signature")
    );
}

#[test]
fn unicode_case_fold_catches_non_ascii_collisions() {
    let mut folded = BTreeMap::new();
    let left = "fixtures/straße.toml".to_owned();
    let right = "fixtures/STRASSE.toml".to_owned();
    let key = CaseMapper::new().fold_string(&right).into_owned();
    folded.insert(key, vec![left, right]);
    assert_eq!(collision_violations(&folded, "case-collision").len(), 2);
}

#[test]
fn binary_fixture_is_not_decoded_as_text() {
    let policy = policy();
    assert!(is_binary("fixtures/picture.raw", &policy));
    assert!(super::inspect_contents("picture.raw", &[0, 0xff, 0], &policy).is_empty());
}

#[test]
fn path_fixtures_cover_windows_portability_and_manifest_escape() {
    let policy = policy();
    let violations = inspect_path("fixtures/CON.txt", &policy);
    assert!(violations.iter().any(|violation| {
        violation.path == "fixtures/CON.txt" && violation.rule == "windows-reserved-name"
    }));
    assert!(
        inspect_path("fixtures/name. ", &policy)
            .iter()
            .any(|violation| violation.rule == "trailing-space-dot")
    );
    assert!(
        inspect_path("fixtures/notLICENSE", &policy)
            .iter()
            .any(|violation| violation.rule == "extension")
    );
    assert!(manifest_escapes_repository(
        "crates/rusttable-core/Cargo.toml",
        "../../../outside"
    ));
    assert!(!manifest_escapes_repository(
        "crates/rusttable-core/Cargo.toml",
        "../rusttable-image"
    ));
    let violations = inspect_manifest_paths(
        "fixtures/manifest.toml",
        b"path = \"../../outside\"\n",
        &Policy {
            manifest_extensions: vec!["toml".to_owned()],
            ..policy
        },
    );
    assert_eq!(violations[0].rule, "manifest-path-traversal");
}

#[test]
fn case_and_unicode_normalization_collisions_are_deterministic() {
    let composed = "fixtures/Caf\u{e9}.toml".to_owned();
    let decomposed = "fixtures/Cafe\u{301}.toml".to_owned();
    assert_eq!(normalized_path(&composed), normalized_path(&decomposed));
    let mut normalized = std::collections::BTreeMap::new();
    normalized.insert(
        "fixtures/café.toml".to_owned(),
        vec![composed.clone(), decomposed.clone()],
    );
    let violations = collision_violations(&normalized, "unicode-normalization-collision");
    assert_eq!(violations.len(), 2);
    assert_eq!(violations[0].path, decomposed);
    assert_eq!(violations[0].rule, "unicode-normalization-collision");
    let mut case = std::collections::BTreeMap::new();
    case.insert(
        "fixtures/readme.md".to_owned(),
        vec![
            "fixtures/README.md".to_owned(),
            "fixtures/readme.md".to_owned(),
        ],
    );
    assert_eq!(collision_violations(&case, "case-collision").len(), 2);
    case.insert(
        "fixtures/duplicate.md".to_owned(),
        vec![
            "fixtures/duplicate.md".to_owned(),
            "fixtures/duplicate.md".to_owned(),
        ],
    );
    assert_eq!(collision_violations(&case, "case-collision").len(), 2);
}

#[cfg(unix)]
#[test]
fn symlink_fixture_is_inspected_without_following_the_target() {
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    let directory = std::env::temp_dir().join(format!(
        "rusttable-file-policy-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).expect("fixture directory");
    std::fs::write(directory.join("target.md"), b"target\n").expect("target");
    symlink("target.md", directory.join("link.md")).expect("symlink");
    let metadata = std::fs::symlink_metadata(directory.join("link.md")).expect("metadata");
    assert!(metadata.file_type().is_symlink());
    std::fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn violation_display_contains_exact_path_and_rule() {
    let violation = Violation::new("fixtures/a.md", "bom", "UTF-8 BOM is not allowed");
    assert_eq!(
        violation.to_string(),
        "fixtures/a.md: bom: UTF-8 BOM is not allowed"
    );
}
