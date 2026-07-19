use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use toml::Value;

use crate::cli::{BaselineVerifyArgs, RefreshBaselineArgs, UpgradeDiffArgs};
use crate::commands::{Result, report};
use crate::process::{EnvironmentProfile, ProcessLimits, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

const TOOLCHAIN_PATH: &str = "rust-toolchain.toml";
const BASELINE_PATH: &str = "quality/compiler-baseline.toml";
const WORKSPACE_MANIFEST: &str = "Cargo.toml";
const LOCKFILE: &str = "Cargo.lock";
const BASELINE_SCHEMA: &str = "rusttable.ecosystem-baseline.v2";
const CANDIDATE_SCHEMA: &str = "rusttable.ecosystem-baseline-candidate.v1";

#[derive(Debug, Deserialize)]
struct Toolchain {
    channel: String,
    components: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ToolchainFile {
    toolchain: Toolchain,
}

#[derive(Debug, Deserialize)]
struct Baseline {
    schema: String,
    channel: String,
    release: String,
    rustc_commit: String,
    rustc_commit_date: String,
    cargo_release: String,
    cargo_commit: String,
    llvm_version: String,
    edition: String,
    rust_version: String,
    stabilization_target: String,
    components: Components,
    policy: BaselinePolicy,
}

#[derive(Debug, Deserialize)]
struct Components {
    required: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BaselinePolicy {
    moving_channels: bool,
    nightly_product_code: bool,
    warnings: String,
    #[serde(rename = "unsafe")]
    unsafe_policy: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct EcosystemBaseline {
    pub schema: String,
    pub rust_channel: String,
    pub rust_release: String,
    pub edition: String,
    pub rust_version: String,
    pub compiler_receipt: String,
    pub dependency_policy: String,
    pub workspace_manifest: String,
    pub lockfile: String,
    pub frameworks: BTreeMap<String, FrameworkBaseline>,
    pub compiler: CompilerBaseline,
    pub dependency_policy_contract: DependencyPolicyContract,
    pub features: FeaturePolicy,
    pub capabilities: CapabilityPolicy,
    #[serde(default)]
    pub packages: Vec<PackageBaseline>,
    pub graph: GraphBaseline,
    #[serde(default)]
    pub platforms: BTreeMap<String, PlatformBaseline>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct FrameworkBaseline {
    pub source: String,
    pub version: String,
    pub rev: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    pub platform_features: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CompilerBaseline {
    pub moving_channel: bool,
    pub nightly_product_code: bool,
    pub required_components: Vec<String>,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub config_identity: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct DependencyPolicyContract {
    pub direct_versions: String,
    pub shared_versions: String,
    pub git_sources: String,
    pub path_sources: String,
    pub lockfile_required: bool,
    #[serde(default)]
    pub advisory_policy: String,
    #[serde(default)]
    pub native_policy: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct FeaturePolicy {
    pub unstable_product_features: bool,
    pub nightly_tooling_in_product_graph: bool,
    pub warnings: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CapabilityPolicy {
    pub stable: Vec<String>,
    pub beta: Vec<String>,
    pub nightly_lab: Vec<String>,
    #[serde(default)]
    pub records: Vec<CapabilityRecord>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct CapabilityRecord {
    pub id: String,
    pub status: String,
    pub owner_issue: u64,
    pub evidence: String,
    pub minimum_version: String,
    pub target_scope: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct PackageBaseline {
    pub id: String,
    pub status: String,
    pub owner_issue: u64,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub features: Vec<String>,
    pub target_scope: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct GraphBaseline {
    pub lock_sha256: String,
    pub metadata_sha256: String,
    pub package_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct PlatformBaseline {
    pub target: String,
    pub features: Vec<String>,
    pub owner_issue: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BaselineCandidate {
    schema: String,
    baseline: EcosystemBaseline,
    derived: DerivedFacts,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct DerivedFacts {
    package_count: usize,
    package_ids: Vec<String>,
    metadata_sha256: String,
    lock_sha256: String,
}

#[derive(Debug, Default)]
struct CommandOutput {
    stdout: String,
    stderr: String,
}

pub(crate) fn verify_baseline(
    root: &RepositoryRoot,
    arguments: &BaselineVerifyArgs,
    runner: &ProcessRunner,
) -> Result {
    let toolchain_text = read(root, TOOLCHAIN_PATH)?;
    let baseline_text = read(root, BASELINE_PATH)?;
    let manifest_text = read(root, WORKSPACE_MANIFEST)?;
    let static_findings = validate_documents(&toolchain_text, &baseline_text, &manifest_text)?;
    if !static_findings.is_empty() {
        return Err(static_findings.join("; "));
    }

    let toolchain: Toolchain = toml::from_str::<ToolchainFile>(&toolchain_text)
        .map(|file| file.toolchain)
        .map_err(|error| format!("{TOOLCHAIN_PATH}: invalid TOML: {error}"))?;
    let baseline: Baseline = toml::from_str(&baseline_text)
        .map_err(|error| format!("{BASELINE_PATH}: invalid TOML: {error}"))?;
    let ecosystem_text = read(root, "quality/ecosystem-baseline.toml")?;
    let ecosystem: EcosystemBaseline = toml::from_str(&ecosystem_text).map_err(|error| {
        "quality/ecosystem-baseline.toml: invalid TOML: ".to_owned() + &error.to_string()
    })?;
    let rustc = command(
        runner,
        root,
        "rustup",
        ["run", &toolchain.channel, "rustc", "-vV"],
    )?;
    let cargo = command(
        runner,
        root,
        "rustup",
        ["run", &toolchain.channel, "cargo", "-Vv"],
    )?;
    let components = command(
        runner,
        root,
        "rustup",
        [
            "component",
            "list",
            "--toolchain",
            &toolchain.channel,
            "--installed",
        ],
    )?;
    let runtime_findings =
        validate_runtime(&baseline, &rustc.stdout, &cargo.stdout, &components.stdout);
    if !runtime_findings.is_empty() {
        return Err(runtime_findings.join("; "));
    }
    let metadata = cargo_metadata(root, runner)?;
    let ecosystem_findings =
        validate_ecosystem_baseline(&ecosystem, &metadata, &read(root, LOCKFILE)?);
    if !ecosystem_findings.is_empty() {
        return Err(ecosystem_findings.join("; "));
    }

    let data = serde_json::json!({
        "schema": "rusttable.ecosystem-baseline-verification.v1",
        "channel": baseline.channel,
        "release": baseline.release,
        "cargo_release": baseline.cargo_release,
        "llvm_version": baseline.llvm_version,
        "components": baseline.components.required,
        "edition": baseline.edition,
        "rust_version": baseline.rust_version,
        "stabilization_target": baseline.stabilization_target,
        "policy": {
            "moving_channels": baseline.policy.moving_channels,
            "nightly_product_code": baseline.policy.nightly_product_code,
            "warnings": baseline.policy.warnings,
            "unsafe": baseline.policy.unsafe_policy,
        },
        "source_hashes": {
            "toolchain": digest(toolchain_text.as_bytes()),
            "compiler_baseline": digest(baseline_text.as_bytes()),
            "workspace_manifest": digest(manifest_text.as_bytes()),
            "lockfile": digest(&read(root, LOCKFILE)?.into_bytes()),
            "ecosystem_baseline": digest(ecosystem_text.as_bytes()),
        },
        "graph": ecosystem.graph,
        "diagnostics": "credentials and absolute command paths omitted",
    });
    write_receipt(root, arguments.receipt.as_deref(), &data)?;
    Ok(report(root, "ecosystem.verify-baseline", data))
}

pub(crate) fn upgrade_diff(root: &RepositoryRoot, arguments: &UpgradeDiffArgs) -> Result {
    let candidate_path = if arguments.candidate.is_absolute() {
        arguments.candidate.clone()
    } else {
        root.join(&arguments.candidate)
    };
    let candidate_text = fs::read_to_string(&candidate_path)
        .map_err(|error| format!("{}: {error}", candidate_path.display()))?;
    let candidate: BaselineCandidate = serde_json::from_str(&candidate_text).map_err(|error| {
        format!(
            "{}: invalid candidate JSON: {error}",
            candidate_path.display()
        )
    })?;
    if candidate.schema != CANDIDATE_SCHEMA {
        return Err(format!(
            "{}: unsupported candidate schema",
            candidate_path.display()
        ));
    }
    let accepted_text = read(root, "quality/ecosystem-baseline.toml")?;
    let accepted: EcosystemBaseline = toml::from_str(&accepted_text)
        .map_err(|error| format!("quality/ecosystem-baseline.toml: invalid TOML: {error}"))?;
    let accepted_json = serde_json::to_value(&accepted).map_err(|error| error.to_string())?;
    let candidate_json =
        serde_json::to_value(&candidate.baseline).map_err(|error| error.to_string())?;
    let mut changes = Vec::new();
    semantic_changes("baseline", &accepted_json, &candidate_json, &mut changes);
    let data = serde_json::json!({
        "schema": "rusttable.ecosystem-upgrade-diff.v2",
        "candidate": candidate_path.display().to_string(),
        "changed": !changes.is_empty(),
        "changes": changes,
        "derived": candidate.derived,
        "blocked_surfaces": ["capabilities", "sources", "features", "native", "advisories", "platforms"],
    });
    Ok(report(root, "ecosystem.upgrade-diff", data))
}

pub(crate) fn refresh_baseline(
    root: &RepositoryRoot,
    arguments: &RefreshBaselineArgs,
    runner: &ProcessRunner,
) -> Result {
    let baseline_text = read(root, "quality/ecosystem-baseline.toml")?;
    let baseline: EcosystemBaseline = toml::from_str(&baseline_text)
        .map_err(|error| format!("quality/ecosystem-baseline.toml: invalid TOML: {error}"))?;
    let metadata = cargo_metadata(root, runner)?;
    let lock = read(root, LOCKFILE)?;
    let derived = derive_facts(&metadata, &lock);
    let mut candidate = baseline;
    BASELINE_SCHEMA.clone_into(&mut candidate.schema);
    candidate.graph = GraphBaseline {
        lock_sha256: derived.lock_sha256.clone(),
        metadata_sha256: derived.metadata_sha256.clone(),
        package_count: derived.package_count,
    };
    let data = BaselineCandidate {
        schema: CANDIDATE_SCHEMA.to_owned(),
        baseline: candidate,
        derived,
    };
    let value = serde_json::to_value(&data).map_err(|error| error.to_string())?;
    write_receipt(root, Some(&arguments.candidate), &value)?;
    Ok(report(root, "ecosystem.refresh-baseline", value))
}

pub(crate) fn validate_documents(
    toolchain_text: &str,
    baseline_text: &str,
    manifest_text: &str,
) -> Result<Vec<String>> {
    let toolchain: Toolchain = toml::from_str::<ToolchainFile>(toolchain_text)
        .map(|file| file.toolchain)
        .map_err(|error| format!("{TOOLCHAIN_PATH}: invalid TOML: {error}"))?;
    let baseline: Baseline = toml::from_str(baseline_text)
        .map_err(|error| format!("{BASELINE_PATH}: invalid TOML: {error}"))?;
    let manifest: Value = toml::from_str(manifest_text)
        .map_err(|error| format!("{WORKSPACE_MANIFEST}: invalid TOML: {error}"))?;
    let mut findings = Vec::new();
    if baseline.schema != "rusttable.compiler-baseline.v1" {
        findings.push(format!("{BASELINE_PATH}: schema is not authoritative"));
    }
    if toolchain.channel != baseline.channel {
        findings.push(format!(
            "{TOOLCHAIN_PATH}: channel does not match {BASELINE_PATH}"
        ));
    }
    if !is_dated_beta(&toolchain.channel) {
        findings.push(format!(
            "{TOOLCHAIN_PATH}: product channel must be a dated beta"
        ));
    }
    if baseline.release != baseline.cargo_release || !baseline.release.starts_with("1.98.0-beta.") {
        findings.push(format!(
            "{BASELINE_PATH}: compiler and Cargo release baseline drifted"
        ));
    }
    if baseline.edition != "2024" || baseline.rust_version != "1.98" {
        findings.push(format!(
            "{BASELINE_PATH}: edition/rust-version baseline drifted"
        ));
    }
    if baseline.rustc_commit.is_empty()
        || baseline.rustc_commit_date.len() != 10
        || baseline.cargo_commit.is_empty()
        || baseline.llvm_version.is_empty()
    {
        findings.push(format!(
            "{BASELINE_PATH}: compiler fingerprint is incomplete"
        ));
    }
    if baseline.components.required != toolchain.components {
        findings.push(format!(
            "{BASELINE_PATH}: required components do not match toolchain"
        ));
    }
    if baseline.policy.moving_channels
        || baseline.policy.nightly_product_code
        || baseline.policy.warnings != "deny"
        || baseline.policy.unsafe_policy != "forbid-by-default"
    {
        findings.push(format!(
            "{BASELINE_PATH}: strict product policy is weakened"
        ));
    }
    let rust_version = manifest
        .get("workspace")
        .and_then(|value| value.get("package"))
        .and_then(|value| value.get("rust-version"))
        .and_then(Value::as_str);
    if rust_version != Some("1.98") {
        findings.push(format!(
            "{WORKSPACE_MANIFEST}: workspace rust-version must be 1.98"
        ));
    }
    Ok(findings)
}

fn cargo_metadata(root: &RepositoryRoot, runner: &ProcessRunner) -> Result<String> {
    let result = runner
        .run(
            ProcessRequest::new(
                "cargo",
                ["metadata", "--locked", "--offline", "--format-version", "1"],
            )
            .profile(EnvironmentProfile::RustTool)
            .limits(ProcessLimits {
                max_stdout_bytes: 4 * 1024 * 1024,
                max_stderr_bytes: 256 * 1024,
                timeout: Some(Duration::from_secs(120)),
            })
            .current_dir(root.path().to_path_buf()),
        )
        .map_err(|error| format!("cargo metadata: {error}"))?;
    if !result.receipt.success() {
        return Err(format!(
            "cargo metadata failed ({}): {}",
            result.receipt.status,
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    String::from_utf8(result.stdout)
        .map_err(|error| format!("cargo metadata: invalid UTF-8: {error}"))
}

fn derive_facts(metadata_text: &str, lock_text: &str) -> DerivedFacts {
    let package_ids = serde_json::from_str::<serde_json::Value>(metadata_text)
        .ok()
        .and_then(|metadata| metadata.get("packages").cloned())
        .and_then(|packages| packages.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|package| {
            Some(format!(
                "{}@{}#{}",
                package.get("name")?.as_str()?,
                package.get("version")?.as_str()?,
                package
                    .get("source")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("workspace")
            ))
        })
        .collect::<Vec<_>>();
    let mut package_ids = package_ids;
    package_ids.sort();
    let metadata_identity = package_ids.join("\n");
    DerivedFacts {
        package_count: package_ids.len(),
        package_ids,
        metadata_sha256: digest(metadata_identity.as_bytes()),
        lock_sha256: digest(lock_text.as_bytes()),
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "The baseline gate reports every independent ecosystem contract finding."
)]
fn validate_ecosystem_baseline(
    baseline: &EcosystemBaseline,
    metadata_text: &str,
    lock_text: &str,
) -> Vec<String> {
    let mut findings = Vec::new();
    if baseline.schema != BASELINE_SCHEMA {
        findings.push("quality/ecosystem-baseline.toml: schema is not authoritative".to_owned());
    }
    if baseline.rust_channel != "beta-2026-07-17"
        || baseline.rust_release != "1.98.0-beta.4"
        || baseline.edition != "2024"
        || baseline.rust_version != "1.98"
    {
        findings.push("quality/ecosystem-baseline.toml: pinned beta identity drifted".to_owned());
    }
    if baseline.compiler.moving_channel
        || baseline.compiler.nightly_product_code
        || baseline.features.unstable_product_features
        || baseline.features.nightly_tooling_in_product_graph
        || baseline.features.warnings != "deny"
    {
        findings
            .push("quality/ecosystem-baseline.toml: strict compiler policy is weakened".to_owned());
    }
    if baseline.dependency_policy_contract.direct_versions != "exact"
        || baseline.dependency_policy_contract.shared_versions != "workspace.dependencies"
        || baseline.dependency_policy_contract.git_sources != "immutable-rev-only"
        || baseline.dependency_policy_contract.path_sources != "workspace-local-only"
        || !baseline.dependency_policy_contract.lockfile_required
    {
        findings
            .push("quality/ecosystem-baseline.toml: dependency policy is incomplete".to_owned());
    }
    for (name, framework) in &baseline.frameworks {
        if framework.version.is_empty()
            || framework.source.is_empty()
            || framework.platform_features.is_empty()
        {
            findings.push(format!(
                "ecosystem framework {name}: source/version/platform contract is incomplete"
            ));
        }
    }
    for package in &baseline.packages {
        if !matches!(package.status.as_str(), "adopted" | "deferred" | "rejected") {
            findings.push(format!("ecosystem package {}: invalid status", package.id));
        }
        if package.owner_issue == 0
            || package.target_scope.is_empty()
            || package.rationale.is_empty()
        {
            findings.push(format!(
                "ecosystem package {}: owner, target scope, and rationale are required",
                package.id
            ));
        }
        if package.status == "adopted" && (package.version.is_none() || package.source.is_none()) {
            findings.push(format!(
                "ecosystem package {}: adopted entries require version and source",
                package.id
            ));
        }
    }
    for capability in &baseline.capabilities.records {
        if !matches!(
            capability.status.as_str(),
            "adopted" | "deferred" | "rejected"
        ) || capability.id.is_empty()
            || capability.owner_issue == 0
            || capability.evidence.is_empty()
            || capability.minimum_version.is_empty()
            || capability.target_scope.is_empty()
            || capability.rationale.is_empty()
        {
            findings.push(format!(
                "capability {}: status, owner, evidence, version, target, and rationale are required",
                capability.id
            ));
        }
    }
    for required in [
        "rust",
        "cargo",
        "llvm",
        "iced",
        "iced_test",
        "wgpu",
        "naga",
        "winit",
        "tokio",
        "tokio-util",
        "rayon",
        "redb",
        "wasmtime",
        "mlua",
        "image",
        "serde",
        "postcard",
        "toml",
    ] {
        if !baseline
            .packages
            .iter()
            .any(|package| package.id == required)
            && !baseline.frameworks.contains_key(required)
        {
            findings.push(format!(
                "ecosystem package {required}: required inventory entry is missing"
            ));
        }
    }
    for target in ["linux", "macos", "windows"] {
        if !baseline.platforms.contains_key(target) {
            findings.push(format!(
                "ecosystem platform {target}: feature ownership is missing"
            ));
        }
    }
    let metadata = match serde_json::from_str::<serde_json::Value>(metadata_text) {
        Ok(metadata) => metadata,
        Err(error) => {
            findings.push(format!("cargo metadata: invalid JSON: {error}"));
            return findings;
        }
    };
    let facts = derive_facts(metadata_text, lock_text);
    if baseline.graph.package_count != facts.package_count
        || baseline.graph.lock_sha256 != facts.lock_sha256
        || baseline.graph.metadata_sha256 != facts.metadata_sha256
    {
        findings.push(
            "ecosystem graph: accepted identity does not match cargo metadata/Cargo.lock"
                .to_owned(),
        );
    }
    let iced = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .find(|package| package.get("name").and_then(serde_json::Value::as_str) == Some("iced"));
    if let Some(framework) = baseline.frameworks.get("iced")
        && iced
            .and_then(|package| package.get("version"))
            .and_then(serde_json::Value::as_str)
            != Some(framework.version.trim_start_matches('='))
    {
        findings.push("ecosystem framework iced: resolved version drifted".to_owned());
    }
    let metadata_packages = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    for package in baseline
        .packages
        .iter()
        .filter(|package| package.status == "adopted")
    {
        let Some(expected_version) = package.version.as_deref() else {
            continue;
        };
        let Some(actual) = metadata_packages.iter().find(|actual| {
            actual.get("name").and_then(serde_json::Value::as_str) == Some(package.id.as_str())
        }) else {
            if !matches!(
                package.id.as_str(),
                "rust" | "cargo" | "llvm" | "native-surfaces"
            ) {
                findings.push(format!(
                    "ecosystem package {}: adopted package is absent from metadata",
                    package.id
                ));
            }
            continue;
        };
        if actual.get("version").and_then(serde_json::Value::as_str)
            != Some(expected_version.trim_start_matches('='))
        {
            findings.push(format!(
                "ecosystem package {}: resolved version drifted",
                package.id
            ));
        }
        if let Some(expected_source) = package.source.as_deref() {
            let actual_source = actual
                .get("source")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("workspace");
            let source_matches = if expected_source == "registry+crates.io" {
                actual_source.starts_with("registry+")
            } else {
                actual_source
                    .starts_with(expected_source.split('#').next().unwrap_or(expected_source))
                    && expected_source
                        .split_once('#')
                        .is_none_or(|(_, rev)| actual_source.contains(rev))
            };
            if !source_matches {
                findings.push(format!(
                    "ecosystem package {}: resolved source drifted",
                    package.id
                ));
            }
        }
        let declared_features = actual
            .get("features")
            .and_then(serde_json::Value::as_object);
        for feature in &package.features {
            if !declared_features.is_some_and(|features| features.contains_key(feature)) {
                findings.push(format!(
                    "ecosystem package {}: feature {feature} is not declared by resolved package",
                    package.id
                ));
            }
        }
    }
    findings
}

fn semantic_changes(
    path: &str,
    accepted: &serde_json::Value,
    candidate: &serde_json::Value,
    changes: &mut Vec<serde_json::Value>,
) {
    match (accepted, candidate) {
        (serde_json::Value::Object(left), serde_json::Value::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .collect::<std::collections::BTreeSet<_>>();
            for key in keys {
                let next = format!("{path}.{key}");
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => semantic_changes(&next, left, right, changes),
                    (Some(left), None) => changes
                        .push(serde_json::json!({"kind":"removed","path":next,"accepted":left})),
                    (None, Some(right)) => changes
                        .push(serde_json::json!({"kind":"added","path":next,"candidate":right})),
                    (None, None) => {}
                }
            }
        }
        (left, right) if left != right => changes.push(serde_json::json!({
            "kind": "changed",
            "path": path,
            "accepted": left,
            "candidate": right,
        })),
        _ => {}
    }
}

fn validate_runtime(
    baseline: &Baseline,
    rustc_output: &str,
    cargo_output: &str,
    components_output: &str,
) -> Vec<String> {
    let rustc = fields(rustc_output);
    let cargo = fields(cargo_output);
    let mut findings = Vec::new();
    compare(
        &mut findings,
        "rustc release",
        rustc.get("release"),
        &baseline.release,
    );
    compare(
        &mut findings,
        "rustc commit",
        rustc.get("commit-hash"),
        &baseline.rustc_commit,
    );
    compare(
        &mut findings,
        "rustc commit-date",
        rustc.get("commit-date"),
        &baseline.rustc_commit_date,
    );
    compare(
        &mut findings,
        "LLVM version",
        rustc.get("LLVM version"),
        &baseline.llvm_version,
    );
    compare(
        &mut findings,
        "Cargo release",
        cargo.get("release"),
        &baseline.cargo_release,
    );
    compare(
        &mut findings,
        "Cargo commit",
        cargo.get("commit-hash"),
        &baseline.cargo_commit,
    );
    for component in &baseline.components.required {
        let marker = format!("{component} (installed)");
        if !components_output.lines().any(|line| {
            line.starts_with(&marker)
                || line == component
                || line.starts_with(&format!("{component}-"))
                || (component == "llvm-tools-preview" && line.starts_with("llvm-tools-"))
        }) {
            findings.push(format!("component {component} is not installed"));
        }
    }
    findings
}

fn compare(findings: &mut Vec<String>, name: &str, actual: Option<&String>, expected: &str) {
    match actual {
        Some(value)
            if value == expected || (name.contains("commit") && value.starts_with(expected)) => {}
        Some(value) => findings.push(format!(
            "{name} mismatch: expected {expected}, found {value}"
        )),
        None => findings.push(format!("{name} is missing from tool output")),
    }
}

fn fields(output: &str) -> BTreeMap<String, String> {
    output
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}

fn command<const N: usize>(
    runner: &ProcessRunner,
    root: &RepositoryRoot,
    program: &str,
    args: [&str; N],
) -> Result<CommandOutput> {
    let result = runner
        .run(ProcessRequest::new(program, args).current_dir(root.path().to_path_buf()))
        .map_err(|error| format!("{program}: {error}"))?;
    let output = CommandOutput {
        stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
    };
    if !result.receipt.success() {
        return Err(format!(
            "{program}: command failed ({}): {}",
            result.receipt.status,
            output.stderr.trim()
        ));
    }
    Ok(output)
}

fn read(root: &RepositoryRoot, path: &str) -> Result<String> {
    fs::read_to_string(root.join(path)).map_err(|error| format!("{path}: {error}"))
}

fn write_receipt(
    root: &RepositoryRoot,
    path: Option<&Path>,
    data: &serde_json::Value,
) -> Result<()> {
    if let Some(path) = path {
        let serialized = serde_json::to_vec_pretty(data).map_err(|error| error.to_string())?;
        let destination = if path.is_absolute() {
            path.to_owned()
        } else {
            root.join(path)
        };
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        fs::write(&destination, serialized)
            .map_err(|error| format!("{}: {error}", destination.display()))?;
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn is_dated_beta(channel: &str) -> bool {
    let Some(date) = channel.strip_prefix("beta-") else {
        return false;
    };
    date.len() == 10
        && date.as_bytes().get(4) == Some(&b'-')
        && date.as_bytes().get(7) == Some(&b'-')
        && date
            .chars()
            .enumerate()
            .all(|(index, character)| matches!(index, 4 | 7) || character.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::{validate_documents, validate_runtime};

    const TOOLCHAIN: &str = r#"[toolchain]
channel = "beta-2026-07-17"
components = ["clippy", "rust-docs", "rust-src", "rustfmt", "llvm-tools-preview"]
"#;
    const BASELINE: &str = r#"schema = "rusttable.compiler-baseline.v1"
channel = "beta-2026-07-17"
release = "1.98.0-beta.4"
rustc_commit = "abcdef"
rustc_commit_date = "2026-07-16"
cargo_release = "1.98.0-beta.4"
cargo_commit = "123456"
llvm_version = "22.1.8"
edition = "2024"
rust_version = "1.98"
stabilization_target = "2026-08-20"
[components]
required = ["clippy", "rust-docs", "rust-src", "rustfmt", "llvm-tools-preview"]
[policy]
moving_channels = false
nightly_product_code = false
warnings = "deny"
unsafe = "forbid-by-default"
"#;
    const MANIFEST: &str = "[workspace.package]\nrust-version = \"1.98\"\n";

    #[test]
    fn exact_dated_beta_documents_pass() {
        assert!(
            validate_documents(TOOLCHAIN, BASELINE, MANIFEST)
                .expect("valid documents")
                .is_empty()
        );
    }

    #[test]
    fn moving_channel_is_rejected() {
        let toolchain = TOOLCHAIN.replace("beta-2026-07-17", "beta");
        let findings =
            validate_documents(&toolchain, BASELINE, MANIFEST).expect("parsed documents");
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("dated beta"))
        );
    }

    #[test]
    fn release_or_component_drift_is_rejected() {
        let baseline = BASELINE
            .replace(
                "\nrelease = \"1.98.0-beta.4\"",
                "\nrelease = \"1.98.0-beta.5\"",
            )
            .replace("llvm-tools-preview", "rust-analyzer");
        let findings =
            validate_documents(TOOLCHAIN, &baseline, MANIFEST).expect("parsed documents");
        assert!(findings.iter().any(|finding| finding.contains("release")));
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("components"))
        );
    }

    #[test]
    fn runtime_fingerprint_checks_every_required_identity() {
        let rustc = "release: 1.98.0-beta.4\ncommit-hash: abcdef123\ncommit-date: 2026-07-16\nLLVM version: 22.1.8\n";
        let cargo = "release: 1.98.0-beta.4\ncommit-hash: 123456789\n";
        let components = "clippy (installed)\nrust-docs (installed)\nrust-src (installed)\nrustfmt (installed)\nllvm-tools-preview (installed)\n";
        let baseline: super::Baseline = toml::from_str(BASELINE).expect("baseline");
        assert!(validate_runtime(&baseline, rustc, cargo, components).is_empty());
    }
}
