#![forbid(unsafe_code)]

mod bench;
mod check;
mod codegen;
mod color;
mod configuration;
mod dist;
mod fixtures;
mod foundation;
mod gpu;
mod migration;
mod operations;
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
    /// Validate color-space and transform contracts.
    Color {
        #[command(subcommand)]
        command: color::ColorCommand,
    },
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
    /// Run product foundation smoke workloads.
    Foundation {
        #[command(subcommand)]
        command: foundation::FoundationCommand,
    },
    /// Qualify the WGPU device and CPU fallback service.
    Gpu {
        #[command(subcommand)]
        command: gpu::GpuCommand,
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
    /// Validate the typed configuration contract and its source accounting.
    Configuration {
        #[command(subcommand)]
        command: configuration::ConfigurationCommand,
    },
    /// Verify issue-owned migration source accounting.
    Migration {
        #[command(subcommand)]
        command: migration::MigrationCommand,
    },
    /// Verify operation descriptors against their darktable source accounting.
    OperationSchema {
        #[command(subcommand)]
        command: operations::OperationSchemaCommand,
    },
    /// Verify immutable operation-stack templates and command contracts.
    OperationStack {
        #[command(subcommand)]
        command: operations::OperationStackCommand,
    },
    /// Generate or verify the static operation registry receipt.
    OperationRegistry {
        #[command(subcommand)]
        command: operations::OperationRegistryCommand,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let root = repository_root();
    let result = match cli.command {
        Task::Check => check::run(&root),
        Task::Color { command } => color::run(&root, &command),
        Task::Codegen { command } => codegen::run(&root, command),
        Task::Fixtures { command } => fixtures::run(&root, command),
        Task::Foundation { command } => foundation::run(&root, command),
        Task::Gpu { command } => gpu::run(&root, command),
        Task::Reference { command } => reference::run(&root, *command),
        Task::Bench { command } => bench::run(&root, command),
        Task::Dist => dist::run(&root),
        Task::Configuration { command } => configuration::run(&root, command),
        Task::Migration { command } => migration::run(&root, command),
        Task::OperationSchema { command } => operations::run_schema(&root, &command),
        Task::OperationStack { command } => operations::run_stack(&root, &command),
        Task::OperationRegistry { command } => operations::run_registry(&root, &command),
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
        for command in [
            "check",
            "color",
            "codegen",
            "fixtures",
            "foundation",
            "gpu",
            "reference",
            "bench",
            "dist",
            "configuration",
            "migration",
            "operation-schema",
            "operation-stack",
            "operation-registry",
        ] {
            assert!(help.contains(command), "missing {command}");
        }
        for retired in ["github", "ecosystem", "parity", "scheduler", "coverage"] {
            assert!(
                !help.contains(retired),
                "retired command {retired} survived"
            );
        }
    }
}
