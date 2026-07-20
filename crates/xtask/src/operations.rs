use std::fs;
use std::path::Path;
use std::str::FromStr;

use clap::Subcommand;
use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
};
use rusttable_processing::descriptor::{exposure_descriptor, rgb_gain_descriptor};
use rusttable_processing::operation_stack::{OperationStackSnapshot, OperationStackTemplate};
use rusttable_processing::{FiniteF32, LinearRgb, PipelineStepIndex, builtin_registry};

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

#[derive(Debug, Subcommand)]
pub(crate) enum OperationRegistryCommand {
    /// Generate or check the deterministic built-in operation receipt.
    Generate,
    Check {
        #[arg(long)]
        against_manifest: bool,
        #[arg(long)]
        execute_builtins: bool,
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

pub(crate) fn run_registry(root: &Path, command: &OperationRegistryCommand) -> Result {
    let receipt_path = root.join("architecture/rusttable-operation-registry.toml");
    match command {
        OperationRegistryCommand::Generate => {
            fs::write(&receipt_path, builtin_registry().receipt())
                .map_err(|error| format!("operation registry: write failed: {error}"))?;
            eprintln!("operation registry generated (definitions=3)");
            Ok(())
        }
        OperationRegistryCommand::Check {
            against_manifest,
            execute_builtins,
        } => {
            let committed = fs::read_to_string(&receipt_path)
                .map_err(|error| format!("operation registry: receipt read failed: {error}"))?;
            if committed != builtin_registry().receipt() {
                return Err("operation registry receipt is stale; run cargo xtask operation-registry generate".to_owned());
            }
            if *against_manifest {
                verify_registry_source_map(root)?;
                let manifest =
                    fs::read_to_string(root.join("architecture/darktable-operations.toml"))
                        .map_err(|error| {
                            format!("operation registry: manifest read failed: {error}")
                        })?;
                if !manifest.contains("name = \"exposure\"") {
                    return Err(
                        "operation registry: exposure is absent from operation manifest".to_owned(),
                    );
                }
            }
            if *execute_builtins {
                execute_builtin_smoke()?;
            }
            eprintln!(
                "operation registry check passed (definitions=3, snapshot={})",
                builtin_registry().identity_hash_hex()
            );
            Ok(())
        }
    }
}

fn execute_builtin_smoke() -> Result {
    let cases = [
        ("rusttable.exposure", [("stops", 0.5), ("", 0.0), ("", 0.0)]),
        (
            "rusttable.linear_offset",
            [("value", 0.1), ("", 0.0), ("", 0.0)],
        ),
        (
            "rusttable.rgb_gain",
            [("red", 1.0), ("green", 0.75), ("blue", 0.5)],
        ),
    ];
    for (index, (key, values)) in cases.into_iter().enumerate() {
        let parameters =
            values
                .into_iter()
                .filter(|(name, _)| !name.is_empty())
                .map(|(name, value)| {
                    (
                        ParameterName::new(name).expect("built-in parameter"),
                        ParameterValue::Scalar(FiniteF64::new(value).expect("finite value")),
                    )
                });
        let operation = Operation::new(
            OperationId::new(u128::try_from(index + 1).expect("small ID")).expect("operation ID"),
            OperationKey::from_str(key).expect("built-in key"),
            true,
            parameters,
        )
        .map_err(|error| format!("operation registry: smoke operation failed: {error}"))?;
        let prepared = builtin_registry()
            .prepare_cpu(&operation)
            .map_err(|error| format!("operation registry: factory failed: {error}"))?;
        let finite = FiniteF32::new(0.25).expect("finite pixel");
        let mut pixels = [LinearRgb::new(finite, finite, finite)];
        prepared
            .execute(PipelineStepIndex::new(index), &mut pixels, 0)
            .map_err(|error| format!("operation registry: executor failed: {error}"))?;
    }
    Ok(())
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

pub(crate) fn verify_registry_source_map(root: &Path) -> Result {
    let path = root.join("architecture/rusttable-operation-registry-source-map.toml");
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("operation registry source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("operation registry source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str)
        != Some("rusttable.operation-registry-source-map.v1")
        || document.get("issue").and_then(toml::Value::as_integer) != Some(265)
        || document
            .get("upstream_commit")
            .and_then(toml::Value::as_str)
            != Some(PINNED_COMMIT)
    {
        return Err(
            "operation registry source map: schema, issue, or pinned commit is invalid".to_owned(),
        );
    }
    verify_registry_responsibilities(root, &document)?;
    verify_registry_operations(root, &document)
}

fn verify_registry_responsibilities(root: &Path, document: &toml::Value) -> Result {
    let responsibilities = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "operation registry source map: responsibilities are missing".to_owned())?;
    let expected_responsibilities = [
        "dynamic-discovery-and-binding",
        "dynamic-symbol-resolution",
        "required-operation-exports",
        "pipeline-preparation",
        "operation-enumeration",
    ];
    if responsibilities.len() != expected_responsibilities.len() {
        return Err("operation registry source map: responsibility count is incomplete".to_owned());
    }
    let mut seen_responsibilities = std::collections::BTreeSet::new();
    for entry in responsibilities {
        let table = entry.as_table().ok_or_else(|| {
            "operation registry source map: responsibility is not a table".to_owned()
        })?;
        let id = table
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                "operation registry source map: responsibility ID is missing".to_owned()
            })?;
        if !expected_responsibilities.contains(&id) || !seen_responsibilities.insert(id) {
            return Err(format!(
                "operation registry source map: unexpected or duplicate responsibility {id}"
            ));
        }
        let owner = table
            .get("rust_path")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("operation registry source map: {id} owner is missing"))?;
        if !root.join(owner).is_file()
            || table.get("status").and_then(toml::Value::as_str) != Some("replaced")
        {
            return Err(format!(
                "operation registry source map: invalid responsibility {id}"
            ));
        }
    }
    Ok(())
}

fn verify_registry_operations(root: &Path, document: &toml::Value) -> Result {
    let entries = document
        .get("operation")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "operation registry source map: operation entries are missing".to_owned())?;
    let expected = [
        "rusttable.exposure",
        "rusttable.linear_offset",
        "rusttable.rgb_gain",
    ];
    if entries.len() != expected.len() {
        return Err(format!(
            "operation registry source map: expected {} entries",
            expected.len()
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "operation registry source map: entry is not a table".to_owned())?;
        let rust_id = table
            .get("rust_id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "operation registry source map: rust_id is missing".to_owned())?;
        if !expected.contains(&rust_id) || !seen.insert(rust_id) {
            return Err(format!(
                "operation registry source map: unexpected or duplicate {rust_id}"
            ));
        }
        let owner = table
            .get("owner")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("operation registry source map: {rust_id} owner is missing"))?;
        if !root.join(owner).is_file() {
            return Err(format!(
                "operation registry source map: missing owner {owner}"
            ));
        }
        if table.get("status").and_then(toml::Value::as_str) != Some("replaced") {
            return Err(format!(
                "operation registry source map: {rust_id} is not replaced"
            ));
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
