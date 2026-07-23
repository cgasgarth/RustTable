use std::path::Path;
use std::process::Command;
use std::thread;

use crate::{Result, codegen, export_contract, fixtures, numerics, operations, run_process_quiet};
type CheckFn = fn(&Path) -> Result;

const CHECKS: &[(&str, CheckFn)] = &[
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
        assert_eq!(CHECKS.len(), 7);
        assert_eq!(CHECKS[0].0, "numerical contracts");
        assert_eq!(CHECKS[2].0, "operation codegen");
        assert_eq!(CHECKS[5].0, "fixtures");
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
}
