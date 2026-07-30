//! Deterministic CPU implementation of Darktable's version-one censorize node.

#![forbid(unsafe_code)]

mod codec;
mod execution;
mod gaussian;
mod rng;

pub use codec::{
    CENSORIZE_COMPATIBILITY_ID, CENSORIZE_PARAMETER_BYTES, CENSORIZE_SCHEMA_VERSION,
    CensorizeCodecError, CensorizeConfig, CensorizeHistory, CensorizeParameterError,
    CensorizeParametersV1,
};
pub use execution::{
    CENSORIZE_RNG_VERSION, CensorizeBackend, CensorizeBlend, CensorizeExecutionError,
    CensorizeMask, CensorizePixel, CensorizePlan, CensorizeReceipt, CensorizeStages,
};
pub use rng::{CensorizeRng, gaussian_noise, splitmix32, xoshiro128plus};

#[must_use]
pub fn censorize_descriptor() -> crate::descriptor::OperationDescriptor {
    crate::descriptor::censorize_descriptor()
}
