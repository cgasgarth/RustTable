//! Deterministic CPU lens correction backed by a pinned Lensfun snapshot.
//!
//! This module owns its parameters, snapshot lookup, checked geometry, ROI
//! enclosure, and scalar execution.  Registry and pixelpipe wiring remain
//! outside the operation so the integration owner can add one prepared binding.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

mod descriptor;
mod execution;
mod geometry;
mod parameters;
mod snapshot;

pub use descriptor::lenscorrection_descriptor;
pub use execution::{LensCorrectionExecution, LensCorrectionExecutionError, LensCorrectionReceipt};
pub use geometry::{LensCorrectionCoordinateError, LensCorrectionPlan, LensCorrectionPlanError};
pub use parameters::{
    CorrectionFlags, LENS_CORRECTION_PARAMETER_BYTES, LENS_CORRECTION_PARAMETER_VERSION,
    LensCorrectionCodecError, LensCorrectionConfig, LensCorrectionHistoryParameters,
    LensCorrectionMethod, LensCorrectionMode, LensCorrectionParameterError,
    LensCorrectionParametersV1, LensGeometry, decode_history, encode_history,
};
pub use snapshot::{
    CameraProfile, DistortionCalibration, LENSFUN_DATABASE_COMMIT, LENSFUN_DATABASE_TIMESTAMP,
    LensProfile, LensfunSnapshot, TcaCalibration, VignettingCalibration,
};

pub const LENS_CORRECTION_COMPATIBILITY_ID: &str = "lenscorrection";
pub const LENS_CORRECTION_RUST_ID: &str = "rusttable.lenscorrection";
pub const LENS_CORRECTION_SCHEMA_VERSION: u16 = 1;
pub const LENS_CORRECTION_IMPLEMENTATION_VERSION: u16 = 1;
