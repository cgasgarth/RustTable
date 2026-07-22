//! Strict, bounded PNG decoding through the workspace's pure-Rust `png` crate.

mod decode;
mod parser;
mod types;

pub use decode::PngDecoder;
pub use types::{
    PNG_BACKEND_ID, PngAnimation, PngBitDepth, PngChunk, PngChunkInventory, PngColorType,
    PngDecodeError, PngDecodeLimits, PngDecodeMode, PngDecodeReceipt, PngDecodeRequest,
    PngDecodeResult, PngHeader, PngMetadataInventory, PngPhysicalResolution, PngPixelData,
    PngProfileInventory, PngSampleLayout, PngTextInventory,
};

pub(crate) use decode::{decode_png_probe, is_png_signature};
