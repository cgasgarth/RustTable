//! Strict, bounded pure-Rust WebP still-image decoding.

mod container;
mod decode;
mod integration;
mod types;

pub(crate) use container::is_webp_signature;
pub use container::{inspect, probe};
pub use decode::WebPDecoder;
pub(crate) use integration::{decode_legacy_rgba8, decode_webp_frame, decode_webp_probe};
pub use types::{
    WEBP_BACKEND_ID, WebPChunk, WebPChunkInventory, WebPCodingMode, WebPContainer,
    WebPDataLocation, WebPDecodeError, WebPDecodeLimits, WebPDecodeMode, WebPDecodeReceipt,
    WebPDecodeRequest, WebPDecodeResult, WebPFeatures, WebPHeader, WebPMetadataChunk,
    WebPMetadataInventory, WebPPixelData,
};

#[cfg(test)]
mod tests;
