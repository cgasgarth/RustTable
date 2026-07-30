use std::fs;
use std::path::Path;

use crate::Result;

const SOURCE_MAP: &str = "architecture/rusttable-edit-history-source-map.toml";
const SCHEMA: &str = "rusttable.edit-history-source-map.v1";

pub(crate) fn verify_source_map(root: &Path, issue: i64) -> Result {
    let document = toml::from_str::<toml::Value>(
        &fs::read_to_string(root.join(SOURCE_MAP))
            .map_err(|error| format!("edit history source map: read failed: {error}"))?,
    )
    .map_err(|error| format!("edit history source map: invalid TOML: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some(SCHEMA)
        || document.get("issue").and_then(toml::Value::as_integer) != Some(issue)
    {
        return Err("edit history source map: schema or issue is invalid".to_owned());
    }
    let entries = document
        .get("responsibility")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "edit history source map: responsibilities are missing".to_owned())?;
    if entries.len() < 10 {
        return Err("edit history source map: accounting table is incomplete".to_owned());
    }
    let mut ids = std::collections::BTreeSet::new();
    for entry in entries {
        let table = entry
            .as_table()
            .ok_or_else(|| "edit history source map: responsibility is not a table".to_owned())?;
        for key in [
            "id",
            "upstream_path",
            "upstream_symbol",
            "rust_path",
            "status",
        ] {
            if table.get(key).and_then(toml::Value::as_str).is_none() {
                return Err(format!(
                    "edit history source map: responsibility missing {key}"
                ));
            }
        }
        let id = table["id"].as_str().expect("validated ID");
        if !ids.insert(id) {
            return Err(format!("edit history source map: duplicate {id}"));
        }
        let owner = table["rust_path"].as_str().expect("validated owner");
        if !root.join(owner).is_file() {
            return Err(format!(
                "edit history source map: missing Rust owner {owner}"
            ));
        }
        if table["status"].as_str() == Some("deferred")
            && table
                .get("owner_issue")
                .and_then(toml::Value::as_integer)
                .is_none()
        {
            return Err(format!(
                "edit history source map: deferred {id} has no owner issue"
            ));
        }
    }
    eprintln!(
        "edit history source map passed (issue={issue} responsibilities={})",
        entries.len()
    );
    Ok(())
}
