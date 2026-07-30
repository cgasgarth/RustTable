use serde::{Deserialize, Serialize};

use super::{NumericalContract, NumericalContractError, ToleranceClass};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImplementationFamily {
    Scalar,
    AutoVectorized,
    ArchitectureSimd,
    PortableSimdLab,
    Gpu,
    Preview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompilerBaseline {
    PrimaryBeta,
    ComparisonOnly,
    BackendToolchain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProbeStatus<T> {
    Verified(T),
    Unsupported { reason: String },
    Failed { finding: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompilerFingerprint {
    pub active_toolchain: String,
    pub rustc_release: String,
    pub rustc_commit_hash: String,
    pub rustc_commit_date: String,
    pub llvm_version: String,
    pub cargo_release: String,
    pub cargo_commit_hash: String,
    pub cargo_commit_date: String,
    pub host: String,
    pub distribution_manifest: ProbeStatus<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImplementationNumerics {
    implementation_id: String,
    scalar_reference_id: String,
    implementation_hash: String,
    family: ImplementationFamily,
    compiler: CompilerBaseline,
    tolerance: ToleranceClass,
    contract: NumericalContract,
}

impl ImplementationNumerics {
    /// Registers complete backend-neutral numerical metadata.
    ///
    /// # Errors
    ///
    /// Rejects malformed stable IDs/hashes and exact claims that contain
    /// backend-defined floating behavior.
    pub fn new(
        implementation_id: impl Into<String>,
        scalar_reference_id: impl Into<String>,
        implementation_hash: impl Into<String>,
        family: ImplementationFamily,
        compiler: CompilerBaseline,
        tolerance: ToleranceClass,
        contract: NumericalContract,
    ) -> Result<Self, NumericalContractError> {
        let implementation_id = implementation_id.into();
        let scalar_reference_id = scalar_reference_id.into();
        let implementation_hash = implementation_hash.into();
        if !valid_id(&implementation_id) || !valid_id(&scalar_reference_id) {
            return Err(NumericalContractError::InvalidIdentifier);
        }
        if implementation_hash.len() != 64
            || !implementation_hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(NumericalContractError::InvalidImplementationHash);
        }
        if tolerance == ToleranceClass::Exact && contract.has_backend_defined_behavior() {
            return Err(NumericalContractError::ExactToleranceWithBackendDefinedBehavior);
        }
        Ok(Self {
            implementation_id,
            scalar_reference_id,
            implementation_hash,
            family,
            compiler,
            tolerance,
            contract,
        })
    }

    #[must_use]
    pub const fn contract(&self) -> NumericalContract {
        self.contract
    }

    #[must_use]
    pub fn implementation_id(&self) -> &str {
        &self.implementation_id
    }

    #[must_use]
    pub fn scalar_reference_id(&self) -> &str {
        &self.scalar_reference_id
    }

    #[must_use]
    pub fn implementation_hash(&self) -> &str {
        &self.implementation_hash
    }

    #[must_use]
    pub const fn family(&self) -> ImplementationFamily {
        self.family
    }

    #[must_use]
    pub const fn compiler(&self) -> CompilerBaseline {
        self.compiler
    }

    #[must_use]
    pub const fn tolerance(&self) -> ToleranceClass {
        self.tolerance
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
}
