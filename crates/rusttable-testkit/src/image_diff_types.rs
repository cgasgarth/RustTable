use std::fmt::Write;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{DIFF_SCHEMA_VERSION, DiffError, MAX_ARTIFACT_BYTES, artifacts};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiffArtifactPayload {
    pub schema_version: u32,
    pub kind: ArtifactKind,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

pub type DiffArtifact = DiffArtifactPayload;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DiffArtifactDescriptor {
    pub schema_version: u32,
    pub kind: ArtifactKind,
    pub width: u32,
    pub height: u32,
    pub media_type: String,
    pub byte_size: usize,
    pub sha256: String,
    pub artifact_id: String,
    pub path_alias: String,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum ArtifactKind {
    HeatmapRgba8,
    BlinkRgba32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlinkPlanes {
    pub source: Vec<f32>,
    pub reference: Vec<f32>,
}

impl DiffArtifactPayload {
    /// Validates the versioned artifact envelope and exact byte length.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload schema, dimensions, or byte length is invalid.
    pub fn validate(&self) -> Result<(), DiffError> {
        artifacts::validate(self)
    }

    /// Parses the blink manifest and its two separately framed canonical planes.
    ///
    /// # Errors
    ///
    /// Returns an error when the payload is not a valid blink artifact.
    pub fn blink_planes(&self) -> Result<BlinkPlanes, DiffError> {
        artifacts::blink_planes(self)
    }
}

impl DiffArtifactDescriptor {
    pub(super) fn from_payload(payload: &DiffArtifactPayload) -> Self {
        let digest = Sha256::digest(&payload.bytes);
        let mut sha256 = String::with_capacity(64);
        for byte in digest {
            write!(&mut sha256, "{byte:02x}").expect("writing to a string cannot fail");
        }
        Self {
            schema_version: payload.schema_version,
            kind: payload.kind,
            width: payload.width,
            height: payload.height,
            media_type: match payload.kind {
                ArtifactKind::HeatmapRgba8 => "image/rgba8".to_owned(),
                ArtifactKind::BlinkRgba32 => "application/x-rusttable-blink+binary".to_owned(),
            },
            byte_size: payload.bytes.len(),
            sha256: sha256.clone(),
            artifact_id: format!("sha256:{sha256}"),
            path_alias: format!(
                "artifacts/{:?}-{}x{}",
                payload.kind, payload.width, payload.height
            ),
        }
    }

    pub(super) fn validate(&self) -> Result<(), DiffError> {
        if self.schema_version != DIFF_SCHEMA_VERSION
            || self.byte_size > MAX_ARTIFACT_BYTES
            || self.sha256.len() != 64
            || !self
                .sha256
                .chars()
                .all(|character| character.is_ascii_hexdigit())
            || self.artifact_id != format!("sha256:{}", self.sha256)
            || self.path_alias.is_empty()
        {
            return Err(DiffError::InvalidReceipt(
                "invalid artifact descriptor".to_owned(),
            ));
        }
        Ok(())
    }
}
