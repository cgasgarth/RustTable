use super::{dependencies, priority, source_block};

#[test]
fn parses_and_deduplicates_dependencies() {
    assert_eq!(
        dependencies("Depends on #3 and #4\nDepends on #3"),
        vec![3, 4]
    );
}

#[test]
fn requires_one_known_priority() {
    assert_eq!(priority(&["priority: P0".to_owned()]), "priority: P0");
    assert_eq!(priority(&[]), "invalid");
}

#[test]
fn extracts_anchor_section() {
    assert!(
        source_block("x\n## Upstream darktable source anchors\nCanonical source: cfe\n## Next\ny")
            .is_some()
    );
}
