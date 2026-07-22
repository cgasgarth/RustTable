#![allow(clippy::all, clippy::pedantic)]

use std::fmt;

use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, DecodeLimits, ImageDescriptor, ImageDimensions,
    PixelFormat, SampleType, StorageLayout,
};

/// Stable identity of the pure-Rust PNG backend.
pub const PNG_BACKEND_ID: &str = "png-0.18.1-pure-rust";

/// PNG sample depth preserved by the decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PngBitDepth {
    One,
    Two,
    Four,
    Eight,
    Sixteen,
}

impl PngBitDepth {
    #[must_use]
    pub const fn bits(self) -> u8 {
        match self {
            Self::One => 1,
            Self::Two => 2,
            Self::Four => 4,
            Self::Eight => 8,
            Self::Sixteen => 16,
        }
    }
}

/// PNG source color model. Indexed sources are expanded in [`PngPixelData`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PngColorType {
    Grayscale,
    GrayscaleAlpha,
    Indexed,
    Rgb,
    Rgba,
}

impl PngColorType {
    #[must_use]
    pub const fn channels(self) -> u8 {
        match self {
            Self::Grayscale | Self::Indexed => 1,
            Self::GrayscaleAlpha => 2,
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }
}

/// Typed channel arrangement after PNG palette and transparency expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PngSampleLayout {
    Gray,
    GrayA,
    Rgb,
    Rgba,
}

impl PngSampleLayout {
    #[must_use]
    pub const fn channels(self) -> usize {
        match self {
            Self::Gray => 1,
            Self::GrayA => 2,
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }

    #[must_use]
    const fn channel_layout(self) -> ChannelLayout {
        match self {
            Self::Gray => ChannelLayout::Gray,
            Self::GrayA => ChannelLayout::GrayA,
            Self::Rgb => ChannelLayout::Rgb,
            Self::Rgba => ChannelLayout::Rgba,
        }
    }
}

/// Decoder mode. Region decoding is deliberately rejected by PNG policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngDecodeMode {
    Header,
    Thumbnail,
    Full,
    Region {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

/// Additional limits applied before handing compressed data to the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngDecodeLimits {
    pub max_source_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_decoded_bytes: u64,
    pub max_chunk_bytes: u64,
    pub max_chunks: u32,
    pub max_compressed_bytes: u64,
    pub max_decompressed_bytes: u64,
    pub max_metadata_bytes: u64,
}

impl PngDecodeLimits {
    /// Creates checked PNG limits from the common image limits.
    pub fn new(
        max_source_bytes: u64,
        max_width: u32,
        max_height: u32,
        max_pixels: u64,
        max_decoded_bytes: u64,
    ) -> Result<Self, rusttable_image::DecodeLimitsError> {
        let common = DecodeLimits::new(
            max_source_bytes,
            max_width,
            max_height,
            max_pixels,
            max_decoded_bytes,
        )?;
        Ok(Self::from_common(common))
    }

    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_source_bytes: 4 * 1024 * 1024 * 1024,
            max_width: 65_535,
            max_height: 65_535,
            max_pixels: 250_000_000,
            max_decoded_bytes: 2_000_000_000,
            max_chunk_bytes: 64 * 1024 * 1024,
            max_chunks: 100_000,
            max_compressed_bytes: 4 * 1024 * 1024 * 1024,
            max_decompressed_bytes: 2_000_000_000,
            max_metadata_bytes: 8 * 1024 * 1024,
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
            max_chunk_bytes: common.max_source_bytes(),
            max_chunks: 100_000,
            max_compressed_bytes: common.max_source_bytes(),
            max_decompressed_bytes: common.max_decoded_bytes().saturating_mul(2),
            max_metadata_bytes: 8 * 1024 * 1024,
        }
    }
}

impl Default for PngDecodeLimits {
    fn default() -> Self {
        Self::standard()
    }
}

/// A cancellation-aware PNG request.
#[derive(Debug, Clone)]
pub struct PngDecodeRequest {
    pub limits: PngDecodeLimits,
    pub mode: PngDecodeMode,
    pub cancellation: crate::raw::RawCancellationToken,
}

impl PngDecodeRequest {
    #[must_use]
    pub fn new(limits: PngDecodeLimits) -> Self {
        Self {
            limits,
            mode: PngDecodeMode::Full,
            cancellation: crate::raw::RawCancellationToken::new(),
        }
    }

    #[must_use]
    pub const fn header(mut self) -> Self {
        self.mode = PngDecodeMode::Header;
        self
    }

    #[must_use]
    pub const fn thumbnail(mut self) -> Self {
        self.mode = PngDecodeMode::Thumbnail;
        self
    }

    #[must_use]
    pub const fn region(mut self, x: u32, y: u32, width: u32, height: u32) -> Self {
        self.mode = PngDecodeMode::Region {
            x,
            y,
            width,
            height,
        };
        self
    }

    #[must_use]
    pub fn with_cancellation(mut self, cancellation: crate::raw::RawCancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

/// One deterministic chunk inventory entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngChunk {
    pub kind: [u8; 4],
    pub length: u32,
}

/// Deterministic chunk inventory, including CRC-validated source order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PngChunkInventory {
    pub chunks: Vec<PngChunk>,
    pub compressed_data_bytes: u64,
    pub decompressed_data_bytes: u64,
}

/// A bounded text inventory that never stores private text values in logs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngTextInventory {
    pub kind: [u8; 4],
    pub keyword: String,
    pub compressed: bool,
    pub bytes: u64,
    pub sha256: [u8; 32],
}

/// Physical pixel resolution from pHYs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngPhysicalResolution {
    pub x_pixels_per_unit: u32,
    pub y_pixels_per_unit: u32,
    pub unit_is_meter: bool,
}

/// ICC profile inventory; bytes are content-addressed and never interpreted for conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngProfileInventory {
    pub bytes: u64,
    pub sha256: [u8; 32],
    pub profile_id: rusttable_color::ProfileId,
    pub data: Vec<u8>,
}

/// Non-pixel PNG metadata inventory.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PngMetadataInventory {
    pub exif_bytes: u64,
    pub exif_sha256: Option<[u8; 32]>,
    pub xmp_chunks: u32,
    pub icc_profile: Option<PngProfileInventory>,
    pub srgb_intent: Option<u8>,
    pub gamma: Option<u32>,
    pub chromaticities: Option<[u32; 8]>,
    pub physical_resolution: Option<PngPhysicalResolution>,
    pub text: Vec<PngTextInventory>,
}

/// APNG declaration and whether a decodable default image is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngAnimation {
    pub frame_count: u32,
    pub play_count: u32,
    pub has_default_image: bool,
}

/// Validated PNG header and source inventories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngHeader {
    pub dimensions: ImageDimensions,
    pub color_type: PngColorType,
    pub bit_depth: PngBitDepth,
    pub interlaced: bool,
    pub has_palette: bool,
    pub has_transparency: bool,
    pub chunks: PngChunkInventory,
    pub metadata: PngMetadataInventory,
    pub animation: Option<PngAnimation>,
    pub color_encoding: rusttable_image::ColorEncoding,
}

/// A lossless typed PNG pixel buffer.  Sixteen-bit values are native-endian `u16` samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngPixelData {
    GrayU8 {
        dimensions: ImageDimensions,
        samples: Vec<u8>,
    },
    GrayAU8 {
        dimensions: ImageDimensions,
        samples: Vec<u8>,
    },
    RgbU8 {
        dimensions: ImageDimensions,
        samples: Vec<u8>,
    },
    RgbaU8 {
        dimensions: ImageDimensions,
        samples: Vec<u8>,
    },
    GrayU16 {
        dimensions: ImageDimensions,
        samples: Vec<u16>,
    },
    GrayAU16 {
        dimensions: ImageDimensions,
        samples: Vec<u16>,
    },
    RgbU16 {
        dimensions: ImageDimensions,
        samples: Vec<u16>,
    },
    RgbaU16 {
        dimensions: ImageDimensions,
        samples: Vec<u16>,
    },
}

impl PngPixelData {
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        match self {
            Self::GrayU8 { dimensions, .. }
            | Self::GrayAU8 { dimensions, .. }
            | Self::RgbU8 { dimensions, .. }
            | Self::RgbaU8 { dimensions, .. }
            | Self::GrayU16 { dimensions, .. }
            | Self::GrayAU16 { dimensions, .. }
            | Self::RgbU16 { dimensions, .. }
            | Self::RgbaU16 { dimensions, .. } => *dimensions,
        }
    }

    #[must_use]
    pub const fn layout(&self) -> PngSampleLayout {
        match self {
            Self::GrayU8 { .. } | Self::GrayU16 { .. } => PngSampleLayout::Gray,
            Self::GrayAU8 { .. } | Self::GrayAU16 { .. } => PngSampleLayout::GrayA,
            Self::RgbU8 { .. } | Self::RgbU16 { .. } => PngSampleLayout::Rgb,
            Self::RgbaU8 { .. } | Self::RgbaU16 { .. } => PngSampleLayout::Rgba,
        }
    }

    #[must_use]
    pub const fn sample_type(&self) -> SampleType {
        match self {
            Self::GrayU8 { .. }
            | Self::GrayAU8 { .. }
            | Self::RgbU8 { .. }
            | Self::RgbaU8 { .. } => SampleType::U8,
            Self::GrayU16 { .. }
            | Self::GrayAU16 { .. }
            | Self::RgbU16 { .. }
            | Self::RgbaU16 { .. } => SampleType::U16,
        }
    }

    #[must_use]
    pub fn format(&self) -> PixelFormat {
        let alpha = match self.layout() {
            PngSampleLayout::Gray | PngSampleLayout::Rgb => AlphaMode::None,
            PngSampleLayout::GrayA | PngSampleLayout::Rgba => AlphaMode::Straight,
        };
        match PixelFormat::new(
            self.sample_type(),
            self.layout().channel_layout(),
            alpha,
            ByteOrder::Native,
            StorageLayout::Interleaved,
        ) {
            Ok(format) => format,
            Err(_) => unreachable!("validated PNG channel format"),
        }
    }

    #[must_use]
    pub fn sample_count(&self) -> usize {
        match self {
            Self::GrayU8 { samples, .. }
            | Self::GrayAU8 { samples, .. }
            | Self::RgbU8 { samples, .. }
            | Self::RgbaU8 { samples, .. } => samples.len(),
            Self::GrayU16 { samples, .. }
            | Self::GrayAU16 { samples, .. }
            | Self::RgbU16 { samples, .. }
            | Self::RgbaU16 { samples, .. } => samples.len(),
        }
    }

    #[must_use]
    pub fn sample_bytes(&self) -> Vec<u8> {
        match self {
            Self::GrayU8 { samples, .. }
            | Self::GrayAU8 { samples, .. }
            | Self::RgbU8 { samples, .. }
            | Self::RgbaU8 { samples, .. } => samples.clone(),
            Self::GrayU16 { samples, .. }
            | Self::GrayAU16 { samples, .. }
            | Self::RgbU16 { samples, .. }
            | Self::RgbaU16 { samples, .. } => samples
                .iter()
                .flat_map(|sample| sample.to_ne_bytes())
                .collect(),
        }
    }

    /// Converts the lossless typed buffer to the legacy checked RGBA8 facade.
    #[must_use]
    pub fn to_rgba8(&self) -> Vec<u8> {
        let mut output =
            Vec::with_capacity(self.dimensions().pixel_count().unwrap_or(0) as usize * 4);
        match self {
            Self::GrayU8 { samples, .. } => {
                for &gray in samples {
                    output.extend_from_slice(&[gray, gray, gray, 255]);
                }
            }
            Self::GrayAU8 { samples, .. } => {
                for pixel in samples.chunks_exact(2) {
                    output.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
                }
            }
            Self::RgbU8 { samples, .. } => {
                for pixel in samples.chunks_exact(3) {
                    output.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
            }
            Self::RgbaU8 { samples, .. } => output.extend_from_slice(samples),
            Self::GrayU16 { samples, .. } => {
                for &gray in samples {
                    let value = (gray >> 8) as u8;
                    output.extend_from_slice(&[value, value, value, 255]);
                }
            }
            Self::GrayAU16 { samples, .. } => {
                for pixel in samples.chunks_exact(2) {
                    output.extend_from_slice(&[
                        (pixel[0] >> 8) as u8,
                        (pixel[0] >> 8) as u8,
                        (pixel[0] >> 8) as u8,
                        (pixel[1] >> 8) as u8,
                    ]);
                }
            }
            Self::RgbU16 { samples, .. } => {
                for pixel in samples.chunks_exact(3) {
                    output.extend_from_slice(&[
                        (pixel[0] >> 8) as u8,
                        (pixel[1] >> 8) as u8,
                        (pixel[2] >> 8) as u8,
                        255,
                    ]);
                }
            }
            Self::RgbaU16 { samples, .. } => {
                for pixel in samples.chunks_exact(4) {
                    output.extend_from_slice(&[
                        (pixel[0] >> 8) as u8,
                        (pixel[1] >> 8) as u8,
                        (pixel[2] >> 8) as u8,
                        (pixel[3] >> 8) as u8,
                    ]);
                }
            }
        }
        output
    }

    pub(crate) fn to_owned_image(
        &self,
        encoding: rusttable_image::ColorEncoding,
    ) -> Result<rusttable_image::OwnedImage, PngDecodeError> {
        let descriptor = ImageDescriptor::new(
            self.dimensions(),
            self.format(),
            encoding,
            rusttable_image::Orientation::Normal,
        )
        .map_err(|_| PngDecodeError::Input(rusttable_image::ImageInputError::ArithmeticOverflow))?;
        rusttable_image::OwnedImage::new(descriptor, self.sample_bytes()).map_err(|error| {
            PngDecodeError::Input(rusttable_image::ImageInputError::DecodedBufferInvariant {
                expected: u64::try_from(error_required_bytes(&error)).unwrap_or(u64::MAX),
                actual: u64::try_from(self.sample_bytes().len()).unwrap_or(u64::MAX),
            })
        })
    }
}

fn error_required_bytes(error: &rusttable_image::ImageViewError) -> usize {
    match error {
        rusttable_image::ImageViewError::BufferTooShort { required, .. } => *required,
        _ => 0,
    }
}

/// Stable audit receipt for a PNG operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngDecodeReceipt {
    pub backend: String,
    pub source_bytes: u64,
    pub source_sha256: [u8; 32],
    pub output_bytes: u64,
    pub color_type: PngColorType,
    pub bit_depth: PngBitDepth,
    pub interlaced: bool,
    pub palette: bool,
    pub transparency: bool,
    pub chunk_count: u32,
    pub decompressed_bytes: u64,
    pub animation: Option<PngAnimation>,
    pub header_only: bool,
}

/// PNG output and its exact receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngDecodeResult {
    pub header: PngHeader,
    pub pixels: Option<PngPixelData>,
    pub image: Option<rusttable_image::OwnedImage>,
    pub receipt: PngDecodeReceipt,
}

/// Typed PNG parsing and backend failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngDecodeError {
    Cancelled,
    Source(crate::raw::RawSourceError),
    UnsupportedAnimation,
    UnsupportedRegion,
    Malformed(String),
    Limit {
        kind: &'static str,
        actual: u64,
        limit: u64,
    },
    Backend(String),
    Input(rusttable_image::ImageInputError),
}

impl fmt::Display for PngDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("PNG decode cancelled"),
            Self::Source(error) => write!(formatter, "PNG source failed: {error:?}"),
            Self::UnsupportedAnimation => {
                formatter.write_str("PNG animation has no valid default image")
            }
            Self::UnsupportedRegion => formatter.write_str("PNG region decoding is unsupported"),
            Self::Malformed(message) => write!(formatter, "malformed PNG: {message}"),
            Self::Limit {
                kind,
                actual,
                limit,
            } => write!(formatter, "PNG {kind} {actual} exceeds limit {limit}"),
            Self::Backend(message) => write!(formatter, "PNG backend failed: {message}"),
            Self::Input(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PngDecodeError {}

impl From<rusttable_image::ImageInputError> for PngDecodeError {
    fn from(error: rusttable_image::ImageInputError) -> Self {
        Self::Input(error)
    }
}
