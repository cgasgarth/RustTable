use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Subcommand};
use rusttable_testkit::bench::{BenchmarkReceipt, compare_baseline};

use crate::{Result, run_process};

#[derive(Debug, Subcommand)]
pub(crate) enum BenchCommand {
    /// Run active catalog and rendering workloads.
    Run(BenchRunArgs),
    /// Compare two benchmark receipts.
    Compare(BenchCompareArgs),
}

#[derive(Debug, Args)]
pub(crate) struct BenchRunArgs {
    /// Use the benchmark's bounded merge-readiness sample counts.
    #[arg(long)]
    check: bool,
}

#[derive(Debug, Args)]
pub(crate) struct BenchCompareArgs {
    #[arg(long)]
    baseline: PathBuf,
    #[arg(long)]
    current: PathBuf,
}

pub(crate) fn run(root: &Path, command: BenchCommand) -> Result {
    match command {
        BenchCommand::Run(arguments) => run_bench(root, &arguments),
        BenchCommand::Compare(arguments) => compare(root, &arguments),
    }
}

fn run_bench(root: &Path, arguments: &BenchRunArgs) -> Result {
    let mut command = Command::new("cargo");
    command.current_dir(root).args([
        "bench",
        "-p",
        "rusttable-processing",
        "--bench",
        "performance_budgets",
        "--locked",
    ]);
    if arguments.check {
        command.args(["--", "--check"]);
    }
    run_process("product benchmarks", &mut command)
}

fn compare(root: &Path, arguments: &BenchCompareArgs) -> Result {
    let baseline = read_receipt(&root.join(&arguments.baseline))?;
    let current = read_receipt(&root.join(&arguments.current))?;
    let comparison = compare_baseline(&current, &baseline).map_err(|error| error.to_string())?;
    println!("{comparison:?}");
    Ok(())
}

fn read_receipt(path: &Path) -> Result<BenchmarkReceipt> {
    let receipt: BenchmarkReceipt = serde_json::from_slice(
        &fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?,
    )
    .map_err(|error| format!("parse {}: {error}", path.display()))?;
    receipt.validate().map_err(|error| error.to_string())?;
    Ok(receipt)
}
