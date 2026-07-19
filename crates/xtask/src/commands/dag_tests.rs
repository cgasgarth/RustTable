use super::model::{
    CargoMetadata, DeclaredEdge, DeclaredExternal, DeclaredPackage, PlatformTarget,
};
use super::verify::DagReport;
use super::*;

fn package(name: &str, role: &str) -> DeclaredPackage {
    DeclaredPackage {
        name: name.to_owned(),
        manifest: format!("crates/{name}/Cargo.toml"),
        role: role.to_owned(),
        integration_owner: name.to_owned(),
        features: Vec::new(),
        feature_sets: Vec::new(),
    }
}

fn edge(from: &str, to: &str, kind: &str) -> DeclaredEdge {
    DeclaredEdge {
        from: from.to_owned(),
        to: to.to_owned(),
        kind: kind.to_owned(),
        target: None,
        contexts: Vec::new(),
        required: true,
        tooling_only: false,
    }
}

fn contract(packages: Vec<DeclaredPackage>, edges: Vec<DeclaredEdge>) -> Contract {
    Contract {
        schema_version: 3,
        composition_root: "app".to_owned(),
        tooling_packages: vec!["testkit".to_owned()],
        packages,
        edges,
        external_dependencies: Vec::new(),
    }
}

fn contract_fixture(name: &str) -> &'static str {
    match name {
        "duplicate-package" => {
            include_str!("../../tests/fixtures/dag/contract-duplicate-package.toml")
        }
        "unknown-edge" => include_str!("../../tests/fixtures/dag/contract-unknown-edge.toml"),
        "cycle" => include_str!("../../tests/fixtures/dag/contract-cycle.toml"),
        "forbidden-product-edges" => {
            include_str!("../../tests/fixtures/dag/contract-forbidden-product-edges.toml")
        }
        _ => panic!("unknown contract fixture {name}"),
    }
}

fn fixture(name: &str) -> CargoMetadata {
    serde_json::from_str(match name {
        "renamed-package" => include_str!("../../tests/fixtures/dag/renamed-package.json"),
        "path-alias" => include_str!("../../tests/fixtures/dag/path-alias.json"),
        "target-specific" => include_str!("../../tests/fixtures/dag/target-specific.json"),
        "optional-feature" => include_str!("../../tests/fixtures/dag/optional-feature.json"),
        "optional-feature-minimal" => {
            include_str!("../../tests/fixtures/dag/optional-feature-minimal.json")
        }
        "build-dependency" => include_str!("../../tests/fixtures/dag/build-dependency.json"),
        "dev-cycle" => include_str!("../../tests/fixtures/dag/dev-cycle.json"),
        "tooling-leak" => include_str!("../../tests/fixtures/dag/tooling-leak.json"),
        "reverse-edge" => include_str!("../../tests/fixtures/dag/reverse-edge.json"),
        "new-package" => include_str!("../../tests/fixtures/dag/new-package.json"),
        "external-dependency" => {
            include_str!("../../tests/fixtures/dag/external-dependency.json")
        }
        "windows-target" => include_str!("../../tests/fixtures/dag/windows-target.json"),
        _ => panic!("unknown fixture {name}"),
    })
    .expect("fixture metadata")
}

fn context(name: &str, metadata: CargoMetadata) -> MetadataContext {
    MetadataContext {
        name: name.to_owned(),
        package: None,
        args: vec!["fixture".to_owned()],
        target: None,
        feature_set: Vec::new(),
        metadata,
    }
}

fn platform() -> PlatformContract {
    PlatformContract {
        schema_version: 1,
        targets: vec![PlatformTarget {
            triple: "x86_64-unknown-linux-gnu".to_owned(),
            os: "linux".to_owned(),
            architecture: "x86_64".to_owned(),
            runner: "ubuntu-latest".to_owned(),
        }],
    }
}

fn verify_fixture(name: &str, contract: &Contract) -> DagReport {
    verify(
        contract,
        &platform(),
        &[context("all-features", fixture(name))],
    )
}

fn external_rule(package: &str, owner: &str) -> DeclaredExternal {
    DeclaredExternal {
        package: package.to_owned(),
        source: Some("registry+https://github.com/rust-lang/crates.io-index".to_owned()),
        source_roles: Vec::new(),
        source_packages: vec![owner.to_owned()],
        kinds: Vec::new(),
        targets: Vec::new(),
        contexts: Vec::new(),
        optional: Some(false),
        allow_transitive: false,
    }
}

#[test]
fn path_alias_resolves_to_the_authoritative_package_name() {
    let report = verify_fixture(
        "path-alias",
        &contract(
            vec![
                package("app", "composition-root"),
                package("core", "product"),
            ],
            vec![edge("app", "core", "normal")],
        ),
    );
    assert!(report.violations.is_empty(), "{report:?}");
}

#[test]
fn renamed_package_fails_inventory_with_actionable_fields() {
    let report = verify_fixture(
        "renamed-package",
        &contract(
            vec![
                package("app", "composition-root"),
                package("core", "product"),
            ],
            vec![],
        ),
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "extra-package")
    );
    assert!(
        report
            .violations
            .iter()
            .all(|violation| !violation.feature_context.is_empty())
    );
}

#[test]
fn target_specific_edges_preserve_target_context() {
    let mut allowed = edge("app", "core", "normal");
    allowed.target = Some("cfg(unix)".to_owned());
    let report = verify_fixture(
        "target-specific",
        &contract(
            vec![
                package("app", "composition-root"),
                package("core", "product"),
            ],
            vec![allowed],
        ),
    );
    assert!(report.violations.is_empty(), "{report:?}");
    assert_eq!(report.discovered_edges[0].target, "cfg(unix)");
}

#[test]
fn optional_feature_edges_may_be_absent_from_minimal_context() {
    let mut allowed = edge("app", "core", "normal");
    allowed.required = false;
    allowed.contexts = vec!["all-features".to_owned()];
    let report = verify(
        &contract(
            vec![
                package("app", "composition-root"),
                package("core", "product"),
            ],
            vec![allowed],
        ),
        &platform(),
        &[
            context("all-features", fixture("optional-feature")),
            context(
                "no-default-features:app",
                fixture("optional-feature-minimal"),
            ),
        ],
    );
    assert!(report.violations.is_empty(), "{report:?}");
    let all = report
        .feature_contexts
        .iter()
        .find(|context| context.name == "all-features")
        .expect("all features");
    assert!(all.violations.is_empty(), "{all:?}");
}

#[test]
fn build_dependencies_are_compared_as_distinct_edges() {
    let mut allowed = edge("app", "core", "build");
    allowed.target = Some("all".to_owned());
    let report = verify_fixture(
        "build-dependency",
        &contract(
            vec![
                package("app", "composition-root"),
                package("core", "product"),
            ],
            vec![allowed],
        ),
    );
    assert!(report.violations.is_empty(), "{report:?}");
}

#[test]
fn dev_cycles_are_rejected() {
    let packages = vec![
        package("app", "composition-root"),
        package("core", "product"),
    ];
    let report = verify_fixture(
        "dev-cycle",
        &contract(
            packages,
            vec![edge("app", "core", "dev"), edge("core", "app", "dev")],
        ),
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "cycle")
    );
}

#[test]
fn product_to_tooling_edges_are_rejected() {
    let report = verify_fixture(
        "tooling-leak",
        &contract(
            vec![
                package("app", "composition-root"),
                package("testkit", "tooling"),
            ],
            vec![edge("app", "testkit", "dev")],
        ),
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "tooling-leak")
    );
}

#[test]
fn newly_added_workspace_packages_are_rejected() {
    let report = verify_fixture(
        "new-package",
        &contract(vec![package("app", "composition-root")], vec![]),
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "extra-package")
    );
}

#[test]
fn composition_root_is_not_a_reverse_dependency() {
    let report = verify_fixture(
        "reverse-edge",
        &contract(
            vec![
                package("app", "composition-root"),
                package("testkit", "tooling"),
            ],
            vec![edge("testkit", "app", "normal")],
        ),
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "composition-root-reverse-edge")
    );
}

#[test]
fn undeclared_reverse_edges_are_reported_explicitly() {
    let report = verify_fixture(
        "path-alias",
        &contract(
            vec![
                package("app", "composition-root"),
                package("core", "product"),
            ],
            vec![edge("core", "app", "normal")],
        ),
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "forbidden-reverse-edge")
    );
}

#[test]
fn external_aliases_are_checked_by_resolved_package_and_preserve_alias_receipts() {
    let mut contract = contract(vec![package("app", "composition-root")], vec![]);
    contract.external_dependencies = vec![external_rule("redb", "app")];
    let report = verify_fixture("external-dependency", &contract);
    assert!(report.violations.is_empty(), "{report:?}");
    let edge = report
        .discovered_edges
        .iter()
        .find(|edge| edge.external)
        .expect("external edge");
    assert_eq!(edge.destination, "redb");
    assert_eq!(edge.alias, "database");
    assert_eq!(
        edge.destination_source,
        "registry+https://github.com/rust-lang/crates.io-index"
    );
    assert!(edge.rule.starts_with("external:redb:"));
}

#[test]
fn unapproved_external_dependency_fails_with_owner_and_provenance_fields() {
    let report = verify_fixture(
        "external-dependency",
        &contract(vec![package("app", "composition-root")], vec![]),
    );
    let violation = report
        .violations
        .iter()
        .find(|violation| violation.code == "undeclared-external-edge")
        .expect("external violation");
    assert_eq!(violation.destination, "redb");
    assert_eq!(violation.alias, "database");
    assert_eq!(violation.source_role, "composition-root");
    assert_eq!(violation.rule, "<unmatched>");
}

#[test]
fn windows_target_predicates_are_applied_to_windows_contexts() {
    let mut allowed = edge("app", "core", "normal");
    allowed.target = Some("cfg(windows)".to_owned());
    let mut windows = context(
        "default@target:x86_64-pc-windows-msvc",
        fixture("windows-target"),
    );
    windows.target = Some("x86_64-pc-windows-msvc".to_owned());
    let report = verify(
        &contract(
            vec![
                package("app", "composition-root"),
                package("core", "product"),
            ],
            vec![allowed],
        ),
        &platform(),
        &[windows],
    );
    assert!(report.violations.is_empty(), "{report:?}");
    assert_eq!(
        report.discovered_edges[0].platform,
        "x86_64-pc-windows-msvc"
    );
}

#[test]
fn contract_parser_rejects_invalid_fixtures_deterministically() {
    for (fixture, expected) in [
        ("duplicate-package", "package names must be unique"),
        ("unknown-edge", "references an undeclared package"),
        ("cycle", "dependency graph contains a cycle"),
        (
            "forbidden-product-edges",
            "product package core cannot depend on",
        ),
    ] {
        let error = Contract::parse(contract_fixture(fixture)).expect_err(fixture);
        assert!(error.contains(expected), "{fixture}: {error}");
    }
}

#[test]
fn unix_target_rules_do_not_match_windows_metadata() {
    let mut allowed = edge("app", "core", "normal");
    allowed.target = Some("cfg(unix)".to_owned());
    let mut windows = context(
        "default@target:x86_64-pc-windows-msvc",
        fixture("windows-target"),
    );
    windows.target = Some("x86_64-pc-windows-msvc".to_owned());
    let report = verify(
        &contract(
            vec![
                package("app", "composition-root"),
                package("core", "product"),
            ],
            vec![allowed],
        ),
        &platform(),
        &[windows],
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "undeclared-edge")
    );
}

#[test]
fn declared_feature_singletons_and_interactions_are_required() {
    let mut app = package("app", "composition-root");
    app.features = vec!["one".to_owned(), "two".to_owned()];
    app.feature_sets = vec![vec!["one".to_owned(), "two".to_owned()]];
    let report = verify_fixture(
        "optional-feature",
        &contract(vec![app, package("core", "product")], vec![]),
    );
    assert!(
        report
            .violations
            .iter()
            .any(|violation| violation.code == "stale-feature")
    );
}
