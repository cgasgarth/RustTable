const ACCOUNTING: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../architecture/source-accounting/issue-223.toml"
));

#[test]
fn issue_223_source_accounting_is_pinned_and_scoped() {
    for required in [
        "issue = 223",
        "canonical_commit = \"cfe57f3bbf5269bfacf31e832267279caa6938ad\"",
        "source_map_issue = 555",
        "dependency_issues = [173, 178, 179, 222, 555]",
        "preserve = [",
        "redesign = [",
        "excluded = [",
        "src/common/film.h",
        "src/control/crawler.c",
        "src/imageio/imageio_common.h",
        "src/control/jobs/sidecar_jobs.c",
    ] {
        assert!(
            ACCOUNTING.contains(required),
            "missing source accounting: {required}"
        );
    }
}
