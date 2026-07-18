#![forbid(unsafe_code)]

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use rusttable_parity::{
    render_manifest, render_operation_manifest, scan_darktable, scan_operations,
};

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
        return Err("usage: rusttable-parity scan-darktable|scan-operations --source <path> [--overrides <path>] [--output <path>]".to_owned());
    }
    let source = argument(args, "--source")?;
    let overrides = argument_or(
        args,
        "--overrides",
        "architecture/capability-overrides.toml",
    );
    let output = argument_or(args, "--output", "architecture/darktable-capabilities.toml");
    let manifest = scan_darktable(&PathBuf::from(source), &PathBuf::from(overrides))
        .map_err(|error| error.to_string())?;
    std::fs::write(
        &output,
        render_manifest(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("write {output}: {error}"))?;
    println!(
        "wrote {output}: {} capabilities",
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
