use std::collections::BTreeMap;
use std::fmt;

use rusttable_ai_native::RuntimeIdentity;
use sha2::{Digest, Sha256};

use crate::ModelIdentity;
use crate::Provider;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualificationFixture<'a> {
    pub identity: &'a str,
    pub expected: &'a [f32],
    pub actual: &'a [f32],
    pub absolute_tolerance: f32,
    pub relative_tolerance: f32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderQualificationReceipt {
    model: ModelIdentity,
    provider: Provider,
    runtime_version: String,
    adapter_version: String,
    target: String,
    contract_hash: [u8; 32],
    fixture_hash: String,
    output_hash: [u8; 32],
    max_error_bits: u32,
}

impl ProviderQualificationReceipt {
    pub fn qualify(
        model: ModelIdentity,
        provider: Provider,
        runtime: &RuntimeIdentity,
        contract_hash: [u8; 32],
        fixture: QualificationFixture<'_>,
    ) -> Result<Self, QualificationError> {
        if fixture.expected.len() != fixture.actual.len()
            || !fixture.absolute_tolerance.is_finite()
            || !fixture.relative_tolerance.is_finite()
            || fixture.absolute_tolerance < 0.0
            || fixture.relative_tolerance < 0.0
            || fixture.identity.is_empty()
        {
            return Err(QualificationError::InvalidFixture);
        }
        let mut max_error = 0.0_f32;
        for (expected, actual) in fixture.expected.iter().zip(fixture.actual) {
            if !expected.is_finite() || !actual.is_finite() {
                return Err(QualificationError::NonFiniteOutput);
            }
            let error = (expected - actual).abs();
            let allowed = fixture.absolute_tolerance
                + fixture.relative_tolerance * expected.abs().max(actual.abs());
            if error > allowed {
                return Err(QualificationError::Tolerance { error, allowed });
            }
            max_error = max_error.max(error);
        }
        let output_hash: [u8; 32] = Sha256::digest(
            fixture
                .actual
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect::<Vec<_>>(),
        )
        .into();
        Ok(Self {
            model,
            provider,
            runtime_version: runtime.runtime_version().to_owned(),
            adapter_version: runtime.adapter_version().to_owned(),
            target: runtime.target().to_owned(),
            contract_hash,
            fixture_hash: fixture.identity.to_owned(),
            output_hash,
            max_error_bits: max_error.to_bits(),
        })
    }

    #[must_use]
    pub const fn model(&self) -> ModelIdentity {
        self.model
    }
    #[must_use]
    pub const fn provider(&self) -> Provider {
        self.provider
    }
    #[must_use]
    pub fn runtime_version(&self) -> &str {
        &self.runtime_version
    }
    #[must_use]
    pub fn adapter_version(&self) -> &str {
        &self.adapter_version
    }
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }
    #[must_use]
    pub const fn contract_hash(&self) -> [u8; 32] {
        self.contract_hash
    }
    #[must_use]
    pub fn fixture_hash(&self) -> &str {
        &self.fixture_hash
    }
    #[must_use]
    pub const fn output_hash(&self) -> [u8; 32] {
        self.output_hash
    }
    #[must_use]
    pub const fn max_error(&self) -> f32 {
        f32::from_bits(self.max_error_bits)
    }

    #[must_use]
    pub fn receipt_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.model.as_bytes());
        hasher.update([self.provider as u8]);
        hasher.update(self.runtime_version.as_bytes());
        hasher.update([0]);
        hasher.update(self.adapter_version.as_bytes());
        hasher.update([0]);
        hasher.update(self.target.as_bytes());
        hasher.update(self.contract_hash);
        hasher.update(self.fixture_hash.as_bytes());
        hasher.update(self.output_hash);
        hasher.update(self.max_error_bits.to_le_bytes());
        hasher.finalize().into()
    }
}

#[derive(Debug, Default)]
pub struct QualificationStore {
    receipts: BTreeMap<(ModelIdentity, Provider), ProviderQualificationReceipt>,
}

impl QualificationStore {
    pub fn insert(&mut self, receipt: ProviderQualificationReceipt) {
        self.receipts
            .insert((receipt.model(), receipt.provider()), receipt);
    }

    #[must_use]
    pub fn get(
        &self,
        model: ModelIdentity,
        provider: Provider,
    ) -> Option<&ProviderQualificationReceipt> {
        self.receipts.get(&(model, provider))
    }

    pub fn require(
        &self,
        model: ModelIdentity,
        provider: Provider,
    ) -> Result<&ProviderQualificationReceipt, QualificationError> {
        self.get(model, provider)
            .ok_or(QualificationError::Unqualified)
    }

    pub fn invalidate_runtime(&mut self, runtime_version: &str) {
        self.receipts
            .retain(|_, receipt| receipt.runtime_version() == runtime_version);
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum QualificationError {
    InvalidFixture,
    NonFiniteOutput,
    Tolerance { error: f32, allowed: f32 },
    Unqualified,
}

impl fmt::Display for QualificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "provider qualification failed: {self:?}")
    }
}

impl std::error::Error for QualificationError {}
