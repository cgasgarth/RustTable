use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::{Result, report};
use crate::process::{EnvironmentProfile, ProcessLimits, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

const POLICY_PATH: &str = "architecture/native-boundaries.toml";
const OWNERSHIP_PATH: &str = "architecture/native-ownership.toml";
const DEPENDENCY_POLICY_PATH: &str = "quality/dependency-sources.toml";
const COMPILER_BASELINE_PATH: &str = "quality/compiler-baseline.toml";
const METADATA_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Deserialize)]
struct Policy {
    schema: String,
    owner_issue: u64,
    primary_compiler: String,
    ownership_map: String,
    workspace_dag: String,
    dependency_policy: String,
    reference_clone: String,
    forbidden_extensions: Vec<String>,
    forbidden_filenames: Vec<String>,
    forbidden_direct_dependencies: Vec<String>,
    forbidden_source_tokens: Vec<String>,
    forbidden_reference_tokens: Vec<String>,
    allowed_unsafe_crates: Vec<String>,
    forbidden_adapter_consumers: Vec<String>,
    adapters: Vec<AdapterSpec>,
    migration_receipts: Vec<MigrationReceipt>,
}

#[derive(Debug, Deserialize)]
struct AdapterSpec {
    id: String,
    adapter_crate: String,
    native_library: String,
    purpose: String,
    owning_api: String,
    allowed_dependents: Vec<String>,
    build_features: Vec<String>,
    unsafe_surface: String,
    callback_model: String,
    thread_model: String,
    removal_condition: String,
    status: String,
}

#[derive(Debug, Deserialize)]
struct MigrationReceipt {
    id: String,
    from_crate: String,
    target_adapter: String,
    native_dependency: String,
    owner_issue: u64,
    status: String,
    evidence: Vec<String>,
    expires: String,
    reason: String,
}

#[derive(Debug, Deserialize)]
struct OwnershipMap {
    schema: String,
    generated: bool,
    generated_by: String,
    source_manifest: String,
    workspace_dag: String,
    dependency_policy: String,
    workspace_members: Vec<String>,
    native_packages: Vec<NativePackageOwner>,
}

#[derive(Debug, Deserialize)]
struct NativePackageOwner {
    name: String,
    owner_issue: u64,
    owner_crate: String,
    classification: String,
    evidence: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DependencyPolicy {
    #[serde(default)]
    native_packages: Vec<NativeDependency>,
}

#[derive(Debug, Deserialize)]
struct NativeDependency {
    name: String,
    owner_issue: u64,
    rationale: String,
}

#[derive(Debug, Deserialize)]
struct CargoMetadata {
    packages: Vec<CargoPackage>,
    workspace_members: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct CargoPackage {
    id: String,
    name: String,
    #[serde(default)]
    links: Option<String>,
    #[serde(default)]
    dependencies: Vec<CargoDependency>,
}

#[derive(Debug, Deserialize)]
struct CargoDependency {
    name: String,
}

#[derive(Debug)]
struct CompilerFingerprint {
    release: String,
    commit_hash: String,
    commit_date: String,
    llvm_version: String,
}

pub(super) fn run(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    receipt_path: Option<&Path>,
) -> Result {
    let policy = parse_toml::<Policy>(root, POLICY_PATH)?;
    let ownership = parse_toml::<OwnershipMap>(root, OWNERSHIP_PATH)?;
    let dependency_policy = parse_toml::<DependencyPolicy>(root, DEPENDENCY_POLICY_PATH)?;
    let compiler = verify_compiler(root, runner, &policy)?;
    let metadata = cargo_metadata(root, runner)?;
    let files = tracked_files(root, runner)?;
    let mut findings = validate_policy(&policy, &ownership, root);
    findings.extend(validate_ownership_map(
        &ownership,
        &metadata,
        &dependency_policy,
        root,
    ));
    findings.extend(scan_repository(root, &files, &policy));
    findings.extend(validate_metadata(&metadata, &policy, &ownership));
    if !findings.is_empty() {
        return Err(format_findings(findings));
    }

    let native_packages = native_package_names(&metadata);
    let workspace_members = workspace_package_names(&metadata);
    let data = serde_json::json!({
        "schema": "rusttable.native-boundaries-receipt.v1",
        "status": "pass",
        "owner_issue": policy.owner_issue,
        "compiler": {
            "release": compiler.release,
            "commit_hash": compiler.commit_hash,
            "commit_date": compiler.commit_date,
            "llvm_version": compiler.llvm_version,
        },
        "workspace_members": workspace_members,
        "native_packages": native_packages,
        "ownership_map_sha256": sha256_file(root.join(OWNERSHIP_PATH))?,
        "policy_sha256": sha256_file(root.join(POLICY_PATH))?,
        "tracked_files_checked": files.len(),
        "migration_receipts": policy.migration_receipts.iter().map(|receipt| receipt.id.as_str()).collect::<Vec<_>>(),
        "reference_clone": {
            "path": policy.reference_clone,
            "policy": "separate-local-read-only-reference; never a RustTable input",
        },
    });
    write_receipt(root, receipt_path, &data)?;
    Ok(report(root, "repo.verify-native-boundaries", data))
}

fn parse_toml<T: for<'de> Deserialize<'de>>(root: &RepositoryRoot, path: &str) -> Result<T> {
    let source = fs::read_to_string(root.join(path))
        .map_err(|error| format!("{path}: cannot read policy: {error}"))?;
    toml::from_str(&source).map_err(|error| format!("{path}: invalid TOML: {error}"))
}

fn verify_compiler(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    policy: &Policy,
) -> Result<CompilerFingerprint> {
    if policy.primary_compiler != COMPILER_BASELINE_PATH {
        return Err(format!(
            "{POLICY_PATH}: primary compiler must be {COMPILER_BASELINE_PATH}"
        ));
    }
    let baseline: toml::Value = parse_toml(root, COMPILER_BASELINE_PATH)?;
    let output = run_command(
        runner,
        ProcessRequest::new("rustc", ["-vV"])
            .profile(EnvironmentProfile::RustTool)
            .current_dir(root.path())
            .limits(ProcessLimits {
                max_stdout_bytes: 16 * 1024,
                max_stderr_bytes: 16 * 1024,
                timeout: METADATA_TIMEOUT,
            }),
        "rustc -vV",
    )?;
    let fields = output
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect::<BTreeMap<_, _>>();
    let compiler = CompilerFingerprint {
        release: required_field(&fields, "release")?,
        commit_hash: required_field(&fields, "commit-hash")?,
        commit_date: required_field(&fields, "commit-date")?,
        llvm_version: required_field(&fields, "LLVM version")?,
    };
    for (field, actual, expected) in [
        (
            "release",
            compiler.release.as_str(),
            baseline_string(&baseline, "release")?,
        ),
        (
            "commit-hash",
            compiler.commit_hash.as_str(),
            baseline_string(&baseline, "rustc_commit")?,
        ),
        (
            "commit-date",
            compiler.commit_date.as_str(),
            baseline_string(&baseline, "rustc_commit_date")?,
        ),
        (
            "LLVM version",
            compiler.llvm_version.as_str(),
            baseline_string(&baseline, "llvm_version")?,
        ),
    ] {
        if actual != expected {
            return Err(format!(
                "primary compiler fingerprint mismatch for {field}: expected {expected}, found {actual}"
            ));
        }
    }
    Ok(compiler)
}

fn cargo_metadata(root: &RepositoryRoot, runner: &ProcessRunner) -> Result<CargoMetadata> {
    let output = run_command(
        runner,
        ProcessRequest::new(
            "cargo",
            [
                "metadata",
                "--locked",
                "--all-features",
                "--format-version",
                "1",
            ],
        )
        .profile(EnvironmentProfile::RustTool)
        .current_dir(root.path())
        .limits(ProcessLimits {
            max_stdout_bytes: 4 * 1024 * 1024,
            max_stderr_bytes: 256 * 1024,
            timeout: METADATA_TIMEOUT,
        }),
        "cargo metadata",
    )?;
    serde_json::from_str(&output).map_err(|error| format!("cargo metadata: invalid JSON: {error}"))
}

fn tracked_files(root: &RepositoryRoot, runner: &ProcessRunner) -> Result<Vec<String>> {
    let output = run_command(
        runner,
        ProcessRequest::new("git", ["ls-files", "-z"])
            .profile(EnvironmentProfile::GitTool)
            .current_dir(root.path())
            .limits(ProcessLimits {
                max_stdout_bytes: 4 * 1024 * 1024,
                max_stderr_bytes: 64 * 1024,
                timeout: METADATA_TIMEOUT,
            }),
        "git ls-files",
    )?;
    Ok(output
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

fn validate_policy(
    policy: &Policy,
    ownership: &OwnershipMap,
    root: &RepositoryRoot,
) -> Vec<String> {
    let mut findings = Vec::new();
    if policy.schema != "rusttable.native-boundaries.v1" || policy.owner_issue != 168 {
        findings.push(format!(
            "{POLICY_PATH}: schema or owner issue is not authoritative"
        ));
    }
    for path in [
        policy.primary_compiler.as_str(),
        policy.ownership_map.as_str(),
        policy.workspace_dag.as_str(),
        policy.dependency_policy.as_str(),
    ] {
        if !root.join(path).is_file() {
            findings.push(format!("{POLICY_PATH}: evidence path is missing: {path}"));
        }
    }
    if policy.reference_clone.is_empty()
        || policy.forbidden_extensions.is_empty()
        || policy.forbidden_filenames.is_empty()
        || policy.forbidden_direct_dependencies.is_empty()
        || policy.forbidden_source_tokens.is_empty()
        || policy.forbidden_reference_tokens.is_empty()
        || policy.allowed_unsafe_crates.is_empty()
        || policy.forbidden_adapter_consumers.is_empty()
    {
        findings.push(format!(
            "{POLICY_PATH}: fail-closed policy lists must not be empty"
        ));
    }
    let mut ids = BTreeSet::new();
    for adapter in &policy.adapters {
        if adapter.id.is_empty()
            || !ids.insert(&adapter.id)
            || adapter.adapter_crate.is_empty()
            || adapter.native_library.is_empty()
            || adapter.purpose.is_empty()
            || adapter.owning_api.is_empty()
            || adapter.allowed_dependents.is_empty()
            || adapter.build_features.is_empty()
            || adapter.unsafe_surface.is_empty()
            || adapter.callback_model.is_empty()
            || adapter.thread_model.is_empty()
            || adapter.removal_condition.is_empty()
            || !adapter.adapter_crate.ends_with("-native")
            || !["reserved", "approved", "transitional"].contains(&adapter.status.as_str())
        {
            findings.push(format!(
                "{POLICY_PATH}: adapter exception is incomplete or unsafe"
            ));
            continue;
        }
        if !adapter
            .callback_model
            .to_ascii_lowercase()
            .contains("panic")
            || !adapter
                .unsafe_surface
                .to_ascii_lowercase()
                .contains("opaque")
        {
            findings.push(format!(
                "{POLICY_PATH}: adapter {} must document panic containment and opaque handles",
                adapter.id
            ));
        }
        if adapter.status == "approved"
            && !ownership.workspace_members.contains(&adapter.adapter_crate)
        {
            findings.push(format!(
                "{POLICY_PATH}: approved adapter {} is not a workspace member",
                adapter.adapter_crate
            ));
        }
    }
    let mut receipt_ids = BTreeSet::new();
    for receipt in &policy.migration_receipts {
        if receipt.id.is_empty()
            || !receipt_ids.insert(&receipt.id)
            || receipt.from_crate.is_empty()
            || !receipt.target_adapter.ends_with("-native")
            || receipt.native_dependency.is_empty()
            || receipt.owner_issue != 168
            || receipt.status != "accepted-transitional"
            || receipt.evidence.is_empty()
            || receipt.expires.len() != 10
            || receipt.reason.is_empty()
        {
            findings.push(format!("{POLICY_PATH}: migration receipt is incomplete"));
        }
        if !policy_has_adapter(policy, &receipt.target_adapter, "transitional") {
            findings.push(format!(
                "{POLICY_PATH}: migration {} has no matching transitional adapter reservation",
                receipt.id
            ));
        }
        for evidence in &receipt.evidence {
            if evidence.starts_with('/') || !root.join(evidence).is_file() {
                findings.push(format!(
                    "{POLICY_PATH}: migration evidence is missing or absolute: {evidence}"
                ));
            }
        }
    }
    findings
}

fn policy_has_adapter(policy: &Policy, crate_name: &str, status: &str) -> bool {
    policy
        .adapters
        .iter()
        .any(|adapter| adapter.adapter_crate == crate_name && adapter.status == status)
}

fn validate_ownership_map(
    ownership: &OwnershipMap,
    metadata: &CargoMetadata,
    dependency_policy: &DependencyPolicy,
    root: &RepositoryRoot,
) -> Vec<String> {
    let mut findings = Vec::new();
    if ownership.schema != "rusttable.native-ownership.v1"
        || !ownership.generated
        || ownership.generated_by != "cargo xtask repo verify-native-boundaries"
        || ownership.source_manifest != POLICY_PATH
        || ownership.workspace_dag != "architecture/workspace-dag.toml"
        || ownership.dependency_policy != DEPENDENCY_POLICY_PATH
    {
        findings.push(format!("{OWNERSHIP_PATH}: generated provenance is invalid"));
    }
    let expected_members = workspace_package_names(metadata);
    let actual_members = ownership
        .workspace_members
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if actual_members != expected_members.iter().cloned().collect() {
        findings.push(format!(
            "{OWNERSHIP_PATH}: workspace ownership map is stale"
        ));
    }
    let actual_native = native_package_names(metadata);
    let mapped_native = ownership
        .native_packages
        .iter()
        .map(|package| package.name.clone())
        .collect::<BTreeSet<_>>();
    if mapped_native != actual_native {
        findings.push(format!(
            "{OWNERSHIP_PATH}: native package map differs from Cargo metadata"
        ));
    }
    let policy_native = dependency_policy
        .native_packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut owners = BTreeSet::new();
    for package in &ownership.native_packages {
        if !owners.insert(&package.name)
            || package.owner_issue == 0
            || package.owner_crate.is_empty()
            || package.evidence.is_empty()
            || !["adapter-transitive", "platform-transitive"]
                .contains(&package.classification.as_str())
            || !policy_native.contains(package.name.as_str())
        {
            findings.push(format!(
                "{OWNERSHIP_PATH}: native ownership entry is incomplete"
            ));
        }
        for evidence in &package.evidence {
            if evidence.starts_with('/') || !root.join(evidence).is_file() {
                findings.push(format!("{OWNERSHIP_PATH}: missing evidence {evidence}"));
            }
        }
        if let Some(declared) = dependency_policy
            .native_packages
            .iter()
            .find(|declared| declared.name == package.name)
            && (declared.owner_issue != package.owner_issue || declared.rationale.is_empty())
        {
            findings.push(format!(
                "{OWNERSHIP_PATH}: owner issue differs for {}",
                package.name
            ));
        }
    }
    findings
}

fn scan_repository(root: &RepositoryRoot, files: &[String], policy: &Policy) -> Vec<String> {
    let mut findings = Vec::new();
    for path in files {
        if path.starts_with("crates/xtask/tests/fixtures/native-boundaries/") {
            continue;
        }
        let allowed_adapter = adapter_path_allowed(path, policy);
        if forbidden_path(path, policy) && !allowed_adapter {
            findings.push(format!("forbidden native/build path: {path}"));
        }
        let extension = Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default();
        if policy
            .forbidden_extensions
            .iter()
            .any(|candidate| candidate == extension)
            && !allowed_adapter
        {
            findings.push(format!("forbidden native source: {path}"));
        }
        if policy
            .forbidden_filenames
            .iter()
            .any(|name| path.ends_with(name))
            && !allowed_adapter
        {
            findings.push(format!("forbidden native/build file: {path}"));
        }
        if !scan_text_file(path) {
            continue;
        }
        let Ok(source) = fs::read_to_string(root.join(path)) else {
            findings.push(format!("cannot read source for boundary scan: {path}"));
            continue;
        };
        findings.extend(scan_source_text(path, &source, policy, allowed_adapter));
    }
    findings
}

fn scan_source_text(
    path: &str,
    source: &str,
    policy: &Policy,
    allowed_adapter: bool,
) -> Vec<String> {
    if path == POLICY_PATH
        || path == OWNERSHIP_PATH
        || path == DEPENDENCY_POLICY_PATH
        || path.starts_with("architecture/")
    {
        return Vec::new();
    }
    let mut findings = Vec::new();
    let is_policy_scanner = path.starts_with("scripts/check-")
        || path.starts_with("scripts/test-native-")
        || path == "crates/xtask/src/commands/channels.rs"
        || path == "crates/xtask/src/commands/native_boundaries.rs";
    if !is_policy_scanner
        && policy
            .forbidden_reference_tokens
            .iter()
            .any(|token| source.contains(token))
    {
        findings.push(format!("reference-tree path escapes are forbidden: {path}"));
    }
    if !is_policy_scanner && !allowed_adapter {
        for token in &policy.forbidden_source_tokens {
            if source.contains(token) {
                findings.push(format!(
                    "forbidden native/compiler token {token:?} in {path}"
                ));
            }
        }
    }
    if path.ends_with(".rs") {
        let unsafe_item = [
            concat!("unsafe", " {"),
            "unsafe fn",
            "unsafe trait",
            "unsafe impl",
            "unsafe extern",
            "#[unsafe(",
        ]
        .iter()
        .any(|token| source.contains(token));
        let allowed_unsafe_crate = path
            .strip_prefix("crates/")
            .and_then(|path| path.split('/').next())
            .is_some_and(|crate_name| {
                policy
                    .allowed_unsafe_crates
                    .iter()
                    .any(|allowed| allowed == crate_name)
            });
        if unsafe_item && !is_policy_scanner && !allowed_adapter && !allowed_unsafe_crate {
            findings.push(format!(
                "unsafe Rust is outside an approved boundary: {path}"
            ));
        }
        if allowed_adapter
            && ["*mut ", "*const ", "NonNull", "c_void", "c_int", "union "]
                .iter()
                .any(|token| source.contains(token))
        {
            findings.push(format!(
                "adapter public/raw layout surface requires a safe wrapper: {path}"
            ));
        }
    }
    findings
}

fn validate_metadata(
    metadata: &CargoMetadata,
    policy: &Policy,
    ownership: &OwnershipMap,
) -> Vec<String> {
    let workspace_ids = metadata.workspace_members.iter().collect::<BTreeSet<_>>();
    let mut findings = Vec::new();
    let transitional = policy
        .migration_receipts
        .iter()
        .map(|receipt| {
            (
                receipt.from_crate.as_str(),
                receipt.native_dependency.as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    for package in &metadata.packages {
        if !workspace_ids.contains(&package.id) {
            continue;
        }
        for dependency in &package.dependencies {
            let lower = dependency.name.to_ascii_lowercase();
            let adapter = policy
                .adapters
                .iter()
                .find(|adapter| adapter.adapter_crate == dependency.name);
            let direct_native = policy
                .forbidden_direct_dependencies
                .iter()
                .any(|candidate| {
                    candidate == &lower || lower.ends_with("-sys") || lower.ends_with("_sys")
                })
                || lower == "mlua";
            if direct_native
                && !transitional.contains(&(package.name.as_str(), dependency.name.as_str()))
                && !package.name.ends_with("-native")
            {
                findings.push(format!(
                    "native dependency {} is not isolated in an adapter crate (consumer {})",
                    dependency.name, package.name
                ));
            }
            if dependency.name.ends_with("-native")
                && policy.forbidden_adapter_consumers.contains(&package.name)
            {
                findings.push(format!(
                    "forbidden adapter edge: {} -> {}",
                    package.name, dependency.name
                ));
            }
            if let Some(adapter) = adapter
                && !adapter.allowed_dependents.contains(&package.name)
            {
                findings.push(format!(
                    "adapter {} is consumed by an unapproved crate: {}",
                    dependency.name, package.name
                ));
            }
        }
        if package.name.ends_with("-native") && !ownership.workspace_members.contains(&package.name)
        {
            findings.push(format!(
                "adapter package is absent from ownership map: {}",
                package.name
            ));
        }
    }
    findings
}

fn workspace_package_names(metadata: &CargoMetadata) -> Vec<String> {
    let ids = metadata.workspace_members.iter().collect::<BTreeSet<_>>();
    let mut names = metadata
        .packages
        .iter()
        .filter(|package| ids.contains(&package.id))
        .map(|package| package.name.clone())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn native_package_names(metadata: &CargoMetadata) -> BTreeSet<String> {
    metadata
        .packages
        .iter()
        .filter(|package| {
            package.name.ends_with("-sys")
                || package.name.ends_with("_sys")
                || package.links.is_some()
        })
        .map(|package| package.name.clone())
        .collect()
}

fn adapter_path_allowed(path: &str, policy: &Policy) -> bool {
    let Some(crate_path) = path.strip_prefix("crates/") else {
        return false;
    };
    let Some(crate_name) = crate_path.split('/').next() else {
        return false;
    };
    crate_name.ends_with("-native")
        && policy
            .adapters
            .iter()
            .any(|adapter| adapter.adapter_crate == crate_name && adapter.status == "approved")
}

fn forbidden_path(path: &str, policy: &Policy) -> bool {
    let lower = path.to_ascii_lowercase();
    lower
        .split('/')
        .any(|component| matches!(component, "cmake" | "packaging" | ".ci"))
        || policy
            .forbidden_reference_tokens
            .iter()
            .any(|token| lower.contains(&token.to_ascii_lowercase()))
}

fn scan_text_file(path: &str) -> bool {
    let path = Path::new(path);
    matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("rs" | "toml" | "yml" | "yaml" | "sh" | "ts" | "js" | "json")
    ) || path.file_name().and_then(|name| name.to_str()) == Some("Cargo.toml")
}

fn run_command(runner: &ProcessRunner, request: ProcessRequest, label: &str) -> Result<String> {
    let result = runner
        .run(request)
        .map_err(|error| format!("{label}: {error}"))?;
    if !result.receipt.success() {
        return Err(format!(
            "{label}: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    String::from_utf8(result.stdout)
        .map_err(|error| format!("{label}: output is not UTF-8: {error}"))
}

fn required_field<'a>(fields: &BTreeMap<&'a str, &'a str>, name: &str) -> Result<String> {
    fields
        .get(name)
        .copied()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("rustc -vV: missing {name}"))
}

fn baseline_string(baseline: &toml::Value, key: &str) -> Result<String> {
    baseline
        .get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| format!("{COMPILER_BASELINE_PATH}: missing {key}"))
}

fn sha256_file(path: PathBuf) -> Result<String> {
    let bytes = fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn write_receipt(
    root: &RepositoryRoot,
    path: Option<&Path>,
    data: &serde_json::Value,
) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    if path.is_absolute() {
        return Err(format!(
            "native boundary receipt must be relative: {}",
            path.display()
        ));
    }
    let destination = root.join(path);
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(data).map_err(|error| format!("receipt: {error}"))?;
    fs::write(&destination, bytes).map_err(|error| format!("{}: {error}", destination.display()))
}

fn format_findings(findings: Vec<String>) -> String {
    findings
        .into_iter()
        .take(32)
        .enumerate()
        .map(|(index, finding)| format!("native boundary finding {}: {finding}", index + 1))
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::{
        AdapterSpec, CargoDependency, CargoMetadata, CargoPackage, NativePackageOwner,
        OwnershipMap, Policy, scan_source_text, validate_metadata,
    };

    fn policy() -> Policy {
        toml::from_str(
            r#"
schema = "rusttable.native-boundaries.v1"
owner_issue = 168
primary_compiler = "quality/compiler-baseline.toml"
ownership_map = "architecture/native-ownership.toml"
workspace_dag = "architecture/workspace-dag.toml"
dependency_policy = "quality/dependency-sources.toml"
reference_clone = "../upstream"
forbidden_extensions = ["c"]
forbidden_filenames = ["build.rs"]
forbidden_direct_dependencies = ["cc"]
forbidden_source_tokens = ["extern \"C\"", "RUSTC_BOOTSTRAP"]
forbidden_reference_tokens = ["../upstream"]
allowed_unsafe_crates = ["rusttable-simd"]
forbidden_adapter_consumers = ["rusttable-image"]
adapters = []
migration_receipts = []
"#,
        )
        .expect("fixture policy")
    }

    #[test]
    fn native_ffi_is_rejected_outside_an_adapter() {
        let findings = scan_source_text(
            "crates/rusttable-image/src/lib.rs",
            "extern \"C\" {}",
            &policy(),
            false,
        );
        assert!(findings.iter().any(|finding| finding.contains("extern")));
    }

    #[test]
    fn reference_clone_path_is_rejected_even_in_rust_source() {
        let findings = scan_source_text(
            "crates/rusttable-image/src/lib.rs",
            "let source = \"../upstream\";",
            &policy(),
            false,
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("reference-tree"))
        );
    }

    #[test]
    fn approved_adapter_source_still_rejects_raw_public_layout_markers() {
        let findings = scan_source_text(
            "crates/rusttable-lua-native/src/lib.rs",
            "pub struct Handle(*mut c_void);",
            &policy(),
            true,
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("safe wrapper"))
        );
    }

    #[test]
    fn checked_in_forbidden_fixtures_cover_the_boundary_patterns() {
        let policy = policy();
        for (path, source, expected) in [
            (
                "crates/rusttable-image/src/lib.rs",
                include_str!("../../tests/fixtures/native-boundaries/forbidden-ffi.rs.fixture"),
                "extern",
            ),
            (
                "crates/rusttable-image/src/lib.rs",
                include_str!(
                    "../../tests/fixtures/native-boundaries/forbidden-reference.rs.fixture"
                ),
                "reference-tree",
            ),
        ] {
            let findings = scan_source_text(path, source, &policy, false);
            assert!(
                findings.iter().any(|finding| finding.contains(expected)),
                "{path}"
            );
        }
    }

    #[test]
    fn adapter_edges_allow_only_declared_consumers() {
        let mut policy = policy();
        policy.adapters.push(AdapterSpec {
            id: "lua".to_owned(),
            adapter_crate: "rusttable-lua-native".to_owned(),
            native_library: "Lua 5.4".to_owned(),
            purpose: "fixture".to_owned(),
            owning_api: "rusttable-scripting".to_owned(),
            allowed_dependents: vec!["rusttable-scripting".to_owned()],
            build_features: vec!["vendored".to_owned()],
            unsafe_surface: "opaque".to_owned(),
            callback_model: "panic-contained".to_owned(),
            thread_model: "single-threaded".to_owned(),
            removal_condition: "fixture".to_owned(),
            status: "approved".to_owned(),
        });
        let ownership = OwnershipMap {
            schema: "fixture".to_owned(),
            generated: true,
            generated_by: "fixture".to_owned(),
            source_manifest: "fixture".to_owned(),
            workspace_dag: "fixture".to_owned(),
            dependency_policy: "fixture".to_owned(),
            workspace_members: vec![
                "rusttable-scripting".to_owned(),
                "rusttable-image".to_owned(),
            ],
            native_packages: vec![NativePackageOwner {
                name: "fixture".to_owned(),
                owner_issue: 168,
                owner_crate: "rusttable-lua-native".to_owned(),
                classification: "fixture".to_owned(),
                evidence: vec!["fixture".to_owned()],
            }],
        };
        let metadata = |consumer: &str| CargoMetadata {
            packages: vec![CargoPackage {
                id: consumer.to_owned(),
                name: consumer.to_owned(),
                links: None,
                dependencies: vec![CargoDependency {
                    name: "rusttable-lua-native".to_owned(),
                }],
            }],
            workspace_members: vec![consumer.to_owned()],
        };

        assert!(
            validate_metadata(&metadata("rusttable-scripting"), &policy, &ownership).is_empty()
        );
        let findings = validate_metadata(&metadata("rusttable-image"), &policy, &ownership);
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("unapproved crate"))
        );
    }

    #[test]
    fn boundary_checker_source_is_exempt_from_its_own_negative_token_scan() {
        let findings = scan_source_text(
            "crates/xtask/src/commands/native_boundaries.rs",
            include_str!("native_boundaries.rs"),
            &policy(),
            false,
        );
        assert!(findings.is_empty(), "{findings:?}");
    }
}
