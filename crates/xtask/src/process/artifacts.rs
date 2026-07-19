use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use super::{ARTIFACT_HEAD_LIMIT, ARTIFACT_MARKER, ARTIFACT_TAIL_LIMIT, ProcessError};

#[derive(Debug)]
pub(super) struct ArtifactWriter {
    pub(super) path: std::path::PathBuf,
    file: File,
    head: Vec<u8>,
    tail: VecDeque<u8>,
    truncated: bool,
}

pub(crate) fn write_bounded_artifact(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut writer = ArtifactWriter::create(path).map_err(|error| error.to_string())?;
    writer.push(bytes);
    writer.finish()
}

impl ArtifactWriter {
    pub(super) fn create(path: &Path) -> Result<Self, ProcessError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| ProcessError::Artifact {
                path: path.to_owned(),
                message: error.to_string(),
            })?;
        }
        let file = File::create(path).map_err(|error| ProcessError::Artifact {
            path: path.to_owned(),
            message: error.to_string(),
        })?;
        Ok(Self {
            path: path.to_owned(),
            file,
            head: Vec::with_capacity(ARTIFACT_HEAD_LIMIT),
            tail: VecDeque::with_capacity(ARTIFACT_TAIL_LIMIT),
            truncated: false,
        })
    }

    pub(super) fn push(&mut self, bytes: &[u8]) {
        let remaining = ARTIFACT_HEAD_LIMIT.saturating_sub(self.head.len());
        let head_count = remaining.min(bytes.len());
        self.head.extend_from_slice(&bytes[..head_count]);
        if head_count < bytes.len() {
            self.truncated = true;
            self.push_tail(&bytes[head_count..]);
        }
    }

    fn push_tail(&mut self, bytes: &[u8]) {
        self.tail.extend(bytes.iter().copied());
        while self.tail.len() > ARTIFACT_TAIL_LIMIT {
            let _ = self.tail.pop_front();
        }
    }

    pub(super) fn finish(mut self) -> Result<(), String> {
        self.file
            .write_all(&self.head)
            .and_then(|()| {
                if self.truncated {
                    self.file.write_all(ARTIFACT_MARKER)?;
                    self.file.write_all(self.tail.make_contiguous())?;
                }
                self.file.flush()
            })
            .map_err(|error| format!("{}: {error}", self.path.display()))
    }
}
