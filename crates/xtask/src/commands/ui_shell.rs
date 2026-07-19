use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::{Result, report};
use crate::cli::UiShellArgs;
use crate::root::RepositoryRoot;

#[derive(Debug, Deserialize)]
struct BoundaryPolicy {
    version: u32,
    max_handwritten_lines: usize,
    allowed_dependencies: Vec<String>,
    forbidden_dependencies: Vec<String>,
}

pub(super) fn run(root: &RepositoryRoot, args: &UiShellArgs) -> Result {
    if args.presets != "all" {
        return Err(format!(
            "ui-shell: unsupported preset set {:?}",
            args.presets
        ));
    }
    let policy: BoundaryPolicy = toml::from_str(
        &fs::read_to_string(root.join("architecture/ui-boundaries.toml"))
            .map_err(|error| format!("ui-shell: cannot read boundary policy: {error}"))?,
    )
    .map_err(|error| format!("ui-shell: invalid boundary policy: {error}"))?;
    if policy.version != 1 || policy.allowed_dependencies != ["iced", "rusttable-core"] {
        return Err(String::from(
            "ui-shell: boundary policy version or allow-list changed",
        ));
    }
    let files = rust_files(&root.join("crates/rusttable-ui/src"))?;
    let mut violations = Vec::new();
    for path in &files {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("ui-shell: cannot read {}: {error}", path.display()))?;
        let line_count = source.lines().count();
        if line_count > policy.max_handwritten_lines {
            violations.push(format!(
                "{} has {line_count} handwritten lines",
                path.display()
            ));
        }
        for forbidden in &policy.forbidden_dependencies {
            if source.lines().any(|line| {
                let line = line.trim_start();
                (line.starts_with("use ") || line.starts_with("extern crate "))
                    && line.contains(forbidden)
            }) {
                violations.push(format!("{} imports forbidden {forbidden}", path.display()));
            }
        }
        for raw_device in ["Device::new(", "Instance::new("] {
            if source.contains(raw_device) {
                violations.push(format!("{} constructs raw WGPU state", path.display()));
            }
        }
    }
    if !violations.is_empty() {
        return Err(violations.join("\n"));
    }
    let a11y = !args.verify_a11y
        || source_contains(&files, "accessibility_enabled")
            && source_contains(&files, "FocusOwner");
    let lifecycle = !args.verify_window_lifecycle
        || source_contains(&files, "RequestExit")
            && source_contains(&files, "TaskCompleted")
            && source_contains(&files, "WindowTaskCancelled");
    if !a11y || !lifecycle {
        return Err(String::from(
            "ui-shell: requested contract receipt verification failed",
        ));
    }
    Ok(report(
        root,
        "ui-shell.verify",
        serde_json::json!({
            "presets": ["one-monitor", "multiple-monitors", "high-dpi", "removed-monitor", "light", "dark", "system", "reduced-motion"],
            "architecture": "ui-boundaries.v1",
            "receipts": ["window", "task", "subscription", "viewport", "shutdown"],
            "verify_a11y": args.verify_a11y,
            "verify_window_lifecycle": args.verify_window_lifecycle,
            "handwritten_file_count": files.len(),
        }),
    ))
}

fn rust_files(directory: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)
        .map_err(|error| format!("ui-shell: cannot read {}: {error}", directory.display()))?
    {
        let entry = entry.map_err(|error| format!("ui-shell: directory entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn source_contains(files: &[PathBuf], needle: &str) -> bool {
    files
        .iter()
        .any(|path| fs::read_to_string(path).is_ok_and(|source| source.contains(needle)))
}

#[cfg(test)]
mod tests {
    use super::BoundaryPolicy;

    #[test]
    fn boundary_policy_keeps_domain_and_storage_out_of_ui() {
        let policy: BoundaryPolicy = toml::from_str(
            r#"version = 1
max_handwritten_lines = 1000
allowed_dependencies = ["iced", "rusttable-core"]
forbidden_dependencies = ["redb"]"#,
        )
        .expect("policy parses");
        assert_eq!(policy.version, 1);
        assert_eq!(policy.max_handwritten_lines, 1000);
        assert_eq!(policy.allowed_dependencies, ["iced", "rusttable-core"]);
        assert_eq!(policy.forbidden_dependencies, ["redb"]);
    }
}
