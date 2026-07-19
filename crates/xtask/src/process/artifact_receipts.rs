use std::fs;
use std::time::SystemTime;

use super::{
    ARTIFACT_LIMIT, ArtifactReceipt, ProcessError, ProcessRequest, hash_bytes, stable_path,
};

pub(super) fn collect(
    request: &ProcessRequest,
    started: SystemTime,
) -> Result<Vec<ArtifactReceipt>, ProcessError> {
    let entries = [
        (request.stdout_artifact.as_ref(), "stdout"),
        (request.stderr_artifact.as_ref(), "stderr"),
    ];
    entries
        .into_iter()
        .filter_map(|(path, kind)| path.map(|path| (path, kind)))
        .map(|(path, kind)| {
            let metadata = fs::symlink_metadata(path).map_err(|error| ProcessError::Artifact {
                path: path.clone(),
                message: error.to_string(),
            })?;
            if !metadata.file_type().is_file() {
                return Err(ProcessError::Artifact {
                    path: path.clone(),
                    message: "artifact must be a regular file".to_owned(),
                });
            }
            let fresh = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(started).ok())
                .is_some();
            if !fresh {
                return Err(ProcessError::Artifact {
                    path: path.clone(),
                    message: "artifact was not created during execution".to_owned(),
                });
            }
            let size = metadata.len();
            if size > ARTIFACT_LIMIT as u64 {
                return Err(ProcessError::Artifact {
                    path: path.clone(),
                    message: "artifact exceeds the bounded size".to_owned(),
                });
            }
            let bytes = fs::read(path).map_err(|error| ProcessError::Artifact {
                path: path.clone(),
                message: error.to_string(),
            })?;
            Ok(ArtifactReceipt {
                path: stable_path(&path.display().to_string(), request.current_dir.as_deref()),
                size_bytes: size,
                sha256: hash_bytes(&bytes),
                kind: kind.to_owned(),
                fresh,
            })
        })
        .collect()
}
