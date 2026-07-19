use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::{Result, report};
use crate::cli::{ReferenceArgs, ReferenceCommand, ReferenceProvisionArgs};
use crate::process::{EnvironmentProfile, ProcessLimits, ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;
use rusttable_testkit::reference::{
    CliReference, ColorProfile, Dimensions, ExecutionMode, OutputFormat, ReferenceIdentityDocument,
    ReferenceIdentityOverrides, ReferenceLimits, ReferenceRequest, ReferenceRunner,
    resolve_reference, verify_reference_unchanged,
};

pub(super) fn run(
    root: &RepositoryRoot,
    command: &ReferenceCommand,
    runner: &ProcessRunner,
) -> Result {
    match command {
        ReferenceCommand::Probe(arguments) => probe(root, arguments),
        ReferenceCommand::Render(arguments) => render(root, arguments),
        ReferenceCommand::Provision(arguments) => provision(root, arguments, runner),
        ReferenceCommand::VerifyBundle(arguments) => verify_bundle(root, arguments),
        ReferenceCommand::RunQualification(arguments) => qualification(root, arguments),
    }
}

fn probe(root: &RepositoryRoot, arguments: &ReferenceArgs) -> Result {
    let identity = resolve_identity(root, arguments)?;
    Ok(report(
        root,
        "reference.probe",
        serde_json::json!({ "identity": identity.receipt() }),
    ))
}

fn render(root: &RepositoryRoot, arguments: &ReferenceArgs) -> Result {
    let identity = resolve_identity(root, arguments)?;
    let request = request(root, arguments, arguments.xmp.is_some(), arguments.gpu)?;
    let receipt = ReferenceRunner::new(identity, ReferenceLimits::default())
        .run(&request)
        .map_err(|error| error.to_string())?;
    Ok(report(
        root,
        "reference.render",
        serde_json::json!({ "receipt": receipt }),
    ))
}

fn verify_bundle(root: &RepositoryRoot, arguments: &ReferenceArgs) -> Result {
    let identity = resolve_identity(root, arguments)?;
    verify_reference_unchanged(&identity).map_err(|error| error.to_string())?;
    Ok(report(
        root,
        "reference.verify-bundle",
        serde_json::json!({ "status": "qualified", "identity": identity.receipt() }),
    ))
}

fn qualification(root: &RepositoryRoot, arguments: &ReferenceArgs) -> Result {
    if arguments.gpu && arguments.cpu {
        return Err("reference qualification cannot select both --cpu and --gpu".to_owned());
    }
    if arguments.repeat == 0 {
        return Err("reference qualification requires --repeat >= 1".to_owned());
    }
    let identity = resolve_identity(root, arguments)?;
    let no_xmp = request(root, arguments, false, false)?;
    let with_xmp = request(root, arguments, true, false)?;
    let runner = ReferenceRunner::new(identity, ReferenceLimits::default());
    let mut receipts = vec![
        runner.run(&no_xmp).map_err(|error| error.to_string())?,
        runner.run(&with_xmp).map_err(|error| error.to_string())?,
    ];
    for _ in 1..arguments.repeat {
        receipts.push(runner.run(&no_xmp).map_err(|error| error.to_string())?);
    }
    if arguments.gpu {
        let gpu = request(root, arguments, false, true)?;
        receipts.push(runner.run(&gpu).map_err(|error| error.to_string())?);
    }
    if receipts.iter().any(|receipt| !receipt.status.is_success()) {
        return Err("reference qualification produced a non-success CPU receipt".to_owned());
    }
    Ok(report(
        root,
        "reference.run-qualification",
        serde_json::json!({
            "status": "passed",
            "cpu_cases": 2 + arguments.repeat.saturating_sub(1),
            "gpu_cases": u32::from(arguments.gpu),
            "repeat": arguments.repeat,
            "receipts": receipts,
        }),
    ))
}

fn resolve_identity(
    root: &RepositoryRoot,
    arguments: &ReferenceArgs,
) -> std::result::Result<rusttable_testkit::reference::ReferenceIdentity, String> {
    resolve_reference(
        root.join(&arguments.identity),
        &ReferenceIdentityOverrides {
            source_path: arguments.source.as_ref().map(|path| root.join(path)),
            executable_path: arguments.executable.as_ref().map(|path| root.join(path)),
            data_dir: arguments.data_dir.as_ref().map(|path| root.join(path)),
        },
    )
    .map_err(|error| error.to_string())
}

fn request(
    root: &RepositoryRoot,
    arguments: &ReferenceArgs,
    with_xmp: bool,
    gpu: bool,
) -> std::result::Result<ReferenceRequest, String> {
    let input = arguments
        .input
        .as_ref()
        .map(|path| root.join(path))
        .ok_or_else(|| "reference render requires --input".to_owned())?;
    if with_xmp && arguments.xmp.is_none() {
        return Err("reference qualification requires --xmp for the XMP case".to_owned());
    }
    Ok(ReferenceRequest {
        source_fixture_id: if with_xmp {
            format!("{}:xmp", arguments.fixture_id)
        } else {
            format!("{}:no-xmp", arguments.fixture_id)
        },
        source_path: input,
        xmp_path: if with_xmp {
            arguments.xmp.as_ref().map(|path| root.join(path))
        } else {
            None
        },
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
    })
}

fn provision(
    root: &RepositoryRoot,
    arguments: &ReferenceProvisionArgs,
    runner: &ProcessRunner,
) -> Result {
    let source = root.join(&arguments.source);
    let executable = root.join(&arguments.executable);
    let data_dir = root.join(&arguments.data_dir);
    let identity_path = root.join(&arguments.identity);
    let actual_commit = git_output(runner, &source, &["rev-parse", "HEAD"])?;
    if actual_commit != arguments.commit {
        return Err(format!(
            "reference source commit mismatch: expected {}, found {}",
            arguments.commit, actual_commit
        ));
    }
    let status = git_output(runner, &source, &["status", "--porcelain"])?;
    if !status.is_empty() {
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
        .map(Path::to_path_buf)
        .unwrap_or_else(|| root.path().to_path_buf());
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
        build_options_hash: required_identity_value(
            "build-options",
            &arguments.build_options_hash,
        )?,
        compiler: required_identity_value("compiler", &arguments.compiler)?,
        native_library_identity: required_identity_value(
            "native-library",
            &arguments.native_library_identity,
        )?,
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
    Ok(report(
        root,
        "reference.provision",
        serde_json::json!({
            "status": "provisioned",
            "identity": identity_path.display().to_string(),
            "commit": document.commit,
            "executable_sha256": document.executable_sha256,
            "data_dir_sha256": document.data_dir_sha256,
            "opencl_bundle_sha256": document.opencl_bundle_sha256,
        }),
    ))
}

fn alias(
    configured: Option<&PathBuf>,
    parent: &Path,
    path: &Path,
) -> std::result::Result<PathBuf, String> {
    let alias = configured
        .map(PathBuf::from)
        .unwrap_or_else(|| path.strip_prefix(parent).unwrap_or(path).to_path_buf());
    if alias.is_absolute() {
        return Err(format!(
            "reference identity alias must be relative: {}",
            alias.display()
        ));
    }
    Ok(alias)
}

fn required_identity_value(label: &str, value: &str) -> std::result::Result<String, String> {
    if value.is_empty() || value.starts_with("unprovisioned-") || value == "local-source" {
        return Err(format!("{label} identity is empty or a placeholder"));
    }
    Ok(value.to_owned())
}

fn git_output(
    runner: &ProcessRunner,
    source: &Path,
    arguments: &[&str],
) -> std::result::Result<String, String> {
    let mut args = vec!["-C".to_owned(), source.display().to_string()];
    args.extend(arguments.iter().map(|argument| (*argument).to_owned()));
    let result = runner
        .run(
            ProcessRequest::new("git", args)
                .profile(EnvironmentProfile::GitTool)
                .limits(ProcessLimits {
                    timeout: std::time::Duration::from_secs(30),
                    max_stdout_bytes: 64 * 1024,
                    max_stderr_bytes: 16 * 1024,
                }),
        )
        .map_err(|error| format!("git invocation: {error}"))?;
    if !result.receipt.success() {
        return Err("git invocation failed".to_owned());
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_owned())
}

fn hash_file(path: &Path) -> std::result::Result<String, String> {
    let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(hash_bytes(&bytes))
}

fn hash_directory(path: &Path) -> std::result::Result<String, String> {
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
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
