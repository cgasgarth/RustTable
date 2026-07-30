use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Write;

use crate::contract::ExportRequest;
use rusttable_core::template::{EvaluationError, EvaluationReceipt, LogicalArtifactName};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArtifactKind {
    Image,
    Sidecar,
    BundleMember,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalArtifact {
    pub kind: ArtifactKind,
    pub name: LogicalArtifactName,
    pub receipt: EvaluationReceipt,
    pub collision_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollisionGroup {
    pub key: String,
    pub artifact_indexes: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportPlan {
    pub artifacts: Vec<LogicalArtifact>,
    pub collision_groups: Vec<CollisionGroup>,
    pub receipt_hash: String,
}

impl ExportPlan {
    /// Builds a logical artifact plan using portable, case-sensitive defaults.
    ///
    /// # Errors
    ///
    /// Returns an evaluation error before any destination access occurs.
    pub fn build(requests: &[ExportRequest]) -> Result<Self, ExportPlanError> {
        Self::build_with_capabilities(requests, DestinationCapabilities::default())
    }

    /// Builds a plan and computes destination-specific comparison keys.
    ///
    /// # Errors
    ///
    /// Returns an evaluation error before any destination access occurs.
    pub fn build_with_capabilities(
        requests: &[ExportRequest],
        capabilities: DestinationCapabilities,
    ) -> Result<Self, ExportPlanError> {
        let mut artifacts = Vec::with_capacity(requests.len());
        for request in requests {
            let (name, receipt) = request
                .template
                .evaluate(&request.context, request.encoder.as_ref())
                .map_err(ExportPlanError::Evaluation)?;
            let collision_key = capabilities.canonical_key(&name.relative_path);
            artifacts.push(LogicalArtifact {
                kind: request.kind,
                name,
                receipt,
                collision_key,
            });
        }
        let mut grouped = BTreeMap::<String, Vec<usize>>::new();
        for (index, artifact) in artifacts.iter().enumerate() {
            grouped
                .entry(artifact.collision_key.clone())
                .or_default()
                .push(index);
        }
        let collision_groups = grouped
            .into_iter()
            .filter_map(|(key, artifact_indexes)| {
                (artifact_indexes.len() > 1).then_some(CollisionGroup {
                    key,
                    artifact_indexes,
                })
            })
            .collect::<Vec<_>>();
        let mut canonical = String::from("export-plan-v1\n");
        for artifact in &artifacts {
            let _ = writeln!(
                canonical,
                "{:?}|{}|{}",
                artifact.kind,
                artifact.name.relative_path,
                artifact.receipt.receipt_hash()
            );
        }
        let receipt_hash = hash_hex(canonical.as_bytes());
        Ok(Self {
            artifacts,
            collision_groups,
            receipt_hash,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DestinationCapabilities {
    pub case_sensitive: bool,
    pub unicode_normalized: bool,
}

impl Default for DestinationCapabilities {
    fn default() -> Self {
        Self {
            case_sensitive: true,
            unicode_normalized: true,
        }
    }
}

impl DestinationCapabilities {
    #[must_use]
    pub fn canonical_key(&self, path: &str) -> String {
        if self.case_sensitive {
            path.to_owned()
        } else {
            path.to_lowercase()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportPlanError {
    Evaluation(EvaluationError),
}

impl fmt::Display for ExportPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "export plan failed: {self:?}")
    }
}

impl std::error::Error for ExportPlanError {}

fn hash_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
