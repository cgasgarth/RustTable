use std::process::Command;

fn app_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_rusttable-app"))
}

#[test]
fn version_probe_is_exact_and_side_effect_free() {
    let diagnostics = tempfile_directory("version");
    let output = app_command()
        .arg("--version")
        .env("RUSTTABLE_DIAGNOSTICS_DIR", &diagnostics)
        .output()
        .expect("version probe should start");
    assert!(output.status.success());
    assert_eq!(
        output.stdout,
        format!("RustTable {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    );
    assert!(output.stderr.is_empty());
    assert!(!diagnostics.exists());
}

#[test]
fn combined_version_arguments_are_rejected_before_startup() {
    let output = app_command()
        .args(["--version", "--unexpected"])
        .output()
        .expect("argument rejection should start");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "unsupported arguments: --version --unexpected\n"
    );
}

fn tempfile_directory(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("rusttable-cli-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    path
}
