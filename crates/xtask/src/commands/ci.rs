use super::{Result, report};
use crate::cli::CiCommand;
use crate::process::{ProcessRequest, ProcessRunner};
use crate::root::RepositoryRoot;

pub(super) fn run(root: &RepositoryRoot, command: &CiCommand, runner: &ProcessRunner) -> Result {
    let (name, script, timeout) = match command {
        CiCommand::Precommit => ("ci.precommit", "scripts/precommit-fast.sh", 60),
        CiCommand::Prepush => ("ci.prepush", "scripts/prepush-fast.sh", 60),
        CiCommand::Pr => ("ci.pr", "scripts/pr-ci.sh", 150),
        CiCommand::Main => ("ci.main", "scripts/main-ci.sh", 300),
    };
    let result = runner
        .run(
            ProcessRequest::new("bash", [script])
                .current_dir(root.path())
                .limits(crate::process::ProcessLimits {
                    max_stdout_bytes: 64 * 1024,
                    max_stderr_bytes: 64 * 1024,
                    timeout: std::time::Duration::from_secs(timeout),
                }),
        )
        .map_err(|error| error.to_string())?;
    if !result.receipt.success() {
        return Err(format!("{name} failed ({})", result.receipt.status));
    }
    Ok(report(
        root,
        name,
        serde_json::json!({ "receipt": result.receipt }),
    ))
}
