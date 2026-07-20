use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::{Result, codegen, fixtures, run_process};

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
