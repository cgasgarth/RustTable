#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use rusttable_testkit::reference::{
    CancellationToken, CapabilityProbe, CliReference, ColorProfile, Dimensions, ExecutionMode,
    OutputFormat, ReferenceError, ReferenceIdentity, ReferenceLimits, ReferencePin,
    ReferenceRequest, ReferenceRunner, ReferenceStatus,
};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());
const VERSION: &str = "5.7.0";
const COMMIT: &str = "cfe57f3bbf5269bfacf31e832267279caa6938ad";

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new(label: &str) -> Self {
        let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "rusttable-reference-{label}-{}-{number}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("temporary directory should be created");
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fake_executable(directory: &TempDirectory, mode: &str) -> PathBuf {
    let path = directory.path().join(format!("fake-{mode}.sh"));
    let version = VERSION;
    let commit = COMMIT;
    let script = r#"#!/bin/sh
case " $* " in
  *" --version "*) printf 'darktable {version} {commit}\n'; exit 0 ;;
  *" --help "*) printf '%s\n' '--configdir --cachedir --datadir --library --disable-opencl --width --height --icc-type --icc --out-ext'; exit 0 ;;
esac
case "{mode}" in
  success) printf 'timestamp=111 pid=1 thread=2\n'; printf 'timestamp=222 pid=3 thread=4\n' >&2; printf 'fake-image\n' > "$RUSTTABLE_REFERENCE_OUTPUT"; exit 0 ;;
  timeout|ignored-cancellation|child-spawn) (sleep 30) & wait ;;
  excessive-output) printf '%4096s' x; exit 0 ;;
  missing-image) exit 0 ;;
  nonzero-exit|partial-output) printf 'partial\n' > "$RUSTTABLE_REFERENCE_OUTPUT"; exit 7 ;;
esac
"#
    .replace("{mode}", mode)
    .replace("{version}", version)
    .replace("{commit}", commit);
    fs::write(&path, script).expect("fake executable should be written");
    let mut permissions = fs::metadata(&path)
        .expect("fake executable metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("fake executable should be executable");
    path
}

fn identity(executable: PathBuf, data_dir: PathBuf) -> ReferenceIdentity {
    ReferenceIdentity {
        source_dir: PathBuf::new(),
        executable,
        version: VERSION.to_owned(),
        commit: COMMIT.to_owned(),
        data_dir,
        executable_sha256: String::new(),
        data_dir_sha256: String::new(),
        opencl_bundle_sha256: String::new(),
        target: String::new(),
        architecture: String::new(),
        build_options_hash: String::new(),
        compiler: String::new(),
        native_library_identity: String::new(),
        cli: CliReference::default(),
        required_flags: [
            "--configdir",
            "--cachedir",
            "--datadir",
            "--library",
            "--disable-opencl",
            "--width",
            "--height",
            "--icc-type",
            "--icc",
            "--out-ext",
        ]
        .iter()
        .map(|flag| (*flag).to_owned())
        .collect(),
        normalized_log_ruleset: 1,
        executable_hash: "fixture-executable".to_owned(),
        data_bundle_hash: "fixture-data".to_owned(),
        target_triple: "aarch64-apple-darwin".to_owned(),
        c_abi_model: "aarch64-apple-darwin".to_owned(),
        build_option_hash: "fixture-options".to_owned(),
    }
}

fn request(source: &Path, timeout_ms: u64) -> ReferenceRequest {
    ReferenceRequest {
        source_fixture_id: "reference.synthetic".to_owned(),
        source_path: source.to_owned(),
        xmp_path: None,
        config_path: None,
        output_format: OutputFormat::Png,
        output_profile: ColorProfile::Srgb,
        dimensions: Dimensions {
            width: 2,
            height: 1,
        },
        timeout_ms,
        execution_mode: ExecutionMode::Cpu,
    }
}

#[test]
fn probe_accepts_pinned_identity_and_rejects_wrong_identity() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDirectory::new("probe");
    let data = directory.path().join("data");
    fs::create_dir(&data).expect("data directory");
    let executable = fake_executable(&directory, "success");
    let pin = ReferencePin {
        version: VERSION.to_owned(),
        commit: COMMIT.to_owned(),
        data_dir: data.clone(),
        required_flags: Vec::new(),
        normalized_log_ruleset: 1,
    };
    let identity = CapabilityProbe::new(&executable, pin.clone())
        .probe()
        .expect("probe should pass");
    assert_eq!(identity.data_dir, data);
    let wrong = ReferencePin {
        version: "5.6.0".to_owned(),
        ..pin
    };
    assert!(matches!(
        CapabilityProbe::new(executable, wrong).probe(),
        Err(rusttable_testkit::reference::ReferenceProbeError::IdentityMismatch { .. })
    ));
}

#[test]
fn success_receipt_is_repeatable_and_keeps_raw_logs() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDirectory::new("success");
    let source = directory.path().join("source.raw");
    fs::write(&source, b"source").expect("source");
    let executable = fake_executable(&directory, "success");
    let data = directory.path().join("data");
    fs::create_dir(&data).expect("data");
    let runner = ReferenceRunner::new(identity(executable, data), ReferenceLimits::default());
    let mut request = request(&source, 3000);
    let xmp = directory.path().join("source.raw.xmp");
    fs::write(&xmp, b"xmp").expect("xmp");
    request.xmp_path = Some(xmp);
    let first = runner
        .run_with_artifacts(&request, &CancellationToken::new())
        .expect("first run");
    let second = runner.run(&request).expect("second run");
    assert_eq!(first.receipt, second);
    assert_eq!(first.artifacts.output, b"fake-image\n");
    assert_eq!(
        first.receipt.reference_identity.executable_hash,
        "fixture-executable"
    );
    assert_eq!(
        first.receipt.reference_identity.data_bundle_hash,
        "fixture-data"
    );
    assert_eq!(
        first.receipt.reference_identity.target_triple,
        "aarch64-apple-darwin"
    );
    assert_eq!(
        first.receipt.xmp_path.as_deref(),
        Some("xmp/source.raw.xmp")
    );
    assert_eq!(
        first.receipt.status,
        ReferenceStatus::Completed(rusttable_testkit::reference::ExitStatus {
            code: Some(0),
            success: true
        })
    );
}

#[test]
fn timeout_cancellation_and_limits_are_bounded() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDirectory::new("limits");
    let source = directory.path().join("source.raw");
    fs::write(&source, b"source").expect("source");
    let data = directory.path().join("data");
    fs::create_dir(&data).expect("data");
    let limits = ReferenceLimits {
        max_stdout_bytes: 64,
        max_stderr_bytes: 64,
        max_output_bytes: 64,
    };
    let timeout_runner = ReferenceRunner::new(
        identity(fake_executable(&directory, "timeout"), data.clone()),
        limits,
    );
    let timed = timeout_runner
        .run(&request(&source, 30))
        .expect("timeout receipt");
    assert_eq!(timed.status, ReferenceStatus::TimedOut);
    let cancel_runner = ReferenceRunner::new(
        identity(
            fake_executable(&directory, "ignored-cancellation"),
            data.clone(),
        ),
        limits,
    );
    let token = CancellationToken::new();
    token.cancel();
    let cancelled = cancel_runner
        .run_with_cancellation(&request(&source, 1000), &token)
        .expect("cancel receipt");
    assert_eq!(cancelled.status, ReferenceStatus::Cancelled);
    let excessive = ReferenceRunner::new(
        identity(fake_executable(&directory, "excessive-output"), data),
        limits,
    );
    assert!(matches!(
        excessive.run(&request(&source, 3000)),
        Err(ReferenceError::StreamLimit(_, _))
    ));
}

#[test]
fn repeated_reference_runs_preserve_completed_exit_status() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDirectory::new("repeated-completion");
    let source = directory.path().join("source.raw");
    fs::write(&source, b"source").expect("source");
    let executable = fake_executable(&directory, "success");
    let data = directory.path().join("data");
    fs::create_dir(&data).expect("data");
    let runner = ReferenceRunner::new(identity(executable, data), ReferenceLimits::default());
    let expected = ReferenceStatus::Completed(rusttable_testkit::reference::ExitStatus {
        code: Some(0),
        success: true,
    });

    for _ in 0..32 {
        let receipt = runner
            .run(&request(&source, 3000))
            .expect("reference run should complete");
        assert_eq!(receipt.status, expected);
    }
}

#[test]
fn missing_and_nonzero_reference_results_are_typed() {
    let _guard = TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let directory = TempDirectory::new("failures");
    let source = directory.path().join("source.raw");
    fs::write(&source, b"source").expect("source");
    let data = directory.path().join("data");
    fs::create_dir(&data).expect("data");
    let missing = ReferenceRunner::new(
        identity(fake_executable(&directory, "missing-image"), data.clone()),
        ReferenceLimits::default(),
    );
    assert!(matches!(
        missing.run(&request(&source, 3000)),
        Err(ReferenceError::MissingOutput(_))
    ));
    let nonzero = ReferenceRunner::new(
        identity(fake_executable(&directory, "nonzero-exit"), data),
        ReferenceLimits::default(),
    );
    let receipt = nonzero
        .run(&request(&source, 3000))
        .expect("nonzero receipt");
    assert!(matches!(receipt.status, ReferenceStatus::Completed(status) if !status.success));
}
