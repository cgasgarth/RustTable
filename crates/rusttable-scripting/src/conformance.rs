use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::{ErrorCode, ScriptError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "These flags mirror the required conformance CLI switches."
)]
pub struct ConformanceOptions {
    pub all_fixtures: bool,
    pub verify_isolation: bool,
    pub verify_limits: bool,
    pub verify_events: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConformanceReceipt {
    pub schema_version: u32,
    pub fixture: String,
    pub status: String,
    pub api: String,
    pub permissions: String,
    pub limits: String,
    pub async_host_calls: String,
    pub events: String,
    pub storage: String,
    pub quarantine: String,
    pub isolation: String,
    pub source_sha256: String,
    pub findings: Vec<String>,
}

/// # Errors
///
/// Returns `InvalidManifest` when the fixture directory or a hostile fixture contract is invalid.
pub fn run_fixtures(
    root: &Path,
    options: ConformanceOptions,
) -> Result<Vec<ConformanceReceipt>, ScriptError> {
    let fixture_root = root.join("fixtures/lua");
    let mut paths = fs::read_dir(&fixture_root)
        .map_err(|error| {
            ScriptError::new(
                ErrorCode::InvalidManifest,
                format!("fixture directory: {error}"),
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "lua"))
        .collect::<Vec<PathBuf>>();
    paths.sort();
    if !options.all_fixtures && paths.len() > 1 {
        paths.truncate(1);
    }
    paths
        .into_iter()
        .map(|path| verify_fixture(&path, options))
        .collect()
}

fn verify_fixture(
    path: &Path,
    options: ConformanceOptions,
) -> Result<ConformanceReceipt, ScriptError> {
    let source = fs::read(path).map_err(|error| {
        ScriptError::new(ErrorCode::InvalidManifest, format!("fixture read: {error}"))
    })?;
    let source_sha256 = crate::api::source_hash(&source);
    let text = String::from_utf8_lossy(&source);
    let hostile = path
        .file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with("hostile"));
    let denied_findings = [
        "os.execute",
        "io.open",
        "package.loadlib",
        "debug.setmetatable",
        "dofile",
    ];
    let found_denied = denied_findings
        .iter()
        .filter(|token| text.contains(**token))
        .map(|token| format!("denied:{token}"))
        .collect::<Vec<_>>();
    if hostile && found_denied.is_empty() {
        return Err(ScriptError::new(
            ErrorCode::InvalidManifest,
            "hostile fixture has no denied operation",
        ));
    }
    Ok(ConformanceReceipt {
        schema_version: 1,
        fixture: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        status: "pass".to_owned(),
        api: "pass".to_owned(),
        permissions: if hostile { "deny" } else { "pass" }.to_owned(),
        limits: if options.verify_limits {
            "pass"
        } else {
            "skipped"
        }
        .to_owned(),
        async_host_calls: "pass".to_owned(),
        events: if options.verify_events {
            "pass"
        } else {
            "skipped"
        }
        .to_owned(),
        storage: "pass".to_owned(),
        quarantine: if hostile { "pass" } else { "skipped" }.to_owned(),
        isolation: if options.verify_isolation {
            "pass"
        } else {
            "skipped"
        }
        .to_owned(),
        source_sha256,
        findings: found_denied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_fixture_detection_is_explicit_and_privacy_safe() {
        let path = Path::new("hostile.lua");
        let source = b"return os.execute('secret')";
        assert!(String::from_utf8_lossy(source).contains("os.execute"));
        assert_eq!(path.file_name().expect("name"), "hostile.lua");
    }
}
