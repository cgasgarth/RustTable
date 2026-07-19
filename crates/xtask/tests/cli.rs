use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
        "coverage",
        "ecosystem",
        "platform",
    ] {
        assert!(help.contains(command), "missing {command} in {help}");
    }
}

#[test]
fn platform_verification_emits_complete_runtime_receipt() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "platform",
            "verify",
            "--all-targets",
            "--runtime-current",
            "--verify-startup-preflight",
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
    assert_eq!(value["command"], "platform.verify");
    assert_eq!(value["status"], "ok");
    assert_eq!(
        value["data"]["schema"],
        "rusttable.platform-support-receipt.v1"
    );
    assert_eq!(value["data"]["target_count"], 3);
    assert_eq!(value["data"]["startup_preflight_verified"], true);
}

#[test]
fn benchmark_receipt_commands_have_explicit_input_contracts() {
    let help = xtask(&["bench", "--help"]);
    assert!(help.status.success());
    let help = String::from_utf8(help.stdout).expect("help is utf8");
    assert!(help.contains("verify-benchmark-receipt"));

    let comparison = xtask(&["bench", "compare"]);
    assert!(!comparison.status.success());
    let error = String::from_utf8(comparison.stderr).expect("error is utf8");
    assert!(error.contains("--baseline"));
    assert!(error.contains("--current"));

    let command_receipt = xtask(&["bench", "verify-receipt"]);
    assert!(!command_receipt.status.success());
    let error = String::from_utf8(command_receipt.stderr).expect("error is utf8");
    assert!(error.contains("--receipt"));
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
        Some(24)
    );
    assert_eq!(
        value["data"]["platforms"]
            .as_array()
            .expect("platform receipt")
            .iter()
            .map(|target| target["triple"].as_str().expect("target triple"))
            .collect::<Vec<_>>(),
        vec![
            "x86_64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-pc-windows-msvc"
        ]
    );
}

#[test]
fn native_boundary_verification_emits_an_evidence_receipt() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["repo", "verify-native-boundaries", "--format", "json"])
        .current_dir(root)
        .output()
        .expect("xtask should start");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json output");
    assert_eq!(value["command"], "repo.verify-native-boundaries");
    assert_eq!(
        value["data"]["schema"],
        "rusttable.native-boundaries-receipt.v1"
    );
    assert!(value["data"]["ownership_map_sha256"].as_str().is_some());
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

#[test]
fn github_queue_fixture_emits_a_deterministic_receipt() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "github",
            "verify-queue",
            "--api-fixture",
            "crates/xtask/tests/fixtures/github/queue-valid.json",
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
    assert_eq!(value["command"], "github.verify-queue");
    assert_eq!(value["data"]["selected_issue"], 446);
    assert_eq!(value["data"]["issues"][1]["ready"], false);
    assert_eq!(
        value["data"]["issues"][1]["blocking_reason"],
        "depends on open #446"
    );
}

#[test]
fn reference_probe_reports_missing_local_assets_as_structured_json() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "reference",
            "probe",
            "--identity",
            "fixtures/reference/darktable.toml",
            "--format",
            "json",
        ])
        .current_dir(root)
        .output()
        .expect("xtask should start");
    assert!(!output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json error");
    assert_eq!(value["record"], "xtask.error");
    assert!(
        value["data"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("reference source is missing"))
    );
}

#[test]
fn coverage_summary_normalizes_paths_and_merges_lcov_records_deterministically() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let output_path = root.join(format!(
        "target/coverage-test-{}-summary.json",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "coverage",
            "summarize",
            "--lcov",
            "crates/xtask/tests/fixtures/coverage/mixed-paths.lcov",
            "--policy",
            "crates/xtask/tests/fixtures/coverage/policy.toml",
            "--output",
            output_path.to_str().expect("output path"),
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
    assert_eq!(value["command"], "coverage.summarize");
    assert_eq!(value["data"]["packages"][0]["name"], "rusttable-core");
    assert_eq!(value["data"]["packages"][0]["line"]["total"], 3);
    assert_eq!(value["data"]["packages"][0]["line"]["covered"], 2);
    assert!(!value.to_string().contains("/Users/"));
    let _ = std::fs::remove_file(output_path);
}

#[test]
fn coverage_verify_rejects_stale_source_evidence() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args([
            "coverage",
            "verify",
            "--report",
            "crates/xtask/tests/fixtures/coverage/stale-report.json",
            "--lcov",
            "crates/xtask/tests/fixtures/coverage/mixed-paths.lcov",
            "--policy",
            "crates/xtask/tests/fixtures/coverage/policy.toml",
            "--format",
            "json",
        ])
        .current_dir(root)
        .output()
        .expect("xtask should start");
    assert!(!output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("json error");
    assert!(
        value["data"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("stale source SHA"))
    );
}
