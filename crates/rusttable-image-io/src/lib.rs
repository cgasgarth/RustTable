#![forbid(unsafe_code)]
#![doc = "Bounded pure-Rust JPEG, PNG, TIFF, and `BigTIFF` file input/output for `RustTable`."]

mod input;
pub mod jpeg;
mod output;
pub mod png;
mod raster_samples;
pub mod raw;
mod registry;
mod svg;
pub mod tiff;

pub mod dng_output;

pub use input::FileImageInput;
pub use jpeg::{
    JPEG_PROBE_BUDGET_BYTES, JpegCodingProcess, JpegComponentModel, JpegDecodeError,
    JpegDecodeMode, JpegDecodeReceipt, JpegDecodeRequest, JpegDecodeResult, JpegDecoder,
    JpegHeader, JpegMetadataSegment, JpegPixelData, JpegSampling, JpegSof,
};
pub use output::FileImageOutput;
pub use png::{
    PNG_BACKEND_ID, PngAnimation, PngBitDepth, PngChunk, PngChunkInventory, PngColorType,
    PngDecodeError, PngDecodeLimits, PngDecodeMode, PngDecodeReceipt, PngDecodeRequest,
    PngDecodeResult, PngDecoder, PngHeader, PngMetadataInventory, PngPhysicalResolution,
    PngPixelData, PngProfileInventory, PngSampleLayout, PngTextInventory,
};
pub use raster_samples::{DecodedRgbSamples, decode_png_rgb_samples};
pub use raw::{
    RAW_PROBE_BUDGET_BYTES, RAWLER_BACKEND_ID, RawByteSource, RawCameraEvidence, RawCameraIdentity,
    RawCancellationToken, RawCapabilityDescriptor, RawCapabilityError, RawCapabilityKey,
    RawCapabilityKind, RawCapabilityManifest, RawCapabilityResolveError, RawCfa, RawChannel,
    RawColorMatrix, RawCompression, RawCompressionEvidence, RawContainerKind, RawContainerProbe,
    RawContainerRegistry, RawDecodeError, RawDecodeLimits, RawDecodeLimitsError, RawDecodeReceipt,
    RawDecodeRequest, RawDecodeResult, RawDimensions, RawFrame, RawFrameParts,
    RawFrameValidationError, RawHeader, RawIlluminant, RawLevelPattern, RawOpcodeDescriptor,
    RawOpcodeStage, RawOrientation, RawPlane, RawPlaneLayout, RawPreviewDescriptor,
    RawPreviewFormat, RawPreviewKind, RawProbeEvidence, RawProbeOutcome, RawRect, RawSourceError,
    RawSourceReceipt, RawlerRawDecoder, SliceRawSource, rawler_capability_manifest,
};
pub use registry::{ImageDecoderRegistry, PROBE_BUDGET_BYTES, ProbeOutcome};
pub use rusttable_image::{DecoderDescriptor, DecoderIdentity};
pub use svg::{ManagedSvgAsset, SVG_SCHEMA_VERSION, SvgError, SvgLimits, SvgRaster};
pub use tiff::{
    TIFF_BACKEND_ID, TiffAlphaSample, TiffByteOrder, TiffChunkKind, TiffChunkLayout,
    TiffCompression, TiffContainer, TiffDataLocation, TiffDecodeError, TiffDecodeLimits,
    TiffDecodeMode, TiffDecodeReceipt, TiffDecodeRequest, TiffDecodeResult, TiffDecoder,
    TiffHeader, TiffMetadataInventory, TiffPage, TiffPhotometric, TiffPixelData, TiffPredictor,
    TiffSampleData, TiffSampleFormat, TiffStorageLayout,
};
