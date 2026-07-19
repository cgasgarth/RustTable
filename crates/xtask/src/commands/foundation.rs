use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Result, report};
use crate::cli::{FoundationCommand, FoundationMode, FoundationVerifyArgs};
use crate::process::{EnvironmentProfile, ProcessLimits, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

const CONTRACT_SCHEMA: &str = "rusttable.foundation-gate.v1";
const RECEIPT_SCHEMA: &str = "rusttable.foundation-contributor.v1";

#[derive(Debug, Deserialize)]
struct GateContract {
    schema: String,
    repository: String,
    parent_issue: u64,
    contributors: BTreeMap<String, ContributorContract>,
    modes: BTreeMap<String, ModeContract>,
}

#[derive(Debug, Deserialize)]
struct ContributorContract {
    issue: u64,
    schemas: Vec<String>,
    #[serde(default)]
    platforms: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModeContract {
    required: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ContributorReceipt {
    schema: String,
    contributor: String,
    issue: u64,
    producer: String,
    status: String,
    mode: String,
    commit: String,
    #[serde(default)]
    immutable_bundle: bool,
    #[serde(default)]
    reference_identity: String,
    #[serde(default)]
    build_identity: String,
    #[serde(default)]
    platforms: Vec<String>,
    #[serde(default)]
    artifact_hashes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct FoundationReceipt {
    schema: &'static str,
    mode: &'static str,
    repository: String,
    parent_issue: u64,
    commit: String,
    contributors: Vec<String>,
    artifact_hash: String,
}

pub(super) fn run(
    root: &RepositoryRoot,
    command: &FoundationCommand,
    runner: &ProcessRunner,
) -> Result {
    match command {
        FoundationCommand::Verify(arguments) => verify(root, arguments, runner),
    }
}

fn verify(
    root: &RepositoryRoot,
    arguments: &FoundationVerifyArgs,
    runner: &ProcessRunner,
) -> Result {
    let contract_path = root.join(&arguments.contract);
    let contract = read_contract(&contract_path)?;
    let mode = mode_name(arguments.mode);
    let mode_contract = contract
        .modes
        .get(mode)
        .ok_or_else(|| format!("foundation gate: mode {mode} is not declared"))?;
    let current_commit = git_commit(root, runner)?;
    let receipt_paths = receipt_paths(&root.join(&arguments.receipts))?;
    let mut receipts = Vec::new();
    for path in &receipt_paths {
        receipts.push(read_receipt(path)?);
    }
    let findings = validate_receipts(&contract, mode, mode_contract, &current_commit, &receipts);
    if !findings.is_empty() {
        return Err(format!(
            "foundation gate {mode}: {} finding(s): {}",
            findings.len(),
            findings.join("; ")
        ));
    }
    let contributors = receipts
        .iter()
        .map(|receipt| receipt.contributor.clone())
        .collect::<Vec<_>>();
    let artifact_hash = hash_json(&receipts)?;
    let receipt = FoundationReceipt {
        schema: CONTRACT_SCHEMA,
        mode,
        repository: contract.repository,
        parent_issue: contract.parent_issue,
        commit: current_commit,
        contributors,
        artifact_hash,
    };
    Ok(report(
        root,
        "foundation.verify",
        serde_json::to_value(receipt).map_err(|error| format!("foundation receipt: {error}"))?,
    ))
}

fn read_contract(path: &Path) -> std::result::Result<GateContract, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("foundation contract {}: {error}", path.display()))?;
    let contract = toml::from_str::<GateContract>(&text)
        .map_err(|error| format!("foundation contract {}: {error}", path.display()))?;
    if contract.schema != CONTRACT_SCHEMA {
        return Err(format!(
            "foundation contract: schema must be {CONTRACT_SCHEMA}, found {}",
            contract.schema
        ));
    }
    if contract.repository != "cgasgarth/RustTable" || contract.parent_issue != 158 {
        return Err(
            "foundation contract: repository or parent issue is not RustTable/#158".to_owned(),
        );
    }
    if contract.contributors.is_empty() || contract.modes.is_empty() {
        return Err("foundation contract: contributors and modes are required".to_owned());
    }
    Ok(contract)
}

fn receipt_paths(directory: &Path) -> std::result::Result<Vec<PathBuf>, String> {
    let mut paths = fs::read_dir(directory)
        .map_err(|error| format!("foundation receipts {}: {error}", directory.display()))?
        .map(|entry| {
            let entry = entry.map_err(|error| format!("foundation receipts: {error}"))?;
            let file_type = entry
                .file_type()
                .map_err(|error| format!("foundation receipt type: {error}"))?;
            Ok(file_type.is_file() && entry.path().extension().is_some_and(|ext| ext == "json"))
                .map(|include| include.then_some(entry.path()))
        })
        .collect::<std::result::Result<Vec<_>, String>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    paths.sort();
    if paths.is_empty() {
        return Err(format!(
            "foundation receipts {}: no JSON contributor receipts",
            directory.display()
        ));
    }
    Ok(paths)
}

fn read_receipt(path: &Path) -> std::result::Result<ContributorReceipt, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("foundation receipt {}: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("foundation receipt {}: {error}", path.display()))
}

fn validate_receipts(
    contract: &GateContract,
    mode: &str,
    mode_contract: &ModeContract,
    current_commit: &str,
    receipts: &[ContributorReceipt],
) -> Vec<String> {
    let mut findings = Vec::new();
    let mut seen = BTreeSet::new();
    for receipt in receipts {
        let Some(spec) = contract.contributors.get(&receipt.contributor) else {
            findings.push(format!("unknown contributor {}", receipt.contributor));
            continue;
        };
        if !seen.insert(receipt.contributor.clone()) {
            findings.push(format!("duplicate contributor {}", receipt.contributor));
        }
        if receipt.schema != RECEIPT_SCHEMA || !spec.schemas.contains(&receipt.schema) {
            findings.push(format!(
                "{}: unsupported schema {}",
                receipt.contributor, receipt.schema
            ));
        }
        if receipt.issue != spec.issue {
            findings.push(format!(
                "{}: issue mismatch, expected #{}, found #{}",
                receipt.contributor, spec.issue, receipt.issue
            ));
        }
        if receipt.status != "passed" {
            findings.push(format!(
                "{}: status is {}",
                receipt.contributor, receipt.status
            ));
        }
        if receipt.mode != mode {
            findings.push(format!(
                "{}: receipt mode is {}, expected {mode}",
                receipt.contributor, receipt.mode
            ));
        }
        if receipt.producer.trim().is_empty()
            || receipt.reference_identity.trim().is_empty()
            || receipt.build_identity.trim().is_empty()
        {
            findings.push(format!(
                "{}: producer/build/reference identities are required",
                receipt.contributor
            ));
        }
        if !receipt.immutable_bundle && receipt.commit != current_commit {
            findings.push(format!(
                "{}: receipt commit does not match current commit",
                receipt.contributor
            ));
        }
        if receipt.artifact_hashes.is_empty()
            || receipt.artifact_hashes.iter().any(|hash| !is_sha256(hash))
        {
            findings.push(format!(
                "{}: artifact hashes must be non-empty SHA-256 values",
                receipt.contributor
            ));
        }
        for platform in &spec.platforms {
            if !receipt.platforms.iter().any(|actual| actual == platform) {
                findings.push(format!(
                    "{}: required platform {platform} is missing",
                    receipt.contributor
                ));
            }
        }
    }
    for contributor in &mode_contract.required {
        if !seen.contains(contributor) {
            findings.push(format!("required contributor {contributor} is missing"));
        }
    }
    findings.sort();
    findings
}

fn mode_name(mode: FoundationMode) -> &'static str {
    match mode {
        FoundationMode::Pr => "pr",
        FoundationMode::Merge => "merge",
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hash_json<T: Serialize>(value: &T) -> std::result::Result<String, String> {
    let bytes = serde_json::to_vec(value).map_err(|error| format!("foundation hash: {error}"))?;
    let mut digest = Sha256::new();
    digest.update(bytes);
    Ok(format!("{:x}", digest.finalize()))
}

fn git_commit(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
) -> std::result::Result<String, String> {
    let result = runner
        .run(
            ProcessRequest::new(
                "git",
                [
                    "-C",
                    &root.path().display().to_string(),
                    "rev-parse",
                    "HEAD",
                ],
            )
            .profile(EnvironmentProfile::GitTool)
            .limits(ProcessLimits {
                timeout: Some(std::time::Duration::from_secs(30)),
                max_stdout_bytes: 1024,
                max_stderr_bytes: 4096,
            }),
        )
        .map_err(|error| format!("foundation git identity: {error}"))?;
    if !result.receipt.success() {
        return Err("foundation git identity: rev-parse failed".to_owned());
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> GateContract {
        toml::from_str(
            r#"schema = "rusttable.foundation-gate.v1"
repository = "cgasgarth/RustTable"
parent_issue = 158

[contributors.governance]
issue = 446
schemas = ["rusttable.foundation-contributor.v1"]

[modes.pr]
required = ["governance"]

[modes.merge]
required = ["governance"]
"#,
        )
        .expect("contract fixture")
    }

    fn receipt() -> ContributorReceipt {
        ContributorReceipt {
            schema: RECEIPT_SCHEMA.to_owned(),
            contributor: "governance".to_owned(),
            issue: 446,
            producer: "test".to_owned(),
            status: "passed".to_owned(),
            mode: "pr".to_owned(),
            commit: "abc".to_owned(),
            immutable_bundle: false,
            reference_identity: "ref".to_owned(),
            build_identity: "build".to_owned(),
            platforms: Vec::new(),
            artifact_hashes: vec!["a".repeat(64)],
        }
    }

    #[test]
    fn accepts_a_complete_receipt() {
        assert!(
            validate_receipts(
                &contract(),
                "pr",
                &contract().modes["pr"],
                "abc",
                &[receipt()]
            )
            .is_empty()
        );
    }

    #[test]
    fn rejects_missing_and_mismatched_receipts() {
        let mut value = receipt();
        value.status = "warning".to_owned();
        value.commit = "old".to_owned();
        let findings =
            validate_receipts(&contract(), "pr", &contract().modes["pr"], "abc", &[value]);
        assert!(findings.iter().any(|finding| finding.contains("status")));
        assert!(findings.iter().any(|finding| finding.contains("commit")));
    }
}
