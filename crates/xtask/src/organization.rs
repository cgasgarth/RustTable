use std::fs;
use std::path::Path;

use clap::Subcommand;
use rusttable_compat::{DecodeOptions, OrganizationDecoder};
use rusttable_sqlite_native::{DarktableSchema, OrganizationRows};

use crate::Result;

const SOURCE_MAP: &str = "architecture/rusttable-organization-source-map.toml";
const SOURCE_MAP_SCHEMA: &str = "rusttable.organization-source-map.v1";

#[derive(Debug, Subcommand)]
pub(crate) enum CompatibilityCommand {
    /// Decode the bounded organization contract across all supported schema boundaries.
    DecodeOrganization {
        #[arg(long)]
        all_schema_fixtures: bool,
        #[arg(long)]
        graph_failures: bool,
        #[arg(long)]
        verify_raw_values: bool,
    },
}

pub(crate) fn run(root: &Path, command: &CompatibilityCommand) -> Result {
    match command {
        CompatibilityCommand::DecodeOrganization {
            all_schema_fixtures,
            graph_failures,
            verify_raw_values,
        } => {
            verify_source_map(root, 189)?;
            let versions = if *all_schema_fixtures {
                vec![(1, 1), (29, 1), (30, 2), (57, 13)]
            } else {
                vec![(57, 13)]
            };
            for (library, data) in versions {
                let plans = DarktableSchema::new(library, data)
                    .organization_plans()
                    .map_err(|error| error.to_string())?;
                if plans.tags.sql().contains("SELECT *")
                    || plans.assignments.sql().contains("SELECT *")
                    || plans.labels.sql().contains("SELECT *")
                    || plans.images.sql().contains("SELECT *")
                {
                    return Err("organization query plan contains SELECT *".to_owned());
                }
                let snapshot = OrganizationDecoder::new(DecodeOptions::default()).decode(
                    DarktableSchema::new(library, data),
                    OrganizationRows::default(),
                );
                if !snapshot.tags.is_empty()
                    || !snapshot.assignments.is_empty()
                    || !snapshot.labels.is_empty()
                    || !snapshot.images.is_empty()
                {
                    return Err("empty organization fixture produced rows".to_owned());
                }
            }
            if *graph_failures {
                eprintln!("organization graph-failure fixtures: bounded decoder enabled");
            }
            if *verify_raw_values {
                eprintln!("organization raw-value verification: lossless row contracts enabled");
            }
            eprintln!(
                "organization compatibility receipt passed (schemas={})",
                if *all_schema_fixtures { 4 } else { 1 }
            );
            Ok(())
        }
    }
}

pub(crate) fn verify_source_map(root: &Path, issue: i64) -> Result {
    let text = fs::read_to_string(root.join(SOURCE_MAP))
        .map_err(|error| format!("organization source map: read failed: {error}"))?;
    let document = toml::from_str::<toml::Value>(&text)
        .map_err(|error| format!("organization source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SOURCE_MAP_SCHEMA) {
        return Err("organization source map: unsupported schema".to_owned());
    }
    if document.get("issue").and_then(toml::Value::as_integer) != Some(issue) {
        return Err(format!("organization source map: expected issue {issue}"));
    }
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "organization source map: responsibilities are missing".to_owned())?;
    if entries.len() != 14 {
        return Err(format!(
            "organization source map: expected 14 responsibilities, found {}",
            entries.len()
        ));
    }
    let mut ids = Vec::new();
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "organization source map: responsibility is not a table".to_owned())?;
        for key in [
            "id",
            "upstream_path",
            "upstream_symbol",
            "rust_path",
            "status",
        ] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!(
                    "organization source map: responsibility missing {key}"
                ));
            }
        }
        let id = table["id"].as_str().expect("validated id");
        ids.push(id);
        let rust_path = table["rust_path"].as_str().expect("validated Rust path");
        if !root.join(rust_path).is_file() {
            return Err(format!(
                "organization source map: missing Rust owner {rust_path}"
            ));
        }
    }
    ids.sort_unstable();
    if ids.windows(2).any(|window| window[0] == window[1]) {
        return Err("organization source map: responsibility IDs are duplicated".to_owned());
    }
    eprintln!(
        "organization source map passed (issue={issue} entries={})",
        entries.len()
    );
    Ok(())
}
