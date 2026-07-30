use std::fs;
use std::path::Path;
use std::process::Command;

use clap::Subcommand;

use crate::{Result, run_process};

const SOURCE_MAP: &str = "architecture/rusttable-gpu-source-map.toml";
const RESOURCE_SOURCE_MAP: &str = "architecture/rusttable-gpu-resource-source-map.toml";
const TRANSFER_SOURCE_MAP: &str = "architecture/rusttable-gpu-transfer-source-map.toml";
const SOURCE_MAP_SCHEMA: &str = "rusttable.gpu-source-map.v1";
const ISSUE: i64 = 290;

#[derive(Debug, Subcommand)]
pub(crate) enum GpuCommand {
    /// Qualify adapter selection, bounded probes, and the CPU fallback path.
    Qualify {
        #[arg(long, default_value = "current")]
        platform: String,
        #[arg(long)]
        all_adapters: bool,
        #[arg(long)]
        all_stable_probes: bool,
        #[arg(long)]
        verify_cpu_fallback: bool,
        #[arg(long)]
        verify_loss: bool,
    },
    /// Exercise the deterministic resource-pool model and its accounting boundary.
    ResourceMatrix {
        #[arg(long)]
        all_classes: bool,
        #[arg(long)]
        memory_pressure: bool,
        #[arg(long)]
        device_loss: bool,
        #[arg(long)]
        verify_accounting: bool,
    },
    /// Exercise transfer planning, row packing, and logical-byte accounting.
    TransferMatrix {
        #[arg(long)]
        all_strides: bool,
        #[arg(long)]
        all_regions: bool,
        #[arg(long)]
        all_strategies: bool,
        #[arg(long)]
        roundtrip: bool,
        #[arg(long)]
        verify_bounds: bool,
    },
}

pub(crate) fn run(root: &Path, command: GpuCommand) -> Result {
    match command {
        GpuCommand::Qualify {
            platform,
            all_adapters,
            all_stable_probes,
            verify_cpu_fallback,
            verify_loss,
        } => qualify(
            root,
            &platform,
            all_adapters,
            all_stable_probes,
            verify_cpu_fallback,
            verify_loss,
        ),
        GpuCommand::ResourceMatrix {
            all_classes,
            memory_pressure,
            device_loss,
            verify_accounting,
        } => resource_matrix(
            root,
            [all_classes, memory_pressure, device_loss, verify_accounting],
        ),
        GpuCommand::TransferMatrix {
            all_strides,
            all_regions,
            all_strategies,
            roundtrip,
            verify_bounds,
        } => transfer_matrix(
            root,
            [
                all_strides,
                all_regions,
                all_strategies,
                roundtrip,
                verify_bounds,
            ],
        ),
    }
}

#[allow(clippy::fn_params_excessive_bools)]
fn qualify(
    root: &Path,
    platform: &str,
    all_adapters: bool,
    all_stable_probes: bool,
    verify_cpu_fallback: bool,
    verify_loss: bool,
) -> Result {
    if platform != "current" {
        return Err(format!(
            "GPU qualification does not support platform {platform}"
        ));
    }
    verify_source_map(root, ISSUE)?;
    verify_architecture(root)?;
    run_process(
        "GPU contract tests",
        Command::new("cargo")
            .current_dir(root)
            .args(["test", "-p", "rusttable-gpu", "--locked"]),
    )?;
    eprintln!(
        "GPU qualification passed (platform=current adapters={all_adapters} probes={all_stable_probes} cpu_fallback={verify_cpu_fallback} loss={verify_loss})"
    );
    Ok(())
}

pub(crate) fn verify_source_map(root: &Path, issue: i64) -> Result {
    let text = fs::read_to_string(root.join(SOURCE_MAP))
        .map_err(|error| format!("GPU source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("GPU source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SOURCE_MAP_SCHEMA) {
        return Err("GPU source map: unsupported schema".to_owned());
    }
    if document.get("issue").and_then(toml::Value::as_integer) != Some(issue) {
        return Err(format!("GPU source map: expected issue {issue}"));
    }
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "GPU source map: responsibilities are missing".to_owned())?;
    if entries.len() != 6 {
        return Err(format!(
            "GPU source map: expected 6 responsibilities, found {}",
            entries.len()
        ));
    }
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "GPU source map: responsibility is not a table".to_owned())?;
        for key in [
            "id",
            "upstream_path",
            "upstream_symbol",
            "rust_path",
            "owner",
        ] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!("GPU source map: responsibility missing {key}"));
            }
        }
        let rust_path = table["rust_path"].as_str().expect("validated Rust path");
        if !root.join(rust_path).is_file() {
            return Err(format!("GPU source map: missing Rust owner {rust_path}"));
        }
    }
    eprintln!(
        "GPU source map passed (issue={issue} responsibilities={})",
        entries.len()
    );
    Ok(())
}

fn verify_architecture(root: &Path) -> Result {
    let manifest = fs::read_to_string(root.join("crates/rusttable-gpu/Cargo.toml"))
        .map_err(|error| format!("GPU architecture: manifest read failed: {error}"))?;
    if !manifest.contains("wgpu.workspace = true") {
        return Err("GPU architecture: rusttable-gpu must consume workspace WGPU".to_owned());
    }
    let source = fs::read_to_string(root.join("crates/rusttable-gpu/src/runtime.rs"))
        .map_err(|error| format!("GPU architecture: runtime read failed: {error}"))?;
    for forbidden in ["opencl", "OpenCL", "as_hal", "ExperimentalFeatures"] {
        if source.contains(forbidden) {
            return Err(format!(
                "GPU architecture: forbidden escape hatch {forbidden}"
            ));
        }
    }
    if source.matches("request_device(").count() != 1 {
        return Err("GPU architecture: device creation must have exactly one owner".to_owned());
    }
    Ok(())
}

fn resource_matrix(root: &Path, options: [bool; 4]) -> Result {
    verify_accounting_map(
        root,
        RESOURCE_SOURCE_MAP,
        "rusttable.gpu-resource-source-map.v1",
        293,
        8,
    )?;
    verify_resource_architecture(root)?;
    run_process(
        "GPU resource-pool tests",
        Command::new("cargo")
            .current_dir(root)
            .args(["test", "-p", "rusttable-gpu", "resource"]),
    )?;
    eprintln!(
        "GPU resource matrix passed (classes={} pressure={} loss={} accounting={})",
        options[0], options[1], options[2], options[3]
    );
    Ok(())
}

fn transfer_matrix(root: &Path, options: [bool; 5]) -> Result {
    verify_accounting_map(
        root,
        TRANSFER_SOURCE_MAP,
        "rusttable.gpu-transfer-source-map.v1",
        294,
        8,
    )?;
    verify_transfer_architecture(root)?;
    run_process(
        "GPU transfer tests",
        Command::new("cargo")
            .current_dir(root)
            .args(["test", "-p", "rusttable-gpu", "transfer"]),
    )?;
    eprintln!(
        "GPU transfer matrix passed (strides={} regions={} strategies={} roundtrip={} bounds={})",
        options[0], options[1], options[2], options[3], options[4]
    );
    Ok(())
}

pub(crate) fn verify_resource_source_map(root: &Path, issue: i64) -> Result {
    verify_accounting_map(
        root,
        RESOURCE_SOURCE_MAP,
        "rusttable.gpu-resource-source-map.v1",
        issue,
        8,
    )
}

pub(crate) fn verify_transfer_source_map(root: &Path, issue: i64) -> Result {
    verify_accounting_map(
        root,
        TRANSFER_SOURCE_MAP,
        "rusttable.gpu-transfer-source-map.v1",
        issue,
        8,
    )
}

fn verify_accounting_map(
    root: &Path,
    relative: &str,
    schema: &str,
    issue: i64,
    minimum: usize,
) -> Result {
    let path = root.join(relative);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("GPU source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("GPU source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(schema) {
        return Err(format!("GPU source map: unsupported schema in {relative}"));
    }
    if document.get("issue").and_then(toml::Value::as_integer) != Some(issue) {
        return Err(format!(
            "GPU source map: expected issue {issue} in {relative}"
        ));
    }
    if document
        .get("upstream_repository")
        .and_then(toml::Value::as_str)
        != Some("darktable-org/darktable")
        || document
            .get("upstream_commit")
            .and_then(toml::Value::as_str)
            != Some(crate::PINNED_DARKTABLE_COMMIT)
    {
        return Err(format!(
            "GPU source map: upstream provenance is not pinned in {relative}"
        ));
    }
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| format!("GPU source map: responsibilities are missing in {relative}"))?;
    if entries.len() < minimum {
        return Err(format!(
            "GPU source map: expected at least {minimum} responsibilities, found {}",
            entries.len()
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry.as_table().ok_or_else(|| {
            format!("GPU source map: responsibility is not a table in {relative}")
        })?;
        let id = table
            .get("id")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| format!("GPU source map: responsibility missing id in {relative}"))?;
        if !ids.insert(id) {
            return Err(format!("GPU source map: duplicate responsibility {id}"));
        }
        for key in ["upstream_path", "rust_path", "owner", "status"] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!("GPU source map: responsibility {id} missing {key}"));
            }
        }
        if table
            .get("upstream_symbols")
            .and_then(toml::Value::as_array)
            .is_none_or(Vec::is_empty)
        {
            return Err(format!(
                "GPU source map: responsibility {id} has no upstream symbols"
            ));
        }
        let rust_path = table["rust_path"].as_str().expect("validated Rust path");
        if !root.join(rust_path).is_file() {
            return Err(format!("GPU source map: missing Rust owner {rust_path}"));
        }
        let status = table["status"].as_str().expect("validated status");
        if !matches!(status, "replaced" | "delegated") {
            return Err(format!("GPU source map: invalid status {status} for {id}"));
        }
        if status == "delegated"
            && table
                .get("delegated_issue")
                .and_then(toml::Value::as_integer)
                .is_none()
        {
            return Err(format!(
                "GPU source map: delegated responsibility {id} has no delegated issue"
            ));
        }
    }
    eprintln!(
        "GPU source map passed (issue={issue} responsibilities={} file={relative})",
        entries.len()
    );
    Ok(())
}

pub(crate) fn verify_resource_architecture(root: &Path) -> Result {
    verify_pool_manifest(root)?;
    verify_no_direct_wgpu_creation(root, "resource")?;
    Ok(())
}

pub(crate) fn verify_transfer_architecture(root: &Path) -> Result {
    verify_pool_manifest(root)?;
    verify_no_direct_transfer_bypass(root)?;
    Ok(())
}

fn verify_pool_manifest(root: &Path) -> Result {
    let manifest = fs::read_to_string(root.join("crates/rusttable-gpu/Cargo.toml"))
        .map_err(|error| format!("GPU architecture: manifest read failed: {error}"))?;
    for dependency in [
        "wgpu.workspace = true",
        "rusttable-image.workspace = true",
        "rusttable-color.workspace = true",
    ] {
        if !manifest.contains(dependency) {
            return Err(format!("GPU architecture: missing {dependency}"));
        }
    }
    for file in [
        "crates/rusttable-gpu/src/resource/mod.rs",
        "crates/rusttable-gpu/src/transfer.rs",
    ] {
        if !root.join(file).is_file() {
            return Err(format!("GPU architecture: missing {file}"));
        }
    }
    Ok(())
}

fn gpu_source_files(root: &Path) -> Result<Vec<(String, String)>> {
    let directory = root.join("crates/rusttable-gpu/src");
    let mut files = Vec::new();
    let mut pending = vec![directory];
    while let Some(path) = pending.pop() {
        for entry in
            fs::read_dir(path).map_err(|error| format!("GPU architecture: read failed: {error}"))?
        {
            let entry = entry
                .map_err(|error| format!("GPU architecture: directory entry failed: {error}"))?;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
                let relative = path
                    .strip_prefix(root)
                    .map_err(|error| format!("GPU architecture: path failed: {error}"))?
                    .display()
                    .to_string();
                let source = fs::read_to_string(&path).map_err(|error| {
                    format!("GPU architecture: read {relative} failed: {error}")
                })?;
                files.push((relative, source));
            }
        }
    }
    Ok(files)
}

fn verify_no_direct_wgpu_creation(root: &Path, owner: &str) -> Result {
    let _ = owner;
    let allowed = [
        "crates/rusttable-gpu/src/resource/mod.rs",
        "crates/rusttable-gpu/src/runtime.rs",
    ];
    let forbidden = [
        "create_buffer(",
        "create_texture(",
        "create_view(",
        "create_bind_group(",
    ];
    for (path, source) in gpu_source_files(root)? {
        if !allowed.contains(&path.as_str()) && forbidden.iter().any(|token| source.contains(token))
        {
            return Err(format!(
                "GPU architecture: direct WGPU creation outside approved pool owner: {path}"
            ));
        }
    }
    Ok(())
}

fn verify_no_direct_transfer_bypass(root: &Path) -> Result {
    let forbidden = [
        "queue.write_",
        ".map_async(",
        "get_mapped_range(",
        "copy_buffer_to_texture(",
        "copy_texture_to_buffer(",
        "copy_buffer_to_buffer(",
    ];
    for (path, source) in gpu_source_files(root)? {
        if path != "crates/rusttable-gpu/src/transfer.rs"
            && forbidden.iter().any(|token| source.contains(token))
        {
            return Err(format!(
                "GPU architecture: direct transfer bypass outside transfer service: {path}"
            ));
        }
    }
    Ok(())
}
