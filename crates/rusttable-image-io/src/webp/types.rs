use std::fmt;

use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, DecodeLimits, ImageDimensions, PixelFormat, SampleType,
    StorageLayout,
};

/// Stable identity of the pinned pure-Rust WebP backend.
pub const WEBP_BACKEND_ID: &str = "image-webp-0.2.4-pure-rust";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WebPContainer {
    Simple,
    Extended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WebPCodingMode {
    LossyVp8,
    LosslessVp8l,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct WebPFeatures {
    pub icc_profile: bool,
    pub alpha: bool,
    pub exif: bool,
    pub xmp: bool,
    pub animation: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebPDecodeMode {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebPDecodeLimits {
    pub max_source_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_decoded_bytes: u64,
    pub max_chunk_bytes: u64,
    pub max_chunks: u32,
    pub max_metadata_bytes: u64,
    pub max_temporary_bytes: u64,
}

impl WebPDecodeLimits {
    /// Creates internally consistent WebP limits from the common image limits.
    ///
    /// # Errors
    ///
    /// Returns the common limit error for zero, overflowing, or inconsistent values.
    pub fn new(
        max_source_bytes: u64,
        max_width: u32,
        max_height: u32,
        max_pixels: u64,
        max_decoded_bytes: u64,
    ) -> Result<Self, rusttable_image::DecodeLimitsError> {
        DecodeLimits::new(
            max_source_bytes,
            max_width,
            max_height,
            max_pixels,
            max_decoded_bytes,
        )
        .map(Self::from_common)
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
            max_metadata_bytes: 8 * 1024 * 1024,
            max_temporary_bytes: common.max_decoded_bytes().saturating_mul(8),
        }
    }

    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_source_bytes: 4 * 1024 * 1024 * 1024,
            max_width: 16_384,
            max_height: 16_384,
            max_pixels: 250_000_000,
            max_decoded_bytes: 1_000_000_000,
            max_chunk_bytes: 4 * 1024 * 1024 * 1024,
            max_chunks: 100_000,
            max_metadata_bytes: 8 * 1024 * 1024,
            max_temporary_bytes: 8_000_000_000,
        }
    }
}

impl Default for WebPDecodeLimits {
    fn default() -> Self {
        Self::standard()
    }
}

#[derive(Debug, Clone)]
pub struct WebPDecodeRequest {
    pub limits: WebPDecodeLimits,
    pub mode: WebPDecodeMode,
    pub cancellation: crate::raw::RawCancellationToken,
}

impl WebPDecodeRequest {
    #[must_use]
    pub fn new(limits: WebPDecodeLimits) -> Self {
        Self {
            limits,
            mode: WebPDecodeMode::Full,
            cancellation: crate::raw::RawCancellationToken::new(),
        }
    }

    #[must_use]
    pub const fn header(mut self) -> Self {
        self.mode = WebPDecodeMode::Header;
        self
    }

    #[must_use]
    pub const fn thumbnail(mut self) -> Self {
        self.mode = WebPDecodeMode::Thumbnail;
        self
    }

    #[must_use]
    pub const fn region(mut self, x: u32, y: u32, width: u32, height: u32) -> Self {
        self.mode = WebPDecodeMode::Region {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebPDataLocation {
    pub offset: u64,
    pub length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPMetadataChunk {
    pub location: WebPDataLocation,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebPMetadataInventory {
    pub icc_profile: Option<WebPMetadataChunk>,
    pub exif: Option<WebPMetadataChunk>,
    pub xmp: Option<WebPMetadataChunk>,
}

impl WebPMetadataInventory {
    #[must_use]
    pub const fn count(&self) -> u8 {
        self.icc_profile.is_some() as u8 + self.exif.is_some() as u8 + self.xmp.is_some() as u8
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPChunk {
    pub kind: [u8; 4],
    pub location: WebPDataLocation,
    pub padded_length: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPChunkInventory {
    pub chunks: Vec<WebPChunk>,
    pub image_data: WebPDataLocation,
    pub alpha_data: Option<WebPDataLocation>,
    pub compressed_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPHeader {
    pub dimensions: ImageDimensions,
    pub container: WebPContainer,
    pub coding: WebPCodingMode,
    pub features: WebPFeatures,
    pub vp8x_flags: Option<u8>,
    pub riff_declared_bytes: u64,
    pub chunks: WebPChunkInventory,
    pub metadata: WebPMetadataInventory,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebPPixelData {
    RgbU8 {
        dimensions: ImageDimensions,
        samples: Vec<u8>,
    },
    RgbaU8 {
        dimensions: ImageDimensions,
        samples: Vec<u8>,
    },
}

impl WebPPixelData {
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        match self {
            Self::RgbU8 { dimensions, .. } | Self::RgbaU8 { dimensions, .. } => *dimensions,
        }
    }

    #[must_use]
    pub const fn has_alpha(&self) -> bool {
        matches!(self, Self::RgbaU8 { .. })
    }

    #[must_use]
    pub fn samples(&self) -> &[u8] {
        match self {
            Self::RgbU8 { samples, .. } | Self::RgbaU8 { samples, .. } => samples,
        }
    }

    #[must_use]
    pub fn into_samples(self) -> Vec<u8> {
        match self {
            Self::RgbU8 { samples, .. } | Self::RgbaU8 { samples, .. } => samples,
        }
    }

    #[must_use]
    pub fn format(&self) -> PixelFormat {
        let (layout, alpha) = if self.has_alpha() {
            (ChannelLayout::Rgba, AlphaMode::Straight)
        } else {
            (ChannelLayout::Rgb, AlphaMode::None)
        };
        PixelFormat::new(
            SampleType::U8,
            layout,
            alpha,
            ByteOrder::Native,
            StorageLayout::Interleaved,
        )
        .unwrap_or_else(|_| unreachable!("validated WebP pixel format"))
    }

    #[must_use]
    pub fn to_rgba8(&self) -> Vec<u8> {
        match self {
            Self::RgbaU8 { samples, .. } => samples.clone(),
            Self::RgbU8 { samples, .. } => {
                let mut rgba = Vec::with_capacity(samples.len() / 3 * 4);
                for rgb in samples.as_chunks::<3>().0 {
                    rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
                }
                rgba
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPDecodeReceipt {
    pub backend: String,
    pub source_bytes: u64,
    pub source_sha256: [u8; 32],
    pub riff_declared_bytes: u64,
    pub dimensions: ImageDimensions,
    pub container: WebPContainer,
    pub coding: WebPCodingMode,
    pub features: WebPFeatures,
    pub metadata: WebPMetadataInventory,
    pub chunk_count: u32,
    pub compressed_bytes: u64,
    pub output_bytes: u64,
    pub mode: WebPDecodeMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPDecodeResult {
    pub header: WebPHeader,
    pub pixels: Option<WebPPixelData>,
    pub receipt: WebPDecodeReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebPDecodeError {
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
    AllocationFailure,
    Backend(String),
}

impl fmt::Display for WebPDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("WebP decode cancelled"),
            Self::Source(error) => write!(formatter, "WebP source failed: {error:?}"),
            Self::UnsupportedAnimation => formatter.write_str("animated WebP is unsupported"),
            Self::UnsupportedRegion => formatter.write_str("WebP region decoding is unsupported"),
            Self::Malformed(message) => write!(formatter, "malformed WebP: {message}"),
            Self::Limit {
                kind,
                actual,
                limit,
            } => write!(formatter, "WebP {kind} {actual} exceeds limit {limit}"),
            Self::AllocationFailure => formatter.write_str("WebP allocation failed"),
            Self::Backend(message) => write!(formatter, "WebP backend failed: {message}"),
        }
    }
}

impl std::error::Error for WebPDecodeError {}
