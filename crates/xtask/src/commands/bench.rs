use std::fs;

use super::{Result, report};
use crate::cli::{BenchCommand, BenchReceiptArgs};
use crate::process::{CommandReceipt, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

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
                .run(ProcessRequest::new("cargo", args).current_dir(root.path()))
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
        BenchCommand::Compare(arguments) => verify_or_placeholder(root, "bench.compare", arguments),
        BenchCommand::VerifyReceipt(arguments) => verify_receipt(root, arguments),
    }
}

fn verify_or_placeholder(
    root: &RepositoryRoot,
    command: &str,
    arguments: &BenchReceiptArgs,
) -> Result {
    let Some(path) = &arguments.receipt else {
        return Ok(report(
            root,
            command,
            serde_json::json!({ "placeholder": true, "message": "comparison API pending" }),
        ));
    };
    let receipt = read_receipt(root, path)?;
    Ok(report(
        root,
        command,
        serde_json::json!({ "placeholder": true, "receipt": receipt }),
    ))
}

fn verify_receipt(root: &RepositoryRoot, arguments: &BenchReceiptArgs) -> Result {
    let path = arguments
        .receipt
        .as_ref()
        .ok_or_else(|| "--receipt is required".to_owned())?;
    let receipt = read_receipt(root, path)?;
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

fn read_receipt(root: &RepositoryRoot, path: &std::path::Path) -> Result<CommandReceipt> {
    let bytes = fs::read(root.join(path)).map_err(|error| error.to_string())?;
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}
