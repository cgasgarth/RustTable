use std::path::Path;

use super::{DagReport, Result};
use crate::process::write_bounded_artifact;
use crate::root::RepositoryRoot;

pub(super) fn write(root: &RepositoryRoot, path: &Path, report: &DagReport) -> Result<()> {
    let serialized = serde_json::to_vec_pretty(report).map_err(|error| error.to_string())?;
    let destination = root.join(path);
    write_bounded_artifact(&destination, &serialized)
        .map_err(|error| format!("workspace DAG artifact {}: {error}", path.display()))
}

pub(super) fn failure_message(report: &DagReport, artifact: Option<&Path>) -> String {
    let first = report
        .first_violation
        .as_ref()
        .expect("violations are non-empty when formatting a failure");
    let artifact = artifact.map_or_else(
        || "<not requested>".to_owned(),
        |path| path.display().to_string(),
    );
    format!(
        "repo.verify-dag failed: first_violation={}; artifact={artifact}",
        serde_json::to_string(first).expect("violation is serializable")
    )
}
