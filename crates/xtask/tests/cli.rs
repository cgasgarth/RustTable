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
    for command in [
        "parity",
        "fixtures",
        "bench",
        "repo",
        "reference",
        "ci",
        "ecosystem",
    ] {
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
fn workspace_dag_emits_a_clean_stable_receipt() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["repo", "verify-dag", "--format", "json"])
        .current_dir(root)
        .output()
        .expect("xtask should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(value["command"], "repo.verify-dag");
    assert_eq!(value["status"], "ok");
    assert_eq!(
        value["data"]["violations"].as_array().map(Vec::len),
        Some(0)
    );
    assert_eq!(
        value["data"]["feature_contexts"].as_array().map(Vec::len),
        Some(5)
    );
}

#[test]
fn github_pr_contract_fixture_runs_without_network_access() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "github",
            "verify-pr-contract",
            "--event",
            "crates/xtask/tests/fixtures/github/valid-event.json",
            "--api-fixture",
            "crates/xtask/tests/fixtures/github/valid-api.json",
            "--format",
            "json",
        ])
        .current_dir(root)
        .output()
        .expect("xtask should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(value["command"], "github.verify-pr-contract");
    assert_eq!(value["data"]["issue"], 171);
}
