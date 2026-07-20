use std::fs;
use std::path::Path;
use std::process::Command;

use clap::Subcommand;

use crate::{Result, run_process};

const SOURCE_MAP: &str = "architecture/rusttable-gpu-source-map.toml";
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
