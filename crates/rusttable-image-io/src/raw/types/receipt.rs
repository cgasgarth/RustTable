use super::{
    RawCameraIdentity, RawCompressionEvidence, RawContainerKind, RawFrame, RawSourceReceipt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDecodeReceipt {
    pub backend: String,
    pub container: RawContainerKind,
    pub camera: RawCameraIdentity,
    pub compression: RawCompressionEvidence,
    pub bit_depth: u8,
    pub source: RawSourceReceipt,
    pub plane_count: u8,
    pub sample_count: u64,
    pub dng: Option<RawDngReceipt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawDngReceipt {
    pub version: Option<[u8; 4]>,
    pub backward_version: Option<[u8; 4]>,
    pub selected_ifd: u64,
    pub preview_count: u8,
    pub opcode_count: u8,
    pub output_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawDecodeResult {
    pub frame: RawFrame,
    pub receipt: RawDecodeReceipt,
}
