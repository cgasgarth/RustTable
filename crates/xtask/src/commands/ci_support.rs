use std::fmt::Write;
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use super::{Check, DeclaredArtifactReceipt, Result};
use crate::root::RepositoryRoot;

pub(super) fn check_process_contract(
    check: &Check,
    surface: &str,
) -> (Vec<String>, Vec<String>, Option<(String, String)>) {
    if check.id != "workspace-dag" {
        return (check.args.clone(), check.artifacts.clone(), None);
    }
    let directory = "target/validation/workspace-dag";
    let report = format!("{directory}/{surface}.json");
    let stdout = format!("{directory}/{surface}.stdout.log");
    let stderr = format!("{directory}/{surface}.stderr.log");
    let mut args = check.args.clone();
    args.extend(["--artifact".to_owned(), report.clone()]);
    let mut artifacts = check.artifacts.clone();
    artifacts.extend([report, stdout.clone(), stderr.clone()]);
    (args, artifacts, Some((stdout, stderr)))
}

pub(super) fn verify_declared_artifacts(
    root: &RepositoryRoot,
    artifacts: &[String],
    started_at: SystemTime,
) -> Result<Vec<DeclaredArtifactReceipt>> {
    let mut receipts = Vec::new();
    for relative in artifacts {
        let path = root.join(relative);
        if std::path::Path::new(relative).is_absolute() {
            return Err(format!(
                "declared artifact {relative} must be relative to the worktree"
            ));
        }
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| format!("declared artifact {relative} is missing: {error}"))?;
        if !metadata.file_type().is_file() {
            return Err(format!(
                "declared artifact {relative} must be a regular file"
            ));
        }
        if metadata.len() > 256 * 1024 {
            return Err(format!(
                "declared artifact {relative} exceeds the 256 KiB limit"
            ));
        }
        let fresh = metadata.modified().is_ok_and(|modified| {
            modified.duration_since(started_at).is_ok()
                || started_at
                    .duration_since(modified)
                    .is_ok_and(|age| age <= std::time::Duration::from_secs(2))
        });
        if !fresh {
            return Err(format!("declared artifact {relative} is stale"));
        }
        let bytes = std::fs::read(&path)
            .map_err(|error| format!("declared artifact {relative} cannot be read: {error}"))?;
        let digest = Sha256::digest(&bytes);
        receipts.push(DeclaredArtifactReceipt {
            path: relative.clone(),
            size_bytes: metadata.len(),
            sha256: digest.iter().fold(String::new(), |mut output, byte| {
                let _ = write!(output, "{byte:02x}");
                output
            }),
            fresh: true,
        });
    }
    Ok(receipts)
}
