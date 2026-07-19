use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Result;
use crate::cli::{CoverageCommand, CoverageRunArgs, CoverageSummarizeArgs, CoverageVerifyArgs};
use crate::process::{
    EnvironmentProfile, NetworkPolicy, ProcessLimits, ProcessRequest, ProcessRunner,
};
use crate::root::RepositoryRoot;

#[path = "coverage_lcov.rs"]
mod coverage_lcov;

use coverage_lcov::{LcovFile, normalize_path, parse_lcov};

const POLICY_SCHEMA: &str = "rusttable.coverage-policy.v1";
const REPORT_SCHEMA: &str = "rusttable.coverage-report.v1";
const TOOLCHAIN: &str = "beta-2026-07-17";
const CARGO_LLVM_COV_VERSION: &str = "0.6.16";
const LLVM_VERSION: &str = "22.1.8";

pub(super) fn run(
    root: &RepositoryRoot,
    command: &CoverageCommand,
    runner: &ProcessRunner,
) -> Result {
    match command {
        CoverageCommand::Run(arguments) => run_coverage(root, arguments, runner),
        CoverageCommand::Verify(arguments) => verify(root, arguments, runner),
        CoverageCommand::Summarize(arguments) => summarize(root, arguments, runner),
    }
}

#[derive(Debug, Deserialize)]
struct Policy {
    schema: String,
    toolchain: String,
    cargo_llvm_cov_version: String,
    llvm_version: String,
    target: String,
    features: Vec<String>,
    generated_markers: Vec<String>,
    #[serde(default)]
    exclusions: Exclusions,
    packages: Vec<PackagePolicy>,
    #[serde(default)]
    exceptions: Vec<CoverageException>,
}

#[derive(Debug, Default, Deserialize)]
struct Exclusions {
    #[serde(default)]
    packages: Vec<String>,
    #[serde(default)]
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct PackagePolicy {
    name: String,
    line_floor: f64,
    baseline_line: f64,
    max_drop: f64,
}

#[derive(Debug, Deserialize)]
struct CoverageException {
    package: String,
    expires: String,
    reason: String,
    follow_up_issue: u64,
}

#[derive(Debug, Deserialize)]
struct Metadata {
    packages: Vec<MetadataPackage>,
}

#[derive(Debug, Deserialize)]
struct MetadataPackage {
    name: String,
    manifest_path: String,
}

#[derive(Debug, Clone)]
struct SourceInfo {
    package: String,
    generated: bool,
    excluded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Metric {
    covered: u64,
    total: u64,
    percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct FileReport {
    path: String,
    line: Metric,
    zero_hit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct PackageReport {
    name: String,
    line: Metric,
    regions: Metric,
    functions: Metric,
    files: Vec<FileReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CoverageReport {
    schema: String,
    source_sha: String,
    toolchain: String,
    rustc: String,
    llvm_cov: String,
    llvm_version: String,
    target: String,
    features: Vec<String>,
    lcov_sha256: String,
    source_files: Vec<String>,
    packages: Vec<PackageReport>,
}

fn run_coverage(
    root: &RepositoryRoot,
    arguments: &CoverageRunArgs,
    runner: &ProcessRunner,
) -> Result {
    let policy = load_policy(root, &arguments.policy)?;
    let metadata = load_metadata(root, runner)?;
    let sources = source_inventory(root, &metadata, &policy)?;
    validate_policy(&policy, &sources, &metadata)?;
    let identity = capture_identity(root, &policy, runner)?;
    let output_dir = root.join(&arguments.output_dir);
    fs::create_dir_all(&output_dir)
        .map_err(|error| format!("coverage output {}: {error}", output_dir.display()))?;
    let raw_lcov = output_dir.join("cargo-llvm-cov.lcov");
    let raw_json = output_dir.join("cargo-llvm-cov.json");
    remove_if_present(&output_dir.join("coverage.lcov"))?;
    remove_if_present(&output_dir.join("coverage.json"))?;
    remove_if_present(&raw_lcov)?;
    remove_if_present(&raw_json)?;
    let _raw_cleanup = RemoveOnDrop(raw_lcov.clone());
    let _json_cleanup = RemoveOnDrop(raw_json.clone());
    let args = [
        format!("+{}", policy.toolchain),
        "llvm-cov".to_owned(),
        "--workspace".to_owned(),
        "--all-features".to_owned(),
        "--lcov".to_owned(),
        "--output-path".to_owned(),
        raw_lcov.display().to_string(),
        "--locked".to_owned(),
    ];
    run_process(runner, root, "cargo", args, Duration::from_mins(30))?;
    let json_args = [
        format!("+{}", policy.toolchain),
        "llvm-cov".to_owned(),
        "--workspace".to_owned(),
        "--all-features".to_owned(),
        "--json".to_owned(),
        "--output-path".to_owned(),
        raw_json.display().to_string(),
        "--locked".to_owned(),
    ];
    run_process(runner, root, "cargo", json_args, Duration::from_mins(30))?;
    let raw = fs::read(&raw_lcov).map_err(|error| format!("coverage LCOV: {error}"))?;
    let raw =
        std::str::from_utf8(&raw).map_err(|_| "coverage LCOV is not valid UTF-8".to_owned())?;
    let parsed = parse_lcov(raw, root, &sources, &policy)?;
    let raw_json = fs::read(&raw_json).map_err(|error| format!("coverage JSON: {error}"))?;
    let mut parsed = parsed;
    apply_json_metrics(&raw_json, root, &sources, &mut parsed)?;
    verify_source_set(&parsed, &sources)?;
    verify_region_records(&parsed, &sources)?;
    let canonical = render_lcov(&parsed);
    let lcov_path = output_dir.join("coverage.lcov");
    fs::write(&lcov_path, &canonical)
        .map_err(|error| format!("coverage LCOV {}: {error}", lcov_path.display()))?;
    let report_data = build_report(
        &parsed,
        &sources,
        &policy,
        git_head(root, runner)?,
        identity,
        sha256(canonical.as_bytes()),
    )?;
    let report_path = output_dir.join("coverage.json");
    write_json(&report_path, &report_data)?;
    Ok(crate::output::Report::new(
        "coverage.run",
        serde_json::to_value(report_data).map_err(|error| error.to_string())?,
    ))
}

fn summarize(
    root: &RepositoryRoot,
    arguments: &CoverageSummarizeArgs,
    runner: &ProcessRunner,
) -> Result {
    let policy = load_policy(root, &arguments.policy)?;
    let metadata = load_metadata(root, runner)?;
    let sources = source_inventory(root, &metadata, &policy)?;
    let lcov_path = root.join(&arguments.lcov);
    let lcov_bytes = fs::read(&lcov_path)
        .map_err(|error| format!("coverage LCOV {}: {error}", lcov_path.display()))?;
    if lcov_bytes.is_empty() {
        return Err("coverage evidence is empty".to_owned());
    }
    let lcov = std::str::from_utf8(&lcov_bytes)
        .map_err(|_| "coverage LCOV is not valid UTF-8".to_owned())?;
    let parsed = parse_lcov(lcov, root, &sources, &policy)?;
    let report_data = build_report(
        &parsed,
        &sources,
        &policy,
        git_head(root, runner)?,
        Identity {
            rustc: "not-captured".to_owned(),
            llvm_cov: format!("cargo-llvm-cov {CARGO_LLVM_COV_VERSION}"),
            llvm_version: policy.llvm_version.clone(),
        },
        sha256(&lcov_bytes),
    )?;
    write_json(&root.join(&arguments.output), &report_data)?;
    Ok(crate::output::Report::new(
        "coverage.summarize",
        serde_json::to_value(report_data).map_err(|error| error.to_string())?,
    ))
}

fn verify(root: &RepositoryRoot, arguments: &CoverageVerifyArgs, runner: &ProcessRunner) -> Result {
    let policy = load_policy(root, &arguments.policy)?;
    let report_path = root.join(&arguments.report);
    let report_bytes = fs::read(&report_path)
        .map_err(|error| format!("coverage report {}: {error}", report_path.display()))?;
    if report_bytes.is_empty() {
        return Err("coverage report is empty".to_owned());
    }
    let report_data: CoverageReport = serde_json::from_slice(&report_bytes)
        .map_err(|error| format!("coverage report: {error}"))?;
    if report_data.schema != REPORT_SCHEMA {
        return Err(format!(
            "unsupported coverage report schema {}",
            report_data.schema
        ));
    }
    let current_sha = git_head(root, runner)?;
    if report_data.source_sha != current_sha {
        return Err(format!(
            "stale source SHA: report {}, current {}",
            report_data.source_sha, current_sha
        ));
    }
    let metadata = load_metadata(root, runner)?;
    let sources = source_inventory(root, &metadata, &policy)?;
    validate_policy(&policy, &sources, &metadata)?;
    let identity = capture_identity(root, &policy, runner)?;
    if report_data.rustc != identity.rustc
        || report_data.llvm_cov != identity.llvm_cov
        || report_data.llvm_version != identity.llvm_version
    {
        return Err("coverage evidence identity does not match the installed toolchain".to_owned());
    }
    if report_data.toolchain != policy.toolchain
        || report_data.llvm_version != policy.llvm_version
        || report_data.target != policy.target
        || report_data.features != policy.features
    {
        return Err("coverage evidence identity does not match the reviewed policy".to_owned());
    }
    let lcov_path = root.join(&arguments.lcov);
    let lcov = fs::read(&lcov_path)
        .map_err(|error| format!("coverage LCOV {}: {error}", lcov_path.display()))?;
    if lcov.is_empty() {
        return Err("coverage evidence is empty".to_owned());
    }
    if sha256(&lcov) != report_data.lcov_sha256 {
        return Err("coverage LCOV does not match its report".to_owned());
    }
    let lcov =
        std::str::from_utf8(&lcov).map_err(|_| "coverage LCOV is not valid UTF-8".to_owned())?;
    let parsed = parse_lcov(lcov, root, &sources, &policy)?;
    verify_region_records(&parsed, &sources)?;
    let expected = build_report(
        &parsed,
        &sources,
        &policy,
        report_data.source_sha.clone(),
        Identity {
            rustc: report_data.rustc.clone(),
            llvm_cov: report_data.llvm_cov.clone(),
            llvm_version: report_data.llvm_version.clone(),
        },
        report_data.lcov_sha256.clone(),
    )?;
    if expected.packages != report_data.packages
        || expected.source_files != report_data.source_files
    {
        return Err("coverage report does not match its LCOV source set".to_owned());
    }
    verify_source_set(&parsed, &sources)?;
    verify_floors(&report_data, &policy)?;
    Ok(crate::output::Report::new(
        "coverage.verify",
        serde_json::json!({"valid": true, "packages": report_data.packages}),
    ))
}

fn load_policy(root: &RepositoryRoot, path: &Path) -> Result<Policy> {
    let path = root.join(path);
    let source = fs::read_to_string(&path)
        .map_err(|error| format!("coverage policy {}: {error}", path.display()))?;
    let policy: Policy =
        toml::from_str(&source).map_err(|error| format!("coverage policy: {error}"))?;
    if policy.schema != POLICY_SCHEMA {
        return Err(format!(
            "unsupported coverage policy schema {}",
            policy.schema
        ));
    }
    Ok(policy)
}

fn load_metadata(root: &RepositoryRoot, runner: &ProcessRunner) -> Result<Metadata> {
    let output = run_process(
        runner,
        root,
        "cargo",
        ["metadata", "--locked", "--no-deps", "--format-version", "1"],
        Duration::from_secs(5),
    )?;
    serde_json::from_slice(&output).map_err(|error| format!("cargo metadata: {error}"))
}

fn source_inventory(
    root: &RepositoryRoot,
    metadata: &Metadata,
    policy: &Policy,
) -> Result<BTreeMap<String, SourceInfo>> {
    let excluded_packages = policy.exclusions.packages.iter().collect::<BTreeSet<_>>();
    let mut sources = BTreeMap::new();
    for package in &metadata.packages {
        let excluded = excluded_packages.contains(&package.name);
        let manifest = PathBuf::from(&package.manifest_path);
        let source_root = manifest
            .parent()
            .ok_or_else(|| format!("package {} has no manifest parent", package.name))?
            .join("src");
        for path in rust_files(&source_root)? {
            let relative = path
                .strip_prefix(root.path())
                .map_err(|_| format!("source path escapes workspace: {}", path.display()))?
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(&path)
                .map_err(|error| format!("source {}: {error}", path.display()))?;
            let generated = policy
                .generated_markers
                .iter()
                .any(|marker| source.lines().any(|line| line.trim() == marker));
            if generated
                || policy
                    .exclusions
                    .paths
                    .iter()
                    .any(|pattern| path_matches(pattern, &relative))
            {
                sources.insert(
                    relative,
                    SourceInfo {
                        package: package.name.clone(),
                        generated: true,
                        excluded,
                    },
                );
            } else {
                sources.insert(
                    relative,
                    SourceInfo {
                        package: package.name.clone(),
                        generated: false,
                        excluded,
                    },
                );
            }
        }
    }
    Ok(sources)
}

fn rust_files(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let entries = fs::read_dir(root)
        .map_err(|error| format!("source directory {}: {error}", root.display()))?;
    for entry in entries {
        let path = entry
            .map_err(|error| format!("source directory: {error}"))?
            .path();
        if path.is_dir() {
            files.extend(rust_files(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn validate_policy(
    policy: &Policy,
    sources: &BTreeMap<String, SourceInfo>,
    metadata: &Metadata,
) -> Result<()> {
    if policy.toolchain != TOOLCHAIN
        || policy.cargo_llvm_cov_version != CARGO_LLVM_COV_VERSION
        || policy.llvm_version != LLVM_VERSION
        || policy.target != "x86_64-unknown-linux-gnu"
        || policy.features != ["all-features"]
    {
        return Err("coverage policy has an invalid pinned identity".to_owned());
    }
    let excluded = policy.exclusions.packages.iter().collect::<BTreeSet<_>>();
    let workspace = metadata
        .packages
        .iter()
        .filter(|package| !excluded.contains(&package.name))
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    let configured = policy
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    if workspace != configured {
        return Err(format!(
            "coverage policy packages do not match workspace: configured [{}], workspace [{}]",
            configured.into_iter().collect::<Vec<_>>().join(", "),
            workspace.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    let mut names = BTreeSet::new();
    for package in &policy.packages {
        if !names.insert(&package.name) {
            return Err(format!("coverage policy repeats package {}", package.name));
        }
        if !(0.0..=100.0).contains(&package.line_floor)
            || !(0.0..=100.0).contains(&package.baseline_line)
            || package.max_drop < 0.0
            || package.baseline_line < package.line_floor
        {
            return Err(format!(
                "coverage policy has invalid floor for {}",
                package.name
            ));
        }
        if package.line_floor < 60.0 {
            let exception = policy
                .exceptions
                .iter()
                .find(|exception| exception.package == package.name);
            match exception {
                Some(exception)
                    if exception.follow_up_issue > 0 && !exception.reason.trim().is_empty() =>
                {
                    if exception.expires < today_utc() {
                        return Err(format!(
                            "coverage exception for {} is expired",
                            package.name
                        ));
                    }
                }
                _ => {
                    return Err(format!(
                        "coverage floor for {} is below 60 without a reviewed exception",
                        package.name
                    ));
                }
            }
        }
    }
    for exception in &policy.exceptions {
        if !names.contains(&exception.package)
            || exception.follow_up_issue == 0
            || exception.expires.len() != 10
        {
            return Err(format!(
                "coverage exception is invalid for {}",
                exception.package
            ));
        }
    }
    if !sources
        .values()
        .any(|source| !source.generated && !source.excluded)
    {
        return Err("workspace has no production Rust source files".to_owned());
    }
    Ok(())
}

fn build_report(
    files: &[LcovFile],
    sources: &BTreeMap<String, SourceInfo>,
    policy: &Policy,
    source_sha: String,
    identity: Identity,
    lcov_sha256: String,
) -> Result<CoverageReport> {
    let mut packages = BTreeMap::<String, Vec<&LcovFile>>::new();
    for file in files {
        let info = sources
            .get(&file.path)
            .ok_or_else(|| format!("unknown coverage source {}", file.path))?;
        if !info.generated && !info.excluded {
            packages.entry(info.package.clone()).or_default().push(file);
        }
    }
    let package_reports = packages
        .into_iter()
        .map(|(name, files)| {
            let mut line_total = 0;
            let mut line_covered = 0;
            let mut region_total = 0;
            let mut region_covered = 0;
            let mut function_total = 0;
            let mut function_covered = 0;
            let mut file_reports = Vec::new();
            for file in files {
                let file_total = file.lines.len() as u64;
                let file_covered = file.lines.values().filter(|hits| **hits > 0).count() as u64;
                line_total += file_total;
                line_covered += file_covered;
                let (file_region_covered, file_region_total) =
                    file.region_counts.unwrap_or_else(|| {
                        (
                            file.regions.values().filter(|hits| **hits > 0).count() as u64,
                            file.regions.len() as u64,
                        )
                    });
                region_total += file_region_total;
                region_covered += file_region_covered;
                function_total += file.functions.len() as u64;
                function_covered +=
                    file.functions.values().filter(|hits| **hits > 0).count() as u64;
                file_reports.push(FileReport {
                    path: file.path.clone(),
                    line: metric(file_covered, file_total),
                    zero_hit: file_total > 0 && file_covered == 0,
                });
            }
            file_reports.sort_by(|left, right| left.path.cmp(&right.path));
            PackageReport {
                name,
                line: metric(line_covered, line_total),
                regions: metric(region_covered, region_total),
                functions: metric(function_covered, function_total),
                files: file_reports,
            }
        })
        .collect::<Vec<_>>();
    if package_reports.is_empty() {
        return Err("coverage evidence contains no production packages".to_owned());
    }
    let source_files = package_reports
        .iter()
        .flat_map(|package| package.files.iter().map(|file| file.path.clone()))
        .collect::<Vec<_>>();
    Ok(CoverageReport {
        schema: REPORT_SCHEMA.to_owned(),
        source_sha,
        toolchain: policy.toolchain.clone(),
        rustc: identity.rustc,
        llvm_cov: identity.llvm_cov,
        llvm_version: identity.llvm_version,
        target: policy.target.clone(),
        features: policy.features.clone(),
        lcov_sha256,
        source_files,
        packages: package_reports,
    })
}

fn verify_floors(report: &CoverageReport, policy: &Policy) -> Result<()> {
    let configured = policy
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    let reported = report
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<BTreeSet<_>>();
    if configured != reported {
        return Err(format!(
            "coverage report package set mismatch: configured [{}], reported [{}]",
            configured.into_iter().collect::<Vec<_>>().join(", "),
            reported.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    for package in &policy.packages {
        let report_package = report
            .packages
            .iter()
            .find(|candidate| candidate.name == package.name)
            .ok_or_else(|| format!("coverage report is missing package {}", package.name))?;
        if report_package.line.percent + f64::EPSILON < package.line_floor {
            return Err(format!(
                "coverage floor regression for {}: {:.2}% < {:.2}%",
                package.name, report_package.line.percent, package.line_floor
            ));
        }
        if report_package.line.percent + f64::EPSILON < package.baseline_line - package.max_drop {
            return Err(format!(
                "coverage maximum-drop regression for {}",
                package.name
            ));
        }
        if report_package.files.iter().any(|file| file.zero_hit) {
            return Err(format!(
                "suspicious zero-hit production file in {}",
                package.name
            ));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct Identity {
    rustc: String,
    llvm_cov: String,
    llvm_version: String,
}

fn capture_identity(
    root: &RepositoryRoot,
    policy: &Policy,
    runner: &ProcessRunner,
) -> Result<Identity> {
    let rustc = String::from_utf8_lossy(&run_process(
        runner,
        root,
        "rustc",
        [format!("+{}", policy.toolchain), "-vV".to_owned()],
        Duration::from_secs(10),
    )?)
    .trim()
    .to_owned();
    let llvm_cov = String::from_utf8_lossy(&run_process(
        runner,
        root,
        "cargo",
        [
            format!("+{}", policy.toolchain),
            "llvm-cov".to_owned(),
            "--version".to_owned(),
        ],
        Duration::from_secs(20),
    )?)
    .trim()
    .to_owned();
    if policy.cargo_llvm_cov_version != CARGO_LLVM_COV_VERSION
        || llvm_cov != format!("cargo-llvm-cov {CARGO_LLVM_COV_VERSION}")
    {
        return Err(format!(
            "cargo llvm-cov identity is not pinned to {CARGO_LLVM_COV_VERSION}"
        ));
    }
    let llvm_version = rustc
        .lines()
        .find_map(|line| line.strip_prefix("LLVM version: "))
        .ok_or_else(|| "rustc identity is missing its LLVM version".to_owned())?
        .trim()
        .to_owned();
    if llvm_version != policy.llvm_version {
        return Err("llvm-cov does not match the reviewed LLVM identity".to_owned());
    }
    Ok(Identity {
        rustc,
        llvm_cov,
        llvm_version,
    })
}

fn run_process<I, S>(
    runner: &ProcessRunner,
    root: &RepositoryRoot,
    program: &str,
    args: I,
    timeout: Duration,
) -> Result<Vec<u8>>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let result = runner
        .run(
            ProcessRequest::new(program, args)
                .profile(if program == "git" {
                    EnvironmentProfile::GitTool
                } else {
                    EnvironmentProfile::RustTool
                })
                .network(NetworkPolicy::None)
                .current_dir(root.path())
                .limits(ProcessLimits {
                    max_stdout_bytes: 8 * 1024 * 1024,
                    max_stderr_bytes: 256 * 1024,
                    timeout: Some(timeout),
                }),
        )
        .map_err(|error| error.to_string())?;
    if !result.receipt.success() {
        return Err(format!(
            "{program} failed ({}): {}",
            result.receipt.status,
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    Ok(result.stdout)
}

fn git_head(root: &RepositoryRoot, runner: &ProcessRunner) -> Result<String> {
    Ok(String::from_utf8_lossy(&run_process(
        runner,
        root,
        "git",
        ["rev-parse", "HEAD"],
        Duration::from_secs(5),
    )?)
    .trim()
    .to_owned())
}

fn verify_source_set(files: &[LcovFile], sources: &BTreeMap<String, SourceInfo>) -> Result<()> {
    let expected = sources
        .iter()
        .filter(|(_, info)| !info.generated && !info.excluded)
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    let reported = files
        .iter()
        .filter(|file| {
            sources
                .get(&file.path)
                .is_some_and(|info| !info.generated && !info.excluded)
        })
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    if expected != reported {
        let missing = expected.difference(&reported).cloned().collect::<Vec<_>>();
        let unexpected = reported.difference(&expected).cloned().collect::<Vec<_>>();
        return Err(format!(
            "coverage source set is incomplete: missing [{}], unexpected [{}]",
            missing.join(", "),
            unexpected.join(", ")
        ));
    }
    Ok(())
}

fn verify_region_records(files: &[LcovFile], sources: &BTreeMap<String, SourceInfo>) -> Result<()> {
    if files.iter().all(|file| {
        sources.get(&file.path).is_none_or(|info| {
            info.generated
                || info.excluded
                || (file.region_counts.is_none_or(|(_, total)| total == 0)
                    && file.regions.is_empty())
        })
    }) {
        return Err("coverage evidence contains no region records".to_owned());
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
struct LlvmCovReport {
    data: Vec<LlvmCovData>,
}

#[derive(Debug, Deserialize)]
struct LlvmCovData {
    files: Vec<LlvmCovFile>,
}

#[derive(Debug, Deserialize)]
struct LlvmCovFile {
    filename: String,
    summary: LlvmCovSummary,
}

#[derive(Debug, Deserialize)]
struct LlvmCovSummary {
    regions: LlvmCovCount,
}

#[derive(Debug, Deserialize)]
struct LlvmCovCount {
    count: u64,
    covered: u64,
}

fn apply_json_metrics(
    bytes: &[u8],
    root: &RepositoryRoot,
    sources: &BTreeMap<String, SourceInfo>,
    files: &mut [LcovFile],
) -> Result<()> {
    let report: LlvmCovReport =
        serde_json::from_slice(bytes).map_err(|error| format!("coverage JSON: {error}"))?;
    let mut seen = BTreeSet::new();
    for data in report.data {
        for json_file in data.files {
            let path = normalize_path(&json_file.filename, root)?;
            let Some(info) = sources.get(&path) else {
                return Err(format!(
                    "coverage JSON references stale or non-workspace source {path}"
                ));
            };
            let file = files
                .iter_mut()
                .find(|file| file.path == path)
                .ok_or_else(|| format!("coverage JSON source is absent from LCOV: {path}"))?;
            file.region_counts = Some((
                json_file.summary.regions.covered,
                json_file.summary.regions.count,
            ));
            if !info.generated && !info.excluded {
                seen.insert(path);
            }
        }
    }
    let expected = sources
        .iter()
        .filter(|(_, info)| !info.generated && !info.excluded)
        .map(|(path, _)| path.clone())
        .collect::<BTreeSet<_>>();
    if expected != seen {
        let missing = expected.difference(&seen).cloned().collect::<Vec<_>>();
        let unexpected = seen.difference(&expected).cloned().collect::<Vec<_>>();
        return Err(format!(
            "coverage JSON source set is incomplete: missing [{}], unexpected [{}]",
            missing.join(", "),
            unexpected.join(", ")
        ));
    }
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("coverage output {}: {error}", path.display())),
    }
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn write_json(path: &Path, value: &CoverageReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("coverage output {}: {error}", parent.display()))?;
    }
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|error| format!("coverage JSON: {error}"))?;
    fs::write(path, bytes).map_err(|error| format!("coverage JSON {}: {error}", path.display()))
}

fn render_lcov(files: &[LcovFile]) -> String {
    let mut output = String::new();
    for file in files {
        let _ = writeln!(output, "TN:\nSF:{}", file.path);
        for ((line, name), hits) in &file.functions {
            let _ = writeln!(output, "FN:{line},{name}\nFNDA:{hits},{name}");
        }
        let _ = writeln!(
            output,
            "FNF:{}\nFNH:{}",
            file.functions.len(),
            file.functions.values().filter(|hits| **hits > 0).count()
        );
        for (line, hits) in &file.lines {
            let _ = writeln!(output, "DA:{line},{hits}");
        }
        let _ = writeln!(
            output,
            "LF:{}\nLH:{}",
            file.lines.len(),
            file.lines.values().filter(|hits| **hits > 0).count()
        );
        if file.regions.is_empty() {
            if let Some((covered, total)) = file.region_counts {
                for region in 0..total {
                    let hits = u64::from(region < covered);
                    let _ = writeln!(output, "BRDA:0,json,{region},{hits}");
                }
            }
        } else {
            for ((line, block, branch), hits) in &file.regions {
                let _ = writeln!(output, "BRDA:{line},{block},{branch},{hits}");
            }
        }
        let _ = writeln!(
            output,
            "BRF:{}\nBRH:{}\nend_of_record",
            file.regions.len(),
            file.regions.values().filter(|hits| **hits > 0).count()
        );
    }
    output
}

#[allow(
    clippy::cast_precision_loss,
    reason = "coverage percentages are presentation values derived from bounded LCOV counts"
)]
fn metric(covered: u64, total: u64) -> Metric {
    let percent = if total == 0 {
        0.0
    } else {
        ((covered as f64 * 100.0 / total as f64) * 100.0).round() / 100.0
    };
    Metric {
        covered,
        total,
        percent,
    }
}

fn sha256(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

fn path_matches(pattern: &str, path: &str) -> bool {
    let pattern = pattern.trim_end_matches('*');
    path == pattern || (!pattern.is_empty() && path.starts_with(pattern))
}

fn today_utc() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    let z = i64::try_from(days).unwrap_or(i64::MAX) + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month + 2) / 5 + 1;
    let month = month + if month < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
#[path = "coverage_tests.rs"]
mod tests;
