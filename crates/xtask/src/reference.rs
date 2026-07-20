use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::{Args, Subcommand};
use rusttable_testkit::reference::{
    CliReference, ColorProfile, Dimensions, ExecutionMode, OutputFormat, ReferenceIdentityDocument,
    ReferenceIdentityOverrides, ReferenceLimits, ReferenceRequest, ReferenceRunner,
    resolve_reference, verify_reference_unchanged,
};
use sha2::{Digest, Sha256};

use crate::{PINNED_DARKTABLE_COMMIT, Result};

#[derive(Debug, Subcommand)]
pub(crate) enum ReferenceCommand {
    /// Record a qualified darktable source, executable, and data bundle.
    Provision(ProvisionArgs),
    /// Run the CPU/XMP qualification matrix, with optional GPU coverage.
    Test(TestArgs),
}

#[derive(Debug, Args)]
pub(crate) struct TestArgs {
    #[arg(long, default_value = "fixtures/reference/darktable.toml")]
    identity: PathBuf,
    #[arg(long)]
    source: Option<PathBuf>,
    #[arg(long)]
    executable: Option<PathBuf>,
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[arg(long)]
    input: PathBuf,
    #[arg(long)]
    xmp: PathBuf,
    #[arg(long, default_value = "reference.fixture")]
    fixture_id: String,
    #[arg(long, default_value_t = 1)]
    width: u32,
    #[arg(long, default_value_t = 1)]
    height: u32,
    #[arg(long)]
    gpu: bool,
    #[arg(long, default_value_t = 1)]
    repeat: u32,
}

#[derive(Debug, Args)]
pub(crate) struct ProvisionArgs {
    #[arg(long, default_value = "fixtures/reference/darktable.toml")]
    identity: PathBuf,
    #[arg(long)]
    source: PathBuf,
    #[arg(long)]
    executable: PathBuf,
    #[arg(long)]
    data_dir: PathBuf,
    #[arg(long)]
    source_alias: Option<PathBuf>,
    #[arg(long)]
    executable_alias: Option<PathBuf>,
    #[arg(long)]
    data_alias: Option<PathBuf>,
    #[arg(long, default_value = "5.7.0")]
    version: String,
    #[arg(long, default_value = PINNED_DARKTABLE_COMMIT)]
    commit: String,
    #[arg(long)]
    build_options_hash: String,
    #[arg(long)]
    compiler: String,
    #[arg(long)]
    native_library_identity: String,
    #[arg(long, default_value = "x86_64-unknown-linux-gnu")]
    target: String,
    #[arg(long, default_value = "x86_64")]
    architecture: String,
}

pub(crate) fn run(root: &Path, command: ReferenceCommand) -> Result {
    match command {
        ReferenceCommand::Provision(arguments) => provision(root, &arguments),
        ReferenceCommand::Test(arguments) => test(root, &arguments),
    }
}

fn test(root: &Path, arguments: &TestArgs) -> Result {
    if arguments.repeat == 0 {
        return Err("reference test requires --repeat >= 1".to_owned());
    }
    let identity = resolve_reference(
        root.join(&arguments.identity),
        &ReferenceIdentityOverrides {
            source_path: arguments.source.as_ref().map(|path| absolute(root, path)),
            executable_path: arguments
                .executable
                .as_ref()
                .map(|path| absolute(root, path)),
            data_dir: arguments.data_dir.as_ref().map(|path| absolute(root, path)),
        },
    )
    .map_err(|error| error.to_string())?;
    if identity.commit != PINNED_DARKTABLE_COMMIT {
        return Err(format!(
            "reference identity uses {}, expected {PINNED_DARKTABLE_COMMIT}",
            identity.commit
        ));
    }
    verify_reference_unchanged(&identity).map_err(|error| error.to_string())?;
    let runner = ReferenceRunner::new(identity, ReferenceLimits::default());
    let mut completed = 0_u32;
    for _ in 0..arguments.repeat {
        run_case(&runner, root, arguments, false, false)?;
        completed = completed.saturating_add(1);
    }
    run_case(&runner, root, arguments, true, false)?;
    completed = completed.saturating_add(1);
    if arguments.gpu {
        run_case(&runner, root, arguments, false, true)?;
        completed = completed.saturating_add(1);
    }
    eprintln!("reference qualification passed: {completed} cases");
    Ok(())
}

fn run_case(
    runner: &ReferenceRunner,
    root: &Path,
    arguments: &TestArgs,
    with_xmp: bool,
    gpu: bool,
) -> Result {
    let request = ReferenceRequest {
        source_fixture_id: format!(
            "{}:{}:{}",
            arguments.fixture_id,
            if with_xmp { "xmp" } else { "plain" },
            if gpu { "gpu" } else { "cpu" }
        ),
        source_path: absolute(root, &arguments.input),
        xmp_path: with_xmp.then(|| absolute(root, &arguments.xmp)),
        config_path: None,
        output_format: OutputFormat::Png,
        output_profile: ColorProfile::Srgb,
        dimensions: Dimensions {
            width: arguments.width,
            height: arguments.height,
        },
        timeout_ms: 30_000,
        execution_mode: if gpu {
            ExecutionMode::Gpu
        } else {
            ExecutionMode::Cpu
        },
    };
    let receipt = runner.run(&request).map_err(|error| error.to_string())?;
    if receipt.status.is_success() {
        Ok(())
    } else {
        Err(format!(
            "reference case {} returned {:?}",
            request.source_fixture_id, receipt.status
        ))
    }
}

fn provision(root: &Path, arguments: &ProvisionArgs) -> Result {
    if arguments.commit != PINNED_DARKTABLE_COMMIT {
        return Err(format!(
            "reference provision must use {PINNED_DARKTABLE_COMMIT}"
        ));
    }
    let source = absolute(root, &arguments.source);
    let executable = absolute(root, &arguments.executable);
    let data_dir = absolute(root, &arguments.data_dir);
    let identity_path = absolute(root, &arguments.identity);
    let actual_commit = git_output(&source, &["rev-parse", "HEAD"])?;
    if actual_commit != arguments.commit {
        return Err(format!(
            "reference source commit mismatch: expected {}, found {actual_commit}",
            arguments.commit
        ));
    }
    if !git_output(&source, &["status", "--porcelain"])?.is_empty() {
        return Err("reference source must be a clean worktree".to_owned());
    }
    if !executable.is_file() {
        return Err(format!(
            "reference executable is missing: {}",
            executable.display()
        ));
    }
    if !data_dir.is_dir() || !data_dir.join("kernels").is_dir() {
        return Err("reference data directory and data/kernels are required".to_owned());
    }
    let parent = identity_path
        .parent()
        .map_or_else(|| root.to_path_buf(), Path::to_path_buf);
    let document = ReferenceIdentityDocument {
        schema_version: 1,
        version: arguments.version.clone(),
        commit: arguments.commit.clone(),
        source_path: alias(arguments.source_alias.as_ref(), &parent, &source)?,
        executable_path: alias(arguments.executable_alias.as_ref(), &parent, &executable)?,
        data_dir: alias(arguments.data_alias.as_ref(), &parent, &data_dir)?,
        executable_sha256: hash_file(&executable)?,
        data_dir_sha256: hash_directory(&data_dir)?,
        opencl_bundle_sha256: hash_directory(&data_dir.join("kernels"))?,
        target: arguments.target.clone(),
        architecture: arguments.architecture.clone(),
        build_options_hash: required("build-options", &arguments.build_options_hash)?,
        compiler: required("compiler", &arguments.compiler)?,
        native_library_identity: required("native-library", &arguments.native_library_identity)?,
        normalized_log_ruleset: 1,
        required_flags: vec![
            "--width".to_owned(),
            "--height".to_owned(),
            "--core".to_owned(),
        ],
        cli: CliReference {
            name: "darktable-cli".to_owned(),
            required_flags: vec![
                "--width".to_owned(),
                "--height".to_owned(),
                "--core".to_owned(),
            ],
            core_flags: vec![
                "--configdir".to_owned(),
                "--cachedir".to_owned(),
                "--datadir".to_owned(),
                "--library".to_owned(),
                "--disable-opencl".to_owned(),
                "--icc-type".to_owned(),
                "--icc".to_owned(),
            ],
            ..CliReference::default()
        },
    };
    let text = toml::to_string_pretty(&document)
        .map_err(|error| format!("reference identity serialization: {error}"))?;
    if let Some(parent) = identity_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    fs::write(&identity_path, format!("{text}\n"))
        .map_err(|error| format!("write {}: {error}", identity_path.display()))?;
    eprintln!(
        "reference provisioned: {} at {}",
        identity_path.display(),
        document.commit
    );
    Ok(())
}

fn git_output(source: &Path, arguments: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .arg("-C")
        .arg(source)
        .args(arguments)
        .output()
        .map_err(|error| format!("git invocation: {error}"))?;
    if !output.status.success() {
        return Err(format!("git invocation failed with {}", output.status));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("git output is not UTF-8: {error}"))
}

fn alias(configured: Option<&PathBuf>, parent: &Path, path: &Path) -> Result<PathBuf> {
    let alias = configured.map_or_else(
        || path.strip_prefix(parent).unwrap_or(path).to_path_buf(),
        PathBuf::from,
    );
    if alias.is_absolute() {
        return Err(format!(
            "reference identity alias must be relative: {}",
            alias.display()
        ));
    }
    Ok(alias)
}

fn required(label: &str, value: &str) -> Result<String> {
    if value.is_empty() || value.starts_with("unprovisioned-") || value == "local-source" {
        Err(format!("{label} identity is empty or a placeholder"))
    } else {
        Ok(value.to_owned())
    }
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(hash_bytes(&bytes))
}

fn hash_directory(path: &Path) -> Result<String> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| format!("read {}: {error}", path.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    entries.sort();
    let mut bytes = Vec::new();
    for entry in entries {
        if entry.is_symlink() {
            return Err(format!(
                "reference directory contains symlink: {}",
                entry.display()
            ));
        }
        bytes.extend_from_slice(
            entry
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .as_bytes(),
        );
        if entry.is_dir() {
            bytes.extend_from_slice(hash_directory(&entry)?.as_bytes());
        } else {
            bytes.extend_from_slice(
                &fs::read(&entry).map_err(|error| format!("read {}: {error}", entry.display()))?,
            );
        }
    }
    Ok(hash_bytes(&bytes))
}

fn hash_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut output, byte| {
            write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
            output
        })
}

fn absolute(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}
