//! Strict, bounded pure-Rust JPEG XL still-image decoding.

mod backend;
mod container;
mod decode;
mod integration;
mod types;

pub(crate) use container::matches_signature as is_jpegxl_signature;
pub use decode::JxlDecoder;
pub(crate) use integration::{decode_jpegxl_frame, decode_jpegxl_probe, decode_legacy_rgba8};
pub use types::{
    JXL_BACKEND_ID, JXL_PROBE_BUDGET_BYTES, JxlAnimation, JxlBitDepth, JxlBoxDescriptor,
    JxlCodingMode, JxlColorEncoding, JxlColorSpace, JxlContainerInventory, JxlContainerKind,
    JxlDecodeError, JxlDecodeLimits, JxlDecodeMode, JxlDecodeReceipt, JxlDecodeRequest,
    JxlDecodeResult, JxlExtraChannel, JxlExtraChannelType, JxlFrameDescriptor, JxlHeader,
    JxlIccProfile, JxlJpegReconstruction, JxlPixelData, JxlPreviewDescriptor, JxlPrimaries,
    JxlRenderingIntent, JxlRoiBehavior, JxlStructuredColor, JxlToneMapping, JxlTransferFunction,
    JxlWhitePoint,
};

#[cfg(test)]
mod tests;
