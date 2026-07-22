#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
#![doc = "Typed color-management contracts for the `RustTable` rewrite."]

mod chromaticity;
mod matrix;
mod planner;
mod profile;
mod scalar;
mod space;
mod transform;

pub use chromaticity::{ChromaticityMatrixError, rgb_to_xyz_matrix, rotate_and_scale_primary};
pub use matrix::{Matrix3, MatrixError};
pub use planner::{BuiltinColorTransformPlanner, ColorTransformPlanner, PlannerError};
pub use profile::{
    Pcs, ProfileClass, ProfileId, ProfileIdError, ProfileModel, ProfileParserVersion,
};
pub use scalar::{FiniteF32, FiniteF32Error};
pub use space::{
    AdaptationMethod, AlphaMode, BuiltinSpace, ColorEncoding, ColorRole, ExtendedRange, Primaries,
    PrimariesError, TransferFunction, TransferFunctionError, WhitePoint, WhitePointError,
};
pub use transform::{
    Adaptation, AlphaTransform, BlackPointCompensation, ColorTransformRequest,
    ColorTransformRequestError, CompositeStep, Intent, Lut1D, Lut1DError, Lut3D, Lut3DError,
    LutInterpolation, LutPacking, MatrixErrorAdapter, Precision, RenderingIntent,
    TransformExecutionError, TransformPlan, TransformPlanError, TransformReceipt, TransformStep,
    TransformStepError, lab_to_xyz, xyz_to_lab,
};

/// Schema version for the stable color DTOs.
pub const COLOR_SCHEMA_VERSION: u16 = 1;

/// Deterministic acceptance result for the built-in color contract matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContractReceipt {
    pub schema_version: u16,
    pub builtins: usize,
    pub identity_plans: usize,
    pub roundtrips: usize,
}

/// Runs the bounded, deterministic built-in contract checks used by `xtask`.
///
/// # Errors
///
/// Returns a planner error when a built-in invariant or known-answer vector is invalid.
pub fn verify_builtin_contracts(
    verify_roundtrip: bool,
    verify_identities: bool,
) -> Result<ContractReceipt, PlannerError> {
    planner::verify_builtin_contracts(verify_roundtrip, verify_identities)
}

/// Encodes a request using the versioned canonical DTO representation.
pub fn encode_request(
    request: &ColorTransformRequest,
) -> Result<Vec<u8>, ColorTransformRequestError> {
    request.canonical_bytes()
}

/// Decodes and validates a canonical request DTO.
pub fn decode_request(bytes: &[u8]) -> Result<ColorTransformRequest, ColorTransformRequestError> {
    ColorTransformRequest::from_canonical_bytes(bytes)
}

/// Encodes a plan using the versioned canonical DTO representation.
pub fn encode_plan(plan: &TransformPlan) -> Result<Vec<u8>, TransformPlanError> {
    plan.canonical_bytes()
}

/// Decodes and validates a canonical plan DTO.
pub fn decode_plan(bytes: &[u8]) -> Result<TransformPlan, TransformPlanError> {
    TransformPlan::from_canonical_bytes(bytes)
}
