use std::fs;

use super::{Result, report};
use crate::cli::{BenchCommand, BenchCompareArgs, BenchReceiptArgs};
use crate::process::{CommandReceipt, EnvironmentProfile, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;
use rusttable_testkit::bench::{BenchmarkReceipt, compare_baseline};

pub(super) fn run(root: &RepositoryRoot, command: &BenchCommand, runner: &ProcessRunner) -> Result {
    match command {
        BenchCommand::Run(arguments) => {
            let mut args = vec![
                "bench",
                "-p",
                "rusttable-processing",
                "--bench",
                "performance_budgets",
                "--locked",
            ];
            if arguments.check {
                args.extend(["--", "--check"]);
            }
            let result = runner
                .run(
                    ProcessRequest::new("cargo", args)
                        .profile(EnvironmentProfile::RustTool)
                        .current_dir(root.path()),
                )
                .map_err(|error| error.to_string())?;
            if let Some(path) = &arguments.receipt {
                let receipt_path = root.join(path);
                fs::write(
                    &receipt_path,
                    serde_json::to_vec_pretty(&result.receipt)
                        .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
            }
            Ok(report(
                root,
                "bench.run",
                serde_json::json!({ "receipt": result.receipt, "receipt_path": arguments.receipt }),
            ))
        }
        BenchCommand::Compare(arguments) => compare(root, arguments),
        BenchCommand::VerifyReceipt(arguments) => verify_command_receipt(root, arguments),
        BenchCommand::VerifyBenchmarkReceipt(arguments) => {
            verify_benchmark_receipt(root, arguments)
        }
    }
}

fn compare(root: &RepositoryRoot, arguments: &BenchCompareArgs) -> Result {
    let baseline = read_benchmark_receipt(root, &arguments.baseline)?;
    let current = read_benchmark_receipt(root, &arguments.current)?;
    let comparison = compare_baseline(&current, &baseline).map_err(|error| error.to_string())?;
    Ok(report(
        root,
        "bench.compare",
        serde_json::to_value(comparison).map_err(|error| error.to_string())?,
    ))
}

fn verify_command_receipt(root: &RepositoryRoot, arguments: &BenchReceiptArgs) -> Result {
    let bytes = fs::read(root.join(&arguments.receipt)).map_err(|error| error.to_string())?;
    let receipt: CommandReceipt =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if receipt.schema_version != 1
        || receipt.program.is_empty()
        || receipt.stdout_hash.len() != 64
        || receipt.stderr_hash.len() != 64
    {
        return Err("invalid command receipt".to_owned());
    }
    Ok(report(
        root,
        "bench.verify-receipt",
        serde_json::json!({ "valid": true, "receipt": receipt }),
    ))
}

fn verify_benchmark_receipt(root: &RepositoryRoot, arguments: &BenchReceiptArgs) -> Result {
    let receipt = read_benchmark_receipt(root, &arguments.receipt)?;
    Ok(report(
        root,
        "bench.verify-benchmark-receipt",
        serde_json::json!({
            "valid": true,
            "scenario": receipt.scenario_id,
            "workload_identity": receipt.workload_identity,
            "build": receipt.environment.build,
            "host": receipt.environment.host,
            "uncertainty": receipt.summary.uncertainty,
        }),
    ))
}

fn read_benchmark_receipt(
    root: &RepositoryRoot,
    path: &std::path::Path,
) -> Result<BenchmarkReceipt> {
    let bytes = fs::read(root.join(path)).map_err(|error| error.to_string())?;
    let receipt: BenchmarkReceipt =
        serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    receipt.validate().map_err(|error| error.to_string())?;
    Ok(receipt)
}
