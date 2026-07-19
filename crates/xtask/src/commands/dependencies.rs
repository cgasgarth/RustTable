use std::collections::BTreeSet;
use std::fs;

use toml::Value;

use crate::commands::{Result, report};
use crate::root::RepositoryRoot;

const LOCKFILE: &str = "Cargo.lock";
const WORKSPACE: &str = "Cargo.toml";

pub(crate) fn verify_policy(root: &RepositoryRoot) -> Result {
    let workspace_text = read(root, WORKSPACE)?;
    let workspace: Value =
        toml::from_str(&workspace_text).map_err(|error| format!("{WORKSPACE}: {error}"))?;
    let mut findings = validate_workspace(&workspace);
    let members = workspace
        .get("workspace")
        .and_then(|value| value.get("members"))
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{WORKSPACE}: workspace.members is missing"))?;
    let mut checked = Vec::new();
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
    }
    if !root.join(LOCKFILE).is_file() {
        findings.push(format!("{LOCKFILE}: committed lockfile is required"));
    }
    if let Some(message) = validate_iced_targets(members, root) {
        findings.push(message);
    }
    if !findings.is_empty() {
        return Err(findings.join("; "));
    }
    let data = serde_json::json!({
        "schema": "rusttable.dependency-policy-verification.v1",
        "workspace_dependency_count": workspace_dependencies(workspace.get("workspace")).len(),
        "members": checked,
        "lockfile": LOCKFILE,
        "provenance": "direct requirements are exact and centrally owned",
        "diagnostics": "credentials and absolute paths omitted",
    });
    Ok(report(root, "dependencies.verify-policy", data))
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

pub(crate) fn validate_manifest(
    path: &str,
    manifest: &Value,
    workspace: Option<&Value>,
) -> Vec<String> {
    let owned = workspace_dependencies(workspace);
    let mut findings = Vec::new();
    for (section_name, section) in dependency_sections(manifest) {
        let Some(table) = section.as_table() else {
            findings.push(format!("{path}: [{section_name}] must be a table"));
            continue;
        };
        for (name, requirement) in table {
            if is_local_dependency(name, requirement) {
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
            if let Some(message) =
                validate_requirement(&format!("{path}: {name}"), requirement, false)
            {
                findings.push(message);
            }
        }
    }
    findings
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

fn is_local_dependency(name: &str, requirement: &Value) -> bool {
    let _ = requirement;
    name.starts_with("rusttable-") || name == "xtask"
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
    use super::{validate_manifest, validate_workspace};

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
}
