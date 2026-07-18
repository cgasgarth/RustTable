use std::path::Path;
use std::process::Command;

fn xtask(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(args)
        .output()
        .expect("xtask should start")
}

#[test]
fn help_exposes_the_complete_initial_command_tree() {
    let output = xtask(&["--help"]);
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).expect("help is utf8");
    for command in ["parity", "fixtures", "bench", "repo", "reference", "ci"] {
        assert!(help.contains(command), "missing {command} in {help}");
    }
}

#[test]
fn json_output_is_one_parseable_ansi_free_record_from_a_subdirectory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["repo", "verify-files", "--format", "json"])
        .current_dir(root.join("crates/rusttable-core"))
        .output()
        .expect("xtask should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output
            .stdout
            .windows(2)
            .any(|window| window == [0x1b, b'['])
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["record"], "xtask.command");
    assert_eq!(value["data"]["repository_root"], root.display().to_string());
}

#[test]
fn product_crates_do_not_depend_on_repository_tooling() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    for entry in std::fs::read_dir(root.join("crates")).expect("crates directory") {
        let entry = entry.expect("crate entry");
        if !entry.file_type().expect("crate type").is_dir() {
            continue;
        }
        if matches!(
            entry.file_name().to_str(),
            Some("rusttable-testkit" | "rusttable-parity" | "xtask")
        ) {
            continue;
        }
        let manifest = std::fs::read_to_string(entry.path().join("Cargo.toml")).expect("manifest");
        assert!(
            !manifest.contains("xtask"),
            "{} depends on xtask",
            entry.path().display()
        );
        assert!(
            !manifest.contains("rusttable-testkit"),
            "{} depends on testkit",
            entry.path().display()
        );
    }
}
