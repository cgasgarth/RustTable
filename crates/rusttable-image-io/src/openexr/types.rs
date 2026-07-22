use std::fmt;

use rusttable_image::{ChannelLayout, DecodeLimits, ImageDimensions, Roi};

use crate::raw::{RawCancellationToken, RawSourceError};

/// Stable identity of the pure-Rust `OpenEXR` backend.
pub const EXR_BACKEND_ID: &str = "exr-1.74.2-pure-rust";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExrSampleType {
    F16,
    F32,
    U32,
}

impl ExrSampleType {
    #[must_use]
    pub const fn bytes(self) -> u64 {
        match self {
            Self::F16 => 2,
            Self::F32 | Self::U32 => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExrCompression {
    None,
    Rle,
    Zips,
    Zip,
    Piz,
    Pxr24,
    B44,
    B44A,
    Dwaa,
    Dwab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExrLevelMode {
    Singular,
    MipMap,
    RipMap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExrStorage {
    ScanLines,
    Tiles {
        width: u32,
        height: u32,
        levels: ExrLevelMode,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExrWindow {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ExrWindow {
    /// Returns checked positive dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`ExrDecodeError::Malformed`] for an empty or unrepresentable window.
    pub fn dimensions(self) -> Result<ImageDimensions, ExrDecodeError> {
        ImageDimensions::new(self.width, self.height).map_err(|_| {
            ExrDecodeError::Malformed(
                "OpenEXR window has zero or unrepresentable dimensions".to_owned(),
            )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ExrLevelIndex {
    pub x: u32,
    pub y: u32,
}

impl ExrLevelIndex {
    #[must_use]
    pub const fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExrChannelRole {
    Red,
    Green,
    Blue,
    Alpha,
    Luminance,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExrChannel {
    pub name: String,
    pub layer: String,
    pub view: String,
    pub role: Option<ExrChannelRole>,
    pub sample_type: ExrSampleType,
    pub x_sampling: u32,
    pub y_sampling: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExrLayerView {
    pub layer: String,
    pub view: String,
    pub channels: Vec<String>,
    pub has_rgb: bool,
    pub has_luminance: bool,
    pub has_alpha: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExrChromaticities {
    pub red: [f32; 2],
    pub green: [f32; 2],
    pub blue: [f32; 2],
    pub white: [f32; 2],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExrBlobMetadata {
    pub attribute: String,
    pub type_name: String,
    pub bytes: u64,
    pub sha256: [u8; 32],
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExrMetadataInventory {
    pub chromaticities: Option<ExrChromaticities>,
    pub adopted_neutral: Option<[f32; 2]>,
    pub white_luminance: Option<f32>,
    pub icc: Option<ExrBlobMetadata>,
    pub xmp: Option<ExrBlobMetadata>,
    pub attribute_names: Vec<String>,
    pub attribute_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExrPart {
    pub index: usize,
    pub name: String,
    pub deep: bool,
    pub compression: ExrCompression,
    pub data_window: ExrWindow,
    pub display_window: ExrWindow,
    pub storage: ExrStorage,
    pub level_count: [u32; 2],
    pub chunk_count: u64,
    pub channels: Vec<ExrChannel>,
    pub layers: Vec<ExrLayerView>,
    pub views: Vec<String>,
    pub metadata: ExrMetadataInventory,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExrHeader {
    pub version: u8,
    pub flags: u32,
    pub parts: Vec<ExrPart>,
    pub default_part: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExrChannelMapping {
    Gray {
        gray: String,
        alpha: Option<String>,
    },
    Rgb {
        red: String,
        green: String,
        blue: String,
        alpha: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExrDecodeMode {
    Header,
    Full,
    Region(Roi),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExrDecodeLimits {
    pub max_source_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_decoded_bytes: u64,
    pub max_decompressed_bytes: u64,
    pub max_header_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_parts: u32,
    pub max_channels_per_part: u32,
    pub max_attributes_per_part: u32,
    pub max_levels_per_axis: u32,
    pub max_chunks: u64,
}

impl ExrDecodeLimits {
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_source_bytes: 4 * 1024 * 1024 * 1024,
            max_width: 65_535,
            max_height: 65_535,
            max_pixels: 250_000_000,
            max_decoded_bytes: 2_000_000_000,
            max_decompressed_bytes: 2_000_000_000,
            max_header_bytes: 16 * 1024 * 1024,
            max_metadata_bytes: 8 * 1024 * 1024,
            max_parts: 64,
            max_channels_per_part: 256,
            max_attributes_per_part: 4_096,
            max_levels_per_axis: 64,
            max_chunks: 1_000_000,
        }
    }

    #[must_use]
    pub const fn from_common(common: DecodeLimits) -> Self {
        let typed_bytes = common.max_decoded_bytes().saturating_mul(4);
        Self {
            max_source_bytes: common.max_source_bytes(),
            max_width: common.max_width(),
            max_height: common.max_height(),
            max_pixels: common.max_pixel_count(),
            max_decoded_bytes: typed_bytes,
            max_decompressed_bytes: typed_bytes.saturating_mul(16),
            max_header_bytes: 16 * 1024 * 1024,
            max_metadata_bytes: 8 * 1024 * 1024,
            max_parts: 64,
            max_channels_per_part: 256,
            max_attributes_per_part: 4_096,
            max_levels_per_axis: 64,
            max_chunks: 1_000_000,
        }
    }
}

impl Default for ExrDecodeLimits {
    fn default() -> Self {
        Self::standard()
    }
}

#[derive(Debug, Clone)]
pub struct ExrDecodeRequest {
    pub limits: ExrDecodeLimits,
    pub mode: ExrDecodeMode,
    pub part: Option<usize>,
    pub layer: Option<String>,
    pub view: Option<String>,
    pub channels: Option<ExrChannelMapping>,
    pub level: ExrLevelIndex,
    pub cancellation: RawCancellationToken,
}

impl ExrDecodeRequest {
    #[must_use]
    pub fn new(limits: ExrDecodeLimits) -> Self {
        Self {
            limits,
            mode: ExrDecodeMode::Full,
            part: None,
            layer: None,
            view: None,
            channels: None,
            level: ExrLevelIndex::default(),
            cancellation: RawCancellationToken::new(),
        }
    }

    #[must_use]
    pub const fn header(mut self) -> Self {
        self.mode = ExrDecodeMode::Header;
        self
    }

    #[must_use]
    pub const fn region(mut self, roi: Roi) -> Self {
        self.mode = ExrDecodeMode::Region(roi);
        self
    }

    #[must_use]
    pub const fn part(mut self, part: usize) -> Self {
        self.part = Some(part);
        self
    }

    #[must_use]
    pub fn layer(mut self, layer: impl Into<String>) -> Self {
        self.layer = Some(layer.into());
        self
    }

    #[must_use]
    pub fn view(mut self, view: impl Into<String>) -> Self {
        self.view = Some(view.into());
        self
    }

    #[must_use]
    pub fn channels(mut self, channels: ExrChannelMapping) -> Self {
        self.channels = Some(channels);
        self
    }

    #[must_use]
    pub const fn level(mut self, level: ExrLevelIndex) -> Self {
        self.level = level;
        self
    }

    #[must_use]
    pub fn with_cancellation(mut self, cancellation: RawCancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExrSampleData {
    /// IEEE binary16 bit patterns in interleaved channel order.
    F16(Vec<u16>),
    F32(Vec<f32>),
}

impl ExrSampleData {
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::F16(values) => values.len(),
            Self::F32(values) => values.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExrPixelData {
    pub dimensions: ImageDimensions,
    pub origin: [i32; 2],
    pub layout: ChannelLayout,
    pub sample_type: ExrSampleType,
    pub samples: ExrSampleData,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExrAlphaAssociation {
    None,
    Associated,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExrMissingChannelFill {
    pub color: f32,
    pub alpha: f32,
    pub outside_data: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExrDecodeReceipt {
    pub backend: String,
    pub source_bytes: u64,
    pub source_sha256: [u8; 32],
    pub version: u8,
    pub flags: u32,
    pub part_index: usize,
    pub part_name: String,
    pub layer: String,
    pub view: String,
    pub channels: Vec<String>,
    pub sample_type: ExrSampleType,
    pub compression: ExrCompression,
    pub data_window: ExrWindow,
    pub display_window: ExrWindow,
    pub storage: ExrStorage,
    pub level: ExrLevelIndex,
    pub region: Option<Roi>,
    pub output_origin: [i32; 2],
    pub output_size: [u32; 2],
    pub output_bytes: u64,
    pub decompressed_bytes: u64,
    pub alpha: ExrAlphaAssociation,
    pub fill: ExrMissingChannelFill,
    pub metadata: ExrMetadataInventory,
    pub header_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExrDecodeResult {
    pub header: ExrHeader,
    pub part: ExrPart,
    pub pixels: Option<ExrPixelData>,
    pub receipt: ExrDecodeReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExrDecodeError {
    Source(RawSourceError),
    NotOpenExr,
    UnsupportedVersion(u8),
    UnsupportedFlags(u32),
    UnsupportedDeepData {
        part: usize,
    },
    UnsupportedSampleType {
        channel: String,
    },
    UnsupportedCompression {
        part: usize,
        compression: String,
    },
    Limit {
        kind: &'static str,
        actual: u64,
        limit: u64,
    },
    Malformed(String),
    InvalidPart(usize),
    InvalidSelection(String),
    InvalidLevel(ExrLevelIndex),
    InvalidRegion,
    Cancelled,
    Backend(String),
    ArithmeticOverflow,
    AllocationFailure,
}

impl fmt::Display for ExrDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => write!(formatter, "OpenEXR source failed: {error:?}"),
            Self::NotOpenExr => formatter.write_str("OpenEXR magic number is missing"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported OpenEXR version {version}")
            }
            Self::UnsupportedFlags(flags) => {
                write!(formatter, "unsupported OpenEXR flags 0x{flags:08x}")
            }
            Self::UnsupportedDeepData { part } => write!(
                formatter,
                "OpenEXR part {part} contains unsupported deep data"
            ),
            Self::UnsupportedSampleType { channel } => write!(
                formatter,
                "OpenEXR channel {channel} has unsupported UINT samples"
            ),
            Self::UnsupportedCompression { part, compression } => write!(
                formatter,
                "OpenEXR part {part} uses unsupported compression {compression}"
            ),
            Self::Limit {
                kind,
                actual,
                limit,
            } => write!(formatter, "OpenEXR {kind} {actual} exceeds limit {limit}"),
            Self::Malformed(message) => write!(formatter, "malformed OpenEXR: {message}"),
            Self::InvalidPart(part) => write!(formatter, "OpenEXR part {part} does not exist"),
            Self::InvalidSelection(message) => {
                write!(formatter, "invalid OpenEXR channel selection: {message}")
            }
            Self::InvalidLevel(level) => write!(
                formatter,
                "OpenEXR level ({}, {}) does not exist",
                level.x, level.y
            ),
            Self::InvalidRegion => formatter.write_str("OpenEXR region is empty or out of bounds"),
            Self::Cancelled => formatter.write_str("OpenEXR decode cancelled"),
            Self::Backend(message) => write!(formatter, "OpenEXR backend failed: {message}"),
            Self::ArithmeticOverflow => formatter.write_str("OpenEXR arithmetic overflowed"),
            Self::AllocationFailure => formatter.write_str("OpenEXR allocation failed"),
        }
    }
}

impl std::error::Error for ExrDecodeError {}
