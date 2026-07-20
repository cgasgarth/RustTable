use std::fs;
use std::path::Path;

use clap::Subcommand;
use rusttable_color::verify_builtin_contracts;

use crate::Result;

const SOURCE_MAP: &str = "architecture/rusttable-color-source-map.toml";
const SOURCE_MAP_SCHEMA: &str = "rusttable.color-source-map.v1";
const ISSUE: i64 = 257;
const PINNED_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";

#[derive(Debug, Subcommand)]
pub(crate) enum ColorCommand {
    /// Validate all built-in color identities and deterministic transform plans.
    Contracts {
        #[arg(long)]
        all_builtins: bool,
        #[arg(long)]
        verify_roundtrip: bool,
        #[arg(long)]
        verify_identities: bool,
    },
}

pub(crate) fn run(root: &Path, command: &ColorCommand) -> Result {
    match command {
        ColorCommand::Contracts {
            all_builtins,
            verify_roundtrip,
            verify_identities,
        } => {
            verify_source_map(root, ISSUE)?;
            verify_architecture(root)?;
            let receipt = verify_builtin_contracts(*verify_roundtrip, *verify_identities)
                .map_err(|error| error.to_string())?;
            if *all_builtins && receipt.builtins < 11 {
                return Err("color contracts: built-in inventory is incomplete".to_owned());
            }
            eprintln!(
                "color contracts passed (schema={} builtins={} identities={} roundtrips={})",
                receipt.schema_version,
                receipt.builtins,
                receipt.identity_plans,
                receipt.roundtrips
            );
            Ok(())
        }
    }
}

pub(crate) fn verify_source_map(root: &Path, issue: i64) -> Result {
    let path = root.join(SOURCE_MAP);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("color source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("color source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SOURCE_MAP_SCHEMA) {
        return Err("color source map: unsupported schema".to_owned());
    }
    if document.get("issue").and_then(toml::Value::as_integer) != Some(issue) {
        return Err(format!("color source map: expected issue {issue}"));
    }
    if document
        .get("upstream_commit")
        .and_then(toml::Value::as_str)
        != Some(PINNED_COMMIT)
    {
        return Err("color source map: upstream commit is not pinned".to_owned());
    }
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "color source map: responsibilities are missing".to_owned())?;
    if entries.len() != 14 {
        return Err(format!(
            "color source map: expected 14 responsibilities, found {}",
            entries.len()
        ));
    }
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "color source map: responsibility is not a table".to_owned())?;
        for key in ["id", "upstream_path", "rust_path", "status"] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!("color source map: responsibility missing {key}"));
            }
        }
        let rust_path = table["rust_path"].as_str().expect("validated rust path");
        if !root.join(rust_path).is_file() {
            return Err(format!("color source map: missing Rust owner {rust_path}"));
        }
        if table["status"].as_str() == Some("deferred")
            && table
                .get("deferred_issue")
                .and_then(toml::Value::as_integer)
                .is_none()
        {
            return Err("color source map: deferred responsibility needs an issue".to_owned());
        }
    }
    Ok(())
}

fn verify_architecture(root: &Path) -> Result {
    let manifest = fs::read_to_string(root.join("crates/rusttable-color/Cargo.toml"))
        .map_err(|error| format!("color architecture: manifest read failed: {error}"))?;
    for forbidden in ["gtk", "iced", "wgpu", "redb", "sqlite", "tokio", "catalog"] {
        if manifest.contains(forbidden) {
            return Err(format!(
                "color architecture: forbidden dependency {forbidden}"
            ));
        }
    }
    let image_manifest = fs::read_to_string(root.join("crates/rusttable-image/Cargo.toml"))
        .map_err(|error| format!("color architecture: image manifest read failed: {error}"))?;
    if !image_manifest.contains("rusttable-color.workspace = true") {
        return Err("color architecture: image crate does not consume rusttable-color".to_owned());
    }
    let image_source = fs::read_to_string(root.join("crates/rusttable-image/src/image.rs"))
        .map_err(|error| format!("color architecture: image source read failed: {error}"))?;
    if !image_source.contains("pub use rusttable_color::ColorEncoding") {
        return Err("color architecture: image crate has a parallel color enum".to_owned());
    }
    if image_source.contains("ColorProfileRef") {
        return Err(
            "color architecture: image crate retains a string profile reference".to_owned(),
        );
    }
    for consumer in [
        "crates/rusttable-image-io/Cargo.toml",
        "crates/rusttable-processing/Cargo.toml",
        "crates/rusttable-render/Cargo.toml",
        "crates/rusttable-export/Cargo.toml",
        "crates/rusttable-pixelpipe/Cargo.toml",
        "crates/rusttable-ui/Cargo.toml",
        "crates/rusttable-display-profile/Cargo.toml",
    ] {
        let manifest = fs::read_to_string(root.join(consumer))
            .map_err(|error| format!("color architecture: read {consumer} failed: {error}"))?;
        if !manifest.contains("rusttable-color.workspace = true") {
            return Err(format!(
                "color architecture: {consumer} lacks color dependency"
            ));
        }
    }
    let output = std::process::Command::new("rg")
        .current_dir(root)
        .args([
            "-n",
            "pub enum ColorEncoding|pub struct ProfileId|pub enum RenderingIntent",
            "crates",
            "--glob",
            "*.rs",
            "--glob",
            "!crates/rusttable-color/src/**",
            "--glob",
            "!crates/xtask/src/color.rs",
        ])
        .output()
        .map_err(|error| format!("color architecture: duplicate scan failed: {error}"))?;
    if !output.stdout.is_empty() {
        return Err(format!(
            "color architecture: parallel color domain declarations found:\n{}",
            String::from_utf8_lossy(&output.stdout)
        ));
    }
    Ok(())
}
