use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::model::Finding;
use crate::root::RepositoryRoot;

use super::super::Result;

pub(super) fn digest_json<T: Serialize>(value: &T) -> Result<String> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| format!("serialize checksum input: {error}"))?;
    Ok(hex_digest(&bytes))
}

pub(super) fn body_hash(body: &str) -> String {
    crate::commands::issue_spec::canonical_body_hash(body)
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        })
}

pub(super) fn parse_toml(root: &RepositoryRoot, path: &str) -> Result<Value> {
    let text =
        fs::read_to_string(root.join(path)).map_err(|error| format!("read {path}: {error}"))?;
    text.parse::<toml::Value>()
        .map(value_to_json)
        .map_err(|error| format!("parse {path}: {error}"))
}

fn value_to_json(value: toml::Value) -> Value {
    match value {
        toml::Value::String(value) => Value::String(value),
        toml::Value::Integer(value) => Value::Number(value.into()),
        toml::Value::Float(value) => {
            serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number)
        }
        toml::Value::Boolean(value) => Value::Bool(value),
        toml::Value::Datetime(value) => Value::String(value.to_string()),
        toml::Value::Array(values) => Value::Array(values.into_iter().map(value_to_json).collect()),
        toml::Value::Table(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, value_to_json(value)))
                .collect(),
        ),
    }
}

pub(super) fn write_json<T: Serialize>(
    root: &RepositoryRoot,
    path: &Path,
    value: &T,
) -> std::result::Result<(), String> {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let mut text = serde_json::to_string_pretty(value)
        .map_err(|error| format!("serialize {}: {error}", path.display()))?;
    text.push('\n');
    fs::write(&target, text).map_err(|error| format!("write {}: {error}", target.display()))
}

pub(super) fn write_toml<T: Serialize>(
    root: &RepositoryRoot,
    path: &Path,
    value: &T,
) -> std::result::Result<(), String> {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let mut text = toml::to_string_pretty(value)
        .map_err(|error| format!("serialize {}: {error}", path.display()))?;
    text = format!("{}\n", text.trim_end());
    fs::write(&target, text).map_err(|error| format!("write {}: {error}", target.display()))
}

pub(super) fn read_json<T: for<'de> Deserialize<'de>>(
    root: &RepositoryRoot,
    path: &Path,
) -> Result<T> {
    let text = fs::read_to_string(root.join(path))
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))
}

pub(super) fn read_json_path(root: &RepositoryRoot, path: &Path) -> Result<Value> {
    let text = fs::read_to_string(root.join(path))
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    serde_json::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))
}

pub(super) fn read_toml<T: for<'de> Deserialize<'de>>(
    root: &RepositoryRoot,
    path: &Path,
) -> Result<T> {
    let text = fs::read_to_string(root.join(path))
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    toml::from_str(&text).map_err(|error| format!("parse {}: {error}", path.display()))
}

pub(super) fn format_findings(context: &str, findings: &[Finding]) -> String {
    let value = serde_json::to_string(findings).unwrap_or_else(|_| "[]".to_owned());
    format!("{context} failed: {value}")
}

pub(super) fn format_findings_value(context: &str, value: &Value) -> String {
    format!(
        "{context} failed: {}",
        serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned())
    )
}
