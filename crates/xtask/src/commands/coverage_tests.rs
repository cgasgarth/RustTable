use super::*;

fn test_root() -> RepositoryRoot {
    RepositoryRoot::discover(&crate::process::ProcessRunner::new()).expect("repository root")
}

fn core_sources() -> BTreeMap<String, SourceInfo> {
    BTreeMap::from([(
        "crates/rusttable-core/src/lib.rs".to_owned(),
        SourceInfo {
            package: "rusttable-core".to_owned(),
            generated: false,
            excluded: false,
        },
    )])
}

fn fixture_policy() -> Policy {
    toml::from_str(include_str!("../../tests/fixtures/coverage/policy.toml"))
        .expect("policy fixture")
}

#[test]
fn normalizes_windows_paths_and_merges_duplicate_lines() {
    let root =
        RepositoryRoot::discover(&crate::process::ProcessRunner::new()).expect("repository root");
    let sources = BTreeMap::from([(
        "crates/rusttable-core/src/lib.rs".to_owned(),
        SourceInfo {
            package: "rusttable-core".to_owned(),
            generated: false,
            excluded: false,
        },
    )]);
    let policy = Policy {
        schema: POLICY_SCHEMA.to_owned(),
        toolchain: "beta-2026-07-17".to_owned(),
        cargo_llvm_cov_version: CARGO_LLVM_COV_VERSION.to_owned(),
        llvm_version: "22.1.8".to_owned(),
        target: "x86_64-unknown-linux-gnu".to_owned(),
        features: vec!["all-features".to_owned()],
        generated_markers: Vec::new(),
        exclusions: Exclusions::default(),
        packages: Vec::new(),
        exceptions: Vec::new(),
    };
    let files = parse_lcov(
        include_str!("../../tests/fixtures/coverage/mixed-paths.lcov"),
        &root,
        &sources,
        &policy,
    )
    .expect("fixture");
    assert_eq!(files[0].path, "crates/rusttable-core/src/lib.rs");
    assert_eq!(files[0].lines.get(&2), Some(&1));
    assert_eq!(files[0].lines.len(), 3);
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "the malformed LCOV fixture matrix is clearer as one fail-closed test"
)]
fn rejects_malformed_and_empty_lcov() {
    let root =
        RepositoryRoot::discover(&crate::process::ProcessRunner::new()).expect("repository root");
    let sources = BTreeMap::new();
    let policy = Policy {
        schema: POLICY_SCHEMA.to_owned(),
        toolchain: "toolchain".to_owned(),
        cargo_llvm_cov_version: CARGO_LLVM_COV_VERSION.to_owned(),
        llvm_version: "llvm".to_owned(),
        target: "x86_64-unknown-linux-gnu".to_owned(),
        features: vec!["all-features".to_owned()],
        generated_markers: Vec::new(),
        exclusions: Exclusions::default(),
        packages: Vec::new(),
        exceptions: Vec::new(),
    };
    assert!(parse_lcov("", &root, &sources, &policy).is_err());
    assert!(parse_lcov("SF:crates/missing.rs\n", &root, &sources, &policy).is_err());
    assert!(
        parse_lcov(
            include_str!("../../tests/fixtures/coverage/malformed.lcov"),
            &root,
            &BTreeMap::from([(
                "crates/rusttable-core/src/lib.rs".to_owned(),
                SourceInfo {
                    package: "rusttable-core".to_owned(),
                    generated: false,
                    excluded: false,
                },
            )]),
            &policy,
        )
        .is_err()
    );
    assert!(
        parse_lcov(
            include_str!("../../tests/fixtures/coverage/deleted-source.lcov"),
            &root,
            &BTreeMap::from([(
                "crates/rusttable-core/src/lib.rs".to_owned(),
                SourceInfo {
                    package: "rusttable-core".to_owned(),
                    generated: false,
                    excluded: false,
                },
            )]),
            &policy,
        )
        .is_err()
    );
    assert!(
        parse_lcov(
            include_str!("../../tests/fixtures/coverage/partial-run.lcov"),
            &root,
            &BTreeMap::from([
                (
                    "crates/rusttable-core/src/lib.rs".to_owned(),
                    SourceInfo {
                        package: "rusttable-core".to_owned(),
                        generated: false,
                        excluded: false,
                    },
                ),
                (
                    "crates/rusttable-core/src/other.rs".to_owned(),
                    SourceInfo {
                        package: "rusttable-core".to_owned(),
                        generated: false,
                        excluded: false,
                    },
                ),
            ]),
            &policy,
        )
        .is_ok()
    );
    let partial = parse_lcov(
        include_str!("../../tests/fixtures/coverage/partial-run.lcov"),
        &root,
        &BTreeMap::from([
            (
                "crates/rusttable-core/src/lib.rs".to_owned(),
                SourceInfo {
                    package: "rusttable-core".to_owned(),
                    generated: false,
                    excluded: false,
                },
            ),
            (
                "crates/rusttable-core/src/other.rs".to_owned(),
                SourceInfo {
                    package: "rusttable-core".to_owned(),
                    generated: false,
                    excluded: false,
                },
            ),
        ]),
        &policy,
    )
    .expect("partial fixture parses before completeness check");
    assert!(
        verify_source_set(
            &partial,
            &BTreeMap::from([
                (
                    "crates/rusttable-core/src/lib.rs".to_owned(),
                    SourceInfo {
                        package: "rusttable-core".to_owned(),
                        generated: false,
                        excluded: false,
                    },
                ),
                (
                    "crates/rusttable-core/src/other.rs".to_owned(),
                    SourceInfo {
                        package: "rusttable-core".to_owned(),
                        generated: false,
                        excluded: false,
                    },
                ),
            ])
        )
        .is_err()
    );
}

#[test]
fn generated_source_is_excluded_from_production_summary() {
    let root =
        RepositoryRoot::discover(&crate::process::ProcessRunner::new()).expect("repository root");
    let sources = BTreeMap::from([(
        "crates/rusttable-core/src/generated.rs".to_owned(),
        SourceInfo {
            package: "rusttable-core".to_owned(),
            generated: true,
            excluded: false,
        },
    )]);
    let policy = Policy {
        schema: POLICY_SCHEMA.to_owned(),
        toolchain: "beta-2026-07-17".to_owned(),
        cargo_llvm_cov_version: CARGO_LLVM_COV_VERSION.to_owned(),
        llvm_version: "22.1.8".to_owned(),
        target: "x86_64-unknown-linux-gnu".to_owned(),
        features: vec!["all-features".to_owned()],
        generated_markers: Vec::new(),
        exclusions: Exclusions::default(),
        packages: Vec::new(),
        exceptions: Vec::new(),
    };
    let files = parse_lcov(
        include_str!("../../tests/fixtures/coverage/generated-module.lcov"),
        &root,
        &sources,
        &policy,
    )
    .expect("generated fixture");
    assert!(
        build_report(
            &files,
            &sources,
            &policy,
            "sha".to_owned(),
            Identity {
                rustc: "rustc".to_owned(),
                llvm_cov: "llvm-cov".to_owned(),
                llvm_version: "llvm".to_owned(),
            },
            "hash".to_owned(),
        )
        .is_err()
    );
}

#[test]
fn policy_rejects_new_packages_and_expired_exceptions() {
    let expired: Policy = toml::from_str(include_str!(
        "../../tests/fixtures/coverage/expired-exception.toml"
    ))
    .expect("expired policy");
    let sources = BTreeMap::from([(
        "crates/rusttable-core/src/lib.rs".to_owned(),
        SourceInfo {
            package: "rusttable-core".to_owned(),
            generated: false,
            excluded: false,
        },
    )]);
    let metadata = Metadata {
        packages: vec![MetadataPackage {
            name: "rusttable-core".to_owned(),
            manifest_path: "crates/rusttable-core/Cargo.toml".to_owned(),
        }],
    };
    let error = validate_policy(&expired, &sources, &metadata).expect_err("expired exception");
    assert!(error.contains("expired"));
    let mut new_package = expired;
    new_package.exceptions.clear();
    let metadata = Metadata {
        packages: vec![
            MetadataPackage {
                name: "rusttable-core".to_owned(),
                manifest_path: "crates/rusttable-core/Cargo.toml".to_owned(),
            },
            MetadataPackage {
                name: "rusttable-new-package".to_owned(),
                manifest_path: "crates/rusttable-new-package/Cargo.toml".to_owned(),
            },
        ],
    };
    let error = validate_policy(&new_package, &sources, &metadata).expect_err("new package");
    assert!(error.contains("do not match workspace"));

    let fixture: Policy = toml::from_str(include_str!(
        "../../tests/fixtures/coverage/new-package.toml"
    ))
    .expect("new package fixture");
    let error = validate_policy(&fixture, &sources, &Metadata { packages: vec![] })
        .expect_err("new package fixture");
    assert!(error.contains("do not match workspace"));
}

#[test]
fn renamed_package_and_zero_hit_files_fail_closed() {
    let root =
        RepositoryRoot::discover(&crate::process::ProcessRunner::new()).expect("repository root");
    let sources = BTreeMap::from([(
        "crates/rusttable-core/src/lib.rs".to_owned(),
        SourceInfo {
            package: "rusttable-core".to_owned(),
            generated: false,
            excluded: false,
        },
    )]);
    let policy: Policy = toml::from_str(include_str!("../../tests/fixtures/coverage/policy.toml"))
        .expect("policy fixture");
    assert!(
        parse_lcov(
            include_str!("../../tests/fixtures/coverage/renamed-package.lcov"),
            &root,
            &sources,
            &policy,
        )
        .is_err()
    );
    let zero_hit = LcovFile {
        path: "crates/rusttable-core/src/lib.rs".to_owned(),
        functions: BTreeMap::new(),
        lines: BTreeMap::from([(1, 0)]),
        regions: BTreeMap::new(),
        region_counts: None,
    };
    let report = build_report(
        &[zero_hit],
        &sources,
        &policy,
        "sha".to_owned(),
        Identity {
            rustc: "rustc".to_owned(),
            llvm_cov: "cargo-llvm-cov 0.6.16".to_owned(),
            llvm_version: "22.1.8".to_owned(),
        },
        "hash".to_owned(),
    )
    .expect("zero-hit report");
    assert!(verify_floors(&report, &policy).is_err());
}

#[test]
fn covered_branch_deletion_fails_the_reviewed_package_floor() {
    let root = test_root();
    let sources = core_sources();
    let policy = fixture_policy();
    let files = parse_lcov(
        include_str!("../../tests/fixtures/coverage/branch-deletion.lcov"),
        &root,
        &sources,
        &policy,
    )
    .expect("branch deletion fixture");
    let report = build_report(
        &files,
        &sources,
        &policy,
        "sha".to_owned(),
        Identity {
            rustc: "rustc".to_owned(),
            llvm_cov: "cargo-llvm-cov 0.6.16".to_owned(),
            llvm_version: "llvm".to_owned(),
        },
        "hash".to_owned(),
    )
    .expect("branch deletion report");
    let error = verify_floors(&report, &policy).expect_err("branch deletion floor");
    assert!(error.contains("coverage floor regression"));
}

#[test]
fn llvm_json_regions_are_attached_to_the_lcov_source_set() {
    let root = test_root();
    let sources = core_sources();
    let policy = fixture_policy();
    let mut files = parse_lcov(
        include_str!("../../tests/fixtures/coverage/mixed-paths.lcov"),
        &root,
        &sources,
        &policy,
    )
    .expect("LCOV fixture");
    let json = br#"{
        "data": [{
            "files": [{
                "filename": "crates/rusttable-core/src/lib.rs",
                "summary": {"regions": {"count": 3, "covered": 2}}
            }]
        }]
    }"#;
    apply_json_metrics(json, &root, &sources, &mut files).expect("JSON metrics");
    assert_eq!(files[0].region_counts, Some((2, 3)));
    assert!(verify_region_records(&files, &sources).is_ok());
}
