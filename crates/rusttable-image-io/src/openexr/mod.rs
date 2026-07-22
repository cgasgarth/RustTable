//! Bounded pure-Rust `OpenEXR` still-image decoding.

mod decode;
mod inspect;
mod selection;
mod types;

pub use decode::ExrDecoder;
pub use types::{
    EXR_BACKEND_ID, ExrAlphaAssociation, ExrBlobMetadata, ExrChannel, ExrChannelMapping,
    ExrChannelRole, ExrChromaticities, ExrCompression, ExrDecodeError, ExrDecodeLimits,
    ExrDecodeMode, ExrDecodeReceipt, ExrDecodeRequest, ExrDecodeResult, ExrHeader, ExrLayerView,
    ExrLevelIndex, ExrLevelMode, ExrMetadataInventory, ExrMissingChannelFill, ExrPart,
    ExrPixelData, ExrSampleData, ExrSampleType, ExrStorage, ExrWindow,
};

pub(crate) use decode::{decode_exr_probe, decode_legacy_rgba8, is_exr_signature};
