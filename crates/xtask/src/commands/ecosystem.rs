use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use toml::Value;

use crate::cli::BaselineVerifyArgs;
use crate::commands::{Result, report};
use crate::process::{ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

const TOOLCHAIN_PATH: &str = "rust-toolchain.toml";
const BASELINE_PATH: &str = "quality/compiler-baseline.toml";
const WORKSPACE_MANIFEST: &str = "Cargo.toml";
const LOCKFILE: &str = "Cargo.lock";

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
        },
        "diagnostics": "credentials and absolute command paths omitted",
    });
    write_receipt(root, arguments.receipt.as_deref(), &data)?;
    Ok(report(root, "ecosystem.verify-baseline", data))
}

pub(crate) fn upgrade_diff(root: &RepositoryRoot) -> Result {
    let files = [TOOLCHAIN_PATH, BASELINE_PATH, WORKSPACE_MANIFEST, LOCKFILE];
    let mut hashes = BTreeMap::new();
    for file in files {
        hashes.insert(file, digest(&read(root, file)?.into_bytes()));
    }
    let data = serde_json::json!({
        "schema": "rusttable.ecosystem-upgrade-diff.v1",
        "baseline": "compare this receipt with the target baseline PR",
        "compiler": {
            "toolchain": hashes[TOOLCHAIN_PATH],
            "compiler_baseline": hashes[BASELINE_PATH],
        },
        "dependency_graph": {
            "manifest": hashes[WORKSPACE_MANIFEST],
            "lockfile": hashes[LOCKFILE],
        },
        "surfaces": ["compiler", "dependencies", "features", "lockfile", "packaging"],
    });
    Ok(report(root, "ecosystem.upgrade-diff", data))
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
