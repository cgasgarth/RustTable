use std::fs;
use std::path::Path;
use std::str::FromStr;

use clap::Subcommand;
use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterText, ParameterValue,
};
use rusttable_processing::descriptor::{OperationFlags, exposure_descriptor, rgb_gain_descriptor};
use rusttable_processing::operation_stack::{OperationStackSnapshot, OperationStackTemplate};
use rusttable_processing::{
    FiniteF32, LinearRgb, OperationClassification, PipelineStepIndex, RasterDimensions,
    RegistryClosure, RegistryClosureEntry, builtin_registry,
};
use serde::Serialize;

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
            eprintln!(
                "operation registry generated (definitions={})",
                builtin_registry().definitions().len()
            );
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
                "operation registry check passed (definitions={}, snapshot={})",
                builtin_registry().definitions().len(),
                builtin_registry().identity_hash_hex()
            );
            Ok(())
        }
    }
}

pub(crate) fn run_manifest(root: &Path, check: bool) -> Result {
    let path = root.join("architecture/operation-capabilities.json");
    let rendered = render_operation_capabilities(root)?;
    if check {
        let committed = fs::read_to_string(&path)
            .map_err(|error| format!("operation manifest: read failed: {error}"))?;
        if committed != rendered {
            return Err(
                "operation manifest is stale; run cargo xtask operation-manifest generate"
                    .to_owned(),
            );
        }
        eprintln!("operation manifest closure verified: {}", path.display());
    } else {
        fs::write(&path, rendered)
            .map_err(|error| format!("operation manifest: write failed: {error}"))?;
        eprintln!("operation manifest closure generated: {}", path.display());
    }
    Ok(())
}

pub(crate) fn verify_operation_manifest(root: &Path) -> Result {
    run_manifest(root, true)
}

#[derive(Debug, Serialize)]
struct OperationCapabilitiesArtifact {
    schema: &'static str,
    reference_commit: String,
    reference_version: String,
    registry_hash: String,
    entries: Vec<RegistryClosureEntry>,
}

fn render_operation_capabilities(root: &Path) -> Result<String> {
    let manifest_path = root.join("architecture/darktable-operations.toml");
    let source = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("operation manifest: read failed: {error}"))?;
    let manifest = rusttable_parity::parse_operation_manifest(&source)
        .map_err(|error| format!("operation manifest: parse failed: {error}"))?;
    rusttable_parity::validate_operation_manifest(&manifest)
        .map_err(|error| format!("operation manifest: validation failed: {error}"))?;

    let registry = builtin_registry();
    let registry_closure = RegistryClosure::from_registry(registry)
        .map_err(|error| format!("operation manifest: registry failed: {error}"))?;
    let mut entries = registry_closure.entries.clone();
    for operation in &manifest.operations {
        let identity = format!(
            "darktable:{}:{}:v{}",
            operation.name, operation.reference_path, operation.module_version
        );
        let registered = registry
            .definitions()
            .iter()
            .find(|definition| definition.descriptor().id.compatibility_name == operation.name);
        let (rust_id, status, implementation_crate, cpu_supported, gpu_supported, reason) =
            match registered {
                Some(definition) => (
                    Some(definition.descriptor().id.rust_id.clone()),
                    if definition
                        .descriptor()
                        .flags
                        .contains(OperationFlags::DEPRECATED)
                    {
                        OperationClassification::DeprecatedImplemented
                    } else {
                        OperationClassification::Implemented
                    },
                    "rusttable-processing".to_owned(),
                    definition.cpu().is_some(),
                    definition.gpu().is_some(),
                    definition
                        .descriptor()
                        .flags
                        .contains(OperationFlags::DEPRECATED)
                        .then(|| {
                            "deprecated compatibility operation; hidden from new-edit discovery"
                                .to_owned()
                        }),
                ),
                None => (
                    None,
                    OperationClassification::IntentionallyUnsupportedBlocking,
                    String::new(),
                    operation.cpu_implementation != "none",
                    !operation.opencl_kernels.is_empty(),
                    Some("reference operation is not yet registered in RustTable".to_owned()),
                ),
            };
        let descriptor_version = u16::try_from(operation.module_version)
            .map_err(|_| format!("operation manifest: version is too large: {identity}"))?;
        let parameter_versions = if operation.parameter_versions.is_empty() {
            vec![descriptor_version]
        } else {
            operation
                .parameter_versions
                .iter()
                .map(|version| {
                    u16::try_from(version.version).map_err(|_| {
                        format!("operation manifest: parameter version is too large: {identity}")
                    })
                })
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        entries.push(RegistryClosureEntry {
            identity,
            compatibility_name: operation.name.clone(),
            rust_id,
            reference_path: operation.reference_path.clone(),
            descriptor_version,
            parameter_versions,
            implementation_crate,
            issue_sequence_id: 395,
            order: operation.default_order,
            cpu_supported,
            gpu_supported,
            cpu_fallback: cpu_supported,
            status,
            reason,
        });
    }
    let closure = RegistryClosure::new(entries)
        .map_err(|error| format!("operation manifest: closure failed: {error}"))?;
    let artifact = OperationCapabilitiesArtifact {
        schema: rusttable_processing::REGISTRY_CLOSURE_SCHEMA,
        reference_commit: manifest.reference.source_commit,
        reference_version: manifest.reference.build_version,
        registry_hash: closure.identity_hash().to_owned(),
        entries: closure.entries,
    };
    let mut rendered = serde_json::to_string_pretty(&artifact)
        .map_err(|error| format!("operation manifest: serialization failed: {error}"))?;
    rendered.push('\n');
    Ok(rendered)
}

#[allow(clippy::too_many_lines)]
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
        execute_smoke_operation(index, &operation)?;
    }
    execute_smoke_operation(
        3,
        &Operation::new(
            OperationId::new(4).expect("operation ID"),
            OperationKey::from_str("rusttable.colorin").expect("built-in key"),
            true,
            [
                (
                    ParameterName::new("input_profile").expect("parameter"),
                    ParameterValue::Text(ParameterText::new("srgb").expect("text")),
                ),
                (
                    ParameterName::new("working_profile").expect("parameter"),
                    ParameterValue::Text(ParameterText::new("linear_rec2020_rgb").expect("text")),
                ),
                (
                    ParameterName::new("intent").expect("parameter"),
                    ParameterValue::Integer(0),
                ),
                (
                    ParameterName::new("normalize").expect("parameter"),
                    ParameterValue::Integer(0),
                ),
                (
                    ParameterName::new("blue_mapping").expect("parameter"),
                    ParameterValue::Bool(true),
                ),
            ],
        )
        .map_err(|error| format!("operation registry: smoke operation failed: {error}"))?,
    )?;
    execute_smoke_operation(
        4,
        &Operation::new(
            OperationId::new(5).expect("operation ID"),
            OperationKey::from_str("rusttable.primaries").expect("built-in key"),
            true,
            [
                (
                    ParameterName::new("achromatic_tint_hue").expect("parameter"),
                    ParameterValue::Scalar(FiniteF64::new(0.0).expect("finite value")),
                ),
                (
                    ParameterName::new("achromatic_tint_purity").expect("parameter"),
                    ParameterValue::Scalar(FiniteF64::new(0.0).expect("finite value")),
                ),
                (
                    ParameterName::new("red_hue").expect("parameter"),
                    ParameterValue::Scalar(FiniteF64::new(0.0).expect("finite value")),
                ),
                (
                    ParameterName::new("red_purity").expect("parameter"),
                    ParameterValue::Scalar(FiniteF64::new(1.0).expect("finite value")),
                ),
                (
                    ParameterName::new("green_hue").expect("parameter"),
                    ParameterValue::Scalar(FiniteF64::new(0.0).expect("finite value")),
                ),
                (
                    ParameterName::new("green_purity").expect("parameter"),
                    ParameterValue::Scalar(FiniteF64::new(1.0).expect("finite value")),
                ),
                (
                    ParameterName::new("blue_hue").expect("parameter"),
                    ParameterValue::Scalar(FiniteF64::new(0.0).expect("finite value")),
                ),
                (
                    ParameterName::new("blue_purity").expect("parameter"),
                    ParameterValue::Scalar(FiniteF64::new(1.0).expect("finite value")),
                ),
            ],
        )
        .map_err(|error| format!("operation registry: smoke operation failed: {error}"))?,
    )?;
    Ok(())
}

fn execute_smoke_operation(index: usize, operation: &Operation) -> Result {
    let prepared = builtin_registry()
        .prepare_cpu(operation)
        .map_err(|error| format!("operation registry: factory failed: {error}"))?;
    let finite = FiniteF32::new(0.25).expect("finite pixel");
    let mut pixels = [LinearRgb::new(finite, finite, finite)];
    prepared
        .execute(
            PipelineStepIndex::new(index),
            &mut pixels,
            RasterDimensions::new(1, 1).expect("smoke dimensions"),
            0,
        )
        .map_err(|error| format!("operation registry: executor failed: {error}"))
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
        "rusttable.invert",
        "rusttable.dither",
        "rusttable.relight",
        "rusttable.shadhi",
        "rusttable.colorin",
        "rusttable.primaries",
        "rusttable.vignette",
        "rusttable.graduatednd",
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
