use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;

use serde::Deserialize;

use crate::{Result, codegen, export_contract, fixtures, numerics, operations, run_process_quiet};
type CheckFn = fn(&Path) -> Result;

const DEFAULT_BASE: &str = "origin/main";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CustomHarnessTest {
    package: &'static str,
    target: &'static str,
}

const CUSTOM_HARNESS_TESTS: &[CustomHarnessTest] = &[
    CustomHarnessTest {
        package: "rusttable-ui",
        target: "exposure_gtk_boundary",
    },
    CustomHarnessTest {
        package: "rusttable-ui",
        target: "darkroom_left_rail_smoke",
    },
    CustomHarnessTest {
        package: "rusttable-ui",
        target: "bauhaus_slider_input_gtk_boundary",
    },
    CustomHarnessTest {
        package: "rusttable-ui",
        target: "neural_restore_strength_gtk_boundary",
    },
    CustomHarnessTest {
        package: "rusttable-ui",
        target: "velvia_gtk_boundary",
    },
    CustomHarnessTest {
        package: "rusttable-ui",
        target: "colorcontrast_gtk_boundary",
    },
    CustomHarnessTest {
        package: "rusttable-ui",
        target: "colorcorrection_gtk_boundary",
    },
    CustomHarnessTest {
        package: "rusttable-ui",
        target: "colorzones_gtk_boundary",
    },
    CustomHarnessTest {
        package: "rusttable-ui",
        target: "vibrance_gtk_boundary",
    },
    CustomHarnessTest {
        package: "rusttable-app",
        target: "darkroom_shell_runtime_smoke",
    },
];

const CHECKS: &[(&str, CheckFn)] = &[
    ("numerical contracts", numerics::verify_registered_choices),
    (
        "cargo format, clippy, tests, and rustdoc",
        run_cargo_pipeline,
    ),
    ("operation codegen", verify_codegen),
    ("operation manifest", verify_operations),
    ("export contract", verify_export_contract),
    ("fixtures", verify_fixtures),
    (
        "dependency advisories, licenses, and sources",
        dependency_checks,
    ),
];

pub(crate) fn run(root: &Path, parallel: bool, changed: bool, base: Option<&str>) -> Result {
    if changed {
        run_changed(root, base.unwrap_or(DEFAULT_BASE))
    } else {
        run_full(root, parallel)
    }
}

fn run_full(root: &Path, parallel: bool) -> Result {
    if parallel {
        run_parallel(root)?;
    } else {
        run_sequential(root)?;
    }
    eprintln!(
        "PASS xtask check (mode={}, branches={}, cargo-owner=1)",
        if parallel { "parallel" } else { "sequential" },
        CHECKS.len()
    );
    Ok(())
}

fn run_changed(root: &Path, base: &str) -> Result {
    let paths = changed_paths(root, base)?;
    let workspace = Workspace::load(root)?;
    let scope = workspace.scope(&paths);
    eprintln!(
        "xtask check: mode=changed partial=true base={base} paths={} packages={} fallback-all={}",
        paths.len(),
        scope.packages.len(),
        scope.fallback_all
    );

    for invocation in cargo_plan(&scope.packages) {
        invocation.run(root)?;
    }
    for contract in &scope.contracts {
        contract.run(root)?;
    }
    if scope.run_cargo_deny {
        dependency_checks(root)?;
    }

    eprintln!(
        "PASS xtask check (mode=changed, partial=true, packages={}, contracts={}, cargo-deny={})",
        scope.packages.len(),
        scope.contracts.len(),
        scope.run_cargo_deny
    );
    Ok(())
}

fn run_sequential(root: &Path) -> Result {
    for (_, check) in CHECKS {
        check(root)?;
    }
    Ok(())
}

fn run_parallel(root: &Path) -> Result {
    run_parallel_checks(root, CHECKS)
}

fn run_parallel_checks(root: &Path, checks: &[(&str, CheckFn)]) -> Result {
    thread::scope(|scope| {
        let handles = checks
            .iter()
            .map(|&(_, check)| scope.spawn(move || check(root)))
            .collect::<Vec<_>>();
        let mut failures = Vec::new();
        for ((label, _), handle) in checks.iter().zip(handles) {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(format!("{label}: {error}")),
                Err(_) => failures.push(format!("{label}: check thread panicked")),
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "pre-commit checks failed:\n{}",
                failures.join("\n")
            ))
        }
    })
}

fn run_cargo_pipeline(root: &Path) -> Result {
    run_process_quiet(
        "format",
        Command::new("cargo")
            .current_dir(root)
            .args(["fmt", "--all", "--", "--check"]),
    )?;
    run_process_quiet(
        "clippy",
        Command::new("cargo").current_dir(root).args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]),
    )?;
    let test_plan = HybridTestPlan::load(root)?;
    run_process_quiet(
        "ordinary tests (nextest)",
        Command::new("cargo")
            .current_dir(root)
            .args(&test_plan.nextest_args),
    )?;
    run_process_quiet(
        "custom harness tests",
        Command::new("cargo")
            .current_dir(root)
            .args(&test_plan.custom_harness_args),
    )?;
    run_process_quiet(
        "rustdoc",
        Command::new("cargo")
            .current_dir(root)
            .env("RUSTDOCFLAGS", "-Dwarnings")
            .args([
                "doc",
                "--workspace",
                "--all-features",
                "--no-deps",
                "--locked",
            ]),
    )?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum TestTargetKind {
    Lib,
    Bin,
    Test,
    Example,
    Bench,
}

impl TestTargetKind {
    fn from_metadata(kinds: &[String]) -> Result<Self> {
        match kinds {
            [kind] if kind == "lib" => Ok(Self::Lib),
            [kind] if kind == "bin" => Ok(Self::Bin),
            [kind] if kind == "test" => Ok(Self::Test),
            [kind] if kind == "example" => Ok(Self::Example),
            [kind] if kind == "bench" => Ok(Self::Bench),
            _ => Err(format!(
                "test-enabled Cargo target has unsupported kinds: {kinds:?}"
            )),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TestTarget {
    package: String,
    name: String,
    kind: TestTargetKind,
}

#[derive(Debug)]
struct HybridTestPartition {
    all: BTreeSet<TestTarget>,
    ordinary: BTreeSet<TestTarget>,
    custom: BTreeSet<TestTarget>,
}

#[derive(Debug, Eq, PartialEq)]
struct HybridTestPlan {
    nextest_args: Vec<String>,
    custom_harness_args: Vec<String>,
}

impl HybridTestPlan {
    fn load(root: &Path) -> Result<Self> {
        let metadata = load_cargo_metadata(root)?;
        let partition = hybrid_test_partition(&metadata)?;
        verify_custom_harness_manifests(&metadata)?;
        Self::from_partition(&partition)
    }

    fn from_partition(partition: &HybridTestPartition) -> Result<Self> {
        let overlap = partition
            .ordinary
            .intersection(&partition.custom)
            .collect::<Vec<_>>();
        let covered = partition
            .ordinary
            .union(&partition.custom)
            .cloned()
            .collect::<BTreeSet<_>>();
        if !overlap.is_empty() || covered != partition.all {
            return Err(format!(
                "hybrid test partition is incomplete or overlapping: overlap={overlap:?}"
            ));
        }

        let custom_names = partition
            .custom
            .iter()
            .map(|target| target.name.as_str())
            .collect::<BTreeSet<_>>();
        if custom_names.len() != partition.custom.len() {
            return Err("custom harness test names must be unique across the workspace".to_owned());
        }
        if let Some(collision) = partition.ordinary.iter().find(|target| {
            target.kind == TestTargetKind::Test && custom_names.contains(target.name.as_str())
        }) {
            return Err(format!(
                "ordinary test target collides with custom harness name: {}/{}",
                collision.package, collision.name
            ));
        }

        let mut nextest_args = [
            "nextest",
            "run",
            "--workspace",
            "--all-features",
            "--locked",
        ]
        .map(str::to_owned)
        .to_vec();
        if partition
            .ordinary
            .iter()
            .any(|target| target.kind == TestTargetKind::Lib)
        {
            nextest_args.push("--lib".to_owned());
        }
        if partition
            .ordinary
            .iter()
            .any(|target| target.kind == TestTargetKind::Bin)
        {
            nextest_args.push("--bins".to_owned());
        }
        append_named_targets(
            &mut nextest_args,
            &partition.ordinary,
            TestTargetKind::Test,
            "--test",
        );
        append_named_targets(
            &mut nextest_args,
            &partition.ordinary,
            TestTargetKind::Example,
            "--example",
        );
        append_named_targets(
            &mut nextest_args,
            &partition.ordinary,
            TestTargetKind::Bench,
            "--bench",
        );

        let mut custom_harness_args = ["test", "--workspace", "--all-features", "--locked"]
            .map(str::to_owned)
            .to_vec();
        for target in CUSTOM_HARNESS_TESTS {
            custom_harness_args.extend(["--test".to_owned(), target.target.to_owned()]);
        }

        Ok(Self {
            nextest_args,
            custom_harness_args,
        })
    }
}

fn append_named_targets(
    args: &mut Vec<String>,
    targets: &BTreeSet<TestTarget>,
    kind: TestTargetKind,
    selector: &str,
) {
    let names = targets
        .iter()
        .filter(|target| target.kind == kind)
        .map(|target| target.name.as_str())
        .collect::<BTreeSet<_>>();
    for name in names {
        args.extend([selector.to_owned(), name.to_owned()]);
    }
}

fn hybrid_test_partition(metadata: &CargoMetadata) -> Result<HybridTestPartition> {
    let workspace_members = metadata
        .workspace_members
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut all = BTreeSet::new();
    for package in metadata
        .packages
        .iter()
        .filter(|package| workspace_members.contains(package.id.as_str()))
    {
        for target in package.targets.iter().filter(|target| target.test) {
            all.insert(TestTarget {
                package: package.name.clone(),
                name: target.name.clone(),
                kind: TestTargetKind::from_metadata(&target.kind)?,
            });
        }
    }

    let custom = CUSTOM_HARNESS_TESTS
        .iter()
        .map(|target| TestTarget {
            package: target.package.to_owned(),
            name: target.target.to_owned(),
            kind: TestTargetKind::Test,
        })
        .collect::<BTreeSet<_>>();
    let missing = custom.difference(&all).collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "configured custom harness targets are not test-enabled workspace targets: {missing:?}"
        ));
    }
    let ordinary = all.difference(&custom).cloned().collect();

    Ok(HybridTestPartition {
        all,
        ordinary,
        custom,
    })
}

#[derive(Debug, Deserialize)]
struct CargoManifest {
    #[serde(default)]
    test: Vec<ManifestTestTarget>,
}

#[derive(Debug, Deserialize)]
struct ManifestTestTarget {
    name: String,
    harness: Option<bool>,
}

fn verify_custom_harness_manifests(metadata: &CargoMetadata) -> Result {
    let workspace_members = metadata
        .workspace_members
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut manifest_custom = BTreeSet::new();
    for package in metadata
        .packages
        .iter()
        .filter(|package| workspace_members.contains(package.id.as_str()))
    {
        let source = fs::read_to_string(&package.manifest_path).map_err(|error| {
            format!(
                "read workspace manifest {}: {error}",
                package.manifest_path.display()
            )
        })?;
        let manifest: CargoManifest = toml::from_str(&source).map_err(|error| {
            format!(
                "parse workspace manifest {}: {error}",
                package.manifest_path.display()
            )
        })?;
        manifest_custom.extend(
            manifest
                .test
                .into_iter()
                .filter(|target| target.harness == Some(false))
                .map(|target| CustomHarnessTestOwned {
                    package: package.name.clone(),
                    target: target.name,
                }),
        );
    }

    let configured = CUSTOM_HARNESS_TESTS
        .iter()
        .map(|target| CustomHarnessTestOwned {
            package: target.package.to_owned(),
            target: target.target.to_owned(),
        })
        .collect::<BTreeSet<_>>();
    if manifest_custom == configured {
        Ok(())
    } else {
        Err(format!(
            "custom harness inventory differs from workspace manifests: configured={configured:?}, manifests={manifest_custom:?}"
        ))
    }
}

#[derive(Debug, Eq, Ord, PartialEq, PartialOrd)]
struct CustomHarnessTestOwned {
    package: String,
    target: String,
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<MetadataPackage>,
    workspace_members: Vec<String>,
    resolve: Option<MetadataResolve>,
}

#[derive(Debug, Deserialize)]
struct MetadataPackage {
    id: String,
    name: String,
    manifest_path: PathBuf,
    targets: Vec<MetadataTarget>,
}

#[derive(Debug, Deserialize)]
struct MetadataTarget {
    name: String,
    kind: Vec<String>,
    test: bool,
}

#[derive(Debug, Deserialize)]
struct MetadataResolve {
    nodes: Vec<MetadataNode>,
}

#[derive(Debug, Deserialize)]
struct MetadataNode {
    id: String,
    deps: Vec<MetadataDependency>,
}

#[derive(Debug, Deserialize)]
struct MetadataDependency {
    pkg: String,
}

#[derive(Debug)]
struct Package {
    id: String,
    name: String,
    root: PathBuf,
}

#[derive(Debug)]
struct Workspace {
    packages: Vec<Package>,
    reverse_dependencies: BTreeMap<String, BTreeSet<String>>,
}

fn load_cargo_metadata(root: &Path) -> Result<CargoMetadata> {
    let output = Command::new("cargo")
        .current_dir(root)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--offline",
            "--all-features",
        ])
        .output()
        .map_err(|error| format!("cargo metadata: could not start: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata --locked --offline --all-features failed with {}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("parse cargo metadata: {error}"))
}

impl Workspace {
    fn load(root: &Path) -> Result<Self> {
        Self::from_metadata(root, load_cargo_metadata(root)?)
    }

    fn from_metadata(root: &Path, metadata: CargoMetadata) -> Result<Self> {
        let member_ids = metadata
            .workspace_members
            .into_iter()
            .collect::<BTreeSet<_>>();
        let mut packages = metadata
            .packages
            .into_iter()
            .filter(|package| member_ids.contains(&package.id))
            .map(|package| {
                let absolute_root = package.manifest_path.parent().ok_or_else(|| {
                    format!(
                        "workspace manifest has no parent: {}",
                        package.manifest_path.display()
                    )
                })?;
                let relative_root = absolute_root.strip_prefix(root).map_err(|_| {
                    format!(
                        "workspace package {} is outside repository root {}",
                        package.manifest_path.display(),
                        root.display()
                    )
                })?;
                Ok(Package {
                    id: package.id,
                    name: package.name,
                    root: relative_root.to_path_buf(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        packages.sort_by(|left, right| {
            right
                .root
                .components()
                .count()
                .cmp(&left.root.components().count())
                .then_with(|| left.name.cmp(&right.name))
        });

        let resolve = metadata
            .resolve
            .ok_or_else(|| "cargo metadata did not include a resolve graph".to_owned())?;
        let mut reverse_dependencies = BTreeMap::<String, BTreeSet<String>>::new();
        for node in resolve
            .nodes
            .into_iter()
            .filter(|node| member_ids.contains(&node.id))
        {
            for dependency in node
                .deps
                .into_iter()
                .filter(|dependency| member_ids.contains(&dependency.pkg))
            {
                reverse_dependencies
                    .entry(dependency.pkg)
                    .or_default()
                    .insert(node.id.clone());
            }
        }
        Ok(Self {
            packages,
            reverse_dependencies,
        })
    }

    fn scope(&self, paths: &BTreeSet<PathBuf>) -> ChangedScope {
        let mut seed_ids = BTreeSet::new();
        let mut direct_names = BTreeSet::new();
        let mut contracts = BTreeSet::new();
        let mut fallback_all = false;
        let mut run_cargo_deny = false;

        for path in paths {
            run_cargo_deny |= is_dependency_or_manifest_path(path);
            if is_root_workspace_path(path) {
                fallback_all = true;
                contracts.extend(Contract::ALL);
                continue;
            }
            if is_documentation_path(path) {
                continue;
            }
            if let Some(package) = self.package_for_path(path) {
                seed_ids.insert(package.id.clone());
                direct_names.insert(package.name.clone());
                continue;
            }
            if let Some(affected) = contracts_for_repository_path(path) {
                contracts.extend(affected);
            } else {
                fallback_all = true;
                contracts.extend(Contract::ALL);
            }
        }

        contracts.extend(contracts_for_packages(&direct_names));
        if fallback_all {
            seed_ids.extend(self.packages.iter().map(|package| package.id.clone()));
        }
        let closure = self.reverse_closure(&seed_ids);
        let packages = self
            .packages
            .iter()
            .filter(|package| closure.contains(&package.id))
            .map(|package| package.name.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        ChangedScope {
            packages,
            contracts,
            run_cargo_deny,
            fallback_all,
        }
    }

    fn package_for_path(&self, path: &Path) -> Option<&Package> {
        self.packages
            .iter()
            .find(|package| path.starts_with(&package.root))
    }

    fn reverse_closure(&self, seeds: &BTreeSet<String>) -> BTreeSet<String> {
        let mut closure = seeds.clone();
        let mut pending = seeds.iter().cloned().collect::<VecDeque<_>>();
        while let Some(package) = pending.pop_front() {
            if let Some(dependents) = self.reverse_dependencies.get(&package) {
                for dependent in dependents {
                    if closure.insert(dependent.clone()) {
                        pending.push_back(dependent.clone());
                    }
                }
            }
        }
        closure
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Contract {
    Numerics,
    Codegen,
    Operations,
    Export,
    Fixtures,
}

impl Contract {
    const ALL: [Self; 5] = [
        Self::Numerics,
        Self::Codegen,
        Self::Operations,
        Self::Export,
        Self::Fixtures,
    ];

    fn run(self, root: &Path) -> Result {
        match self {
            Self::Numerics => numerics::verify_registered_choices(root),
            Self::Codegen => verify_codegen(root),
            Self::Operations => verify_operations(root),
            Self::Export => verify_export_contract(root),
            Self::Fixtures => verify_fixtures(root),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ChangedScope {
    packages: Vec<String>,
    contracts: BTreeSet<Contract>,
    run_cargo_deny: bool,
    fallback_all: bool,
}

fn is_root_workspace_path(path: &Path) -> bool {
    matches!(
        path.to_str(),
        Some("Cargo.toml" | "Cargo.lock" | "rust-toolchain.toml")
    ) || path.starts_with(".cargo")
}

fn is_dependency_or_manifest_path(path: &Path) -> bool {
    path.file_name() == Some(OsStr::new("Cargo.toml"))
        || matches!(path.to_str(), Some("Cargo.lock" | "deny.toml"))
}

fn is_documentation_path(path: &Path) -> bool {
    path.extension() == Some(OsStr::new("md"))
        || path.starts_with("docs")
        || path.starts_with("doc")
        || path.starts_with("dev-doc")
        || matches!(
            path.to_str(),
            Some("LICENSE" | "CONTRIBUTING" | "RELEASE_NOTES")
        )
}

fn contracts_for_repository_path(path: &Path) -> Option<BTreeSet<Contract>> {
    let mut contracts = BTreeSet::new();
    if path.starts_with("fixtures") {
        contracts.insert(Contract::Fixtures);
        return Some(contracts);
    }
    match path.to_str()? {
        "architecture/rusttable-numerics.toml" | "architecture/rusttable-shader-manifest.toml" => {
            contracts.insert(Contract::Numerics);
        }
        "architecture/darktable-operations.toml"
        | "architecture/operation-overrides.toml"
        | "architecture/operation-capabilities.json"
        | "architecture/rusttable-operation-registry.toml"
        | "architecture/rusttable-operation-registry-source-map.toml" => {
            contracts.insert(Contract::Codegen);
            contracts.insert(Contract::Operations);
        }
        "architecture/rusttable-export-contract.json" => {
            contracts.insert(Contract::Export);
        }
        _ => return None,
    }
    Some(contracts)
}

fn contracts_for_packages(packages: &BTreeSet<String>) -> BTreeSet<Contract> {
    let mut contracts = BTreeSet::new();
    for package in packages {
        match package.as_str() {
            "rusttable-core" => {
                contracts.insert(Contract::Numerics);
                contracts.insert(Contract::Export);
            }
            "rusttable-export" => {
                contracts.insert(Contract::Export);
            }
            "rusttable-gpu" => {
                contracts.insert(Contract::Numerics);
            }
            "rusttable-parity" => {
                contracts.insert(Contract::Codegen);
                contracts.insert(Contract::Operations);
            }
            "rusttable-processing" => {
                contracts.insert(Contract::Operations);
            }
            _ => {}
        }
    }
    contracts
}

#[derive(Debug, Eq, PartialEq)]
struct CargoInvocation {
    label: &'static str,
    args: Vec<String>,
    deny_rustdoc_warnings: bool,
}

impl CargoInvocation {
    fn run(&self, root: &Path) -> Result {
        let mut command = Command::new("cargo");
        command.current_dir(root).args(&self.args);
        if self.deny_rustdoc_warnings {
            command.env("RUSTDOCFLAGS", "-Dwarnings");
        }
        run_process_quiet(self.label, &mut command)
    }
}

fn cargo_plan(packages: &[String]) -> Vec<CargoInvocation> {
    if packages.is_empty() {
        return Vec::new();
    }
    let package_args = || {
        packages
            .iter()
            .flat_map(|package| ["--package".to_owned(), package.clone()])
            .collect::<Vec<_>>()
    };
    let mut format = vec!["fmt".to_owned()];
    format.extend(package_args());
    format.extend(["--".to_owned(), "--check".to_owned()]);

    let mut clippy = vec!["clippy".to_owned()];
    clippy.extend(package_args());
    clippy.extend(
        [
            "--all-targets",
            "--all-features",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]
        .map(str::to_owned),
    );

    let mut tests = vec!["test".to_owned()];
    tests.extend(package_args());
    tests.extend(["--all-targets", "--all-features", "--locked"].map(str::to_owned));

    let mut rustdoc = vec!["doc".to_owned()];
    rustdoc.extend(package_args());
    rustdoc.extend(["--all-features", "--no-deps", "--locked"].map(str::to_owned));

    vec![
        CargoInvocation {
            label: "format",
            args: format,
            deny_rustdoc_warnings: false,
        },
        CargoInvocation {
            label: "clippy",
            args: clippy,
            deny_rustdoc_warnings: false,
        },
        CargoInvocation {
            label: "tests",
            args: tests,
            deny_rustdoc_warnings: false,
        },
        CargoInvocation {
            label: "rustdoc",
            args: rustdoc,
            deny_rustdoc_warnings: true,
        },
    ]
}

fn changed_paths(root: &Path, base: &str) -> Result<BTreeSet<PathBuf>> {
    let commit = resolve_base(root, base)?;
    let committed = format!("{commit}...HEAD");
    let queries = [
        vec!["diff", "--name-only", "-z", &committed, "--"],
        vec!["diff", "--name-only", "-z", "--"],
        vec!["diff", "--cached", "--name-only", "-z", "--"],
        vec!["ls-files", "--others", "--exclude-standard", "-z", "--"],
    ];
    let mut paths = BTreeSet::new();
    for args in queries {
        let output = git_output(root, &args)?;
        paths.extend(parse_nul_paths(&output)?);
    }
    Ok(paths)
}

fn resolve_base(root: &Path, base: &str) -> Result<String> {
    let revision = format!("{base}^{{commit}}");
    let output = git_output(
        root,
        &["rev-parse", "--verify", "--end-of-options", &revision],
    )?;
    let commit = std::str::from_utf8(&output)
        .map_err(|error| format!("Git base commit is not UTF-8: {error}"))?
        .trim();
    if commit.is_empty() {
        Err(format!("Git base {base} resolved to an empty commit"))
    } else {
        Ok(commit.to_owned())
    }
}

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .map_err(|error| format!("git {}: could not start: {error}", args.join(" ")))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(format!(
            "git {} failed with {}\n{}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn parse_nul_paths(output: &[u8]) -> Result<Vec<PathBuf>> {
    if output.is_empty() {
        return Ok(Vec::new());
    }
    if output.last() != Some(&0) {
        return Err("Git path output was not NUL-terminated".to_owned());
    }
    output[..output.len() - 1]
        .split(|byte| *byte == 0)
        .map(|raw| {
            if raw.is_empty() {
                return Err("Git path output contained an empty path".to_owned());
            }
            String::from_utf8(raw.to_vec())
                .map(PathBuf::from)
                .map_err(|error| {
                    format!("Git reported a non-UTF-8 path; refusing a partial check: {error}")
                })
        })
        .collect()
}

fn verify_codegen(root: &Path) -> Result {
    codegen::verify_committed(root)
}

fn verify_operations(root: &Path) -> Result {
    operations::verify_operation_manifest(root)
}

fn verify_export_contract(root: &Path) -> Result {
    export_contract::run(root, true)
}

fn verify_fixtures(root: &Path) -> Result {
    fixtures::verify(root, Path::new("fixtures/manifest.toml"))
}

fn dependency_checks(root: &Path) -> Result {
    run_process_quiet(
        "dependency advisories, licenses, and sources",
        Command::new("cargo").current_dir(root).args([
            "deny",
            "check",
            "--hide-inclusion-graph",
            "advisories",
            "bans",
            "licenses",
            "sources",
        ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passing_check(root: &Path) -> Result {
        root.exists()
            .then_some(())
            .ok_or_else(|| "test path does not exist".to_owned())
    }

    fn first_failing_check(_: &Path) -> Result {
        Err("first failure".to_owned())
    }

    fn second_failing_check(_: &Path) -> Result {
        Err("second failure".to_owned())
    }

    fn test_workspace() -> Workspace {
        Workspace {
            packages: vec![
                Package {
                    id: "core-id".to_owned(),
                    name: "core".to_owned(),
                    root: PathBuf::from("crates/core"),
                },
                Package {
                    id: "app-id".to_owned(),
                    name: "app".to_owned(),
                    root: PathBuf::from("crates/app"),
                },
                Package {
                    id: "tool-id".to_owned(),
                    name: "tool".to_owned(),
                    root: PathBuf::from("tools/tool"),
                },
            ],
            reverse_dependencies: BTreeMap::from([
                ("core-id".to_owned(), BTreeSet::from(["app-id".to_owned()])),
                ("app-id".to_owned(), BTreeSet::from(["tool-id".to_owned()])),
            ]),
        }
    }

    #[test]
    fn precommit_plan_has_one_shared_cargo_owner() {
        assert_eq!(
            CHECKS
                .iter()
                .filter(|(label, _)| label.starts_with("cargo "))
                .count(),
            1
        );
        assert_eq!(CHECKS.len(), 7);
        assert_eq!(CHECKS[0].0, "numerical contracts");
        assert_eq!(CHECKS[2].0, "operation codegen");
        assert_eq!(CHECKS[5].0, "fixtures");
    }

    #[test]
    fn parallel_check_runner_aggregates_all_failures() {
        let checks: &[(&str, CheckFn)] = &[
            ("first", first_failing_check),
            ("pass", passing_check),
            ("second", second_failing_check),
        ];
        let error = run_parallel_checks(Path::new("."), checks).expect_err("checks passed");
        assert_eq!(
            error,
            "pre-commit checks failed:\nfirst: first failure\nsecond: second failure"
        );
    }

    #[test]
    fn git_path_parser_preserves_whitespace_and_newlines() {
        let paths = parse_nul_paths(b"plain.rs\0space name.rs\0line\nbreak.rs\0")
            .expect("paths should parse");
        assert_eq!(
            paths,
            ["plain.rs", "space name.rs", "line\nbreak.rs"]
                .map(PathBuf::from)
                .to_vec()
        );
        assert!(parse_nul_paths(b"not-terminated").is_err());
    }

    #[test]
    fn package_mapping_uses_the_containing_workspace_root() {
        let workspace = test_workspace();
        assert_eq!(
            workspace
                .package_for_path(Path::new("crates/core/src/lib.rs"))
                .map(|package| package.name.as_str()),
            Some("core")
        );
        assert!(
            workspace
                .package_for_path(Path::new("scripts/check.sh"))
                .is_none()
        );
    }

    #[test]
    fn reverse_closure_includes_all_workspace_dependents() {
        let workspace = test_workspace();
        let closure = workspace.reverse_closure(&BTreeSet::from(["core-id".to_owned()]));
        assert_eq!(
            closure,
            BTreeSet::from([
                "core-id".to_owned(),
                "app-id".to_owned(),
                "tool-id".to_owned()
            ])
        );
    }

    #[test]
    fn unknown_production_paths_fall_back_to_every_package_and_contract() {
        let workspace = test_workspace();
        let scope = workspace.scope(&BTreeSet::from([PathBuf::from("scripts/new-check.sh")]));
        assert_eq!(scope.packages, ["app", "core", "tool"]);
        assert_eq!(scope.contracts, BTreeSet::from(Contract::ALL));
        assert!(scope.fallback_all);
    }

    #[test]
    fn documentation_paths_select_nothing() {
        let workspace = test_workspace();
        let scope = workspace.scope(&BTreeSet::from([
            PathBuf::from("docs/design.md"),
            PathBuf::from("crates/core/README.md"),
        ]));
        assert_eq!(scope.packages, Vec::<String>::new());
        assert!(scope.contracts.is_empty());
        assert!(!scope.fallback_all);
    }

    #[test]
    fn root_workspace_changes_select_every_package() {
        let workspace = test_workspace();
        let scope = workspace.scope(&BTreeSet::from([
            PathBuf::from("Cargo.toml"),
            PathBuf::from(".cargo/README.md"),
        ]));
        assert_eq!(scope.packages, ["app", "core", "tool"]);
        assert!(scope.run_cargo_deny);
        assert!(scope.fallback_all);
    }

    #[test]
    fn package_changes_select_reverse_dependency_closure() {
        let workspace = test_workspace();
        let scope = workspace.scope(&BTreeSet::from([PathBuf::from("crates/core/src/lib.rs")]));
        assert_eq!(scope.packages, ["app", "core", "tool"]);
        assert!(!scope.run_cargo_deny);
        assert!(!scope.fallback_all);
    }

    #[test]
    fn cargo_deny_is_limited_to_dependency_and_manifest_changes() {
        for path in [
            "Cargo.lock",
            "Cargo.toml",
            "crates/core/Cargo.toml",
            "deny.toml",
        ] {
            assert!(is_dependency_or_manifest_path(Path::new(path)), "{path}");
        }
        for path in ["rust-toolchain.toml", ".cargo/config.toml", "src/lib.rs"] {
            assert!(!is_dependency_or_manifest_path(Path::new(path)), "{path}");
        }
    }

    #[test]
    fn command_plan_scopes_every_cargo_stage() {
        let plan = cargo_plan(&["app".to_owned(), "core".to_owned()]);
        assert_eq!(plan.len(), 4);
        for invocation in &plan {
            assert!(
                invocation
                    .args
                    .windows(2)
                    .any(|pair| pair == ["--package", "app"])
            );
            assert!(
                invocation
                    .args
                    .windows(2)
                    .any(|pair| pair == ["--package", "core"])
            );
            assert!(
                !invocation
                    .args
                    .iter()
                    .any(|argument| argument == "--workspace")
            );
        }
        assert!(
            plan[1]
                .args
                .ends_with(&["-D".to_owned(), "warnings".to_owned()])
        );
        assert!(
            plan[1]
                .args
                .iter()
                .any(|argument| argument == "--all-targets")
        );
        assert!(
            plan[2]
                .args
                .iter()
                .any(|argument| argument == "--all-targets")
        );
        assert!(plan[3].deny_rustdoc_warnings);
    }

    #[test]
    fn hybrid_test_plan_partitions_every_test_target_once() {
        let root = crate::repository_root();
        let metadata = load_cargo_metadata(&root).expect("workspace metadata should load");
        let partition =
            hybrid_test_partition(&metadata).expect("workspace tests should partition cleanly");
        let covered = partition
            .ordinary
            .union(&partition.custom)
            .cloned()
            .collect::<BTreeSet<_>>();

        assert_eq!(covered, partition.all);
        assert!(partition.ordinary.is_disjoint(&partition.custom));
        assert_eq!(partition.custom.len(), CUSTOM_HARNESS_TESTS.len());
        for target in &partition.all {
            let memberships = usize::from(partition.ordinary.contains(target))
                + usize::from(partition.custom.contains(target));
            assert_eq!(
                memberships, 1,
                "target was omitted or duplicated: {target:?}"
            );
        }
    }

    #[test]
    fn custom_harness_manifest_inventory_matches_plan() {
        let root = crate::repository_root();
        let metadata = load_cargo_metadata(&root).expect("workspace metadata should load");
        verify_custom_harness_manifests(&metadata)
            .expect("every harness=false test must be in the conventional plan");
    }

    #[test]
    fn hybrid_test_commands_cover_each_partition_without_overlap() {
        let root = crate::repository_root();
        let metadata = load_cargo_metadata(&root).expect("workspace metadata should load");
        let partition =
            hybrid_test_partition(&metadata).expect("workspace tests should partition cleanly");
        let plan = HybridTestPlan::from_partition(&partition).expect("test plan should build");

        assert!(plan.nextest_args.starts_with(&[
            "nextest".to_owned(),
            "run".to_owned(),
            "--workspace".to_owned(),
            "--all-features".to_owned(),
            "--locked".to_owned(),
        ]));
        assert!(plan.custom_harness_args.starts_with(&[
            "test".to_owned(),
            "--workspace".to_owned(),
            "--all-features".to_owned(),
            "--locked".to_owned(),
        ]));

        for target in &partition.ordinary {
            let covered = match target.kind {
                TestTargetKind::Lib => plan.nextest_args.iter().any(|arg| arg == "--lib"),
                TestTargetKind::Bin => plan.nextest_args.iter().any(|arg| arg == "--bins"),
                TestTargetKind::Test => {
                    selected_name_count(&plan.nextest_args, "--test", &target.name) == 1
                }
                TestTargetKind::Example => {
                    selected_name_count(&plan.nextest_args, "--example", &target.name) == 1
                }
                TestTargetKind::Bench => {
                    selected_name_count(&plan.nextest_args, "--bench", &target.name) == 1
                }
            };
            assert!(
                covered,
                "ordinary target is absent from nextest: {target:?}"
            );
        }
        for target in &partition.custom {
            assert_eq!(
                selected_name_count(&plan.nextest_args, "--test", &target.name),
                0,
                "custom harness leaked into nextest: {target:?}"
            );
            assert_eq!(
                selected_name_count(&plan.custom_harness_args, "--test", &target.name),
                1,
                "custom harness is absent or duplicated: {target:?}"
            );
        }
    }

    fn selected_name_count(args: &[String], selector: &str, name: &str) -> usize {
        args.windows(2)
            .filter(|pair| pair[0] == selector && pair[1] == name)
            .count()
    }

    #[test]
    fn affected_repository_contracts_are_selected_without_all_packages() {
        let workspace = test_workspace();
        let scope = workspace.scope(&BTreeSet::from([PathBuf::from(
            "architecture/rusttable-export-contract.json",
        )]));
        assert!(scope.packages.is_empty());
        assert_eq!(scope.contracts, BTreeSet::from([Contract::Export]));
        assert!(!scope.fallback_all);
    }
}
