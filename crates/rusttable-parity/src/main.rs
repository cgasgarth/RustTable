#![forbid(unsafe_code)]

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use rusttable_parity::{
    render_manifest, render_operation_manifest, render_receipt, scan_darktable_with_identity,
    scan_operations,
};
use rusttable_testkit::reference::{ReferenceIdentityOverrides, resolve_reference};

fn main() -> ExitCode {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rusttable-parity: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let command = args.first().map(String::as_str);
    if command == Some("scan-operations") {
        return run_operations(args);
    }
    if command != Some("scan-darktable") {
        return Err("usage: rusttable-parity scan-darktable [--identity <path>] [--source <path> --executable <path> --data-dir <path>] [--overrides <path>] [--output <path>]".to_owned());
    }
    let identity = argument_or(args, "--identity", "fixtures/reference/darktable.toml");
    let source = optional_argument(args, "--source");
    let executable = optional_argument(args, "--executable");
    let data_dir = optional_argument(args, "--data-dir");
    let overrides = argument_or(
        args,
        "--overrides",
        "architecture/capability-overrides.toml",
    );
    let output = argument_or(args, "--output", "architecture/darktable-capabilities.toml");
    let receipt = argument_or(
        args,
        "--receipt",
        "architecture/darktable-capabilities.receipt.toml",
    );
    let identity = resolve_reference(
        PathBuf::from(identity),
        &ReferenceIdentityOverrides {
            source_path: source.map(PathBuf::from),
            executable_path: executable.map(PathBuf::from),
            data_dir: data_dir.map(PathBuf::from),
        },
    )
    .map_err(|error| error.to_string())?;
    let manifest = scan_darktable_with_identity(&identity, &PathBuf::from(overrides))
        .map_err(|error| error.to_string())?;
    std::fs::write(
        &output,
        render_manifest(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {output}: {error}"))?;
    std::fs::write(
        &receipt,
        render_receipt(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {receipt}: {error}"))?;
    println!(
        "wrote {output} and {receipt}: {} capabilities",
        manifest.capabilities.len()
    );
    Ok(())
}

fn run_operations(args: &[String]) -> Result<(), String> {
    let source = argument(args, "--source")?;
    let overrides = argument_or(args, "--overrides", "architecture/operation-overrides.toml");
    let output = argument_or(args, "--output", "architecture/darktable-operations.toml");
    let manifest = scan_operations(&PathBuf::from(source), &PathBuf::from(overrides))
        .map_err(|error| error.to_string())?;
    std::fs::write(
        &output,
        render_operation_manifest(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {output}: {error}"))?;
    println!("wrote {output}: {} operations", manifest.operations.len());
    Ok(())
}

fn argument(args: &[String], name: &str) -> Result<String, String> {
    let value = argument_or(args, name, "");
    if value.is_empty() {
        Err(format!("missing {name}"))
    } else {
        Ok(value)
    }
}

fn argument_or(args: &[String], name: &str, default: &str) -> String {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map_or_else(|| default.to_owned(), |pair| pair[1].clone())
}

fn optional_argument(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}
