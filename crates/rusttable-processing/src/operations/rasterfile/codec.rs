use super::decoder::decode_asset;
use rusttable_masks::MaskRaster;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

pub const RASTERFILE_COMPATIBILITY_ID: &str = "rasterfile";
pub const RASTERFILE_RUST_ID: &str = "rusttable.rasterfile";
pub const RASTERFILE_SCHEMA_VERSION: u16 = 1;
pub const RASTERFILE_PARAMETER_VERSION: u16 = 1;
pub const RASTERFILE_IMPLEMENTATION_VERSION: u16 = 1;
pub const RASTERFILE_PARAMETER_BYTES: usize = 1028;
pub const RASTERFILE_DECODER_VERSION: u16 = 1;
pub const RASTERFILE_PATH_BYTES: usize = 512;

const MODE_OFFSET: usize = 0;
const PATH_OFFSET: usize = 4;
const FILE_OFFSET: usize = PATH_OFFSET + RASTERFILE_PATH_BYTES;

/// RGB channel-selection flags preserved from the legacy operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RasterFileChannelMode(u8);

impl RasterFileChannelMode {
    pub const RED: Self = Self(1);
    pub const GREEN: Self = Self(2);
    pub const BLUE: Self = Self(4);
    pub const RED_GREEN: Self = Self(3);
    pub const RED_BLUE: Self = Self(5);
    pub const GREEN_BLUE: Self = Self(6);
    pub const ALL: Self = Self(7);

    pub fn new(bits: u8) -> Result<Self, RasterFileCodecError> {
        if (1..=7).contains(&bits) {
            Ok(Self(bits))
        } else {
            Err(RasterFileCodecError::InvalidChannelMode(bits))
        }
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn selects(self, channel: usize) -> bool {
        self.0 & (1 << channel) != 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RasterFileParametersV1 {
    mode: RasterFileChannelMode,
    path: [u8; RASTERFILE_PATH_BYTES],
    file: [u8; RASTERFILE_PATH_BYTES],
}

impl RasterFileParametersV1 {
    #[must_use]
    pub fn defaults() -> Self {
        Self {
            mode: RasterFileChannelMode::ALL,
            path: [0; RASTERFILE_PATH_BYTES],
            file: [0; RASTERFILE_PATH_BYTES],
        }
    }

    pub fn new(
        mode: RasterFileChannelMode,
        path: impl AsRef<[u8]>,
        file: impl AsRef<[u8]>,
    ) -> Result<Self, RasterFileCodecError> {
        Ok(Self {
            mode,
            path: bounded_buffer(path.as_ref())?,
            file: bounded_buffer(file.as_ref())?,
        })
    }

    #[must_use]
    pub const fn mode(&self) -> RasterFileChannelMode {
        self.mode
    }

    #[must_use]
    pub fn path(&self) -> &[u8] {
        trim_buffer(&self.path)
    }

    #[must_use]
    pub fn file(&self) -> &[u8] {
        trim_buffer(&self.file)
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; RASTERFILE_PARAMETER_BYTES] {
        let mut bytes = [0; RASTERFILE_PARAMETER_BYTES];
        bytes[MODE_OFFSET..PATH_OFFSET].copy_from_slice(&u32::from(self.mode.bits()).to_le_bytes());
        bytes[PATH_OFFSET..FILE_OFFSET].copy_from_slice(&self.path);
        bytes[FILE_OFFSET..].copy_from_slice(&self.file);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RasterFileCodecError> {
        if bytes.len() != RASTERFILE_PARAMETER_BYTES {
            return Err(RasterFileCodecError::InvalidLength {
                expected: RASTERFILE_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mode = u32::from_le_bytes(bytes[MODE_OFFSET..PATH_OFFSET].try_into().expect("mode"));
        if mode > u32::from(u8::MAX) {
            return Err(RasterFileCodecError::InvalidChannelMode(u8::MAX));
        }
        let mode = RasterFileChannelMode::new(u8::try_from(mode).expect("bounded mode"))?;
        let mut path = [0; RASTERFILE_PATH_BYTES];
        path.copy_from_slice(&bytes[PATH_OFFSET..FILE_OFFSET]);
        let mut file = [0; RASTERFILE_PATH_BYTES];
        file.copy_from_slice(&bytes[FILE_OFFSET..]);
        Ok(Self { mode, path, file })
    }

    #[must_use]
    pub fn source_name(&self) -> &[u8] {
        if self.path().is_empty() {
            self.file()
        } else {
            self.path()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasterFileHistory {
    V1(Box<RasterFileParametersV1>),
    Opaque { version: u16, bytes: Vec<u8> },
}

pub fn decode_history(
    version: u16,
    bytes: &[u8],
) -> Result<RasterFileHistory, RasterFileCodecError> {
    if version == RASTERFILE_PARAMETER_VERSION {
        Ok(RasterFileHistory::V1(Box::new(
            RasterFileParametersV1::from_bytes(bytes)?,
        )))
    } else {
        Ok(RasterFileHistory::Opaque {
            version,
            bytes: bytes.to_vec(),
        })
    }
}

pub fn migrate_history(
    history: RasterFileHistory,
) -> Result<RasterFileParametersV1, RasterFileCodecError> {
    match history {
        RasterFileHistory::V1(parameters) => Ok(*parameters),
        RasterFileHistory::Opaque { version, .. } => {
            Err(RasterFileCodecError::OpaqueSource(version))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterMaskFormat {
    Png,
    Pfm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RasterMaskLimits {
    pub max_source_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixel_count: u64,
    pub max_mask_bytes: u64,
    pub max_metadata_bytes: usize,
}

impl Default for RasterMaskLimits {
    fn default() -> Self {
        Self {
            max_source_bytes: 64 * 1024 * 1024,
            max_width: 16_384,
            max_height: 16_384,
            max_pixel_count: 16_384 * 16_384,
            max_mask_bytes: 256 * 1024 * 1024,
            max_metadata_bytes: RASTERFILE_PATH_BYTES,
        }
    }
}

impl RasterMaskLimits {
    pub fn validate(
        &self,
        source_bytes: usize,
        width: u32,
        height: u32,
    ) -> Result<(), RasterMaskAssetError> {
        let source_bytes =
            u64::try_from(source_bytes).map_err(|_| RasterMaskAssetError::ArithmeticOverflow)?;
        if source_bytes > self.max_source_bytes {
            return Err(RasterMaskAssetError::SourceTooLarge {
                actual: source_bytes,
                limit: self.max_source_bytes,
            });
        }
        if width == 0 || height == 0 || width > self.max_width || height > self.max_height {
            return Err(RasterMaskAssetError::DimensionLimit { width, height });
        }
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or(RasterMaskAssetError::ArithmeticOverflow)?;
        if pixels > self.max_pixel_count {
            return Err(RasterMaskAssetError::PixelLimit {
                actual: pixels,
                limit: self.max_pixel_count,
            });
        }
        let mask_bytes = pixels
            .checked_mul(4)
            .ok_or(RasterMaskAssetError::ArithmeticOverflow)?;
        if mask_bytes > self.max_mask_bytes {
            return Err(RasterMaskAssetError::MaskTooLarge {
                actual: mask_bytes,
                limit: self.max_mask_bytes,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RasterMaskAsset {
    original_bytes: Vec<u8>,
    original_byte_hash: [u8; 32],
    format: RasterMaskFormat,
    width: u32,
    height: u32,
    mode: RasterFileChannelMode,
    mask: MaskRaster,
    decoded_mask_hash: [u8; 32],
    identity: [u8; 32],
}

impl RasterMaskAsset {
    pub fn decode(
        bytes: impl Into<Vec<u8>>,
        mode: RasterFileChannelMode,
        limits: RasterMaskLimits,
    ) -> Result<Self, RasterMaskAssetError> {
        let bytes = bytes.into();
        let decoded = decode_asset(&bytes, mode, limits)?;
        let original_byte_hash: [u8; 32] = Sha256::digest(&bytes).into();
        let decoded_mask_hash = decoded.mask.identity();
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.rasterfile.asset.v1");
        hasher.update(original_byte_hash);
        hasher.update(decoded_mask_hash);
        hasher.update([match decoded.format {
            RasterMaskFormat::Png => 1,
            RasterMaskFormat::Pfm => 2,
        }]);
        hasher.update(decoded.width.to_le_bytes());
        hasher.update(decoded.height.to_le_bytes());
        hasher.update([mode.bits()]);
        hasher.update(RASTERFILE_DECODER_VERSION.to_le_bytes());
        Ok(Self {
            original_bytes: bytes,
            original_byte_hash,
            format: decoded.format,
            width: decoded.width,
            height: decoded.height,
            mode,
            mask: decoded.mask,
            decoded_mask_hash,
            identity: hasher.finalize().into(),
        })
    }

    pub fn from_path(
        path: impl AsRef<std::path::Path>,
        mode: RasterFileChannelMode,
        limits: RasterMaskLimits,
    ) -> Result<Self, RasterMaskAssetError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|error| RasterMaskAssetError::Io {
            basename: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("source")
                .to_owned(),
            message: error.kind(),
        })?;
        Self::decode(bytes, mode, limits)
    }

    #[must_use]
    pub fn original_bytes(&self) -> &[u8] {
        &self.original_bytes
    }
    #[must_use]
    pub const fn original_byte_hash(&self) -> [u8; 32] {
        self.original_byte_hash
    }
    #[must_use]
    pub const fn format(&self) -> RasterMaskFormat {
        self.format
    }
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    #[must_use]
    pub const fn mode(&self) -> RasterFileChannelMode {
        self.mode
    }
    #[must_use]
    pub const fn mask(&self) -> &MaskRaster {
        &self.mask
    }
    #[must_use]
    pub const fn decoded_mask_hash(&self) -> [u8; 32] {
        self.decoded_mask_hash
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    #[must_use]
    pub fn memory_bytes(&self) -> usize {
        self.original_bytes
            .len()
            .saturating_add(self.mask.memory_bytes())
    }
}

#[derive(Debug, Clone)]
pub struct RasterMaskCache {
    budget: usize,
    resident: usize,
    entries: BTreeMap<[u8; 32], RasterMaskAsset>,
}

impl RasterMaskCache {
    #[must_use]
    pub fn new(budget: usize) -> Self {
        Self {
            budget,
            resident: 0,
            entries: BTreeMap::new(),
        }
    }
    #[must_use]
    pub const fn resident_bytes(&self) -> usize {
        self.resident
    }
    #[must_use]
    pub fn get(&self, identity: &[u8; 32]) -> Option<&RasterMaskAsset> {
        self.entries.get(identity)
    }
    pub fn insert(&mut self, asset: RasterMaskAsset) -> Result<(), RasterMaskAssetError> {
        let bytes = asset.memory_bytes();
        if bytes > self.budget {
            return Err(RasterMaskAssetError::CacheBudget {
                required: bytes,
                budget: self.budget,
            });
        }
        if let Some(previous) = self.entries.remove(&asset.identity()) {
            self.resident = self.resident.saturating_sub(previous.memory_bytes());
        }
        while self.resident.saturating_add(bytes) > self.budget {
            let Some(identity) = self.entries.keys().next().copied() else {
                break;
            };
            if let Some(previous) = self.entries.remove(&identity) {
                self.resident = self.resident.saturating_sub(previous.memory_bytes());
            }
        }
        self.resident = self
            .resident
            .checked_add(bytes)
            .ok_or(RasterMaskAssetError::ArithmeticOverflow)?;
        self.entries.insert(asset.identity(), asset);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasterFileCodecError {
    InvalidLength { expected: usize, actual: usize },
    InvalidChannelMode(u8),
    PathTooLong { actual: usize, limit: usize },
    OpaqueSource(u16),
}

impl fmt::Display for RasterFileCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "rasterfile payload is {actual} bytes, expected {expected}"
            ),
            Self::InvalidChannelMode(mode) => {
                write!(formatter, "rasterfile channel mode {mode} is invalid")
            }
            Self::PathTooLong { actual, limit } => write!(
                formatter,
                "rasterfile source metadata is {actual} bytes, limit is {limit}"
            ),
            Self::OpaqueSource(version) => write!(
                formatter,
                "rasterfile history version {version} is opaque and blocking"
            ),
        }
    }
}
impl std::error::Error for RasterFileCodecError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasterMaskAssetError {
    ArithmeticOverflow,
    SourceTooLarge {
        actual: u64,
        limit: u64,
    },
    DimensionLimit {
        width: u32,
        height: u32,
    },
    PixelLimit {
        actual: u64,
        limit: u64,
    },
    MaskTooLarge {
        actual: u64,
        limit: u64,
    },
    UnsupportedFormat,
    Malformed {
        format: Option<RasterMaskFormat>,
        reason: String,
    },
    UnsupportedChannels,
    UnsupportedBitDepth,
    NonFiniteSample,
    DimensionMismatch {
        input: (u32, u32),
        asset: (u32, u32),
    },
    Io {
        basename: String,
        message: std::io::ErrorKind,
    },
    CacheBudget {
        required: usize,
        budget: usize,
    },
}

impl fmt::Display for RasterMaskAssetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => formatter.write_str("rasterfile arithmetic overflowed"),
            Self::SourceTooLarge { actual, limit } => write!(
                formatter,
                "rasterfile source is {actual} bytes, limit is {limit}"
            ),
            Self::DimensionLimit { width, height } => write!(
                formatter,
                "rasterfile dimensions {width}x{height} exceed limits"
            ),
            Self::PixelLimit { actual, limit } => write!(
                formatter,
                "rasterfile has {actual} pixels, limit is {limit}"
            ),
            Self::MaskTooLarge { actual, limit } => write!(
                formatter,
                "rasterfile mask is {actual} bytes, limit is {limit}"
            ),
            Self::UnsupportedFormat => {
                formatter.write_str("rasterfile format is unsupported; expected PNG or PFM")
            }
            Self::Malformed { format, reason } => {
                write!(formatter, "malformed {format:?} rasterfile: {reason}")
            }
            Self::UnsupportedChannels => {
                formatter.write_str("rasterfile must contain exactly RGB channels")
            }
            Self::UnsupportedBitDepth => formatter.write_str("rasterfile bit depth is unsupported"),
            Self::NonFiniteSample => formatter.write_str("rasterfile contains a non-finite sample"),
            Self::DimensionMismatch { input, asset } => write!(
                formatter,
                "rasterfile dimensions {asset:?} differ from input {input:?}"
            ),
            Self::Io { basename, message } => write!(
                formatter,
                "rasterfile I/O failed for {basename}: {message:?}"
            ),
            Self::CacheBudget { required, budget } => write!(
                formatter,
                "rasterfile cache requires {required} bytes, budget is {budget}"
            ),
        }
    }
}
impl std::error::Error for RasterMaskAssetError {}

fn bounded_buffer(source: &[u8]) -> Result<[u8; RASTERFILE_PATH_BYTES], RasterFileCodecError> {
    if source.len() >= RASTERFILE_PATH_BYTES {
        return Err(RasterFileCodecError::PathTooLong {
            actual: source.len(),
            limit: RASTERFILE_PATH_BYTES - 1,
        });
    }
    let mut output = [0; RASTERFILE_PATH_BYTES];
    output[..source.len()].copy_from_slice(source);
    Ok(output)
}

fn trim_buffer(buffer: &[u8; RASTERFILE_PATH_BYTES]) -> &[u8] {
    let end = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    &buffer[..end]
}
