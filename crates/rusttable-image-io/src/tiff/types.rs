use std::fmt;

use rusttable_image::{DecodeLimits, ImageDimensions, Orientation, Roi};

use crate::raw::{RawCancellationToken, RawSourceError};

/// Stable identity of the pure-Rust TIFF backend.
pub const TIFF_BACKEND_ID: &str = "tiff-0.11.3-pure-rust";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffContainer {
    Classic,
    BigTiff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffByteOrder {
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffSampleFormat {
    Unsigned,
    Signed,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffPhotometric {
    WhiteIsZero,
    BlackIsZero,
    Palette,
    Rgb,
    Cmyk,
    YCbCr,
    CieLab,
    IccLab,
    Cfa,
    LinearRaw,
}

impl TiffPhotometric {
    #[must_use]
    pub const fn color_samples(self) -> u16 {
        match self {
            Self::WhiteIsZero | Self::BlackIsZero | Self::Palette | Self::Cfa | Self::LinearRaw => {
                1
            }
            Self::Rgb | Self::YCbCr | Self::CieLab | Self::IccLab => 3,
            Self::Cmyk => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffCompression {
    None,
    PackBits,
    Lzw,
    Deflate,
    AdobeDeflate,
    Zstd,
    Jpeg,
    JpegXl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffPredictor {
    None,
    Horizontal,
    FloatingPoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffStorageLayout {
    Chunky,
    Planar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffAlphaSample {
    Unspecified,
    Premultiplied,
    Straight,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TiffChunkKind {
    Strips,
    Tiles,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiffChunkLayout {
    pub kind: TiffChunkKind,
    pub width: u32,
    pub height: u32,
    pub count: u32,
    pub compressed_bytes: u64,
    pub locations: Vec<TiffDataLocation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TiffDngMatrix {
    pub illuminant: u16,
    pub coefficients: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TiffDngMetadata {
    pub version: Option<[u8; 4]>,
    pub backward_version: Option<[u8; 4]>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub active_area: Option<[u32; 4]>,
    pub default_crop_origin: Option<[f32; 2]>,
    pub default_crop_size: Option<[f32; 2]>,
    pub masked_areas: Vec<[u32; 4]>,
    pub cfa_repeat: Option<(u8, u8)>,
    pub cfa_pattern: Option<Vec<u8>>,
    pub black_repeat: Option<(u8, u8)>,
    pub black_levels: Vec<f32>,
    pub white_levels: Vec<f32>,
    pub as_shot_neutral: Vec<f32>,
    pub matrices: Vec<TiffDngMatrix>,
    pub opcodes: Vec<(u16, Vec<u8>)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiffDataLocation {
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TiffMetadataInventory {
    pub icc: Option<TiffDataLocation>,
    pub exif_ifd: Option<u64>,
    pub gps_ifd: Option<u64>,
    pub xmp: Option<TiffDataLocation>,
    pub iptc: Option<TiffDataLocation>,
    pub photoshop: Option<TiffDataLocation>,
    pub metadata_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TiffPage {
    pub index: usize,
    pub ifd_offset: u64,
    pub parent_ifd_offset: Option<u64>,
    pub reduced_image: bool,
    pub dimensions: ImageDimensions,
    pub bits_per_sample: Vec<u8>,
    pub sample_formats: Vec<TiffSampleFormat>,
    pub samples_per_pixel: u16,
    pub photometric: TiffPhotometric,
    pub compression: TiffCompression,
    pub predictor: TiffPredictor,
    pub storage: TiffStorageLayout,
    pub orientation: Orientation,
    pub alpha: Vec<TiffAlphaSample>,
    pub chunks: TiffChunkLayout,
    pub dng: Option<TiffDngMetadata>,
    pub color_map: Option<Vec<u16>>,
    pub ycbcr_subsampling: Option<(u16, u16)>,
    pub metadata: TiffMetadataInventory,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TiffHeader {
    pub container: TiffContainer,
    pub byte_order: TiffByteOrder,
    pub pages: Vec<TiffPage>,
    pub default_page: usize,
}

impl TiffHeader {
    #[must_use]
    pub fn default_page(&self) -> &TiffPage {
        &self.pages[self.default_page]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TiffSampleData {
    U8(Vec<u8>),
    U16(Vec<u16>),
    U32(Vec<u32>),
    I8(Vec<i8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    /// IEEE binary16 bit patterns in native sample order.
    F16(Vec<u16>),
    F32(Vec<f32>),
}

impl TiffSampleData {
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::U8(values) => values.len(),
            Self::U16(values) | Self::F16(values) => values.len(),
            Self::U32(values) => values.len(),
            Self::I8(values) => values.len(),
            Self::I16(values) => values.len(),
            Self::I32(values) => values.len(),
            Self::F32(values) => values.len(),
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub const fn bytes_per_sample(&self) -> usize {
        match self {
            Self::U8(_) | Self::I8(_) => 1,
            Self::U16(_) | Self::I16(_) | Self::F16(_) => 2,
            Self::U32(_) | Self::I32(_) | Self::F32(_) => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TiffPixelData {
    pub dimensions: ImageDimensions,
    pub samples_per_pixel: u16,
    pub storage: TiffStorageLayout,
    pub samples: TiffSampleData,
}

impl TiffPixelData {
    #[must_use]
    pub fn sample_bytes(&self) -> u64 {
        u64::try_from(self.samples.len())
            .unwrap_or(u64::MAX)
            .saturating_mul(u64::try_from(self.samples.bytes_per_sample()).unwrap_or(u64::MAX))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TiffDecodeMode {
    Header,
    Full,
    Region(Roi),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TiffDecodeLimits {
    pub max_source_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_decoded_bytes: u64,
    pub max_decompressed_bytes: u64,
    pub max_temporary_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_ifd_value_bytes: u64,
    pub max_pages: u32,
    pub max_tags: u32,
    pub max_chunks: u32,
}

impl TiffDecodeLimits {
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_source_bytes: 4 * 1024 * 1024 * 1024,
            max_width: 65_535,
            max_height: 65_535,
            max_pixels: 250_000_000,
            max_decoded_bytes: 2_000_000_000,
            max_decompressed_bytes: 2_000_000_000,
            max_temporary_bytes: 256 * 1024 * 1024,
            max_metadata_bytes: 8 * 1024 * 1024,
            max_ifd_value_bytes: 8 * 1024 * 1024,
            max_pages: 1_024,
            max_tags: 100_000,
            max_chunks: 1_000_000,
        }
    }

    #[must_use]
    pub const fn from_common(common: DecodeLimits) -> Self {
        Self {
            max_source_bytes: common.max_source_bytes(),
            max_width: common.max_width(),
            max_height: common.max_height(),
            max_pixels: common.max_pixel_count(),
            max_decoded_bytes: common.max_decoded_bytes(),
            max_decompressed_bytes: common.max_decoded_bytes(),
            max_temporary_bytes: common.max_decoded_bytes(),
            max_metadata_bytes: 8 * 1024 * 1024,
            max_ifd_value_bytes: 8 * 1024 * 1024,
            max_pages: 1_024,
            max_tags: 100_000,
            max_chunks: 1_000_000,
        }
    }
}

impl Default for TiffDecodeLimits {
    fn default() -> Self {
        Self::standard()
    }
}

#[derive(Debug, Clone)]
pub struct TiffDecodeRequest {
    pub limits: TiffDecodeLimits,
    pub mode: TiffDecodeMode,
    pub page: Option<usize>,
    pub cancellation: RawCancellationToken,
}

impl TiffDecodeRequest {
    #[must_use]
    pub fn new(limits: TiffDecodeLimits) -> Self {
        Self {
            limits,
            mode: TiffDecodeMode::Full,
            page: None,
            cancellation: RawCancellationToken::new(),
        }
    }

    #[must_use]
    pub const fn header(mut self) -> Self {
        self.mode = TiffDecodeMode::Header;
        self
    }

    #[must_use]
    pub const fn region(mut self, roi: Roi) -> Self {
        self.mode = TiffDecodeMode::Region(roi);
        self
    }

    #[must_use]
    pub const fn page(mut self, page: usize) -> Self {
        self.page = Some(page);
        self
    }

    #[must_use]
    pub fn with_cancellation(mut self, cancellation: RawCancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TiffDecodeReceipt {
    pub backend: String,
    pub source_bytes: u64,
    pub source_sha256: [u8; 32],
    pub page_index: usize,
    pub ifd_offset: u64,
    pub region: Option<Roi>,
    pub output_bytes: u64,
    pub decompressed_bytes: u64,
    pub header_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TiffDecodeResult {
    pub header: TiffHeader,
    pub page: TiffPage,
    pub pixels: Option<TiffPixelData>,
    pub receipt: TiffDecodeReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TiffDecodeError {
    Source(RawSourceError),
    Malformed(String),
    Unsupported {
        feature: &'static str,
        value: u64,
    },
    Limit {
        kind: &'static str,
        actual: u64,
        limit: u64,
    },
    InvalidPage(usize),
    InvalidRegion,
    Cancelled,
    Backend(String),
    ArithmeticOverflow,
    AllocationFailure,
}

impl fmt::Display for TiffDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => write!(formatter, "TIFF source failed: {error:?}"),
            Self::Malformed(message) => write!(formatter, "malformed TIFF: {message}"),
            Self::Unsupported { feature, value } => {
                write!(formatter, "unsupported TIFF {feature}: {value}")
            }
            Self::Limit {
                kind,
                actual,
                limit,
            } => write!(formatter, "TIFF {kind} {actual} exceeds limit {limit}"),
            Self::InvalidPage(page) => write!(formatter, "TIFF page {page} does not exist"),
            Self::InvalidRegion => formatter.write_str("TIFF region is empty or out of bounds"),
            Self::Cancelled => formatter.write_str("TIFF decode cancelled"),
            Self::Backend(message) => write!(formatter, "TIFF backend failed: {message}"),
            Self::ArithmeticOverflow => formatter.write_str("TIFF arithmetic overflowed"),
            Self::AllocationFailure => formatter.write_str("TIFF allocation failed"),
        }
    }
}

impl std::error::Error for TiffDecodeError {}
