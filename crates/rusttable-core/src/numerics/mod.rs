//! Backend-neutral numerical contracts and deterministic scalar helpers.
//!
//! This module deliberately has no GPU, codec, or operation dependencies. Product
//! implementations select these policies; they do not redefine their semantics.

pub mod conversion;
mod error;
pub mod inspection;
mod metadata;
pub mod ordered;
mod policy;
pub mod reduction;

pub use error::{NumericalContractError, NumericalError};
pub use metadata::{
    CompilerBaseline, CompilerFingerprint, ImplementationFamily, ImplementationNumerics,
    ProbeStatus,
};
pub use policy::{
    ConversionPolicy, ConversionRange, FloatDomainPolicy, FmaPolicy, NonFinitePolicy,
    NumericalContract, ReductionPolicy, RoundingPolicy, SubnormalPolicy, ToleranceClass,
    TranscendentalPolicy,
};

/// Numerical receipt schema emitted by current verification tooling.
pub const NUMERICS_SCHEMA: &str = "rusttable.numerics.v1";

/// Requested primary archive from issue #286.
///
/// Verification must still prove that the archive exists and is the active
/// repository toolchain; this declaration is not a compiler fingerprint.
pub const REQUESTED_PRIMARY_TOOLCHAIN: &str = "beta-2026-07-18";
