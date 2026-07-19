use std::fs;
use std::time::{Duration, SystemTime};

use super::{
    ARTIFACT_LIMIT, ArtifactReceipt, ProcessError, ProcessRequest, hash_bytes, stable_path,
};

pub(super) fn collect(
    request: &ProcessRequest,
    started: SystemTime,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<Vec<ArtifactReceipt>, ProcessError> {
    let entries = [
        (request.stdout_artifact.as_ref(), "stdout", stdout),
        (request.stderr_artifact.as_ref(), "stderr", stderr),
    ];
    entries
        .into_iter()
        .filter_map(|(path, kind, output)| path.map(|path| (path, kind, output)))
        .map(|(path, kind, output)| {
            let metadata = match fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && output.is_empty() => {
                    return Ok(absent_receipt(request, path, kind));
                }
                Err(error) => {
                    return Err(ProcessError::Artifact {
                        path: path.clone(),
                        message: error.to_string(),
                    });
                }
            };
            if !metadata.file_type().is_file() {
                return Err(ProcessError::Artifact {
                    path: path.clone(),
                    message: "artifact must be a regular file".to_owned(),
                });
            }
            let fresh = metadata.modified().is_ok_and(|modified| {
                modified.duration_since(started).is_ok()
                    || started
                        .duration_since(modified)
                        .is_ok_and(|age| age <= Duration::from_secs(2))
            });
            if !fresh {
                if metadata.len() == 0 && output.is_empty() {
                    return Ok(absent_receipt(request, path, kind));
                }
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
                present: true,
            })
        })
        .collect()
}

fn absent_receipt(request: &ProcessRequest, path: &std::path::Path, kind: &str) -> ArtifactReceipt {
    ArtifactReceipt {
        path: stable_path(&path.display().to_string(), request.current_dir.as_deref()),
        size_bytes: 0,
        sha256: hash_bytes(&[]),
        kind: kind.to_owned(),
        fresh: false,
        present: false,
    }
}

#[cfg(test)]
mod tests {
    use super::{Duration, collect};
    use crate::process::ProcessRequest;
    use std::time::SystemTime;

    #[test]
    fn represents_missing_empty_streams_without_requiring_files() {
        let root =
            std::env::temp_dir().join(format!("rusttable-absent-artifacts-{}", std::process::id()));
        let request = ProcessRequest::new("sh", std::iter::empty::<&str>())
            .artifacts(root.join("stdout"), root.join("stderr"));
        let receipts = collect(&request, SystemTime::now(), &[], &[]).expect("empty streams");
        assert_eq!(receipts.len(), 2);
        assert!(receipts.iter().all(|receipt| !receipt.present));
        assert!(receipts.iter().all(|receipt| !receipt.fresh));
        assert!(receipts.iter().all(|receipt| receipt.size_bytes == 0));
    }

    #[test]
    fn rejects_missing_nonempty_stream_artifacts() {
        let root = std::env::temp_dir().join(format!(
            "rusttable-required-artifacts-{}",
            std::process::id()
        ));
        let request = ProcessRequest::new("sh", std::iter::empty::<&str>())
            .artifacts(root.join("stdout"), root.join("stderr"));
        let error =
            collect(&request, SystemTime::now(), b"output", &[]).expect_err("required artifact");
        assert!(error.to_string().contains("cannot write process artifact"));
    }

    #[test]
    fn accepts_small_filesystem_clock_skew() {
        let started = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(9);
        assert!(modified.duration_since(started).is_err());
        assert!(
            started
                .duration_since(modified)
                .is_ok_and(|age| age <= Duration::from_secs(2))
        );
    }

    #[test]
    fn rejects_stale_artifacts_beyond_clock_skew() {
        let started = SystemTime::UNIX_EPOCH + Duration::from_secs(10);
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(7);
        assert!(
            started
                .duration_since(modified)
                .is_ok_and(|age| age > Duration::from_secs(2))
        );
    }
}
