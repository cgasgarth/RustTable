//! Strict, bounded PNG image I/O through pure-Rust format leaves.
//!
//! Direct ports are kept under this module so PNG parsing, sample conversion,
//! and deterministic encoding can be reviewed against Darktable's native PNG
//! responsibilities without changing shared registries or application seams.

mod decode;
mod encode;
mod parser;
mod types;

pub use decode::PngDecoder;
pub use encode::{PngEncodeError, PngEncodeOptions, PngEncoder};
pub use types::{
    PNG_BACKEND_ID, PngAnimation, PngBitDepth, PngChunk, PngChunkInventory, PngCicp, PngColorType,
    PngDecodeError, PngDecodeLimits, PngDecodeMode, PngDecodeReceipt, PngDecodeRequest,
    PngDecodeResult, PngHeader, PngMetadataInventory, PngPhysicalResolution, PngPixelData,
    PngProfileInventory, PngSampleLayout, PngTextInventory,
};

pub(crate) use decode::{decode_png_probe, is_png_signature};
