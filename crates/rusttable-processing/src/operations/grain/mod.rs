//! Deterministic, full-image-coordinate film grain at the scene-linear RGB boundary.

#![forbid(unsafe_code)]

mod descriptor;
mod execution;
mod noise;
mod parameters;

pub use descriptor::{GRAIN_COMPATIBILITY_ID, GRAIN_SCHEMA_VERSION, grain_descriptor, presets};
pub use execution::{GrainGpuParameters, GrainPlan, wgpu_passes};
pub use noise::{grain_hash, grain_noise, hash_to_unit};
pub use parameters::{
    GRAIN_LEGACY_PARAMETER_BYTES, GRAIN_V1_PARAMETER_BYTES, GRAIN_V2_PARAMETER_BYTES, GrainChannel,
    GrainCodecError, GrainConfig, GrainHistory, GrainParameterError, GrainParametersV1,
    GrainParametersV2,
};
