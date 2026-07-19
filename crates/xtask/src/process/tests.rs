use super::{EnvironmentProfile, ProcessLimits, ProcessRequest, ProcessRunner};
use std::time::Duration;

#[test]
fn bounds_stdout_and_returns_a_stable_receipt() {
    let result = ProcessRunner::new()
        .run(
            ProcessRequest::new("sh", ["-c", "printf 1234567890"]).limits(ProcessLimits {
                max_stdout_bytes: 4,
                max_stderr_bytes: 16,
                timeout: Duration::from_secs(2),
            }),
        )
        .expect("process result");
    assert_eq!(result.receipt.status, "completed");
    assert_eq!(result.stdout, b"1234");
    assert!(result.receipt.stdout_truncated);
    assert_eq!(result.receipt.schema_version, 2);
}

#[cfg(unix)]
#[test]
fn bounded_artifacts_preserve_success_and_failure() {
    for (name, command, exit_code) in [
        (
            "success",
            "i=0; while [ $i -lt 400000 ]; do printf x; i=$((i+1)); done",
            0,
        ),
        (
            "failure",
            "i=0; while [ $i -lt 400000 ]; do printf x; i=$((i+1)); done; exit 7",
            7,
        ),
    ] {
        let root =
            std::env::temp_dir().join(format!("rusttable-process-{name}-{}", std::process::id()));
        let stdout = root.join("stdout.log");
        let stderr = root.join("stderr.log");
        let result = ProcessRunner::new()
            .run(
                ProcessRequest::new("sh", ["-c", command])
                    .limits(ProcessLimits {
                        max_stdout_bytes: 8,
                        max_stderr_bytes: 8,
                        timeout: Duration::from_secs(10),
                    })
                    .artifacts(&stdout, &stderr),
            )
            .expect("process result");
        let artifact = std::fs::read_to_string(&stdout).expect("stdout artifact");
        assert_eq!(result.receipt.exit_code, Some(exit_code));
        assert_eq!(result.receipt.success(), exit_code == 0);
        assert!(result.receipt.stdout_truncated);
        assert!(artifact.contains("output truncated"));
        assert!(artifact.len() <= super::ARTIFACT_LIMIT);
        assert_eq!(result.receipt.artifacts.len(), 2);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn empty_artifacts_are_fresh_after_process_completion() {
    let root =
        std::env::temp_dir().join(format!("rusttable-empty-artifacts-{}", std::process::id()));
    let stdout = root.join("stdout.log");
    let stderr = root.join("stderr.log");
    let result = ProcessRunner::new()
        .run(
            ProcessRequest::new("sh", ["-c", "exit 0"])
                .limits(ProcessLimits {
                    max_stdout_bytes: 16,
                    max_stderr_bytes: 16,
                    timeout: Duration::from_secs(2),
                })
                .artifacts(&stdout, &stderr),
        )
        .expect("empty artifacts are valid");
    assert!(result.receipt.success());
    assert_eq!(result.receipt.artifacts.len(), 2);
    assert_eq!(std::fs::metadata(stdout).expect("stdout artifact").len(), 0);
    assert_eq!(std::fs::metadata(stderr).expect("stderr artifact").len(), 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn artifact_paths_are_platform_joined_and_stable() {
    let path = std::path::Path::new("target")
        .join("validation")
        .join("workspace-dag")
        .join("pull_request.stdout.log");
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("pull_request.stdout.log")
    );
    assert!(path.to_string_lossy().contains("workspace-dag"));
}

#[cfg(unix)]
#[test]
fn timeout_terminates_a_child_tree() {
    let result = ProcessRunner::new()
        .run(
            ProcessRequest::new("sh", ["-c", "(sleep 30) & wait"]).limits(ProcessLimits {
                max_stdout_bytes: 64,
                max_stderr_bytes: 64,
                timeout: Duration::from_millis(100),
            }),
        )
        .expect("process result");
    assert_eq!(result.receipt.status, "timed-out");
    assert_eq!(result.receipt.process_ownership, "unix-process-session");
    assert_eq!(result.receipt.cleanup.outcome, "reaped");
}

#[test]
fn request_environment_is_allowlisted_and_receipt_is_redacted() {
    let result = ProcessRunner::new()
        .run(
            ProcessRequest::new("/bin/sh", ["-c", "printf %s \"$VISIBLE\""])
                .profile(EnvironmentProfile::TestFixture)
                .env("VISIBLE", "ok")
                .secret("RUSTTABLE_TOKEN", "do-not-persist")
                .limits(ProcessLimits {
                    max_stdout_bytes: 16,
                    max_stderr_bytes: 16,
                    timeout: Duration::from_secs(2),
                }),
        )
        .expect("process result");
    assert_eq!(result.stdout, b"ok");
    assert!(
        result
            .receipt
            .environment_names
            .contains(&"VISIBLE".to_owned())
    );
    assert!(
        !result
            .receipt
            .environment_names
            .contains(&"RUSTTABLE_TOKEN".to_owned())
    );
    assert!(
        !result
            .receipt
            .redacted_args
            .iter()
            .any(|arg| arg == "do-not-persist")
    );
    assert_ne!(
        result.receipt.environment_policy_hash,
        result.receipt.environment_hash
    );
}

#[test]
fn secret_arguments_are_not_retained_in_receipts() {
    let result = ProcessRunner::new()
        .run(
            ProcessRequest::new(
                "/bin/sh",
                ["-c", "printf %s \"$1\"", "placeholder", "unused"],
            )
            .secret_arg(3, "private-token"),
        )
        .expect("process result");
    assert_eq!(result.stdout, b"private-token");
    assert!(
        result
            .receipt
            .args
            .iter()
            .all(|arg| !arg.contains("private-token"))
    );
}

#[test]
fn explicit_paths_are_stable_aliases_in_durable_receipts() {
    let directory = std::env::temp_dir().join(format!("rusttable-receipt-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("receipt directory");
    let result = ProcessRunner::new()
        .run(
            ProcessRequest::new("/bin/sh", ["-c", "exit 0"])
                .profile(EnvironmentProfile::TestFixture)
                .current_dir(&directory)
                .limits(ProcessLimits {
                    max_stdout_bytes: 16,
                    max_stderr_bytes: 16,
                    timeout: Duration::from_secs(2),
                }),
        )
        .expect("process result");
    assert_eq!(result.receipt.current_dir.as_deref(), Some("<worktree>"));
    assert_eq!(
        result.receipt.path_aliases.get("current_dir"),
        Some(&"<worktree>".to_owned())
    );
    let _ = std::fs::remove_dir_all(directory);
}
