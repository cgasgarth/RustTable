//! Deprecated darktable `clipping` compatibility operation.

mod codec;
mod descriptor;
mod execution;
mod geometry;

pub use codec::{
    CLIPPING_COMPATIBILITY_ID, CLIPPING_IMPLEMENTATION_VERSION, CLIPPING_MAX_DIMENSION,
    CLIPPING_PARAMETER_VERSION, CLIPPING_RUST_ID, CLIPPING_SCHEMA_VERSION, ClippingConfig,
    ClippingHistory, ClippingInterpolation, ClippingParametersV2, ClippingParametersV3,
    ClippingParametersV4, ClippingParametersV5, decode_history, migrate_history,
};
pub use descriptor::{CLIPPING_DESCRIPTOR_PARAMETER_COUNT, clipping_descriptor, history_policy};
pub use execution::{
    CLIPPING_WGSL, ClippingExecution, ClippingExecutionError, ClippingReceipt, wgpu_dispatch,
    wgpu_passes,
};
pub use geometry::{ClippingPlan, ClippingPlanError, CropRect, KeystoneMode, TransformPointError};
