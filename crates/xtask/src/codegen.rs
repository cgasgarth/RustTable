use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::{PINNED_DARKTABLE_COMMIT, Result};

#[derive(Debug, Subcommand)]
pub(crate) enum CodegenCommand {
    /// Generate or verify the darktable operation compatibility manifest.
    Operations(OperationsArgs),
}

#[derive(Debug, Args)]
pub(crate) struct OperationsArgs {
    /// Verify committed output instead of writing it.
    #[arg(long)]
    check: bool,
    /// Pinned darktable source checkout. Required when generating.
    #[arg(long)]
    source: Option<PathBuf>,
    #[arg(long, default_value = "architecture/operation-overrides.toml")]
    overrides: PathBuf,
    #[arg(long, default_value = "architecture/darktable-operations.toml")]
    output: PathBuf,
}

pub(crate) fn run(root: &Path, command: CodegenCommand) -> Result {
    match command {
        CodegenCommand::Operations(arguments) => operations(root, &arguments),
    }
}

pub(crate) fn verify_committed(root: &Path) -> Result {
    let path = root.join("architecture/darktable-operations.toml");
    let source =
        fs::read_to_string(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let manifest =
        rusttable_parity::parse_operation_manifest(&source).map_err(|error| error.to_string())?;
    rusttable_parity::validate_operation_manifest(&manifest).map_err(|error| error.to_string())?;
    if manifest.reference.source_commit != PINNED_DARKTABLE_COMMIT {
        return Err(format!(
            "operation manifest uses {}, expected {PINNED_DARKTABLE_COMMIT}",
            manifest.reference.source_commit
        ));
    }
    eprintln!(
        "operation manifest verified: {} operations at {}",
        manifest.operations.len(),
        PINNED_DARKTABLE_COMMIT
    );
    Ok(())
}

fn operations(root: &Path, arguments: &OperationsArgs) -> Result {
    let Some(source) = arguments.source.as_ref() else {
        if arguments.check {
            return verify_committed(root);
        }
        return Err("codegen operations requires --source unless --check is used".to_owned());
    };
    let source = absolute(root, source);
    let overrides = absolute(root, &arguments.overrides);
    let output = absolute(root, &arguments.output);
    let actual_commit = reference_commit(&source)?;
    if actual_commit != PINNED_DARKTABLE_COMMIT {
        return Err(format!(
            "darktable source is at {actual_commit}, expected {PINNED_DARKTABLE_COMMIT}"
        ));
    }
    let manifest = rusttable_parity::scan_operations(&source, &overrides)
        .map_err(|error| error.to_string())?;
    let rendered = rusttable_parity::render_operation_manifest(&manifest)
        .map_err(|error| error.to_string())?;
    if arguments.check {
        let committed = fs::read_to_string(&output)
            .map_err(|error| format!("read {}: {error}", output.display()))?;
        if committed != rendered {
            return Err(format!(
                "{} is stale; run cargo xtask codegen operations --source {}",
                output.display(),
                source.display()
            ));
        }
        eprintln!("operation codegen is current: {}", output.display());
    } else {
        fs::write(&output, rendered)
            .map_err(|error| format!("write {}: {error}", output.display()))?;
        eprintln!(
            "generated {} operations in {}",
            manifest.operations.len(),
            output.display()
        );
    }
    Ok(())
}

fn reference_commit(source: &Path) -> Result<String> {
    let output = std::process::Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .args(["-C", &source.display().to_string(), "rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("inspect darktable source: {error}"))?;
    if !output.status.success() {
        return Err(format!("{} is not a Git checkout", source.display()));
    }
    String::from_utf8(output.stdout)
        .map(|commit| commit.trim().to_owned())
        .map_err(|error| format!("darktable commit is not UTF-8: {error}"))
}

fn absolute(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}
