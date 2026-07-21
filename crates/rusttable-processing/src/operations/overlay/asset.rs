use rusttable_image::{DecodeLimits, ImageInput};
use rusttable_image_io::FileImageInput;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverlayDecodedFormat {
    Rgba8Straight,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayAsset {
    original_bytes: Vec<u8>,
    pixels: Vec<u8>,
    width: u32,
    height: u32,
    format: OverlayDecodedFormat,
    identity: [u8; 32],
    decoded_hash: [u8; 32],
    original_hash: [u8; 32],
    profile_hash: [u8; 32],
    profile_present: bool,
}
impl OverlayAsset {
    pub fn decode(
        bytes: impl Into<Vec<u8>>,
        limits: DecodeLimits,
    ) -> Result<Self, OverlayAssetError> {
        let original_bytes = bytes.into();
        let original_hash = Sha256::digest(&original_bytes).into();
        let input = FileImageInput::new(limits);
        let decoded = input
            .decode_bytes(&original_bytes)
            .map_err(|e| OverlayAssetError::Decode(e.to_string()))?;
        let dimensions = decoded.dimensions();
        let pixels = decoded.pixels().to_vec();
        let decoded_hash = Sha256::digest(&pixels).into();
        let profile_present = decoded.descriptor().profile().is_some();
        let profile_hash =
            Sha256::digest(format!("{:?}", decoded.descriptor().profile()).as_bytes()).into();
        let mut h = Sha256::new();
        h.update(b"rusttable.overlay.asset.v1");
        h.update(decoded_hash);
        h.update(original_hash);
        h.update(dimensions.width().to_le_bytes());
        h.update(dimensions.height().to_le_bytes());
        h.update(profile_hash);
        let identity = h.finalize().into();
        Ok(Self {
            original_bytes,
            pixels,
            width: dimensions.width(),
            height: dimensions.height(),
            format: OverlayDecodedFormat::Rgba8Straight,
            identity,
            decoded_hash,
            original_hash,
            profile_hash,
            profile_present,
        })
    }
    pub fn from_rgba8(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, OverlayAssetError> {
        if width == 0
            || height == 0
            || pixels.len()
                != usize::try_from(u64::from(width) * u64::from(height) * 4)
                    .map_err(|_| OverlayAssetError::ArithmeticOverflow)?
        {
            return Err(OverlayAssetError::InvalidPixels);
        }
        let decoded_hash = Sha256::digest(&pixels).into();
        let mut h = Sha256::new();
        h.update(b"rusttable.overlay.asset.synthetic.v1");
        h.update(decoded_hash);
        h.update(width.to_le_bytes());
        h.update(height.to_le_bytes());
        let identity = h.finalize().into();
        Ok(Self {
            original_bytes: pixels.clone(),
            pixels,
            width,
            height,
            format: OverlayDecodedFormat::Rgba8Straight,
            identity,
            decoded_hash,
            original_hash: decoded_hash,
            profile_hash: [0; 32],
            profile_present: false,
        })
    }
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
    pub const fn width(&self) -> u32 {
        self.width
    }
    pub const fn height(&self) -> u32 {
        self.height
    }
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    pub const fn decoded_hash(&self) -> [u8; 32] {
        self.decoded_hash
    }
    pub const fn original_hash(&self) -> [u8; 32] {
        self.original_hash
    }
    pub const fn profile_hash(&self) -> [u8; 32] {
        self.profile_hash
    }
    pub const fn profile_present(&self) -> bool {
        self.profile_present
    }
    pub const fn format(&self) -> OverlayDecodedFormat {
        self.format
    }
    pub fn original_bytes(&self) -> &[u8] {
        &self.original_bytes
    }
    pub fn memory_bytes(&self) -> usize {
        self.pixels.len()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OverlayAssetError {
    Decode(String),
    InvalidPixels,
    ArithmeticOverflow,
    LimitExceeded,
}
impl fmt::Display for OverlayAssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "overlay asset error: {self:?}")
    }
}
impl std::error::Error for OverlayAssetError {}
#[derive(Debug, Clone)]
pub struct OverlayAssetStore {
    assets: BTreeMap<[u8; 32], OverlayAsset>,
    bytes: usize,
    budget: usize,
}
impl OverlayAssetStore {
    pub fn new(budget: usize) -> Self {
        Self {
            assets: BTreeMap::new(),
            bytes: 0,
            budget,
        }
    }
    pub fn insert(&mut self, asset: OverlayAsset) -> Result<[u8; 32], OverlayAssetError> {
        let id = asset.identity();
        if self.assets.contains_key(&id) {
            return Ok(id);
        }
        if asset.memory_bytes() > self.budget
            || self
                .bytes
                .checked_add(asset.memory_bytes())
                .is_none_or(|v| v > self.budget)
        {
            return Err(OverlayAssetError::LimitExceeded);
        }
        self.bytes += asset.memory_bytes();
        self.assets.insert(id, asset);
        Ok(id)
    }
    pub fn get(&self, id: &[u8; 32]) -> Option<&OverlayAsset> {
        self.assets.get(id)
    }
    pub const fn bytes(&self) -> usize {
        self.bytes
    }
}
