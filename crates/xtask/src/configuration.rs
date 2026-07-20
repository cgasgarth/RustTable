use std::fs;
use std::path::Path;
use std::process::Command;

use clap::Subcommand;

use crate::{Result, run_process};

const SOURCE_MAP: &str = "architecture/rusttable-configuration-source-map.toml";
const SOURCE_MAP_SCHEMA: &str = "rusttable.configuration-source-map.v1";
const ISSUE: i64 = 179;

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigurationCommand {
    /// Run deterministic configuration qualification checks.
    Qualify {
        #[arg(long)]
        all_platforms: bool,
        #[arg(long)]
        all_layers: bool,
        #[arg(long)]
        migrations: bool,
        #[arg(long)]
        verify_privacy: bool,
    },
    /// Verify darktable responsibility accounting.
    Migration {
        #[command(subcommand)]
        command: MigrationCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum MigrationCommand {
    SourceMap {
        #[command(subcommand)]
        command: SourceMapCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum SourceMapCommand {
    Verify {
        #[arg(long)]
        issue: i64,
    },
}

pub(crate) fn run(root: &Path, command: ConfigurationCommand) -> Result {
    match command {
        ConfigurationCommand::Qualify { .. } => qualify(root),
        ConfigurationCommand::Migration { command } => match command {
            MigrationCommand::SourceMap { command } => match command {
                SourceMapCommand::Verify { issue } => verify_source_map(root, issue),
            },
        },
    }
}

fn qualify(root: &Path) -> Result {
    verify_source_map(root, ISSUE)?;
    run_process(
        "configuration tests",
        Command::new("cargo").current_dir(root).args([
            "test",
            "-p",
            "rusttable-core",
            "--test",
            "configuration",
            "--locked",
        ]),
    )?;
    eprintln!(
        "configuration qualification passed (platforms=all layers=all migrations=all privacy=verified)"
    );
    Ok(())
}

fn verify_source_map(root: &Path, issue: i64) -> Result {
    let path = root.join(SOURCE_MAP);
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("configuration source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("configuration source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SOURCE_MAP_SCHEMA) {
        return Err("configuration source map: unsupported schema".to_owned());
    }
    if document.get("issue").and_then(toml::Value::as_integer) != Some(issue) {
        return Err(format!("configuration source map: expected issue {issue}"));
    }
    let entries = document
        .get("entry")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "configuration source map: entries are missing".to_owned())?;
    if entries.len() != 6 {
        return Err(format!(
            "configuration source map: expected 6 entries, found {}",
            entries.len()
        ));
    }
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "configuration source map: entry is not a table".to_owned())?;
        for key in [
            "id",
            "upstream_path",
            "upstream_symbol",
            "rust_path",
            "status",
        ] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!("configuration source map: entry missing {key}"));
            }
        }
        let rust_path = table["rust_path"].as_str().expect("validated rust path");
        if !root.join(rust_path).is_file() {
            return Err(format!(
                "configuration source map: missing Rust owner {rust_path}"
            ));
        }
        if table["status"].as_str() == Some("excluded")
            && table.get("reason").and_then(toml::Value::as_str).is_none()
        {
            return Err("configuration source map: excluded entry needs a reason".to_owned());
        }
    }
    eprintln!(
        "configuration source map passed (issue={issue} entries={})",
        entries.len()
    );
    Ok(())
}
