//! Deterministic CPU implementation of darktable's deprecated `defringe` node.

#![forbid(unsafe_code)]

mod codec;
mod descriptor;
mod execution;

pub use codec::{
    DEFRINGE_ALIAS, DEFRINGE_COMPATIBILITY_ID, DEFRINGE_PARAMETER_BYTES, DEFRINGE_RADIUS_DEFAULT,
    DEFRINGE_RADIUS_MAX, DEFRINGE_RADIUS_MIN, DEFRINGE_SCHEMA_VERSION, DEFRINGE_THRESHOLD_DEFAULT,
    DEFRINGE_THRESHOLD_MAX, DEFRINGE_THRESHOLD_MIN, DefringeCodecError, DefringeConfig,
    DefringeHistory, DefringeMode, DefringeParameterError, DefringeParametersV1,
};
pub use descriptor::defringe_descriptor;
pub use execution::{
    DEFRINGE_GAUSSIAN_ORDER, DEFRINGE_MAGIC_THRESHOLD_COEFFICIENT, DefringeAnalysis,
    DefringeBackend, DefringeBlend, DefringeExecutionError, DefringeMask, DefringeOutcome,
    DefringePixel, DefringePlan, DefringeReceipt,
};
