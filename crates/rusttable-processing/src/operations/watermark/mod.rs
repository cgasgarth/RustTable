//! Managed SVG watermark planning, history, and canonical CPU compositing.
//!
//! The generic operation/pixelpipe model currently has no managed asset/context
//! port. This module is therefore the complete backend plan seam; registry
//! discovery is explicit but remains unavailable until that port is added.

#![forbid(unsafe_code)]

mod codec;
mod context;
mod descriptor;
mod execution;

pub use codec::{
    WATERMARK_ALLOWED_FONT_SET_HASH, WATERMARK_COMPATIBILITY_ID, WATERMARK_IMPLEMENTATION_VERSION,
    WATERMARK_PARAMETER_VERSION, WATERMARK_RUST_ID, WATERMARK_SCHEMA_VERSION, WatermarkAnchor,
    WatermarkCodecError, WatermarkHistory, WatermarkParametersV1, WatermarkParametersV7,
    WatermarkScaleMode, decode_history, migrate_history,
};
pub use context::{ExpandedWatermark, WatermarkContext, WatermarkContextError};
pub use descriptor::watermark_descriptor;
pub use execution::{WatermarkExecutionError, WatermarkPlan, WatermarkReceipt};
