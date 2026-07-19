use std::fs;
use std::path::Path;

use super::{Result, report};
use crate::process::ProcessRunner;
use crate::root::RepositoryRoot;

#[path = "dag_context.rs"]
mod context;
#[path = "dag_graph.rs"]
mod graph;
#[path = "dag_metadata.rs"]
mod metadata;
#[path = "dag_model.rs"]
mod model;
#[path = "dag_receipt.rs"]
mod receipt;
#[path = "dag_verify.rs"]
mod verify;

pub(super) use context::load_contexts;
pub(super) use model::{Contract, MetadataContext, PlatformContract};
pub(super) use verify::{DagReport, verify};

const CONTRACT_PATH: &str = "architecture/workspace-dag.toml";
const PLATFORM_CONTRACT_PATH: &str = "architecture/platform-support.toml";

pub(super) fn run(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    artifact: Option<&Path>,
) -> Result {
    let platform = load_platform_contract(root)?;
    platform.validate()?;
    let contract = load_contract(root)?;
    contract.validate()?;
    let contexts = load_contexts(root, &contract, &platform, runner)?;
    let verification = verify(&contract, &platform, &contexts);
    let serialized = serde_json::to_value(&verification).map_err(|error| error.to_string())?;
    if let Some(path) = artifact {
        receipt::write(root, path, &verification)?;
    }
    if verification.violations.is_empty() {
        Ok(report(root, "repo.verify-dag", serialized))
    } else {
        Err(receipt::failure_message(&verification, artifact))
    }
}

fn load_contract(root: &RepositoryRoot) -> Result<Contract> {
    let path = root.join(CONTRACT_PATH);
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("{CONTRACT_PATH}: cannot read contract: {error}"))?;
    Contract::parse(&source)
}

fn load_platform_contract(root: &RepositoryRoot) -> Result<PlatformContract> {
    let path = root.join(PLATFORM_CONTRACT_PATH);
    let source = fs::read_to_string(&path).map_err(|error| {
        format!("{PLATFORM_CONTRACT_PATH}: cannot read platform contract: {error}")
    })?;
    toml::from_str(&source)
        .map_err(|error| format!("{PLATFORM_CONTRACT_PATH}: invalid TOML: {error}"))
}

#[cfg(test)]
#[path = "dag_tests.rs"]
mod tests;
