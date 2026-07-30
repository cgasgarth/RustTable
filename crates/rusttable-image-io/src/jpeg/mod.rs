//! Bounded, pure-Rust JPEG still-image boundary.

mod decode;
mod parser;
mod types;

pub use decode::{
    JpegDecodeError, JpegDecodeMode, JpegDecodeReceipt, JpegDecodeRequest, JpegDecodeResult,
    JpegDecoder,
};
pub(crate) use parser::probe_bounded;
pub use parser::{inspect, probe};
pub use types::{
    JPEG_PROBE_BUDGET_BYTES, JpegCodingProcess, JpegComponentModel, JpegHeader,
    JpegMetadataSegment, JpegPixelData, JpegSampling, JpegSof,
};
