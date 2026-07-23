use std::path::Path;
use std::process::Command;

use rusttable_core::numerics::{CompilerFingerprint, ProbeStatus, REQUESTED_PRIMARY_TOOLCHAIN};

use super::policy::PolicyDocument;
use super::receipt::CheckReceipt;
use crate::Result;

pub(super) fn verify_active(root: &Path, document: &PolicyDocument) -> CheckReceipt {
    let fingerprint = match active_fingerprint(root) {
        Ok(fingerprint) => fingerprint,
        Err(error) => return CheckReceipt::blocking("compiler-fingerprint", error),
    };
    if document.requested_primary_toolchain != REQUESTED_PRIMARY_TOOLCHAIN {
        return CheckReceipt::blocking(
            "compiler-contract",
            "policy does not name the issue #286 requested primary archive",
        );
    }
    if !fingerprint
        .active_toolchain
        .starts_with(REQUESTED_PRIMARY_TOOLCHAIN)
    {
        return CheckReceipt::blocking(
            "compiler-primary-beta",
            format!(
                "requested {REQUESTED_PRIMARY_TOOLCHAIN}, observed {}; the requested rustup archive is not substituted",
                fingerprint.active_toolchain
            ),
        );
    }
    CheckReceipt::passed(
        "compiler-primary-beta",
        format!(
            "{} {} LLVM {}",
            fingerprint.active_toolchain, fingerprint.rustc_release, fingerprint.llvm_version
        ),
    )
}

pub(super) fn active_fingerprint(root: &Path) -> Result<CompilerFingerprint> {
    fingerprint(root)
}

pub(super) fn fingerprint_installed(root: &Path, requested: &str) -> Result<CompilerFingerprint> {
    let installed = output(root, "rustup", &["toolchain", "list"])?;
    let host = field(&version_output(root, "rustc")?, "host")?;
    let expected_name = format!("{requested}-{host}");
    let installed_name = installed
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .find(|name| *name == requested || *name == expected_name);
    let Some(installed_name) = installed_name else {
        return Err(format!(
            "toolchain {requested} is not installed; comparison was not guessed or downloaded"
        ));
    };
    fingerprint_with_toolchain(root, installed_name)
}

fn fingerprint(root: &Path) -> Result<CompilerFingerprint> {
    let active_toolchain = output(root, "rustup", &["show", "active-toolchain"])?
        .split_whitespace()
        .next()
        .ok_or_else(|| "rustup returned no active toolchain".to_owned())?
        .to_owned();
    let rustc = version_output(root, "rustc")?;
    let cargo = version_output(root, "cargo")?;
    build_fingerprint(active_toolchain, &rustc, &cargo)
}

fn fingerprint_with_toolchain(root: &Path, toolchain: &str) -> Result<CompilerFingerprint> {
    let rustc = rustup_version_output(root, toolchain, "rustc")?;
    let cargo = rustup_version_output(root, toolchain, "cargo")?;
    build_fingerprint(toolchain.to_owned(), &rustc, &cargo)
}

fn build_fingerprint(
    active_toolchain: String,
    rustc: &str,
    cargo: &str,
) -> Result<CompilerFingerprint> {
    Ok(CompilerFingerprint {
        active_toolchain,
        rustc_release: field(rustc, "release")?,
        rustc_commit_hash: field(rustc, "commit-hash")?,
        rustc_commit_date: field(rustc, "commit-date")?,
        llvm_version: field(rustc, "LLVM version")?,
        cargo_release: field(cargo, "release")?,
        cargo_commit_hash: field(cargo, "commit-hash")?,
        cargo_commit_date: field(cargo, "commit-date")?,
        host: field(rustc, "host")?,
        distribution_manifest: ProbeStatus::Unsupported {
            reason: "rustup does not expose an authenticated distribution-manifest hash through the repository toolchain inputs".to_owned(),
        },
    })
}

fn version_output(root: &Path, program: &str) -> Result<String> {
    let mut command = Command::new(program);
    command.current_dir(root).arg("-vV");
    command_output(program, &mut command)
}

fn rustup_version_output(root: &Path, toolchain: &str, program: &str) -> Result<String> {
    output(root, "rustup", &["run", toolchain, program, "-vV"])
}

fn output(root: &Path, program: &str, arguments: &[&str]) -> Result<String> {
    let mut command = Command::new(program);
    command.current_dir(root).args(arguments);
    command_output(program, &mut command)
}

fn command_output(label: &str, command: &mut Command) -> Result<String> {
    let output = command
        .output()
        .map_err(|error| format!("{label}: could not start: {error}"))?;
    if !output.status.success() {
        return Err(format!("{label}: exited with {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("{label}: output is not UTF-8: {error}"))
}

fn field(text: &str, name: &str) -> Result<String> {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{name}: ")))
        .map(str::to_owned)
        .ok_or_else(|| format!("compiler fingerprint is missing {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_named_verbose_version_fields() {
        let text = "release: 1.98.0-beta.4\ncommit-hash: abc\nhost: target\n";
        assert_eq!(field(text, "release").unwrap(), "1.98.0-beta.4");
        assert!(field(text, "LLVM version").is_err());
    }
}
