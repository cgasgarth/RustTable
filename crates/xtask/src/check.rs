use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::{collections::BTreeSet, env};

use crate::{Result, codegen, export_contract, fixtures, numerics, operations, run_process_quiet};
use sha2::{Digest, Sha256};

const FORBIDDEN_NATIVE_EXTENSIONS: &[&str] = &[
    "c", "cc", "cl", "cmake", "cpp", "cxx", "h", "hh", "hpp", "m", "mm", "s",
];
const FORBIDDEN_NATIVE_FILENAMES: &[&str] = &["CMakeLists.txt", "ConfigureChecks.cmake"];
const MAX_CHANGED_RUST_LINES: usize = 1_000;
type CheckFn = fn(&Path) -> Result;

const CHECKS: &[(&str, CheckFn)] = &[
    ("source policy", verify_sources),
    ("numerical contracts", numerics::verify_registered_choices),
    (
        "cargo format, clippy, tests, and rustdoc",
        run_cargo_pipeline,
    ),
    ("operation codegen", verify_codegen),
    ("operation manifest", verify_operations),
    ("export contract", verify_export_contract),
    ("fixtures", verify_fixtures),
    (
        "dependency advisories, licenses, and sources",
        dependency_checks,
    ),
];

pub(crate) fn run(root: &Path, parallel: bool) -> Result {
    if parallel {
        run_parallel(root)?;
    } else {
        run_sequential(root)?;
    }
    eprintln!(
        "PASS xtask check (mode={}, branches={}, cargo-owner=1)",
        if parallel { "parallel" } else { "sequential" },
        CHECKS.len()
    );
    Ok(())
}

fn run_sequential(root: &Path) -> Result {
    for (_, check) in CHECKS {
        check(root)?;
    }
    Ok(())
}

fn run_parallel(root: &Path) -> Result {
    run_parallel_checks(root, CHECKS)
}

fn run_parallel_checks(root: &Path, checks: &[(&str, CheckFn)]) -> Result {
    thread::scope(|scope| {
        let handles = checks
            .iter()
            .map(|&(_, check)| scope.spawn(move || check(root)))
            .collect::<Vec<_>>();
        let mut failures = Vec::new();
        for ((label, _), handle) in checks.iter().zip(handles) {
            match handle.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => failures.push(format!("{label}: {error}")),
                Err(_) => failures.push(format!("{label}: check thread panicked")),
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "pre-commit checks failed:\n{}",
                failures.join("\n")
            ))
        }
    })
}

fn run_cargo_pipeline(root: &Path) -> Result {
    run_process_quiet(
        "format",
        Command::new("cargo")
            .current_dir(root)
            .args(["fmt", "--all", "--", "--check"]),
    )?;
    run_process_quiet(
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
    run_process_quiet(
        "tests",
        Command::new("cargo").current_dir(root).args([
            "test",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--locked",
        ]),
    )?;
    run_process_quiet(
        "rustdoc",
        Command::new("cargo")
            .current_dir(root)
            .env("RUSTDOCFLAGS", "-Dwarnings")
            .args([
                "doc",
                "--workspace",
                "--all-features",
                "--no-deps",
                "--locked",
            ]),
    )?;
    Ok(())
}

fn verify_codegen(root: &Path) -> Result {
    codegen::verify_committed(root)
}

fn verify_operations(root: &Path) -> Result {
    operations::verify_operation_manifest(root)
}

fn verify_export_contract(root: &Path) -> Result {
    export_contract::run(root, true)
}

fn verify_fixtures(root: &Path) -> Result {
    fixtures::verify(root, Path::new("fixtures/manifest.toml"))
}

fn dependency_checks(root: &Path) -> Result {
    run_process_quiet(
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
    run_process_quiet(
        "source policy",
        Command::new("bash").arg(root.join("scripts/check-source-policy.sh")),
    )?;
    verify_asset_provenance(root)?;
    verify_changed_rust_source_sizes(root)?;
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
    }
    Ok(())
}

fn verify_changed_rust_source_sizes(root: &Path) -> Result {
    let base = source_size_diff_base(root)?;
    let mut paths = git_paths(
        root,
        &[
            "diff",
            "--name-only",
            "--diff-filter=ACMR",
            "-z",
            &base,
            "--",
            "*.rs",
        ],
    )?;
    paths.extend(git_paths(
        root,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            "*.rs",
        ],
    )?);

    let mut violations = Vec::new();
    for relative in paths {
        if source_size_exempt(&relative) {
            continue;
        }
        let source = fs::read_to_string(root.join(&relative))
            .map_err(|error| format!("source size policy: read {}: {error}", relative.display()))?;
        let lines = source.lines().count();
        if source_exceeds_line_limit(&source) {
            violations.push(format!("{} ({lines} lines)", relative.display()));
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "source size policy: changed handwritten Rust exceeds {MAX_CHANGED_RUST_LINES} lines: {}",
            violations.join(", ")
        ))
    }
}

fn source_size_diff_base(root: &Path) -> Result<String> {
    let mut candidates = Vec::new();
    if let Ok(base) = env::var("RUSTTABLE_SOURCE_SIZE_BASE") {
        candidates.push(base);
    }
    if let Ok(base) = env::var("GITHUB_BASE_REF") {
        candidates.push(format!("origin/{base}"));
    }
    candidates.push("origin/main".to_owned());
    candidates.push("HEAD".to_owned());

    for candidate in candidates {
        let output = Command::new("git")
            .current_dir(root)
            .args(["merge-base", "HEAD", &candidate])
            .output()
            .map_err(|error| format!("source size policy: could not find diff base: {error}"))?;
        if output.status.success() {
            return String::from_utf8(output.stdout)
                .map(|base| base.trim().to_owned())
                .map_err(|_| "source size policy: diff base is not UTF-8".to_owned());
        }
    }
    Err("source size policy: no usable Git diff base".to_owned())
}

fn git_paths(root: &Path, arguments: &[&str]) -> Result<BTreeSet<PathBuf>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .map_err(|error| format!("source size policy: could not list changed files: {error}"))?;
    if !output.status.success() {
        return Err("source size policy: Git file listing failed".to_owned());
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            std::str::from_utf8(path)
                .map(PathBuf::from)
                .map_err(|_| "source size policy: changed path is not UTF-8".to_owned())
        })
        .collect()
}

fn source_size_exempt(path: &Path) -> bool {
    path.starts_with("architecture")
        || path
            .components()
            .any(|component| component.as_os_str() == "generated")
        || path.file_name() == Some(OsStr::new("generated.rs"))
}

fn source_exceeds_line_limit(source: &str) -> bool {
    source.lines().count() > MAX_CHANGED_RUST_LINES
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

    fn passing_check(root: &Path) -> Result {
        root.exists()
            .then_some(())
            .ok_or_else(|| "test path does not exist".to_owned())
    }

    fn first_failing_check(_: &Path) -> Result {
        Err("first failure".to_owned())
    }

    fn second_failing_check(_: &Path) -> Result {
        Err("second failure".to_owned())
    }

    #[test]
    fn precommit_plan_has_one_shared_cargo_owner() {
        assert_eq!(
            CHECKS
                .iter()
                .filter(|(label, _)| label.starts_with("cargo "))
                .count(),
            1
        );
        assert_eq!(CHECKS.len(), 8);
        assert_eq!(CHECKS[0].0, "source policy");
        assert_eq!(CHECKS[1].0, "numerical contracts");
        assert_eq!(CHECKS[3].0, "operation codegen");
        assert_eq!(CHECKS[6].0, "fixtures");
    }

    #[test]
    fn parallel_check_runner_aggregates_all_failures() {
        let checks: &[(&str, CheckFn)] = &[
            ("first", first_failing_check),
            ("pass", passing_check),
            ("second", second_failing_check),
        ];
        let error = run_parallel_checks(Path::new("."), checks).expect_err("checks passed");
        assert_eq!(
            error,
            "pre-commit checks failed:\nfirst: first failure\nsecond: second failure"
        );
    }

    #[test]
    fn native_source_extensions_are_rejected() {
        assert!(forbidden_native_path(Path::new("src/legacy.c")));
        assert!(forbidden_native_path(Path::new("CMakeLists.txt")));
        assert!(!forbidden_native_path(Path::new("src/lib.rs")));
    }

    #[test]
    fn changed_source_size_policy_targets_handwritten_rust() {
        assert!(!source_size_exempt(Path::new("crates/app/src/runtime.rs")));
        assert!(source_size_exempt(Path::new(
            "crates/app/src/generated/registry.rs"
        )));
        assert!(source_size_exempt(Path::new("crates/app/src/generated.rs")));
        assert!(!source_exceeds_line_limit(
            &"line\n".repeat(MAX_CHANGED_RUST_LINES)
        ));
        assert!(source_exceeds_line_limit(
            &"line\n".repeat(MAX_CHANGED_RUST_LINES + 1)
        ));
    }
}
