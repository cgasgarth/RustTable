//! Darktable-compatible image operations implemented at the Rust processing boundary.
//!
//! The modules in this directory own parameter migrations, checked planning,
//! deterministic scalar execution, and diagnostic receipts.  They are called
//! by the existing operation registry and pixelpipe; they are not a second
//! pipeline.

pub mod colorcorrection;
pub mod colorin;
pub mod colorout;
pub mod colorreconstruction;
mod common;
pub mod crop;
pub mod enlargecanvas;
pub mod finalscale;
pub mod flip;
pub mod highlights;
pub mod primaries;
pub mod rotatepixels;
pub mod scalepixels;
pub mod temperature;

pub use common::{
    OperationExecutionError, ReconstructionBudget, ReconstructionDiagnostics, ReconstructionReceipt,
};
