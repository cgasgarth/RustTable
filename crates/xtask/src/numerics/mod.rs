use std::path::Path;

use clap::Subcommand;
use rusttable_core::numerics::REQUESTED_PRIMARY_TOOLCHAIN;
use rusttable_gpu::shader::validate_checked_in;

use crate::Result;

mod compiler;
mod policy;
mod receipt;

use receipt::{CheckReceipt, NumericsReceipt};

#[derive(Debug, Subcommand)]
pub(crate) enum NumericsCommand {
    /// Verify observable compiler/profile/source and implementation registrations.
    Verify {
        #[arg(long)]
        all_implementations: bool,
        #[arg(long)]
        all_profiles: bool,
    },
    /// Fingerprint requested compiler lanes without claiming unregistered corpus results.
    CompareCompilers {
        #[arg(long)]
        primary_beta: bool,
        #[arg(long)]
        rolling_beta: bool,
        #[arg(long)]
        previous_stable: bool,
        #[arg(long)]
        current_nightly: bool,
    },
}

pub(crate) fn run(root: &Path, command: &NumericsCommand) -> Result {
    match command {
        NumericsCommand::Verify {
            all_implementations,
            all_profiles,
        } => verify(root, *all_implementations, *all_profiles),
        NumericsCommand::CompareCompilers {
            primary_beta,
            rolling_beta,
            previous_stable,
            current_nightly,
        } => {
            let requested = [
                (*primary_beta, "primary-beta", REQUESTED_PRIMARY_TOOLCHAIN),
                (*rolling_beta, "rolling-beta", "beta"),
                (*previous_stable, "previous-stable", "1.97.1"),
                (*current_nightly, "current-nightly", "nightly"),
            ]
            .into_iter()
            .filter_map(|(selected, lane, toolchain)| selected.then_some((lane, toolchain)))
            .collect();
            compare_compilers(root, requested)
        }
    }
}

pub(crate) fn verify_registered_choices(root: &Path) -> Result {
    let document = policy::load(root)?;
    let mut checks = policy::verify(root, &document, true)?;
    checks.push(verify_shaders(true));
    let blockers = checks
        .iter()
        .filter(|check| check.status == receipt::CheckStatus::Blocking)
        .map(|check| check.id.as_str())
        .collect::<Vec<_>>();
    if blockers.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "numerical contract validation failed: {}",
            blockers.join(", ")
        ))
    }
}

fn verify(root: &Path, all_implementations: bool, all_profiles: bool) -> Result {
    let document = policy::load(root)?;
    let mut checks = policy::verify(root, &document, all_profiles)?;
    checks.push(compiler::verify_active(root, &document));
    checks.push(verify_shaders(all_implementations));
    checks.push(CheckReceipt::unsupported(
        "native-floating-environment",
        "safe Rust exposes no portable MXCSR/FPCR/fenv read boundary; no native probe was guessed",
    ));
    checks.push(CheckReceipt::unsupported(
        "cross-platform-codegen",
        "this host cannot prove other architectures or WGPU backends; hosted runners remain required",
    ));
    checks.push(CheckReceipt::unsupported(
        "compiler-codegen-probes",
        "the current repository has no authoritative contraction, conversion, subnormal, or vectorization probe fixtures",
    ));
    checks.push(CheckReceipt::unsupported(
        "rust-1.97.1-miscompilation-sentinel",
        "the current repository has no checked-in reproducer, so the primary-beta sentinel was not claimed",
    ));
    checks.push(CheckReceipt::unsupported(
        "operation-implementation-closure",
        "the current operation registry has no authoritative numerical fields; only the existing shader registry is checked",
    ));
    let fingerprint = compiler::active_fingerprint(root).ok();
    let receipt = NumericsReceipt::verification(checks, fingerprint)?;
    receipt.emit()?;
    receipt.blocking_result("numerics verify")
}

fn verify_shaders(all_implementations: bool) -> CheckReceipt {
    let registry = match validate_checked_in() {
        Ok(registry) => registry,
        Err(error) => {
            return CheckReceipt::blocking("shader-numerics", error.to_string());
        }
    };
    if !all_implementations {
        return CheckReceipt::passed(
            "shader-numerics",
            format!(
                "{} authoritative shader entries checked (the registry is indivisible)",
                registry.entries().len()
            ),
        );
    }
    for entry in registry.entries() {
        let metadata = &entry.identity.implementation_numerics;
        if metadata.contract() != entry.reflection.numerical.contract
            || metadata.tolerance() != entry.reflection.numerical.tolerance
            || metadata.scalar_reference_id() != entry.reflection.numerical.canonical_cpu_reference
        {
            return CheckReceipt::blocking(
                "shader-numerics",
                format!(
                    "{} has inconsistent numerical metadata",
                    entry.id().stable_name()
                ),
            );
        }
    }
    CheckReceipt::passed(
        "shader-numerics",
        format!(
            "{} implementations have typed contracts and Pointwise backend tolerances",
            registry.entries().len()
        ),
    )
}

fn compare_compilers(root: &Path, requested: Vec<(&str, &str)>) -> Result {
    if requested.is_empty() {
        return Err("numerics compare-compilers: select at least one compiler lane".to_owned());
    }
    let mut checks = Vec::new();
    let mut fingerprints = Vec::new();
    for (lane, toolchain) in requested {
        match compiler::fingerprint_installed(root, toolchain) {
            Ok(fingerprint) => {
                checks.push(CheckReceipt::passed(
                    format!("compiler-{lane}"),
                    format!(
                        "observed {} {} LLVM {}",
                        fingerprint.active_toolchain,
                        fingerprint.rustc_release,
                        fingerprint.llvm_version
                    ),
                ));
                fingerprints.push((lane.to_owned(), fingerprint));
            }
            Err(error) => checks.push(CheckReceipt::blocking(format!("compiler-{lane}"), error)),
        }
    }
    checks.push(CheckReceipt::unsupported(
        "numerical-corpus-comparison",
        "no compiler-neutral numerical corpus executor is registered in the current repository; no output parity was claimed",
    ));
    let receipt = NumericsReceipt::comparison(checks, fingerprints)?;
    receipt.emit()?;
    receipt.blocking_result("numerics compare-compilers")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_surface_names_truthful_numerics_actions() {
        use clap::CommandFactory as _;
        let help = crate::Cli::command().render_long_help().to_string();
        assert!(help.contains("numerics"));
        assert!(REQUESTED_PRIMARY_TOOLCHAIN.contains("2026-07-18"));
    }

    #[test]
    fn shader_registry_has_no_exact_backend_defined_claim() {
        let check = verify_shaders(true);
        assert_eq!(check.status, receipt::CheckStatus::Passed);
    }
}
