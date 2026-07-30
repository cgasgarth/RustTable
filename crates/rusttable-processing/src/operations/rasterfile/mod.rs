//! Deprecated external raster-mask compatibility operation.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

mod codec;
mod decoder;
mod descriptor;
mod execution;

pub use codec::{
    RASTERFILE_COMPATIBILITY_ID, RASTERFILE_IMPLEMENTATION_VERSION, RASTERFILE_PARAMETER_BYTES,
    RASTERFILE_PARAMETER_VERSION, RASTERFILE_RUST_ID, RASTERFILE_SCHEMA_VERSION,
    RasterFileChannelMode, RasterFileCodecError, RasterFileHistory, RasterFileParametersV1,
    RasterMaskAsset, RasterMaskAssetError, RasterMaskCache, RasterMaskFormat, RasterMaskLimits,
    decode_history, migrate_history,
};
pub use descriptor::rasterfile_descriptor;
pub use execution::{
    RASTERFILE_WGSL, RasterFileExecutionError, RasterFileForm, RasterFilePlan, RasterFileReceipt,
    RasterFileTile, RasterFileVectorizationReceipt, wgpu_dispatch,
};
