//! Darktable-compatible image operations implemented at the Rust processing boundary.
//!
//! The modules in this directory own parameter migrations, checked planning,
//! deterministic scalar execution, and diagnostic receipts.  They are called
//! by the existing operation registry and pixelpipe; they are not a second
//! pipeline.

pub mod colorreconstruction;
mod common;
pub mod highlights;

pub use common::{
    OperationExecutionError, ReconstructionBudget, ReconstructionDiagnostics, ReconstructionReceipt,
};
