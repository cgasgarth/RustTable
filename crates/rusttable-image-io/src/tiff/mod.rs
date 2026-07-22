//! Strict TIFF and `BigTIFF` decoding with Darktable `src/imageio/imageio_tiff.c` lineage.

mod decode;
mod parser;
mod types;

pub use decode::TiffDecoder;
pub use types::{
    TIFF_BACKEND_ID, TiffAlphaSample, TiffByteOrder, TiffChunkKind, TiffChunkLayout,
    TiffCompression, TiffContainer, TiffDataLocation, TiffDecodeError, TiffDecodeLimits,
    TiffDecodeMode, TiffDecodeReceipt, TiffDecodeRequest, TiffDecodeResult, TiffHeader,
    TiffMetadataInventory, TiffPage, TiffPhotometric, TiffPixelData, TiffPredictor, TiffSampleData,
    TiffSampleFormat, TiffStorageLayout,
};

pub(crate) use decode::{decode_legacy_rgba8, decode_tiff_probe, is_tiff_signature};
