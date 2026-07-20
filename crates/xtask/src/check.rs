use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::{Result, codegen, fixtures, run_process};
use sha2::{Digest, Sha256};

const FORBIDDEN_NATIVE_EXTENSIONS: &[&str] = &[
    "c", "cc", "cl", "cmake", "cpp", "cxx", "h", "hh", "hpp", "m", "mm", "s",
];
const FORBIDDEN_NATIVE_FILENAMES: &[&str] = &["CMakeLists.txt", "ConfigureChecks.cmake"];
const MAX_HANDWRITTEN_LINES: usize = 1_000;

pub(crate) fn run(root: &Path) -> Result {
    verify_sources(root)?;
    run_process(
        "format",
        Command::new("cargo")
            .current_dir(root)
            .args(["fmt", "--all", "--", "--check"]),
    )?;
    run_process(
        "clippy",
        Command::new("cargo").current_dir(root).args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]),
    )?;
    run_process(
        "tests",
        Command::new("cargo").current_dir(root).args([
            "test",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
        ]),
    )?;
    codegen::verify_committed(root)?;
    fixtures::verify(root, Path::new("fixtures/manifest.toml"))?;
    dependency_checks(root)?;
    eprintln!("RustTable check passed");
    Ok(())
}

fn dependency_checks(root: &Path) -> Result {
    run_process(
        "dependency advisories, licenses, and sources",
        Command::new("cargo").current_dir(root).args([
            "deny",
            "check",
            "--hide-inclusion-graph",
            "advisories",
            "bans",
            "licenses",
            "sources",
        ]),
    )
}

fn verify_sources(root: &Path) -> Result {
    run_process(
        "source policy",
        Command::new("bash").arg(root.join("scripts/check-source-policy.sh")),
    )?;
    verify_asset_provenance(root)?;
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ])
        .output()
        .map_err(|error| format!("source policy: could not list tracked files: {error}"))?;
    if !output.status.success() {
        return Err("source policy: git ls-files failed".to_owned());
    }
    for raw in output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
    {
        let relative = std::str::from_utf8(raw)
            .map_err(|_| "source policy: tracked path is not UTF-8".to_owned())?;
        let path = root.join(relative);
        if !path.is_file() {
            continue;
        }
        if forbidden_native_path(&path) {
            return Err(format!(
                "source policy: native source is forbidden: {relative}"
            ));
        }
        if path.extension() == Some(OsStr::new("rs")) {
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("source policy: read {relative}: {error}"))?;
            if !source
                .lines()
                .next()
                .is_some_and(|line| line.contains("GENERATED"))
                && source.lines().count() > MAX_HANDWRITTEN_LINES
            {
                return Err(format!(
                    "source policy: {relative} exceeds {MAX_HANDWRITTEN_LINES} lines"
                ));
            }
        }
    }
    Ok(())
}

fn verify_asset_provenance(root: &Path) -> Result {
    let manifest_path = root.join("architecture/rusttable-assets.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("asset policy: read {}: {error}", manifest_path.display()))?;
    let document = toml::from_str::<toml::Value>(&manifest)
        .map_err(|error| format!("asset policy: invalid manifest: {error}"))?;
    if document.get("schema").and_then(toml::Value::as_str) != Some("rusttable.asset-provenance.v1")
    {
        return Err("asset policy: unsupported manifest schema".to_owned());
    }
    let assets = document
        .get("assets")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "asset policy: manifest has no assets".to_owned())?;
    if assets.len() != 1 {
        return Err(format!(
            "asset policy: expected one retained data asset, found {}",
            assets.len()
        ));
    }
    let asset = assets[0]
        .as_table()
        .ok_or_else(|| "asset policy: asset entry is not a table".to_owned())?;
    for key in [
        "path",
        "consumer",
        "source_repository",
        "source_commit",
        "source_path",
        "sha256",
    ] {
        if asset.get(key).and_then(toml::Value::as_str).is_none() {
            return Err(format!("asset policy: asset is missing {key}"));
        }
    }
    let path = asset["path"].as_str().expect("validated path");
    if path != "data/shortcutsrc" {
        return Err(format!("asset policy: unexpected retained asset {path}"));
    }
    if asset["source_repository"].as_str() != Some("darktable-org/darktable")
        || asset["source_commit"].as_str() != Some(crate::PINNED_DARKTABLE_COMMIT)
        || asset["source_path"].as_str() != Some("data/shortcutsrc")
    {
        return Err("asset policy: shortcutsrc provenance is not pinned".to_owned());
    }
    let asset_path = root.join(path);
    let bytes =
        fs::read(&asset_path).map_err(|error| format!("asset policy: read {path}: {error}"))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    if asset["sha256"].as_str() != Some(digest.as_str()) {
        return Err(format!(
            "asset policy: {path} hash does not match its manifest"
        ));
    }
    let consumer = root.join("crates/rusttable-input/src/persistence.rs");
    let source = fs::read_to_string(&consumer)
        .map_err(|error| format!("asset policy: read {}: {error}", consumer.display()))?;
    if !source.contains("include_str!(\"../../../data/shortcutsrc\")")
        || !source.contains("bundled_darktable_shortcuts_have_pinned_provenance")
    {
        return Err("asset policy: shortcutsrc consumer/provenance test is missing".to_owned());
    }
    Ok(())
}

fn forbidden_native_path(path: &Path) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| FORBIDDEN_NATIVE_FILENAMES.contains(&name))
        || path
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| {
                FORBIDDEN_NATIVE_EXTENSIONS
                    .iter()
                    .any(|forbidden| extension.eq_ignore_ascii_case(forbidden))
            })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_source_extensions_are_rejected() {
        assert!(forbidden_native_path(Path::new("src/legacy.c")));
        assert!(forbidden_native_path(Path::new("CMakeLists.txt")));
        assert!(!forbidden_native_path(Path::new("src/lib.rs")));
    }
}
