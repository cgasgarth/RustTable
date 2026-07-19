use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::{Result, report};
use crate::cli::OfflineClosureArgs;
use crate::process::{
    EnvironmentProfile, NetworkPolicy, ProcessLimits, ProcessRequest, ProcessRunner,
};
use crate::root::RepositoryRoot;

const LOCKFILE: &str = "Cargo.lock";
const RECEIPT_SCHEMA: &str = "rusttable.offline-source-closure.v1";

#[derive(Debug, Deserialize)]
struct LockFile {
    #[serde(default)]
    package: Vec<LockPackage>,
}

#[derive(Debug, Deserialize)]
struct LockPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
}

#[derive(Debug, Deserialize)]
struct VendorChecksum {
    package: Option<String>,
}

struct TempDirectory(PathBuf);

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Clone, Serialize)]
struct Step {
    command: String,
    duration_ms: u128,
    status: String,
    detail: Option<String>,
}

pub(crate) fn vendor_closure(
    root: &RepositoryRoot,
    arguments: &OfflineClosureArgs,
    runner: &ProcessRunner,
) -> Result {
    let temp = temporary_directory("vendor")?;
    let prepared = prepare(root, &temp.0, runner)?;
    let receipt = base_receipt(root, &prepared);
    write_receipt(root, arguments.receipt.as_deref(), &receipt)?;
    Ok(report(root, "dependencies.vendor-closure", receipt))
}

pub(crate) fn verify_offline(
    root: &RepositoryRoot,
    arguments: &OfflineClosureArgs,
    runner: &ProcessRunner,
) -> Result {
    let temp = temporary_directory("verify")?;
    let prepared = prepare(root, &temp.0, runner)?;
    let mut steps = prepared.steps.clone();
    let environment = clean_environment(&prepared);
    let test = run(
        root,
        runner,
        "cargo",
        [
            "test",
            "--workspace",
            "--all-features",
            "--all-targets",
            "--frozen",
        ],
        &environment,
        NetworkPolicy::None,
        Duration::from_mins(35),
    )?;
    steps.push(test.step);
    if !test.success {
        return Err(format!("offline workspace test failed: {}", test.detail));
    }
    ensure_lock_unchanged(root, &prepared.lock_sha)?;
    let build = run(
        root,
        runner,
        "cargo",
        [
            "build",
            "-p",
            "rusttable-app",
            "--all-features",
            "--release",
            "--frozen",
        ],
        &environment,
        NetworkPolicy::None,
        Duration::from_mins(35),
    )?;
    steps.push(build.step);
    if !build.success {
        return Err(format!(
            "offline release application build failed: {}",
            build.detail
        ));
    }
    ensure_lock_unchanged(root, &prepared.lock_sha)?;
    let binary = prepared.target.join("release/rusttable-app");
    let binary_hash = if binary.is_file() {
        Some(file_digest(&binary)?)
    } else {
        return Err(format!(
            "offline release build did not produce {}",
            binary.display()
        ));
    };
    let mut receipt = base_receipt(root, &prepared);
    receipt["commands"] = serde_json::to_value(steps).map_err(|error| error.to_string())?;
    receipt["final_binary_sha256"] = Value::String(binary_hash.expect("hash was just set"));
    receipt["status"] = Value::String("verified".to_owned());
    write_receipt(root, arguments.receipt.as_deref(), &receipt)?;
    Ok(report(root, "dependencies.verify-offline", receipt))
}

struct Prepared {
    cargo_home: PathBuf,
    target: PathBuf,
    source_sha: String,
    lock_sha: String,
    package_count: usize,
    lock_package_count: usize,
    native_prerequisites: Vec<String>,
    vendor_tree_sha: String,
    steps: Vec<Step>,
    rustc: String,
    cargo: String,
    target_triple: String,
}

fn prepare(root: &RepositoryRoot, temp_root: &Path, runner: &ProcessRunner) -> Result<Prepared> {
    fs::create_dir_all(temp_root)
        .map_err(|error| format!("offline closure temp directory: {error}"))?;
    let cargo_home = temp_root.join("cargo-home");
    let target = temp_root.join("target");
    let vendor = temp_root.join("vendor");
    fs::create_dir_all(&cargo_home).map_err(|error| format!("offline Cargo home: {error}"))?;
    let lock_text =
        fs::read_to_string(root.join(LOCKFILE)).map_err(|error| format!("{LOCKFILE}: {error}"))?;
    let lock: LockFile =
        toml::from_str(&lock_text).map_err(|error| format!("{LOCKFILE}: invalid TOML: {error}"))?;
    let source_sha = git_sha(root, runner)?;
    let rustc = tool_output(runner, root, "rustc", ["-vV"], &cargo_home)?;
    let cargo = tool_output(runner, root, "cargo", ["-Vv"], &cargo_home)?;
    let target_triple = rustc
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .unwrap_or("unknown")
        .to_owned();
    fs::create_dir_all(&vendor).map_err(|error| format!("vendor directory: {error}"))?;
    let vendor_command = "cargo vendor --locked --versioned-dirs <temporary-vendor>".to_owned();
    let vendor_output = run_raw(
        root,
        runner,
        "cargo",
        [
            "vendor",
            "--locked",
            "--versioned-dirs",
            vendor.to_str().unwrap_or_default(),
        ],
        &BTreeMap::from([
            ("CARGO_HOME".to_owned(), cargo_home.display().to_string()),
            ("CARGO_TARGET_DIR".to_owned(), target.display().to_string()),
        ]),
        NetworkPolicy::Read,
        Duration::from_mins(20),
    )?;
    if !vendor_output.success {
        return Err(format!("{vendor_command} failed: {}", vendor_output.detail));
    }
    let config = String::from_utf8(vendor_output.stdout)
        .map_err(|error| format!("cargo vendor config: {error}"))?;
    fs::write(cargo_home.join("config.toml"), config.as_bytes())
        .map_err(|error| format!("offline Cargo config: {error}"))?;
    let inventory = reconcile(&lock, &vendor)?;
    let vendor_tree_sha = tree_digest(&vendor)?;
    let metadata = run(
        root,
        runner,
        "cargo",
        ["metadata", "--locked", "--offline", "--format-version", "1"],
        &BTreeMap::from([
            ("CARGO_HOME".to_owned(), cargo_home.display().to_string()),
            ("CARGO_TARGET_DIR".to_owned(), target.display().to_string()),
            ("CARGO_NET_OFFLINE".to_owned(), "true".to_owned()),
        ]),
        NetworkPolicy::None,
        Duration::from_secs(120),
    )?;
    if !metadata.success {
        return Err(format!(
            "offline cargo metadata failed: {}",
            metadata.detail
        ));
    }
    validate_metadata(&lock, &metadata.stdout)?;
    let manifest = serde_json::json!({
        "schema": RECEIPT_SCHEMA,
        "lock_sha256": digest(lock_text.as_bytes()),
        "packages": inventory.package_ids,
        "package_count": lock.package.len(),
        "vendored_package_count": inventory.package_ids.len(),
        "vendor_tree_sha256": vendor_tree_sha,
    });
    fs::write(
        temp_root.join("closure-manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("closure manifest: {error}"))?;
    let mut steps = vec![vendor_output.step, metadata.step];
    steps[0].command = vendor_command;
    Ok(Prepared {
        cargo_home,
        target,
        source_sha,
        lock_sha: digest(lock_text.as_bytes()),
        package_count: inventory.package_ids.len(),
        lock_package_count: lock.package.len(),
        native_prerequisites: inventory.native_prerequisites,
        vendor_tree_sha,
        steps,
        rustc,
        cargo,
        target_triple,
    })
}

#[derive(Debug)]
struct Inventory {
    package_ids: Vec<String>,
    native_prerequisites: Vec<String>,
}

fn reconcile(lock: &LockFile, vendor: &Path) -> Result<Inventory> {
    let expected = lock
        .package
        .iter()
        .filter(|package| package.source.is_some())
        .map(|package| (format!("{}-{}", package.name, package.version), package))
        .collect::<BTreeMap<_, _>>();
    let mut actual = BTreeSet::new();
    for entry in fs::read_dir(vendor).map_err(|error| format!("vendor inventory: {error}"))? {
        let entry = entry.map_err(|error| format!("vendor inventory: {error}"))?;
        if entry
            .file_type()
            .map_err(|error| format!("vendor inventory: {error}"))?
            .is_dir()
        {
            actual.insert(entry.file_name().to_string_lossy().into_owned());
        }
    }
    let expected_names = expected.keys().cloned().collect::<BTreeSet<_>>();
    if let Some(missing) = expected_names.difference(&actual).next() {
        return Err(format!(
            "offline closure missing vendored package {missing}"
        ));
    }
    if let Some(stale) = actual.difference(&expected_names).next() {
        return Err(format!(
            "offline closure has unreferenced vendored package {stale}"
        ));
    }
    let mut native = BTreeSet::new();
    for (name, package) in expected {
        let checksum_path = vendor.join(&name).join(".cargo-checksum.json");
        let checksum_text = fs::read_to_string(&checksum_path).map_err(|error| {
            format!("offline closure {name}: missing checksum manifest: {error}")
        })?;
        let checksum: VendorChecksum = serde_json::from_str(&checksum_text).map_err(|error| {
            format!("offline closure {name}: invalid checksum manifest: {error}")
        })?;
        if package
            .source
            .as_deref()
            .is_some_and(|source| source.starts_with("registry+"))
            && checksum.package.as_deref() != package.checksum.as_deref()
        {
            return Err(format!(
                "offline closure {name}: registry checksum differs from Cargo.lock"
            ));
        }
        if package.name.ends_with("-sys") || package.name.ends_with("_sys") {
            native.insert(package.name.clone());
        }
    }
    Ok(Inventory {
        package_ids: expected_names.into_iter().collect(),
        native_prerequisites: native.into_iter().collect(),
    })
}

fn clean_environment(prepared: &Prepared) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "CARGO_HOME".to_owned(),
            prepared.cargo_home.display().to_string(),
        ),
        (
            "CARGO_TARGET_DIR".to_owned(),
            prepared.target.display().to_string(),
        ),
        ("CARGO_NET_OFFLINE".to_owned(), "true".to_owned()),
        ("CARGO_TERM_COLOR".to_owned(), "never".to_owned()),
    ])
}

fn validate_metadata(lock: &LockFile, metadata: &[u8]) -> Result<()> {
    let metadata: Value = serde_json::from_slice(metadata)
        .map_err(|error| format!("offline cargo metadata: invalid JSON: {error}"))?;
    let packages = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| "offline cargo metadata: packages array is missing".to_owned())?;
    if packages.len() != lock.package.len() {
        return Err(format!(
            "offline closure package count differs: Cargo.lock has {}, cargo metadata resolved {}",
            lock.package.len(),
            packages.len()
        ));
    }
    let lock_ids = lock
        .package
        .iter()
        .map(|package| {
            format!(
                "{}@{}#{}",
                package.name,
                package.version,
                package.source.as_deref().unwrap_or("workspace")
            )
        })
        .collect::<BTreeSet<_>>();
    let metadata_ids = packages
        .iter()
        .filter_map(|package| {
            Some(format!(
                "{}@{}#{}",
                package.get("name")?.as_str()?,
                package.get("version")?.as_str()?,
                package
                    .get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("workspace")
            ))
        })
        .collect::<BTreeSet<_>>();
    if lock_ids != metadata_ids {
        return Err(
            "offline closure package identity differs between Cargo.lock and cargo metadata"
                .to_owned(),
        );
    }
    Ok(())
}

fn ensure_lock_unchanged(root: &RepositoryRoot, expected: &str) -> Result<()> {
    let bytes = fs::read(root.join(LOCKFILE)).map_err(|error| format!("{LOCKFILE}: {error}"))?;
    if digest(&bytes) != expected {
        return Err(format!("{LOCKFILE}: changed during offline verification"));
    }
    Ok(())
}

struct RunResult {
    stdout: Vec<u8>,
    success: bool,
    detail: String,
    step: Step,
}

fn run<const N: usize>(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    program: &str,
    args: [&str; N],
    environment: &BTreeMap<String, String>,
    network: NetworkPolicy,
    timeout: Duration,
) -> Result<RunResult> {
    run_raw(root, runner, program, args, environment, network, timeout)
}

fn run_raw<const N: usize>(
    root: &RepositoryRoot,
    runner: &ProcessRunner,
    program: &str,
    args: [&str; N],
    environment: &BTreeMap<String, String>,
    network: NetworkPolicy,
    timeout: Duration,
) -> Result<RunResult> {
    let start = Instant::now();
    let result = runner
        .run(
            ProcessRequest::new(program, args)
                .profile(EnvironmentProfile::Empty)
                .network(network)
                .environment(environment.clone())
                .limits(ProcessLimits {
                    max_stdout_bytes: 16 * 1024 * 1024,
                    max_stderr_bytes: 1024 * 1024,
                    timeout: Some(timeout),
                })
                .current_dir(root.path().to_path_buf()),
        )
        .map_err(|error| format!("{program}: {error}"))?;
    let success = result.receipt.success();
    let detail = if success {
        String::new()
    } else {
        let stderr = String::from_utf8_lossy(&result.stderr).trim().to_owned();
        if stderr.is_empty() {
            format!("status {}", result.receipt.status)
        } else {
            stderr
        }
    };
    Ok(RunResult {
        stdout: result.stdout,
        success,
        detail: detail.clone(),
        step: Step {
            command: std::iter::once(program)
                .chain(args)
                .collect::<Vec<_>>()
                .join(" "),
            duration_ms: start.elapsed().as_millis(),
            status: if success { "passed" } else { "failed" }.to_owned(),
            detail: (!success).then_some(detail),
        },
    })
}

fn tool_output<const N: usize>(
    runner: &ProcessRunner,
    root: &RepositoryRoot,
    program: &str,
    args: [&str; N],
    cargo_home: &Path,
) -> Result<String> {
    let result = runner
        .run(
            ProcessRequest::new(program, args)
                .profile(EnvironmentProfile::Empty)
                .environment(BTreeMap::from([(
                    "CARGO_HOME".to_owned(),
                    cargo_home.display().to_string(),
                )]))
                .limits(ProcessLimits {
                    max_stdout_bytes: 256 * 1024,
                    max_stderr_bytes: 64 * 1024,
                    timeout: Some(Duration::from_secs(30)),
                })
                .current_dir(root.path().to_path_buf()),
        )
        .map_err(|error| format!("{program}: {error}"))?;
    if !result.receipt.success() {
        return Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    String::from_utf8(result.stdout).map_err(|error| format!("{program}: invalid UTF-8: {error}"))
}

fn git_sha(root: &RepositoryRoot, runner: &ProcessRunner) -> Result<String> {
    tool_output(
        runner,
        root,
        "git",
        ["rev-parse", "HEAD"],
        &root.join("target"),
    )
    .map(|value| value.trim().to_owned())
}

fn base_receipt(root: &RepositoryRoot, prepared: &Prepared) -> Value {
    serde_json::json!({
        "schema": RECEIPT_SCHEMA,
        "status": "vendored",
        "source_sha": prepared.source_sha,
        "lockfile": { "path": LOCKFILE, "sha256": prepared.lock_sha },
        "rustc": prepared.rustc.lines().find(|line| line.starts_with("release: ")).unwrap_or_default(),
        "cargo": prepared.cargo.lines().next().unwrap_or_default(),
        "target_triple": prepared.target_triple,
        "package_count": prepared.lock_package_count,
        "vendored_package_count": prepared.package_count,
        "native_prerequisites": prepared.native_prerequisites,
        "vendor_tree_sha256": prepared.vendor_tree_sha,
        "commands": prepared.steps,
        "final_binary_sha256": Value::Null,
        "diagnostics": "temporary paths, credentials, and absolute command paths omitted",
        "repository_root": root.path().display().to_string(),
    })
}

fn write_receipt(root: &RepositoryRoot, path: Option<&Path>, value: &Value) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        root.join(path)
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("receipt directory: {error}"))?;
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("{}: {error}", path.display()))
}

fn temporary_directory(label: &str) -> Result<TempDirectory> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "rusttable-offline-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(TempDirectory(path))
}

fn file_digest(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    Ok(digest(&bytes))
}

fn tree_digest(root: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    let mut hasher = Sha256::new();
    for relative in files {
        let path = root.join(&relative);
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update([0]);
        hasher.update(fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?);
        hasher.update([0]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(current).map_err(|error| format!("{}: {error}", current.display()))? {
        let entry = entry.map_err(|error| format!("{}: {error}", current.display()))?;
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("{}: {error}", path.display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "offline closure rejects symlink {}",
                path.display()
            ));
        }
        if file_type.is_dir() {
            collect_files(root, &path, files)?;
        } else if file_type.is_file() {
            files.push(relative.to_owned());
        }
    }
    Ok(())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::{LockFile, reconcile, tree_digest};
    use std::fs;
    use std::path::PathBuf;

    fn fixture_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "rusttable-offline-test-{}-{}",
            std::process::id(),
            name
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("fixture root");
        root
    }

    fn lock(source: &str, checksum: Option<&str>) -> LockFile {
        let checksum = checksum
            .map(|value| format!("checksum = \"{value}\"\n"))
            .unwrap_or_default();
        toml::from_str(&format!(
            "version = 4\n\n[[package]]\nname = \"demo\"\nversion = \"1.0.0\"\nsource = \"{source}\"\n{checksum}"
        ))
        .expect("fixture lockfile")
    }

    fn vendored(root: &std::path::Path, checksum: &str) {
        let package = root.join("demo-1.0.0");
        fs::create_dir_all(&package).expect("package directory");
        fs::write(package.join("lib.rs"), "pub fn demo() {}\n").expect("package source");
        fs::write(
            package.join(".cargo-checksum.json"),
            format!("{{\"package\":\"{checksum}\",\"files\":{{}}}}"),
        )
        .expect("checksum manifest");
    }

    #[test]
    fn fixture_catalog_covers_all_required_failure_modes() {
        for name in [
            "missing-package",
            "incorrect-checksum",
            "git-submodule",
            "build-download",
            "network-access",
            "path-escape",
            "modified-lockfile",
            "stale-entry",
        ] {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/offline-closure")
                .join(format!("{name}.toml"));
            let text = fs::read_to_string(path).expect("fixture metadata");
            assert!(text.contains("scenario"));
            assert!(text.contains("expected"));
        }
    }

    #[test]
    fn reconcile_rejects_missing_package_and_stale_entry() {
        let root = fixture_root("inventory");
        let error = reconcile(
            &lock("registry+https://example.invalid", Some("abc")),
            &root,
        )
        .expect_err("missing package must fail");
        assert!(error.contains("missing vendored package demo-1.0.0"));
        vendored(&root, "abc");
        fs::create_dir(root.join("stale-1.0.0")).expect("stale entry");
        let error = reconcile(
            &lock("registry+https://example.invalid", Some("abc")),
            &root,
        )
        .expect_err("stale entry must fail");
        assert!(error.contains("unreferenced vendored package stale-1.0.0"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn reconcile_rejects_registry_checksum_drift() {
        let root = fixture_root("checksum");
        vendored(&root, "wrong");
        let error = reconcile(
            &lock("registry+https://example.invalid", Some("expected")),
            &root,
        )
        .expect_err("checksum drift must fail");
        assert!(error.contains("registry checksum differs from Cargo.lock"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn git_packages_require_a_checksum_manifest_but_not_a_registry_checksum() {
        let root = fixture_root("git");
        vendored(&root, "ignored");
        let result = reconcile(
            &lock("git+https://example.invalid/repo?rev=0123456789ab", None),
            &root,
        )
        .expect("git package should reconcile");
        assert_eq!(result.package_ids, ["demo-1.0.0"]);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn tree_digest_rejects_symlink_escape() {
        let root = fixture_root("symlink");
        std::os::unix::fs::symlink("/tmp", root.join("escape")).expect("symlink");
        let error = tree_digest(&root).expect_err("symlink must fail closed");
        assert!(error.contains("rejects symlink"));
        let _ = fs::remove_dir_all(root);
    }
}
