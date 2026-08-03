//! Pure-Rust camera-RAW container, capability, and sensor-frame boundary.

mod adapter;
mod dng;
mod manifest;
mod probe;
mod types;

pub use adapter::{RawByteSource, RawlerRawDecoder, SliceRawSource};
pub use manifest::{
    RAWLER_CAPABILITY_MANIFEST_CAMERA_COUNT, RAWLER_CAPABILITY_MANIFEST_SHA256,
    rawler_capability_manifest,
};
pub use probe::{RAW_PROBE_BUDGET_BYTES, RawContainerRegistry};
pub use types::*;

use rusttable_image::{
    DecodeLimits, DecodedFrame, DecodedImage, ImageInputError, ImageProbe, InputFormat,
};

pub(crate) fn is_raw(bytes: &[u8]) -> bool {
    !matches!(
        RawContainerRegistry::standard().probe_bytes(bytes),
        RawProbeOutcome::NoMatch
    )
}

pub(crate) fn probe_raw(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    adapter::probe_legacy(bytes, limits)
}

pub(crate) fn decode_raw(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    adapter::decode_legacy_preview(bytes, limits)
}

pub(crate) fn decode_raw_frame(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedFrame, ImageInputError> {
    adapter::decode_linear_frame(bytes, limits)
}

pub(crate) fn decode_raw_legacy_frame(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedFrame, ImageInputError> {
    adapter::decode_legacy_frame(bytes, limits)
}

pub(crate) fn canonical_metadata_receipt(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<Option<Vec<u8>>, ImageInputError> {
    if !is_raw(bytes) {
        return Ok(None);
    }
    let request = RawDecodeRequest::new(adapter::raw_limits(limits)?);
    let result = RawlerRawDecoder::new()
        .decode_bytes(bytes, &request)
        .map_err(adapter::raw_input_error)?;
    result
        .receipt
        .metadata
        .to_canonical_bytes()
        .map(Some)
        .map_err(|_| ImageInputError::MalformedInput {
            format: InputFormat::Raw,
            message: "RAW metadata receipt could not be serialized".to_owned(),
        })
}
