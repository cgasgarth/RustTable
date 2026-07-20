use std::fs;
use std::path::Path;

use clap::Subcommand;
use rusttable_processing::descriptor::{exposure_descriptor, rgb_gain_descriptor};
use rusttable_processing::operation_stack::{OperationStackSnapshot, OperationStackTemplate};

use crate::Result;

const SOURCE_MAP_SCHEMA: &str = "rusttable.operation-source-map.v1";
const PINNED_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";

#[derive(Debug, Subcommand)]
pub(crate) enum OperationSchemaCommand {
    /// Validate representative descriptors and their workspace ownership.
    Verify {
        #[arg(long)]
        all_registered_fixtures: bool,
        #[arg(long)]
        against_manifest: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum OperationStackCommand {
    /// Validate the initial workflow templates and their workspace ownership.
    Verify {
        #[arg(long)]
        templates: String,
        #[arg(long)]
        against_manifest: bool,
    },
}

pub(crate) fn run_schema(root: &Path, command: &OperationSchemaCommand) -> Result {
    match command {
        OperationSchemaCommand::Verify {
            all_registered_fixtures,
            against_manifest,
        } => {
            verify_source_map(root, 263)?;
            if !all_registered_fixtures || !against_manifest {
                return Err(
                    "operation schema verification requires both fixture and manifest checks"
                        .to_owned(),
                );
            }
            for descriptor in [exposure_descriptor(), rgb_gain_descriptor()] {
                descriptor
                    .validate()
                    .map_err(|error| format!("operation schema: {error}"))?;
                let first = descriptor
                    .canonical_hash()
                    .map_err(|error| format!("operation schema: {error}"))?;
                let second = descriptor
                    .canonical_hash()
                    .map_err(|error| format!("operation schema: {error}"))?;
                if first != second {
                    return Err("operation schema: canonical hash is unstable".to_owned());
                }
            }
            verify_manifest(root, &["postcard", "serde", "sha2"])?;
            eprintln!("operation schema verification passed (fixtures=2)");
            Ok(())
        }
    }
}

pub(crate) fn run_stack(root: &Path, command: &OperationStackCommand) -> Result {
    match command {
        OperationStackCommand::Verify {
            templates,
            against_manifest,
        } => {
            verify_source_map(root, 264)?;
            if templates != "all" || !against_manifest {
                return Err(
                    "operation stack verification requires --templates all and manifest checks"
                        .to_owned(),
                );
            }
            for template in [
                OperationStackTemplate::raster_basic(),
                OperationStackTemplate::raw_basic(),
            ] {
                let snapshot = OperationStackSnapshot::new(template);
                snapshot
                    .validate()
                    .map_err(|error| format!("operation stack: {error}"))?;
            }
            verify_manifest(root, &["postcard", "serde", "sha2"])?;
            eprintln!("operation stack verification passed (templates=2)");
            Ok(())
        }
    }
}

pub(crate) fn verify_source_map(root: &Path, issue: i64) -> Result {
    let filename = match issue {
        263 => "rusttable-operation-descriptor-source-map.toml",
        264 => "rusttable-operation-stack-source-map.toml",
        _ => return Err(format!("operation source map: unsupported issue {issue}")),
    };
    let path = root.join("architecture").join(filename);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("operation source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("operation source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SOURCE_MAP_SCHEMA) {
        return Err("operation source map: unsupported schema".to_owned());
    }
    if document.get("issue").and_then(toml::Value::as_integer) != Some(issue) {
        return Err(format!("operation source map: expected issue {issue}"));
    }
    if document
        .get("upstream_commit")
        .and_then(toml::Value::as_str)
        != Some(PINNED_COMMIT)
    {
        return Err("operation source map: upstream commit is not pinned".to_owned());
    }
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "operation source map: responsibilities are missing".to_owned())?;
    let expected_ids: &[&str] = if issue == 263 {
        &[
            "operation-instance-descriptor",
            "operation-shared-descriptor",
            "operation-abi-surface",
            "roi-contract",
            "tiling-contract",
            "history-parameter-contract",
            "representative-descriptors",
        ]
    } else {
        &[
            "operation-order",
            "operation-instance",
            "style-history-order",
            "history-order-state",
            "ui-history-synchronization",
        ]
    };
    if entries.len() != expected_ids.len() {
        return Err(format!(
            "operation source map: expected {} responsibilities, found {}",
            expected_ids.len(),
            entries.len()
        ));
    }
    let mut seen_ids = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "operation source map: responsibility is not a table".to_owned())?;
        for key in [
            "id",
            "upstream_path",
            "upstream_symbol",
            "rust_path",
            "status",
        ] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!(
                    "operation source map: responsibility missing {key}"
                ));
            }
        }
        let id = table["id"].as_str().expect("validated responsibility ID");
        if !expected_ids.contains(&id) || !seen_ids.insert(id) {
            return Err(format!(
                "operation source map: unexpected or duplicate responsibility {id}"
            ));
        }
        let rust_path = table["rust_path"]
            .as_str()
            .expect("validated Rust owner path");
        if !root.join(rust_path).is_file() {
            return Err(format!(
                "operation source map: missing Rust owner {rust_path}"
            ));
        }
        if table["status"].as_str() == Some("deferred")
            && table
                .get("deferred_issue")
                .and_then(toml::Value::as_integer)
                .is_none()
        {
            return Err("operation source map: deferred responsibility needs an issue".to_owned());
        }
    }
    Ok(())
}

fn verify_manifest(root: &Path, dependencies: &[&str]) -> Result {
    let manifest = fs::read_to_string(root.join("crates/rusttable-processing/Cargo.toml"))
        .map_err(|error| format!("operation manifest: read failed: {error}"))?;
    for dependency in dependencies {
        if !manifest.contains(&format!("{dependency}.workspace = true")) {
            return Err(format!(
                "operation manifest: missing workspace dependency {dependency}"
            ));
        }
    }
    if !manifest.contains("[lints]\nworkspace = true") {
        return Err("operation manifest: workspace lint inheritance is missing".to_owned());
    }
    Ok(())
}
