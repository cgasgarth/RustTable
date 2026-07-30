//! Deterministic CPU implementation of darktable's deprecated CLAHE node.

#![forbid(unsafe_code)]

mod codec;
mod descriptor;
mod execution;

pub use codec::{
    CLAHE_ALIAS, CLAHE_COMPATIBILITY_ID, CLAHE_PARAMETER_BYTES, CLAHE_RADIUS_DEFAULT,
    CLAHE_RADIUS_MAX, CLAHE_RADIUS_MIN, CLAHE_SCHEMA_VERSION, CLAHE_SLOPE_DEFAULT, CLAHE_SLOPE_MAX,
    CLAHE_SLOPE_MIN, ClaheCodecError, ClaheConfig, ClaheHistory, ClaheParameterError,
    ClaheParametersV1,
};
pub use descriptor::clahe_descriptor;
pub use execution::{
    CLAHE_BINS, CLAHE_HISTOGRAM_ENTRIES, ClaheBackend, ClaheBlend, ClaheExecutionError, ClaheMask,
    ClaheOutcome, ClahePixel, ClahePlan, ClaheReceipt,
};
