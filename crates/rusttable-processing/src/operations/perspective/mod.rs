//! Checked `ashift` perspective correction and deterministic structure analysis.
//!
//! This module is intentionally self-contained so the operation registry can
//! adopt it without taking ownership of its geometry or analysis internals.

mod analysis;
mod codec;
pub mod descriptor;
mod execution;
mod geometry;

pub use analysis::{
    AnalysisConfig, AnalysisError, AnalysisResult, AnalysisStatus, DetectedLine, LineAnalysis,
    LineKind, LineSegment, LuminanceFrame, analyze_lines, detect_lines,
};
pub use codec::{
    ASHIFT_COMPATIBILITY_ID, ASHIFT_IMPLEMENTATION_VERSION, ASHIFT_MAX_DIMENSION,
    ASHIFT_MAX_SAVED_LINES, ASHIFT_PARAMETER_VERSION, ASHIFT_RUST_ID, ASHIFT_SCHEMA_VERSION,
    AutoMethod, CropMode, FitAxis, LensModel, PerspectiveConfig, PerspectiveConfigError,
    PerspectiveHistory, PerspectiveHistoryError, PerspectiveParametersV1, PerspectiveParametersV2,
    PerspectiveParametersV3, PerspectiveParametersV4, PerspectiveParametersV5, Quad,
    decode_history, migrate_history,
};
pub use descriptor::{
    ASHIFT_DESCRIPTOR_LINE_COMPONENTS, ASHIFT_DESCRIPTOR_LINE_PARAMETER_COUNT,
    ASHIFT_DESCRIPTOR_PARAMETER_COUNT, perspective_descriptor,
};
pub use execution::{
    BoundaryMode, Interpolation, PerspectiveExecution, PerspectiveExecutionError, PerspectivePlan,
    PerspectiveReceipt,
};
pub use geometry::{
    Homography, HomographyError, Point, PointError, Rect, RectError, TransformError,
};
