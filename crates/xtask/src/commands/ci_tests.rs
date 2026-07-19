use super::*;

fn check(id: &str, group: &str) -> Check {
    Check {
        id: id.to_owned(),
        label: id.to_owned(),
        program: "true".to_owned(),
        args: Vec::new(),
        owner: "test".to_owned(),
        prerequisites: Vec::new(),
        prerequisites_by_surface: BTreeMap::new(),
        surfaces: vec!["precommit".to_owned()],
        parallel_group: group.to_owned(),
        parallel_group_by_surface: BTreeMap::new(),
        resources: Vec::new(),
        network: "none".to_owned(),
        artifacts: Vec::new(),
        platforms: vec!["all".to_owned()],
        severity: "error".to_owned(),
        merge_only: false,
        accept_warning: false,
    }
}

fn contract(checks: Vec<Check>) -> Contract {
    Contract {
        budgets: BTreeMap::from([
            ("prepush".to_owned(), 170),
            ("pull_request".to_owned(), 150),
            ("main".to_owned(), 2_700),
        ]),
        checks,
    }
}

#[test]
fn rejects_duplicate_ids_and_labels() {
    let error = contract(vec![check("same", "a"), check("same", "b")])
        .validate()
        .expect_err("duplicate");
    assert!(error.contains("duplicate"));
}

#[test]
fn rejects_fast_network_checks() {
    let mut item = check("network", "a");
    item.network = "read".to_owned();
    let error = contract(vec![item]).validate().expect_err("network");
    assert!(error.contains("network = none"));
}

#[test]
fn rejects_unknown_platforms() {
    let mut item = check("platform", "a");
    item.platforms = vec!["solaris".to_owned()];
    let error = contract(vec![item]).validate().expect_err("platform");
    assert!(error.contains("unknown platform solaris"));
}

#[test]
fn rejects_duplicate_resources() {
    let mut item = check("resource", "a");
    item.resources = vec!["cargo-target-dir".to_owned(), "cargo-target-dir".to_owned()];
    let error = contract(vec![item])
        .validate()
        .expect_err("duplicate resource");
    assert!(error.contains("duplicate resource"));
}

#[test]
fn resource_batch_keeps_unrelated_checks_parallel() {
    let mut first = check("first", "a");
    first.resources = vec!["cargo-target-dir".to_owned()];
    let mut second = check("second", "a");
    second.resources = vec!["cargo-target-dir".to_owned()];
    let independent = check("independent", "a");
    let mut ready = vec![&first, &second, &independent];

    let batch = take_resource_compatible_batch(&mut ready);
    assert_eq!(
        batch
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "independent"]
    );
    assert_eq!(
        ready
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        vec!["second"]
    );
}

#[test]
fn surface_budget_is_not_preallocated_to_checks() {
    let mut first = check("a", "a");
    first.surfaces = vec!["prepush".to_owned()];
    first.args = vec!["a".to_owned()];
    let mut second = check("b", "b");
    second.surfaces = vec!["prepush".to_owned()];
    second.args = vec!["b".to_owned()];
    contract(vec![first, second])
        .validate()
        .expect("surface deadline is enforced at execution time");
}

#[test]
fn rejects_a_precommit_time_budget() {
    let mut contract = contract(vec![check("uncapped", "a")]);
    contract.budgets.insert("precommit".to_owned(), 1);
    let error = contract.validate().expect_err("precommit must be uncapped");
    assert!(error.contains("precommit must not declare a time budget"));
}

#[test]
fn rejects_recursive_surface_invocation() {
    let mut item = check("recursive", "a");
    item.program = "cargo".to_owned();
    item.args = vec!["xtask".to_owned(), "ci".to_owned(), "precommit".to_owned()];
    let error = contract(vec![item]).validate().expect_err("recursive");
    assert!(error.contains("recursively"));
}

#[test]
fn rejects_unknown_prerequisites() {
    let mut item = check("dependent", "dependent");
    item.prerequisites = vec!["missing".to_owned()];
    let error = contract(vec![item]).validate().expect_err("prerequisite");
    assert!(error.contains("unknown prerequisite"));
}

#[test]
fn rejects_incompatible_prerequisite_groups() {
    let mut dependent = check("dependent", "same");
    dependent.prerequisites = vec!["prerequisite".to_owned()];
    let prerequisite = check("prerequisite", "same");
    let error = contract(vec![dependent, prerequisite])
        .validate()
        .expect_err("parallel group");
    assert!(error.contains("incompatible prerequisite group"));
}

#[test]
fn rejects_surface_prerequisite_that_runs_after_dependent() {
    let mut dependent = check("dependent", "dependent");
    dependent.surfaces = vec!["prepush".to_owned()];
    dependent.prerequisites_by_surface =
        BTreeMap::from([("prepush".to_owned(), vec!["prerequisite".to_owned()])]);
    let mut prerequisite = check("prerequisite", "prerequisite");
    prerequisite.surfaces = vec!["prepush".to_owned()];
    dependent.parallel_group_by_surface =
        BTreeMap::from([("prepush".to_owned(), "rust-02".to_owned())]);
    prerequisite.parallel_group_by_surface =
        BTreeMap::from([("prepush".to_owned(), "rust-03".to_owned())]);
    let error = contract(vec![dependent, prerequisite])
        .validate()
        .expect_err("ordered prerequisite");
    assert!(error.contains("must run before"));
}

#[test]
fn rejects_pull_request_checks_that_drift_from_main() {
    let mut item = check("pr", "pr");
    item.surfaces = vec!["pull_request".to_owned()];
    let error = contract(vec![item]).validate().expect_err("surface drift");
    assert!(error.contains("drifts from main"));
}

#[test]
fn rejects_merge_only_checks_on_a_fast_surface() {
    let mut item = check("merge-only", "main");
    item.merge_only = true;
    item.surfaces = vec!["prepush".to_owned(), "main".to_owned()];
    let error = contract(vec![item])
        .validate()
        .expect_err("merge-only fast check");
    assert!(error.contains("merge-only"));
}

#[test]
fn checked_in_contract_keeps_local_coverage_and_merge_only_exhaustiveness() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let contract = Contract::load(&root).expect("validation contract");
    contract.validate().expect("valid validation contract");
    assert!(!contract.budgets.contains_key("precommit"));
    assert_eq!(contract.budgets["prepush"], 150);
    assert_eq!(contract.budgets["pull_request"], 150);

    for id in [
        "fmt",
        "metadata",
        "rust-clippy",
        "rust-test",
        "source",
        "native-boundaries",
        "workflow-policy",
        "repository-files",
        "workspace-dag",
        "workspace-rust-version",
        "workspace-layout",
        "cache-workflow-policy",
    ] {
        let item = contract
            .checks
            .iter()
            .find(|check| check.id == id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert!(item.on("prepush"), "{id} must run on prepush");
    }
    let rust_build_all = contract
        .checks
        .iter()
        .find(|check| check.id == "rust-build-all")
        .expect("full debug build");
    let rust_test = contract
        .checks
        .iter()
        .find(|check| check.id == "rust-test")
        .expect("library test");
    for surface in ["prepush", "pull_request"] {
        if surface == "pull_request" {
            assert_eq!(rust_test.parallel_group_for(surface), "rust-01-test");
            assert!(rust_test.prerequisites_for(surface).is_empty());
            continue;
        }
        assert_eq!(rust_test.parallel_group_for(surface), "rust-01-test");
        assert!(rust_test.prerequisites_for(surface).is_empty());
    }
    let library_lint = contract
        .checks
        .iter()
        .find(|check| check.id == "rust-clippy")
        .expect("library lint");
    assert!(library_lint.on("prepush"));
    assert!(!library_lint.on("pull_request"));
    assert_eq!(library_lint.parallel_group_for("prepush"), "rust-02-clippy");
    assert_eq!(library_lint.prerequisites_for("prepush"), vec!["rust-test"]);
    for item in std::iter::once(rust_build_all).chain(
        ["rust-test-all", "rust-clippy-all"].iter().map(|id| {
            contract
                .checks
                .iter()
                .find(|check| check.id == *id)
                .unwrap_or_else(|| panic!("missing {id}"))
        }),
    ) {
        assert!(item.on("precommit"), "{} must run on precommit", item.id);
        assert!(item.on("main"), "{} must run on main", item.id);
        assert!(!item.merge_only, "{} cannot be merge-only", item.id);
    }

    for id in ["rust-test-all", "rust-clippy-all"] {
        let item = contract
            .checks
            .iter()
            .find(|check| check.id == id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert_eq!(
            item.surfaces,
            vec!["precommit".to_owned(), "main".to_owned()]
        );
        assert!(!item.merge_only);
    }
    let coverage = contract
        .checks
        .iter()
        .find(|check| check.id == "coverage")
        .expect("merge-only coverage");
    assert_eq!(coverage.surfaces, vec!["main".to_owned()]);
    assert!(coverage.merge_only);
    assert!(coverage.args.iter().any(|arg| arg.contains("coverage run")));
    assert!(
        contract
            .checks
            .iter()
            .filter(|check| check.on("prepush"))
            .all(|check| !check.merge_only)
    );
}

#[test]
fn records_a_failed_check_receipt() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let mut item = check("failure", "a");
    item.program = "sh".to_owned();
    item.args = vec!["-c".to_owned(), "exit 7".to_owned()];
    let receipt = execute_check(
        &root,
        &item,
        "precommit",
        &runner,
        &Arc::new(AtomicBool::new(false)),
        Some(Instant::now() + Duration::from_secs(5)),
    );
    assert_eq!(receipt.status, "failed");
    assert!(
        receipt
            .detail
            .expect("failure detail")
            .contains("completed")
    );
}

#[test]
fn warning_failure_does_not_cancel_siblings_or_group() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let mut warning = check("warning", "a");
    warning.program = "sh".to_owned();
    warning.args = vec!["-c".to_owned(), "exit 7".to_owned()];
    warning.severity = "warning".to_owned();
    let cancellation = Arc::new(AtomicBool::new(false));
    let receipt = execute_check(
        &root,
        &warning,
        "precommit",
        &runner,
        &cancellation,
        Some(Instant::now() + Duration::from_secs(5)),
    );
    assert_eq!(receipt.status, "warning");
    assert!(!cancellation.load(Ordering::Acquire));
}

#[test]
fn records_a_surface_deadline_receipt() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let mut item = check("timeout", "a");
    item.program = "sh".to_owned();
    item.args = vec!["-c".to_owned(), "sleep 2".to_owned()];
    let receipt = execute_check(
        &root,
        &item,
        "precommit",
        &runner,
        &Arc::new(AtomicBool::new(false)),
        Some(Instant::now() + Duration::from_millis(50)),
    );
    assert_eq!(receipt.status, "failed");
    assert!(
        receipt
            .detail
            .expect("timeout detail")
            .contains("surface deadline exhausted")
    );
}

#[test]
fn check_can_exceed_a_former_individual_cap_with_surface_time_remaining() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let mut item = check("longer-than-legacy-cap", "a");
    item.program = "sh".to_owned();
    item.args = vec!["-c".to_owned(), "sleep 0.1".to_owned()];
    let receipt = execute_check(
        &root,
        &item,
        "precommit",
        &runner,
        &Arc::new(AtomicBool::new(false)),
        Some(Instant::now() + Duration::from_secs(1)),
    );
    assert_eq!(receipt.status, "passed");
}

#[test]
fn uncapped_precommit_runs_without_a_process_timeout() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let mut item = check("uncapped", "a");
    item.program = "sh".to_owned();
    item.args = vec!["-c".to_owned(), "sleep 0.1".to_owned()];
    let receipt = execute_check(
        &root,
        &item,
        "precommit",
        &runner,
        &Arc::new(AtomicBool::new(false)),
        None,
    );
    assert_eq!(receipt.status, "passed");
}

#[test]
fn surface_budget_terminates_hung_owned_work() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let mut item = check("hung", "a");
    item.program = "sh".to_owned();
    item.args = vec!["-c".to_owned(), "sleep 2".to_owned()];
    item.surfaces = vec!["prepush".to_owned()];
    let contract = Contract {
        budgets: BTreeMap::from([(String::from("prepush"), 1)]),
        checks: vec![item],
    };

    let error = execute_surface_for_group(&root, &contract, "prepush", None, &runner)
        .expect_err("surface deadline");
    assert!(error.contains("surface budget of 1s exhausted"));
    assert!(error.contains("surface deadline exhausted"));
}

#[test]
fn selected_group_receipt_omits_other_parallel_groups() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let mut rust_check = check("rust", "rust");
    rust_check.surfaces = vec!["pull_request".to_owned()];
    let mut repository_check = check("repository", "repository");
    repository_check.surfaces = vec!["pull_request".to_owned()];
    let contract = contract(vec![rust_check, repository_check]);

    let receipt =
        execute_surface_for_group(&root, &contract, "pull_request", Some("rust"), &runner)
            .expect("selected group succeeds");

    assert_eq!(receipt.group.as_deref(), Some("rust"));
    assert_eq!(
        receipt
            .checks
            .iter()
            .map(|check| check.id.as_str())
            .collect::<Vec<_>>(),
        vec!["rust"]
    );
    assert_eq!(
        receipt.omitted[0].reason,
        "not assigned to selected group rust"
    );
}

#[test]
fn selected_group_rejects_unmet_prerequisites() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let mut dependent = check("dependent", "dependent");
    dependent.surfaces = vec!["pull_request".to_owned()];
    dependent.prerequisites = vec!["prerequisite".to_owned()];
    let mut prerequisite = check("prerequisite", "prerequisite");
    prerequisite.surfaces = vec!["pull_request".to_owned()];
    let contract = Contract {
        budgets: BTreeMap::from([(String::from("pull_request"), 150)]),
        checks: vec![dependent, prerequisite],
    };

    let error =
        execute_surface_for_group(&root, &contract, "pull_request", Some("dependent"), &runner)
            .expect_err("selected group with prerequisite");
    assert!(error.contains("unmet prerequisite prerequisite"));
}

#[test]
fn omits_checks_for_other_platforms_before_execution() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let mut item = check("other-platform", "rust");
    item.platforms = vec![if current_platform() == "linux" {
        "macos".to_owned()
    } else {
        "linux".to_owned()
    }];
    let contract = contract(vec![item]);

    let receipt = execute_surface_for_group(&root, &contract, "precommit", None, &runner)
        .expect("platform omission succeeds");

    assert!(receipt.checks.is_empty());
    assert_eq!(
        receipt.omitted[0].reason,
        format!("not assigned to platform {}", current_platform())
    );
}

#[test]
fn workspace_dag_receipt_contract_has_deterministic_artifacts() {
    let item = check("workspace-dag", "repository");
    let (args, artifacts, process_artifacts) =
        ci_support::check_process_contract(&item, "pull_request");
    assert!(args.ends_with(&[
        "--artifact".to_owned(),
        "target/validation/workspace-dag/pull_request.json".to_owned(),
    ]));
    assert_eq!(
        artifacts,
        vec![
            "target/validation/workspace-dag/pull_request.json".to_owned(),
            "target/validation/workspace-dag/pull_request.stdout.log".to_owned(),
            "target/validation/workspace-dag/pull_request.stderr.log".to_owned(),
        ]
    );
    assert_eq!(
        process_artifacts,
        Some((
            "target/validation/workspace-dag/pull_request.stdout.log".to_owned(),
            "target/validation/workspace-dag/pull_request.stderr.log".to_owned(),
        ))
    );
}

#[test]
fn cancels_siblings_and_keeps_failure_receipts_sorted() {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).expect("repository root");
    let mut failure = check("a-failure", "group");
    failure.program = "sh".to_owned();
    failure.args = vec!["-c".to_owned(), "exit 7".to_owned()];
    failure.surfaces = vec!["prepush".to_owned()];
    let mut sibling = check("z-sibling", "group");
    sibling.program = "sh".to_owned();
    sibling.args = vec!["-c".to_owned(), "sleep 10".to_owned()];
    sibling.surfaces = vec!["prepush".to_owned()];
    let contract = Contract {
        budgets: BTreeMap::from([(String::from("prepush"), 150)]),
        checks: vec![failure, sibling],
    };

    let error = execute_surface_for_group(&root, &contract, "prepush", None, &runner)
        .expect_err("failing group");
    let failure_position = error.find("\"id\":\"a-failure\"").expect("failure receipt");
    let sibling_position = error.find("\"id\":\"z-sibling\"").expect("sibling receipt");
    assert!(failure_position < sibling_position);
    assert!(error.contains("cancelled"));
}
