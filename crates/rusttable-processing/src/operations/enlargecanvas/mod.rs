//! Checked canvas enlargement geometry and scalar fill/copy execution.
//!
//! Registry and pixelpipe wiring deliberately live outside this module.  The
//! plan is the single source of truth for dimensions, placement, ROI mapping,
//! coordinate transforms, and image copying.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

mod descriptor;
mod execution;
mod geometry;
mod parameters;

pub use descriptor::enlargecanvas_descriptor;
pub use execution::{
    EnlargeCanvasExecution, EnlargeCanvasExecutionError, EnlargeCanvasImageError,
    EnlargeCanvasImageExecution,
};
pub use geometry::{
    CanvasRect, EnlargeCanvasGeometry, EnlargeCanvasGeometryError, EnlargeCanvasPlan,
    EnlargeCanvasPlanError,
};
pub use parameters::{
    CanvasColor, CanvasFill, EnlargeCanvasCodecError, EnlargeCanvasConfig,
    EnlargeCanvasHistoryParameters, EnlargeCanvasParameterError, EnlargeCanvasParametersV1,
    decode_history,
};

pub type EnlargeCanvasColor = CanvasColor;
pub type EnlargeCanvasFill = CanvasFill;

pub const ENLARGECANVAS_COMPATIBILITY_ID: &str = "enlargecanvas";
pub const ENLARGECANVAS_RUST_ID: &str = "rusttable.enlargecanvas";
pub const ENLARGECANVAS_SCHEMA_VERSION: u16 = 1;
pub const ENLARGECANVAS_PARAMETER_VERSION: u16 = 1;
pub const ENLARGECANVAS_PARAMETER_BYTES: usize = 20;
pub const ENLARGECANVAS_MAX_PERCENT: f32 = 100.0;
pub const ENLARGECANVAS_MAX_DIMENSION: u32 = 1 << 30;
