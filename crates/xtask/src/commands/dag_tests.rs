use super::*;

fn package(name: &str, role: &str) -> DeclaredPackage {
    DeclaredPackage {
        name: name.to_owned(),
        manifest: format!("crates/{name}/Cargo.toml"),
        role: role.to_owned(),
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
    }
}

fn contract(packages: Vec<DeclaredPackage>, edges: Vec<DeclaredEdge>) -> Contract {
    Contract {
        schema_version: 1,
        composition_root: "app".to_owned(),
        tooling_packages: vec!["testkit".to_owned()],
        target_platforms: Vec::new(),
        packages,
        edges,
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
        _ => panic!("unknown fixture {name}"),
    })
    .expect("fixture metadata")
}

fn context(name: &str, metadata: CargoMetadata) -> MetadataContext {
    MetadataContext {
        name: name.to_owned(),
        package: None,
        args: vec!["fixture".to_owned()],
        metadata,
    }
}

fn verify_fixture(name: &str, contract: &Contract) -> DagReport {
    verify(contract, &[context("all-features", fixture(name))])
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
