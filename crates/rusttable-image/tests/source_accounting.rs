const ACCOUNTING: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../architecture/source-accounting/issue-222.toml"
));

#[test]
fn issue_222_source_accounting_is_pinned_and_scoped() {
    for required in [
        "issue = 222",
        "canonical_commit = \"cfe57f3bbf5269bfacf31e832267279caa6938ad\"",
        "source_map_issue = 555",
        "dependency_issues = [159, 162, 269, 555]",
        "preserve = [",
        "redesign = [",
        "excluded = [",
        "src/common/image.h",
        "src/develop/pixelpipe.h",
        "src/common/colorspaces.h",
        "src/imageio/imageio_rawspeed.cc",
    ] {
        assert!(
            ACCOUNTING.contains(required),
            "missing source accounting: {required}"
        );
    }
}
