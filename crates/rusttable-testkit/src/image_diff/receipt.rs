use serde::{Deserialize, Serialize};

use super::{
    DIFF_SCHEMA_VERSION, DiffArtifactDescriptor, DiffArtifactPayload, DiffError, DiffMetrics,
    DiffOutlier, DiffPolicy, MAX_OUTLIERS,
};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiffReceipt {
    pub schema_version: u32,
    pub policy: DiffPolicy,
    pub metrics: DiffMetrics,
    pub outliers: Vec<DiffOutlier>,
    pub artifacts: Vec<DiffArtifactDescriptor>,
    #[serde(skip)]
    pub artifact_payloads: Vec<DiffArtifactPayload>,
    pub passed: bool,
}

impl DiffReceipt {
    /// Serializes a validated receipt with stable field ordering.
    ///
    /// # Errors
    ///
    /// Returns an error when the receipt is invalid or cannot be serialized.
    pub fn stable_json(&self) -> Result<String, DiffError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|error| DiffError::Serialization(error.to_string()))
    }

    #[must_use]
    pub fn artifact_payloads(&self) -> &[DiffArtifactPayload] {
        &self.artifact_payloads
    }

    /// Decodes and validates a current-schema receipt.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON decoding or receipt validation fails.
    pub fn from_json(json: &str) -> Result<Self, DiffError> {
        let receipt: Self = serde_json::from_str(json)
            .map_err(|error| DiffError::Serialization(error.to_string()))?;
        receipt.validate()?;
        Ok(receipt)
    }

    /// Validates schema, policy, bounds, and artifact descriptor consistency.
    ///
    /// # Errors
    ///
    /// Returns an error when any receipt invariant is violated.
    pub fn validate(&self) -> Result<(), DiffError> {
        if self.schema_version != DIFF_SCHEMA_VERSION {
            return Err(DiffError::InvalidReceipt(
                "receipt schema is not current".to_owned(),
            ));
        }
        self.policy.validate()?;
        if self.outliers.len() > MAX_OUTLIERS {
            return Err(DiffError::InvalidReceipt(
                "receipt retains more than the bounded outlier limit".to_owned(),
            ));
        }
        for artifact in &self.artifacts {
            artifact.validate()?;
        }
        if !self.artifact_payloads.is_empty() {
            if self.artifacts.len() != self.artifact_payloads.len() {
                return Err(DiffError::InvalidReceipt(
                    "artifact descriptor and payload counts differ".to_owned(),
                ));
            }
            for (descriptor, payload) in self.artifacts.iter().zip(&self.artifact_payloads) {
                payload.validate()?;
                if descriptor != &DiffArtifactDescriptor::from_payload(payload) {
                    return Err(DiffError::InvalidReceipt(
                        "artifact descriptor does not match payload".to_owned(),
                    ));
                }
            }
        }
        Ok(())
    }
}
