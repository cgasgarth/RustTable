//! GTK-facing orchestration for the selected persisted-edit PNG workflow.
//!
//! This maps Darktable's `src/libs/export.c` request/progress boundary and
//! `src/imageio/imageio.c` render handoff to the existing Rust PNG publisher,
//! which owns the bounded encoding, verification, and disk publication stages.

mod metadata;
mod request;
mod worker;

pub use request::{
    ExportCancellation, ExportCollisionSelection, ExportRequest, ExportSettings, ExportSize,
    ExportSizeError, ExportSizeSelection, MAX_OUTPUT_BYTES, MAX_OUTPUT_EDGE,
};
pub use worker::{
    ExportCompletion, ExportRunError, ExportStage, ExportStatus, run, run_with_progress,
};
