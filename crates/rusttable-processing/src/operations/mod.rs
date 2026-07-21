//! Darktable-compatible image operations implemented at the Rust processing boundary.
//!
//! The modules in this directory own parameter migrations, checked planning,
//! deterministic scalar execution, and diagnostic receipts.  They are called
//! by the existing operation registry and pixelpipe; they are not a second
//! pipeline.

pub mod bloom;
pub mod colorcorrection;
pub mod colorin;
pub mod colorout;
pub mod colorreconstruction;
mod common;
pub mod convolution;
pub mod crop;
pub mod dither;
pub mod enlargecanvas;
pub mod finalscale;
pub mod flip;
pub mod graduatednd;
pub mod grain;
pub mod highlights;
pub mod invert;
pub mod lenscorrection;
pub mod perspective;
pub mod primaries;
pub mod relight;
pub mod rotatepixels;
pub mod scalepixels;
pub mod shadhi;
pub mod soften;
pub mod temperature;
pub mod vignette;

pub use common::{
    OperationExecutionError, ReconstructionBudget, ReconstructionDiagnostics, ReconstructionReceipt,
};
