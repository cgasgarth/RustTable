#![forbid(unsafe_code)]

mod bench;
mod check;
mod codegen;
mod dist;
mod fixtures;
mod reference;

use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, ExitCode};

use clap::{Parser, Subcommand};

pub(crate) const PINNED_DARKTABLE_COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";

pub(crate) type Result<T = ()> = std::result::Result<T, String>;

#[derive(Debug, Parser)]
#[command(
    bin_name = "cargo xtask",
    about = "RustTable product engineering tasks",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Debug, Subcommand)]
enum Task {
    /// Run the complete local merge-readiness gate.
    Check,
    /// Generate product compatibility data.
    Codegen {
        #[command(subcommand)]
        command: codegen::CodegenCommand,
    },
    /// Verify the real product fixture corpus.
    Fixtures {
        #[command(subcommand)]
        command: fixtures::FixturesCommand,
    },
    /// Provision or exercise the pinned darktable reference.
    Reference {
        #[command(subcommand)]
        command: Box<reference::ReferenceCommand>,
    },
    /// Run or compare real product benchmarks.
    Bench {
        #[command(subcommand)]
        command: bench::BenchCommand,
    },
    /// Build the host platform distribution artifact.
    Dist,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let root = repository_root();
    let result = match cli.command {
        Task::Check => check::run(&root),
        Task::Codegen { command } => codegen::run(&root, command),
        Task::Fixtures { command } => fixtures::run(&root, command),
        Task::Reference { command } => reference::run(&root, *command),
        Task::Bench { command } => bench::run(&root, command),
        Task::Dist => dist::run(&root),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

pub(crate) fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask must remain under crates/xtask")
        .to_path_buf()
}

pub(crate) fn run_process(label: &str, command: &mut ProcessCommand) -> Result {
    eprintln!("==> {label}");
    let status = command
        .status()
        .map_err(|error| format!("{label}: could not start: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label}: failed with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn public_command_surface_is_small_and_product_focused() {
        let help = Cli::command().render_long_help().to_string();
        for command in ["check", "codegen", "fixtures", "reference", "bench", "dist"] {
            assert!(help.contains(command), "missing {command}");
        }
        for retired in [
            "github",
            "foundation",
            "ecosystem",
            "migration",
            "parity",
            "scheduler",
            "coverage",
        ] {
            assert!(
                !help.contains(retired),
                "retired command {retired} survived"
            );
        }
    }
}
