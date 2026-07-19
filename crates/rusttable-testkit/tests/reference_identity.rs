#![cfg(unix)]

use std::fmt::Write as _;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use rusttable_testkit::reference::{
    ReferenceIdentityOverrides, ReferenceProbeError, resolve_reference,
};
use sha2::{Digest, Sha256};

static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    executable: PathBuf,
    data: PathBuf,
    identity: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let number = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rusttable-reference-identity-{}-{number}",
            std::process::id()
        ));
        let source = root.join("source");
        let executable = root.join("darktable-cli");
        let data = root.join("data");
        fs::create_dir_all(source.join(".git")).expect("source git directory");
        fs::create_dir_all(data.join("kernels")).expect("data directory");
        fs::write(source.join("tracked.c"), "reference").expect("source file");
        fs::write(data.join("kernels/basic.cl"), "kernel").expect("kernel");
        fs::write(&executable, executable_script()).expect("executable");
        let mut permissions = fs::metadata(&executable)
            .expect("executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("executable permissions");
        let identity = root.join("identity.toml");
        let fixture = Self {
            root,
            source,
            executable,
            data,
            identity,
        };
        fixture.write_identity("source", "darktable-cli", "data");
        fixture
    }

    fn write_identity(&self, source: &str, executable: &str, data: &str) {
        let executable_hash = hash_file(&self.executable);
        let data_hash = hash_directory(&self.data);
        let kernel_hash = hash_directory(&self.data.join("kernels"));
        let source_commit = source_commit(&self.source);
        let contents = format!(
            "schema_version = 1\nversion = \"5.7.0\"\ncommit = \"{source_commit}\"\nsource_path = \"{source}\"\nexecutable_path = \"{executable}\"\ndata_dir = \"{data}\"\nexecutable_sha256 = \"{executable_hash}\"\ndata_dir_sha256 = \"{data_hash}\"\nopencl_bundle_sha256 = \"{kernel_hash}\"\ntarget = \"{}\"\narchitecture = \"{}\"\nbuild_options_hash = \"{}\"\ncompiler = \"rustc-test\"\nnative_library_identity = \"native-test\"\nnormalized_log_ruleset = 1\n\n[cli]\nname = \"darktable-cli\"\nreference_hash = \"cli-reference-v1\"\n",
            expected_target(),
            std::env::consts::ARCH,
            "build-options-v1"
        );
        fs::write(&self.identity, contents).expect("identity document");
    }

    fn overrides(&self) -> ReferenceIdentityOverrides {
        ReferenceIdentityOverrides {
            source_path: Some(self.source.clone()),
            executable_path: Some(self.executable.clone()),
            data_dir: Some(self.data.clone()),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn missing_reference_assets_fail_with_stable_diagnostics() {
    let fixture = Fixture::new();
    let _ = fs::remove_file(&fixture.executable);
    let error = resolve_reference(&fixture.identity, &ReferenceIdentityOverrides::default())
        .expect_err("missing executable must fail");
    assert!(matches!(
        error,
        ReferenceProbeError::MissingExecutable { .. }
    ));
}

#[test]
fn source_executable_and_data_mismatches_are_not_selectable() {
    let fixture = Fixture::new();
    fs::write(fixture.source.join("changed.c"), "changed").expect("source mutation");
    let overrides = fixture.overrides();
    let error = resolve_reference(&fixture.identity, &overrides)
        .expect_err("dirty source must fail closed");
    assert!(matches!(error, ReferenceProbeError::SourceMismatch { .. }));

    let fixture = Fixture::new();
    fs::write(&fixture.executable, "changed").expect("executable mutation");
    let overrides = fixture.overrides();
    let error = resolve_reference(&fixture.identity, &overrides)
        .expect_err("changed executable must fail closed");
    assert!(matches!(
        error,
        ReferenceProbeError::ExecutableMismatch { .. }
    ));

    let fixture = Fixture::new();
    fs::write(fixture.data.join("changed.dat"), "changed").expect("data mutation");
    let overrides = fixture.overrides();
    let error = resolve_reference(&fixture.identity, &overrides)
        .expect_err("changed data must fail closed");
    assert!(matches!(error, ReferenceProbeError::DataMismatch { .. }));
}

#[test]
fn overrides_are_all_or_nothing_and_remain_local() {
    let fixture = Fixture::new();
    let partial = ReferenceIdentityOverrides {
        source_path: Some(fixture.source.clone()),
        ..ReferenceIdentityOverrides::default()
    };
    let error = resolve_reference(&fixture.identity, &partial).expect_err("partial override");
    assert!(matches!(
        error,
        ReferenceProbeError::AmbiguousOverride { .. }
    ));

    let identity = resolve_reference(&fixture.identity, &fixture.overrides())
        .expect("complete local override");
    assert_eq!(identity.source_dir, fixture.source);
    assert_eq!(identity.executable, fixture.executable);
    assert_eq!(identity.data_dir, fixture.data);
}

#[test]
fn cli_reference_receipt_is_stable_and_positional_order_is_explicit() {
    let fixture = Fixture::new();
    let identity = resolve_reference(&fixture.identity, &fixture.overrides())
        .expect("identity should resolve");
    assert_eq!(identity.cli.name, "darktable-cli");
    assert_eq!(identity.cli.reference_hash, "cli-reference-v1");
    let request = rusttable_testkit::reference::ReferenceRequest {
        source_fixture_id: "fixture.raw".to_owned(),
        source_path: PathBuf::from("source.raw"),
        xmp_path: Some(PathBuf::from("source.raw.xmp")),
        config_path: None,
        output_format: rusttable_testkit::reference::OutputFormat::Png,
        output_profile: rusttable_testkit::reference::ColorProfile::Srgb,
        dimensions: rusttable_testkit::reference::Dimensions {
            width: 2,
            height: 1,
        },
        timeout_ms: 1000,
        execution_mode: rusttable_testkit::reference::ExecutionMode::Cpu,
    };
    let arguments =
        rusttable_testkit::reference::cli_arguments(&identity, &request, Path::new("out.png"));
    assert_eq!(
        &arguments[..4],
        ["source.raw", "source.raw.xmp", "out.png", "--width"]
    );
    assert!(
        arguments
            .windows(2)
            .any(|pair| pair == ["--core", "--configdir"])
    );
}

#[test]
fn resolved_reference_inputs_are_read_only_after_resolution() {
    let fixture = Fixture::new();
    let identity =
        rusttable_testkit::reference::resolve_reference(&fixture.identity, &fixture.overrides())
            .expect("identity should resolve");
    fs::write(fixture.data.join("changed.dat"), "changed").expect("data mutation");
    let error = rusttable_testkit::reference::verify_reference_unchanged(&identity)
        .expect_err("reference data must be read-only");
    assert!(matches!(error, ReferenceProbeError::DataMismatch { .. }));
}

fn executable_script() -> &'static str {
    "#!/bin/sh\ncase \" $* \" in *\" --version \"*) echo 'darktable 5.7.0';; *\" --help \"*) echo '--configdir --cachedir --datadir --library --disable-opencl --width --height --icc-type --icc --out-ext --core';; esac\n"
}

fn expected_target() -> &'static str {
    if cfg!(target_os = "macos") {
        "aarch64-apple-darwin"
    } else {
        "x86_64-unknown-linux-gnu"
    }
}

fn source_commit(source: &Path) -> String {
    let output = Command::new("git")
        .args(["init", "--quiet", source.to_str().expect("source path")])
        .output()
        .expect("git init");
    assert!(output.status.success());
    let _ = Command::new("git")
        .args(["-C", source.to_str().expect("source path"), "add", "."])
        .status()
        .expect("git add");
    let _ = Command::new("git")
        .args([
            "-C",
            source.to_str().expect("source path"),
            "-c",
            "user.name=RustTable",
            "-c",
            "user.email=rusttable@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ])
        .status()
        .expect("git commit");
    String::from_utf8(
        Command::new("git")
            .args([
                "-C",
                source.to_str().expect("source path"),
                "rev-parse",
                "HEAD",
            ])
            .output()
            .expect("git rev-parse")
            .stdout,
    )
    .expect("commit is utf8")
    .trim()
    .to_owned()
}

fn hash_file(path: &Path) -> String {
    hash(&fs::read(path).expect("hash file"))
}

fn hash_directory(path: &Path) -> String {
    let mut entries = fs::read_dir(path)
        .expect("hash directory")
        .map(|entry| entry.expect("directory entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    let mut bytes = Vec::new();
    for entry in entries {
        bytes.extend_from_slice(
            entry
                .file_name()
                .expect("entry name")
                .to_string_lossy()
                .as_bytes(),
        );
        if entry.is_dir() {
            bytes.extend_from_slice(hash_directory(&entry).as_bytes());
        } else {
            bytes.extend_from_slice(&fs::read(entry).expect("entry bytes"));
        }
    }
    hash(&bytes)
}

fn hash(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        let _ = write!(result, "{byte:02x}");
    }
    result
}
