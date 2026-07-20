use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use toml::Value;

use crate::commands::{Result, report};
use crate::process::{EnvironmentProfile, ProcessLimits, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

const LOCKFILE: &str = "Cargo.lock";
const WORKSPACE: &str = "Cargo.toml";
const POLICY: &str = "quality/dependency-sources.toml";
const INHERITED_PACKAGE_FIELDS: [&str; 4] = ["edition", "license", "rust-version", "version"];
const CENTRALIZED_EXTERNAL_DEPENDENCIES: [&str; 4] =
    ["process-wrap", "tokio", "wasmtime", "wasmtime-wasi"];

#[derive(Debug, Deserialize)]
struct DependencyPolicy {
    schema: String,
    default_registry: String,
    lockfile: String,
    exact_workspace_versions: bool,
    git_dependencies: String,
    unreviewed_advisories: String,
    duplicate_minor_patch_lines: String,
    exceptions: Exceptions,
    #[serde(default)]
    native_packages: Vec<NativePackage>,
}

#[derive(Debug, Deserialize)]
struct Exceptions {
    #[serde(default)]
    git: Vec<GitException>,
    #[serde(default)]
    duplicate_versions: Vec<DuplicateException>,
    #[serde(default)]
    advisories: Vec<AdvisoryException>,
}

#[derive(Debug, Deserialize)]
struct GitException {
    url: String,
    rev: String,
    issue: u64,
}

#[derive(Debug, Deserialize)]
struct DuplicateException {
    name: String,
    versions: Vec<String>,
    reason: String,
    issue: u64,
    review: String,
}

#[derive(Debug, Deserialize)]
struct AdvisoryException {
    id: String,
    package: String,
    version: String,
    source: String,
    disposition: String,
    issue: u64,
    review: String,
    removal_condition: String,
}

#[derive(Debug, Deserialize)]
struct NativePackage {
    name: String,
    owner_issue: u64,
    rationale: String,
}

pub(crate) fn verify(root: &RepositoryRoot, runner: &ProcessRunner, offline: bool) -> Result {
    if !offline {
        return Err("dependencies verify: --offline is required".to_owned());
    }
    verify_policy(root, runner)
}

pub(crate) fn verify_policy(root: &RepositoryRoot, runner: &ProcessRunner) -> Result {
    let workspace_text = read(root, WORKSPACE)?;
    let workspace: Value =
        toml::from_str(&workspace_text).map_err(|error| format!("{WORKSPACE}: {error}"))?;
    let policy_text = read(root, POLICY)?;
    let policy: DependencyPolicy =
        toml::from_str(&policy_text).map_err(|error| format!("{POLICY}: invalid TOML: {error}"))?;
    let mut findings = validate_policy_document(&policy);
    findings.extend(validate_workspace(&workspace));
    let members = workspace
        .get("workspace")
        .and_then(|value| value.get("members"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{WORKSPACE}: workspace.members is missing"))?;
    let mut checked = Vec::new();
    let mut manifests = Vec::new();
    for member in members {
        let member = member
            .as_str()
            .ok_or_else(|| format!("{WORKSPACE}: workspace member is not a string"))?;
        let path = format!("{member}/Cargo.toml");
        let text = read(root, &path)?;
        let manifest: Value = toml::from_str(&text).map_err(|error| format!("{path}: {error}"))?;
        findings.extend(validate_manifest(
            &path,
            &manifest,
            workspace.get("workspace"),
        ));
        checked.push(path);
        manifests.push((
            checked.last().expect("path was just added").clone(),
            manifest,
        ));
    }
    findings.extend(validate_manifest_policy(&workspace, &manifests));
    if !root.join(LOCKFILE).is_file() {
        findings.push(format!("{LOCKFILE}: committed lockfile is required"));
    }
    if let Some(message) = validate_iced_targets(members, root) {
        findings.push(message);
    }
    let lock_text = read(root, LOCKFILE)?;
    findings.extend(validate_lock_graph(root, runner, &policy, &lock_text));
    if !findings.is_empty() {
        return Err(findings.join("; "));
    }
    let data = serde_json::json!({
        "schema": "rusttable.dependency-policy-verification.v1",
        "workspace_dependency_count": workspace_dependencies(workspace.get("workspace")).len(),
        "members": checked,
        "lockfile": LOCKFILE,
        "package_policy": "complete metadata and lock graph reconciled",
        "native_packages": policy.native_packages.len(),
        "duplicate_exceptions": policy.exceptions.duplicate_versions.len(),
        "advisory_exceptions": policy.exceptions.advisories.len(),
        "provenance": "direct and transitive requirements are exact and centrally owned",
        "manifest_policy": "workspace package fields, lint inheritance, naming, and internal dependencies are fail-closed",
        "diagnostics": "credentials and absolute paths omitted",
    });
    Ok(report(root, "dependencies.verify-policy", data))
}

fn validate_policy_document(policy: &DependencyPolicy) -> Vec<String> {
    let mut findings = Vec::new();
    if policy.schema != "rusttable.dependency-sources.v2" {
        findings.push(format!("{POLICY}: schema is not authoritative"));
    }
    if policy.default_registry != "crates.io"
        || policy.lockfile != LOCKFILE
        || !policy.exact_workspace_versions
        || policy.git_dependencies != "deny-unless-immutable-commit-exception"
        || policy.unreviewed_advisories != "deny"
        || policy.duplicate_minor_patch_lines != "deny"
    {
        findings.push(format!(
            "{POLICY}: source/advisory/duplicate policy is weakened"
        ));
    }
    for exception in &policy.exceptions.git {
        if exception.url.is_empty() || exception.rev.len() < 12 || exception.issue == 0 {
            findings.push(format!(
                "{POLICY}: Git exceptions require URL, commit, and owner issue"
            ));
        }
    }
    for exception in &policy.exceptions.duplicate_versions {
        if exception.name.is_empty()
            || exception.versions.is_empty()
            || exception.reason.is_empty()
            || exception.issue == 0
            || exception.review.is_empty()
        {
            findings.push(format!(
                "{POLICY}: duplicate exceptions require exact versions, reason, owner, and review"
            ));
        }
    }
    for exception in &policy.exceptions.advisories {
        if exception.id.is_empty()
            || exception.package.is_empty()
            || exception.version.is_empty()
            || exception.source.is_empty()
            || exception.disposition.is_empty()
            || exception.issue == 0
            || exception.review.is_empty()
            || exception.removal_condition.is_empty()
        {
            findings.push(format!(
                "{POLICY}: advisory exceptions must be exact and time-bounded"
            ));
        }
    }
    for package in &policy.native_packages {
        if package.name.is_empty() || package.owner_issue == 0 || package.rationale.is_empty() {
            findings.push(format!("{POLICY}: native package ownership is incomplete"));
        }
    }
    findings
}

#[expect(
    clippy::too_many_lines,
    reason = "The lock-graph gate reports each independent provenance finding."
)]
fn validate_lock_graph(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    policy: &DependencyPolicy,
    lock_text: &str,
) -> Vec<String> {
    let mut findings = Vec::new();
    let lock: Value = match toml::from_str(lock_text) {
        Ok(lock) => lock,
        Err(error) => {
            findings.push(format!("{LOCKFILE}: invalid TOML: {error}"));
            return findings;
        }
    };
    let packages = lock
        .get("package")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if packages.is_empty() {
        findings.push(format!("{LOCKFILE}: package graph is empty"));
    }
    let mut versions = BTreeMap::<String, BTreeSet<String>>::new();
    for package in &packages {
        let Some(name) = package.get("name").and_then(Value::as_str) else {
            findings.push(format!("{LOCKFILE}: package name is missing"));
            continue;
        };
        let Some(version) = package.get("version").and_then(Value::as_str) else {
            findings.push(format!("{LOCKFILE}: {name} version is missing"));
            continue;
        };
        versions
            .entry(name.to_owned())
            .or_default()
            .insert(version.to_owned());
        let source = package.get("source").and_then(Value::as_str);
        if source.is_some_and(|source| source.starts_with("registry+"))
            && package.get("checksum").and_then(Value::as_str).is_none()
        {
            findings.push(format!(
                "{LOCKFILE}: registry package {name} {version} has no checksum"
            ));
        }
        if let Some(source) = source.filter(|source| source.starts_with("git+")) {
            let allowed = policy.exceptions.git.iter().any(|exception| {
                source.contains(&exception.url) && source.contains(&exception.rev)
            });
            if !allowed {
                findings.push(format!(
                    "{LOCKFILE}: Git package {name} {version} is not an approved immutable source"
                ));
            }
        }
    }
    for (name, found_versions) in versions.iter().filter(|(_, versions)| versions.len() > 1) {
        let allowed = policy
            .exceptions
            .duplicate_versions
            .iter()
            .any(|exception| {
                exception.name == *name
                    && found_versions
                        .iter()
                        .all(|version| exception.versions.contains(version))
            });
        if !allowed {
            findings.push(format!(
                "{LOCKFILE}: duplicate versions for {name} require an exact reviewed exception ({})",
                found_versions.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    let metadata = match runner.run(
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
    ) {
        Ok(result) if result.receipt.success() => {
            match serde_json::from_slice::<serde_json::Value>(&result.stdout) {
                Ok(metadata) => Some(metadata),
                Err(error) => {
                    findings.push(format!("cargo metadata: invalid JSON: {error}"));
                    None
                }
            }
        }
        Ok(result) => {
            findings.push(format!(
                "cargo metadata failed ({}): {}",
                result.receipt.status,
                String::from_utf8_lossy(&result.stderr).trim()
            ));
            None
        }
        Err(error) => {
            findings.push(format!("cargo metadata: {error}"));
            None
        }
    };
    if let Some(metadata) = metadata {
        let metadata_packages = metadata
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len);
        if metadata_packages != packages.len() {
            findings.push(format!(
                "{LOCKFILE}: lock graph has {} packages but cargo metadata resolved {metadata_packages}",
                packages.len()
            ));
        }
        let native_names = metadata
            .get("packages")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter(|package| {
                let name = package
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                name.ends_with("-sys")
                    || name.ends_with("_sys")
                    || package.get("links").is_some_and(|links| !links.is_null())
            })
            .filter_map(|package| package.get("name").and_then(serde_json::Value::as_str))
            .collect::<BTreeSet<_>>();
        let owned_names = policy
            .native_packages
            .iter()
            .map(|package| package.name.as_str())
            .collect::<BTreeSet<_>>();
        for name in native_names {
            if !owned_names.contains(name) {
                findings.push(format!(
                    "{POLICY}: native package {name} has no explicit owner"
                ));
            }
        }
    }
    findings
}

pub(crate) fn validate_workspace(workspace: &Value) -> Vec<String> {
    let mut findings = Vec::new();
    let dependencies = workspace_dependencies(workspace.get("workspace"));
    if dependencies.is_empty() {
        findings.push(format!("{WORKSPACE}: [workspace.dependencies] is required"));
    }
    for name in dependencies {
        let value = workspace
            .get("workspace")
            .and_then(|value| value.get("dependencies"))
            .and_then(|value| value.get(&name))
            .expect("workspace dependency set came from the same table");
        if let Some(message) =
            validate_requirement(&format!("workspace dependency {name}"), value, true)
        {
            findings.push(message);
        }
    }
    findings
}

fn validate_manifest_policy(workspace: &Value, manifests: &[(String, Value)]) -> Vec<String> {
    let mut findings = Vec::new();
    let workspace_dependencies = workspace_dependencies(workspace.get("workspace"));
    let workspace_version = workspace
        .get("workspace")
        .and_then(|value| value.get("package"))
        .and_then(|value| value.get("version"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut members = BTreeMap::new();

    for (path, manifest) in manifests {
        let Some(package) = manifest.get("package").and_then(Value::as_table) else {
            findings.push(format!("{path}: [package] is required"));
            continue;
        };
        let expected_name = Path::new(path)
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let name = package
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if name != expected_name || (!name.starts_with("rusttable-") && name != "xtask") {
            findings.push(format!(
                "{path}: package.name must be the canonical workspace crate name {expected_name}"
            ));
        }
        if !members.insert(name.to_owned(), path.clone()).is_none() {
            findings.push(format!("{path}: package.name {name} is duplicated"));
        }
        for field in INHERITED_PACKAGE_FIELDS {
            let inherited = package
                .get(field)
                .and_then(Value::as_table)
                .and_then(|value| value.get("workspace"))
                .and_then(Value::as_bool)
                == Some(true);
            if !inherited {
                findings.push(format!("{path}: package.{field}.workspace must be true"));
            }
        }
        let lint_inheritance = manifest
            .get("lints")
            .and_then(Value::as_table)
            .and_then(|value| value.get("workspace"))
            .and_then(Value::as_bool)
            == Some(true);
        if !lint_inheritance {
            findings.push(format!("{path}: [lints] workspace = true is required"));
        }
    }

    for (name, path) in &members {
        let Some(requirement) = workspace
            .get("workspace")
            .and_then(|value| value.get("dependencies"))
            .and_then(|value| value.get(name))
        else {
            findings.push(format!(
                "{WORKSPACE}: internal crate {name} must be declared in [workspace.dependencies]"
            ));
            continue;
        };
        let Some(requirement) = requirement.as_table() else {
            findings.push(format!(
                "{WORKSPACE}: internal crate {name} must declare a path and exact version"
            ));
            continue;
        };
        let expected_path = path.trim_end_matches("/Cargo.toml");
        let exact_version = format!("={workspace_version}");
        if requirement.get("path").and_then(Value::as_str) != Some(expected_path)
            || requirement.get("version").and_then(Value::as_str) != Some(exact_version.as_str())
        {
            findings.push(format!(
                "{WORKSPACE}: internal crate {name} must use path {expected_path} and version {exact_version}"
            ));
        }
    }
    for name in CENTRALIZED_EXTERNAL_DEPENDENCIES {
        if !workspace_dependencies.contains(name) {
            findings.push(format!(
                "{WORKSPACE}: centralized dependency {name} is missing"
            ));
        }
    }
    findings
}

pub(crate) fn validate_manifest(
    path: &str,
    manifest: &Value,
    workspace: Option<&Value>,
) -> Vec<String> {
    let owned = workspace_dependencies(workspace);
    let internal = internal_workspace_dependencies(workspace);
    let mut findings = Vec::new();
    for (section_name, section) in dependency_sections(manifest) {
        let Some(table) = section.as_table() else {
            findings.push(format!("{path}: [{section_name}] must be a table"));
            continue;
        };
        for (name, requirement) in table {
            if internal.contains(name) {
                if !inherits_workspace_dependency(requirement) {
                    findings.push(format!(
                        "{path}: internal dependency {name} must use workspace = true"
                    ));
                }
                continue;
            }
            if requirement.get("path").is_some() {
                findings.push(format!(
                    "{path}: external dependency {name} uses a local path"
                ));
                continue;
            }
            if requirement.get("workspace").and_then(Value::as_bool) == Some(true) {
                if !owned.contains(name) {
                    findings.push(format!(
                        "{path}: {name} inherits an undeclared workspace dependency"
                    ));
                }
                continue;
            }
            if CENTRALIZED_EXTERNAL_DEPENDENCIES.contains(&name.as_str()) {
                findings.push(format!(
                    "{path}: centralized dependency {name} must use workspace = true"
                ));
                continue;
            }
            if let Some(message) =
                validate_requirement(&format!("{path}: {name}"), requirement, false)
            {
                findings.push(message);
            }
        }
    }
    findings
}

fn inherits_workspace_dependency(requirement: &Value) -> bool {
    requirement.get("workspace").and_then(Value::as_bool) == Some(true)
        && requirement.get("path").is_none()
        && requirement.get("version").is_none()
}

fn validate_requirement(
    context: &str,
    requirement: &Value,
    workspace_entry: bool,
) -> Option<String> {
    if let Some(version) = requirement.as_str() {
        if !is_exact_version(version) {
            return Some(format!(
                "{context}: version must be exact (=x.y.z), found {version}"
            ));
        }
        return None;
    }
    let Some(table) = requirement.as_table() else {
        return Some(format!(
            "{context}: dependency requirement is not a string or table"
        ));
    };
    if table.get("workspace").and_then(Value::as_bool) == Some(true) {
        return if workspace_entry {
            Some(format!(
                "{context}: workspace dependency cannot inherit itself"
            ))
        } else {
            None
        };
    }
    if let Some(version) = table.get("version").and_then(Value::as_str) {
        if !is_exact_version(version) {
            return Some(format!(
                "{context}: version must be exact (=x.y.z), found {version}"
            ));
        }
    } else if table.get("git").is_none() {
        return Some(format!(
            "{context}: exact registry version or immutable git source is required"
        ));
    }
    if table.get("git").is_some() {
        let rev = table.get("rev").and_then(Value::as_str);
        if rev.is_none_or(str::is_empty)
            || table.get("branch").is_some()
            || table.get("tag").is_some()
        {
            return Some(format!(
                "{context}: git dependencies require an immutable rev only"
            ));
        }
    }
    None
}

fn dependency_sections(manifest: &Value) -> Vec<(&str, &Value)> {
    let mut sections = Vec::new();
    for name in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if let Some(value) = manifest.get(name) {
            sections.push((name, value));
        }
    }
    if let Some(targets) = manifest.as_table() {
        for (target, value) in targets {
            if target.starts_with("target.") {
                if let Some(dependencies) = value.get("dependencies") {
                    sections.push((target, dependencies));
                }
                if let Some(dependencies) = value.get("dev-dependencies") {
                    sections.push((target, dependencies));
                }
                if let Some(dependencies) = value.get("build-dependencies") {
                    sections.push((target, dependencies));
                }
            }
        }
    }
    sections
}

fn workspace_dependencies(workspace: Option<&Value>) -> BTreeSet<String> {
    workspace
        .and_then(|value| value.get("dependencies"))
        .and_then(Value::as_table)
        .map(|table| table.keys().cloned().collect())
        .unwrap_or_default()
}

fn internal_workspace_dependencies(workspace: Option<&Value>) -> BTreeSet<String> {
    workspace
        .and_then(|value| value.get("dependencies"))
        .and_then(Value::as_table)
        .into_iter()
        .flatten()
        .filter_map(|(name, requirement)| {
            requirement
                .get("path")
                .and_then(Value::as_str)
                .filter(|path| path.starts_with("crates/"))
                .map(|_| name.clone())
        })
        .collect()
}

fn is_exact_version(version: &str) -> bool {
    let rest = version.strip_prefix('=');
    rest.is_some_and(|value| {
        let pieces = value.split('.').collect::<Vec<_>>();
        pieces.len() == 3
            && pieces[..2].iter().all(|piece| {
                !piece.is_empty() && piece.chars().all(|character| character.is_ascii_digit())
            })
            && pieces[2].split_once('-').map_or_else(
                || {
                    pieces[2]
                        .chars()
                        .all(|character| character.is_ascii_digit())
                },
                |(patch, prerelease)| {
                    !patch.is_empty()
                        && patch.chars().all(|character| character.is_ascii_digit())
                        && !prerelease.is_empty()
                        && prerelease
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric() || character == '-')
                },
            )
    })
}

fn validate_iced_targets(members: &[Value], root: &RepositoryRoot) -> Option<String> {
    let mut target_manifests = Vec::new();
    for member in members.iter().filter_map(Value::as_str) {
        if member == "crates/rusttable-app" || member == "crates/rusttable-ui" {
            target_manifests.push(format!("{member}/Cargo.toml"));
        }
    }
    for path in target_manifests {
        let text = fs::read_to_string(root.join(&path)).ok()?;
        let manifest: Value = toml::from_str(&text).ok()?;
        let targets = manifest.get("target")?.as_table()?.iter();
        let mut linux_x11 = false;
        for (target, value) in targets {
            let iced = value.get("dependencies").and_then(|deps| deps.get("iced"));
            let features = iced
                .and_then(|dep| dep.get("features"))
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<BTreeSet<_>>()
                })
                .unwrap_or_default();
            if target.contains("target_os = \"linux\"") {
                linux_x11 |= features.contains("x11");
            } else if features.contains("x11") {
                return Some(format!("{path}: Iced X11 feature is enabled outside Linux"));
            }
        }
        if !linux_x11 {
            return Some(format!(
                "{path}: Linux Iced target must enable x11 explicitly"
            ));
        }
    }
    None
}

fn read(root: &RepositoryRoot, path: &str) -> Result<String> {
    fs::read_to_string(root.join(path)).map_err(|error| format!("{path}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::{validate_manifest, validate_manifest_policy, validate_workspace};

    #[test]
    fn exact_workspace_requirements_pass() {
        let workspace: toml::Value =
            toml::from_str("[workspace.dependencies]\nserde = \"=1.0.228\"\n").expect("workspace");
        assert!(validate_workspace(&workspace).is_empty());
        let manifest: toml::Value =
            toml::from_str("[dependencies]\nserde.workspace = true\n").expect("manifest");
        assert!(validate_manifest("Cargo.toml", &manifest, workspace.get("workspace")).is_empty());
    }

    #[test]
    fn caret_and_floating_git_requirements_fail() {
        let workspace: toml::Value =
            toml::from_str("[workspace.dependencies]\nserde = \"1.0.228\"\n").expect("workspace");
        let findings = validate_workspace(&workspace);
        assert!(findings.iter().any(|finding| finding.contains("exact")));
        let manifest: toml::Value = toml::from_str(
            "[dependencies]\nserde = { git = \"https://example.invalid/repo\", branch = \"main\" }\n",
        )
        .expect("manifest");
        let findings = validate_manifest("Cargo.toml", &manifest, workspace.get("workspace"));
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("immutable rev"))
        );
    }

    #[test]
    fn exact_prerelease_versions_are_allowed() {
        let workspace: toml::Value = toml::from_str(
            "[workspace.dependencies]\niced = { version = \"=0.15.0-dev\", git = \"https://example.invalid/iced\", rev = \"abc\" }\n",
        )
        .expect("workspace");
        assert!(validate_workspace(&workspace).is_empty());
    }

    #[test]
    fn undeclared_inheritance_and_external_paths_fail() {
        let workspace: toml::Value =
            toml::from_str("[workspace.dependencies]\nserde = \"=1.0.228\"\n").expect("workspace");
        let manifest: toml::Value = toml::from_str(
            "[dependencies]\nserde.workspace = true\ncodec = { path = \"../codec\" }\n",
        )
        .expect("manifest");
        let findings = validate_manifest("Cargo.toml", &manifest, Some(&workspace));
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("undeclared"))
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("external dependency codec"))
        );
    }

    #[test]
    fn manifest_policy_requires_inherited_metadata_and_lints() {
        let workspace: toml::Value = toml::from_str(
            "[workspace.package]\nversion = \"0.1.0\"\n\
             [workspace.dependencies]\n\
             rusttable-core = { path = \"crates/rusttable-core\", version = \"=0.1.0\" }\n\
             process-wrap = \"=9.1.0\"\n\
             tokio = \"=1.51.1\"\n\
             wasmtime = \"=46.0.1\"\n\
             wasmtime-wasi = \"=46.0.1\"\n",
        )
        .expect("workspace");
        let manifest: toml::Value = toml::from_str(
            "[package]\nname = \"rusttable-core\"\n\
             edition.workspace = true\n\
             license.workspace = true\n\
             rust-version.workspace = true\n\
             version.workspace = true\n\
             [lints]\nworkspace = true\n",
        )
        .expect("manifest");
        assert!(
            validate_manifest_policy(
                &workspace,
                &[("crates/rusttable-core/Cargo.toml".to_owned(), manifest)]
            )
            .is_empty()
        );

        let incomplete: toml::Value = toml::from_str(
            "[package]\nname = \"rusttable-core\"\n\
             edition.workspace = true\n\
             license.workspace = true\n\
             rust-version.workspace = true\n\
             version.workspace = true\n",
        )
        .expect("manifest");
        let findings = validate_manifest_policy(
            &workspace,
            &[("crates/rusttable-core/Cargo.toml".to_owned(), incomplete)],
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("[lints] workspace = true"))
        );
    }

    #[test]
    fn internal_and_centralized_dependencies_must_inherit_workspace_values() {
        let workspace: toml::Value = toml::from_str(
            "[workspace.dependencies]\n\
             rusttable-core = { path = \"crates/rusttable-core\", version = \"=0.1.0\" }\n\
             process-wrap = \"=9.1.0\"\n\
             tokio = \"=1.51.1\"\n\
             wasmtime = \"=46.0.1\"\n\
             wasmtime-wasi = \"=46.0.1\"\n",
        )
        .expect("workspace");
        let manifest: toml::Value = toml::from_str(
            "[dependencies]\n\
             rusttable-core = { path = \"../rusttable-core\", version = \"0.1.0\" }\n\
             tokio = \"=1.51.1\"\n",
        )
        .expect("manifest");
        let findings = validate_manifest("Cargo.toml", &manifest, workspace.get("workspace"));
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("rusttable-core must use workspace = true"))
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains("tokio must use workspace = true"))
        );
    }
}
