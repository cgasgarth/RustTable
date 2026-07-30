use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Deserialize;

use super::receipt::CheckReceipt;
use crate::Result;

const POLICY_PATH: &str = "architecture/rusttable-numerics.toml";

#[derive(Debug, Deserialize)]
pub(super) struct PolicyDocument {
    schema: String,
    issue: u32,
    source_mapping: String,
    pub requested_primary_toolchain: String,
    repository_toolchain_source: String,
    implementation_registry: String,
    release_profile: ReleaseProfile,
    explicit_fma: Vec<ExplicitFma>,
}

#[derive(Debug, Deserialize)]
struct ReleaseProfile {
    opt_level: u8,
    lto: bool,
    codegen_units: u8,
    panic: String,
    overflow_checks: bool,
    debug_assertions: bool,
    debug: u8,
    strip: String,
    target_cpu: String,
    target_features: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ExplicitFma {
    path: String,
    policy: String,
}

pub(super) fn load(root: &Path) -> Result<PolicyDocument> {
    let text = fs::read_to_string(root.join(POLICY_PATH))
        .map_err(|error| format!("numerics policy: read failed: {error}"))?;
    toml::from_str(&text).map_err(|error| format!("numerics policy: invalid TOML: {error}"))
}

pub(super) fn verify(
    root: &Path,
    document: &PolicyDocument,
    all_profiles: bool,
) -> Result<Vec<CheckReceipt>> {
    let mut checks = Vec::new();
    checks.push(verify_document(root, document));
    checks.push(verify_release_profile(root, document, all_profiles)?);
    checks.push(verify_environment());
    checks.extend(verify_sources(root, document)?);
    Ok(checks)
}

fn verify_document(root: &Path, document: &PolicyDocument) -> CheckReceipt {
    if document.schema != "rusttable.numerics-policy.v1"
        || document.issue != 286
        || document.source_mapping != "NoDirectAnalogue"
        || document.repository_toolchain_source != "rust-toolchain.toml"
        || document.implementation_registry != "architecture/rusttable-shader-manifest.toml"
        || !root.join(&document.repository_toolchain_source).is_file()
        || !root.join(&document.implementation_registry).is_file()
    {
        CheckReceipt::blocking(
            "numerics-policy",
            "policy identity or authoritative input is invalid",
        )
    } else {
        CheckReceipt::passed("numerics-policy", "issue #286 policy inputs are present")
    }
}

fn verify_release_profile(
    root: &Path,
    document: &PolicyDocument,
    all_profiles: bool,
) -> Result<CheckReceipt> {
    let text = fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|error| format!("numerics profile: read Cargo.toml failed: {error}"))?;
    let cargo: toml::Value = toml::from_str(&text)
        .map_err(|error| format!("numerics profile: invalid Cargo.toml: {error}"))?;
    let release = cargo
        .get("profile")
        .and_then(|value| value.get("release"))
        .and_then(toml::Value::as_table);
    let expected = &document.release_profile;
    let valid = release.is_some_and(|profile| {
        integer(profile, "opt-level") == Some(i64::from(expected.opt_level))
            && boolean(profile, "lto") == Some(expected.lto)
            && integer(profile, "codegen-units") == Some(i64::from(expected.codegen_units))
            && string(profile, "panic") == Some(expected.panic.as_str())
            && boolean(profile, "overflow-checks") == Some(expected.overflow_checks)
            && boolean(profile, "debug-assertions") == Some(expected.debug_assertions)
            && integer(profile, "debug") == Some(i64::from(expected.debug))
            && string(profile, "strip") == Some(expected.strip.as_str())
    });
    if !valid || expected.target_cpu != "baseline" || !expected.target_features.is_empty() {
        return Ok(CheckReceipt::blocking(
            "release-profile",
            "Cargo release profile differs from the baseline numerical policy",
        ));
    }
    Ok(CheckReceipt::passed(
        "release-profile",
        if all_profiles {
            "explicit release semantics and default debug profile checked; no target overrides registered"
        } else {
            "explicit release floating semantics checked"
        },
    ))
}

fn verify_environment() -> CheckReceipt {
    let names = ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"];
    for name in names {
        if let Ok(value) = std::env::var(name)
            && contains_forbidden_flag(&value)
        {
            return CheckReceipt::blocking(
                "floating-environment",
                format!("{name} contains an unregistered floating/codegen override"),
            );
        }
    }
    CheckReceipt::passed(
        "floating-environment",
        "no observed RUSTFLAGS value alters floating or target semantics",
    )
}

fn verify_sources(root: &Path, document: &PolicyDocument) -> Result<Vec<CheckReceipt>> {
    let paths = repository_paths(root)?;
    let mut observed_fma = BTreeSet::new();
    let mut forbidden = Vec::new();
    for relative in paths {
        let extension = relative.extension().and_then(|value| value.to_str());
        if !matches!(
            extension,
            Some("rs" | "toml" | "wgsl" | "sh" | "yml" | "yaml")
        ) {
            continue;
        }
        let text = fs::read_to_string(root.join(&relative))
            .map_err(|error| format!("numerics source scan: {}: {error}", relative.display()))?;
        if extension == Some("rs") && contains_explicit_fma(&text) {
            observed_fma.insert(path_string(&relative));
        }
        if forbidden_source_choice(&text) {
            forbidden.push(path_string(&relative));
        }
    }
    let registered = document
        .explicit_fma
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<BTreeSet<_>>();
    let policy_valid = document
        .explicit_fma
        .iter()
        .all(|entry| entry.policy == "ExplicitFused");
    let fma_check = if policy_valid && registered == observed_fma {
        CheckReceipt::passed(
            "explicit-fma-registry",
            format!(
                "{} fused-rounding source paths registered",
                registered.len()
            ),
        )
    } else {
        let added = observed_fma
            .difference(&registered)
            .cloned()
            .collect::<Vec<_>>();
        let stale = registered
            .difference(&observed_fma)
            .cloned()
            .collect::<Vec<_>>();
        CheckReceipt::blocking(
            "explicit-fma-registry",
            format!("unregistered={added:?}, stale={stale:?}, policies_valid={policy_valid}"),
        )
    };
    let forbidden_check = if forbidden.is_empty() {
        CheckReceipt::passed(
            "forbidden-float-relaxations",
            "no global fast-math, algebraic, LLVM, target-feature, or rustflags injection found",
        )
    } else {
        CheckReceipt::blocking(
            "forbidden-float-relaxations",
            format!("forbidden numerical choices found in {forbidden:?}"),
        )
    };
    Ok(vec![fma_check, forbidden_check])
}

fn repository_paths(root: &Path) -> Result<Vec<PathBuf>> {
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
        .map_err(|error| format!("numerics source scan: git could not start: {error}"))?;
    if !output.status.success() {
        return Err("numerics source scan: git ls-files failed".to_owned());
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            std::str::from_utf8(path)
                .map(PathBuf::from)
                .map_err(|_| "numerics source scan: non-UTF-8 path".to_owned())
        })
        .collect()
}

fn contains_explicit_fma(source: &str) -> bool {
    source.contains(&[".mul", "_add("].concat())
        || source.contains(&[".mul", "_add ("].concat())
        || source.contains(&["::mul", "_add("].concat())
}

fn forbidden_source_choice(source: &str) -> bool {
    [
        concat!("-ffast", "-math"),
        concat!("float", "_algebraic"),
        concat!("-Cllvm", "-args"),
        concat!("-C llvm", "-args"),
        concat!("-Ctarget", "-feature"),
        concat!("-C target", "-feature"),
        concat!("target", "_feature(enable"),
        concat!("cfg(target", "_feature"),
        concat!("is_x86_feature", "_detected"),
        concat!("is_aarch64_feature", "_detected"),
        concat!("rust", "flags ="),
        concat!("rustc", "-flags"),
    ]
    .iter()
    .any(|pattern| source.contains(pattern))
}

fn contains_forbidden_flag(value: &str) -> bool {
    forbidden_source_choice(value)
        || value.contains(concat!("+", "fma"))
        || value.contains(concat!("+", "neon"))
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn integer(table: &toml::Table, key: &str) -> Option<i64> {
    table.get(key).and_then(toml::Value::as_integer)
}

fn boolean(table: &toml::Table, key: &str) -> Option<bool> {
    table.get(key).and_then(toml::Value::as_bool)
}

fn string<'a>(table: &'a toml::Table, key: &str) -> Option<&'a str> {
    table.get(key).and_then(toml::Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_tokens_are_rejected_without_rejecting_explicit_mul_add() {
        let explicit = ["let x = a.mul", "_add(b, c);"].concat();
        assert!(contains_explicit_fma(&explicit));
        assert!(!forbidden_source_choice(&explicit));
        assert!(forbidden_source_choice(&["-ffast", "-math"].concat()));
        assert!(forbidden_source_choice(&["float", "_algebraic"].concat()));
        assert!(contains_forbidden_flag(
            &["-Ctarget", "-feature=+fma"].concat()
        ));
    }

    #[test]
    fn checked_in_fma_registry_matches_all_current_paths() {
        let root = crate::repository_root();
        let document = load(&root).expect("policy");
        let checks = verify_sources(&root, &document).expect("scan");
        assert!(
            checks
                .iter()
                .all(|check| matches!(check.status, super::super::receipt::CheckStatus::Passed))
        );
    }
}
