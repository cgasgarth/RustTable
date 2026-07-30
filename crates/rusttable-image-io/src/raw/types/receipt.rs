use super::{
    RawCameraIdentity, RawColorMatrix, RawCompressionEvidence, RawContainerKind, RawFrame,
    RawLevelPattern, RawMetadataReceipt, RawPreviewDescriptor, RawRect, RawSourceReceipt,
    RawVendorFamily,
};

#[derive(Debug, Clone, PartialEq)]
pub struct RawDecodeReceipt {
    pub backend: String,
    pub backend_format: String,
    pub container: RawContainerKind,
    pub family: RawVendorFamily,
    pub camera_profile_id: String,
    pub capability_manifest_sha256: [u8; 32],
    pub camera: RawCameraIdentity,
    pub compression: RawCompressionEvidence,
    pub bit_depth: u8,
    pub sensor_area: RawRect,
    pub active_area: RawRect,
    pub crop_area: RawRect,
    pub masked_areas: Vec<RawRect>,
    pub black_levels: RawLevelPattern,
    pub white_levels: Vec<f32>,
    pub white_balance: Vec<Option<f32>>,
    pub color_matrices: Vec<RawColorMatrix>,
    pub previews: Vec<RawPreviewDescriptor>,
    pub quirk_ids: Vec<String>,
    pub source: RawSourceReceipt,
    pub plane_count: u8,
    pub sample_count: u64,
    pub dng: Option<RawDngReceipt>,
    pub metadata: RawMetadataReceipt,
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
