use std::fmt;

use rusttable_image::{AlphaMode, ChannelLayout, DecodeLimits, ImageDimensions, Orientation, Roi};

use crate::raw::{RawCancellationToken, RawSourceError};

pub const JXL_BACKEND_ID: &str = "jxl-oxide-0.12.6";
pub const JXL_PROBE_BUDGET_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JxlContainerKind {
    BareCodestream,
    Isobmff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JxlBoxDescriptor {
    pub box_type: [u8; 4],
    pub offset: u64,
    pub total_bytes: u64,
    pub payload_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JxlContainerInventory {
    pub kind: JxlContainerKind,
    pub boxes: Vec<JxlBoxDescriptor>,
    pub codestream_bytes: u64,
    pub codestream_parts: u32,
    pub jpeg_reconstruction_box: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JxlCodingMode {
    VarDct,
    Modular,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JxlBitDepth {
    Integer {
        bits_per_sample: u32,
    },
    Float {
        bits_per_sample: u32,
        exponent_bits: u32,
    },
}

impl JxlBitDepth {
    #[must_use]
    pub const fn bits_per_sample(self) -> u32 {
        match self {
            Self::Integer { bits_per_sample }
            | Self::Float {
                bits_per_sample, ..
            } => bits_per_sample,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JxlColorSpace {
    Gray,
    Rgb,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JxlWhitePoint {
    D65,
    EqualEnergy,
    Dci,
    Custom([f32; 2]),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JxlPrimaries {
    Srgb,
    Bt2100,
    P3,
    Custom {
        red: [f32; 2],
        green: [f32; 2],
        blue: [f32; 2],
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JxlTransferFunction {
    Gamma(f32),
    Bt709,
    Linear,
    Srgb,
    Pq,
    Dci,
    Hlg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JxlRenderingIntent {
    Perceptual,
    Relative,
    Saturation,
    Absolute,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JxlStructuredColor {
    pub color_space: JxlColorSpace,
    pub white_point: JxlWhitePoint,
    pub primaries: JxlPrimaries,
    pub transfer: JxlTransferFunction,
    pub rendering_intent: JxlRenderingIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JxlIccProfile {
    pub color_space: JxlColorSpace,
    pub bytes: Vec<u8>,
    pub sha256: [u8; 32],
}

#[derive(Debug, Clone, PartialEq)]
pub enum JxlColorEncoding {
    Structured(JxlStructuredColor),
    Icc(JxlIccProfile),
}

impl JxlColorEncoding {
    #[must_use]
    pub const fn color_space(&self) -> JxlColorSpace {
        match self {
            Self::Structured(color) => color.color_space,
            Self::Icc(profile) => profile.color_space,
        }
    }

    #[must_use]
    pub const fn is_hdr(&self) -> bool {
        matches!(
            self,
            Self::Structured(JxlStructuredColor {
                transfer: JxlTransferFunction::Pq | JxlTransferFunction::Hlg,
                ..
            })
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JxlToneMapping {
    pub intensity_target: f32,
    pub min_nits: f32,
    pub relative_to_max_display: bool,
    pub linear_below: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JxlExtraChannelType {
    Alpha {
        associated: bool,
    },
    Depth,
    SpotColor {
        red: f32,
        green: f32,
        blue: f32,
        solidity: f32,
    },
    SelectionMask,
    Black,
    Cfa {
        channel: u32,
    },
    Thermal,
    NonOptional,
    Optional,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JxlExtraChannel {
    pub index: usize,
    pub name: String,
    pub channel_type: JxlExtraChannelType,
    pub bit_depth: JxlBitDepth,
    pub dimension_shift: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JxlPreviewDescriptor {
    pub dimensions: ImageDimensions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JxlFrameDescriptor {
    pub index: usize,
    pub displayed: bool,
    pub coding: JxlCodingMode,
    pub duration_ticks: u32,
    pub is_last: bool,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JxlAnimation {
    pub declared: bool,
    pub displayed_frames: usize,
    pub total_frames: usize,
    pub ticks_per_second_numerator: Option<u32>,
    pub ticks_per_second_denominator: Option<u32>,
    pub loop_count: Option<u32>,
    pub has_timecodes: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JxlJpegReconstruction {
    Available,
    Invalid,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JxlHeader {
    pub dimensions: ImageDimensions,
    pub orientation: Orientation,
    pub bit_depth: JxlBitDepth,
    pub color: JxlColorEncoding,
    pub xyb_encoded: bool,
    pub tone_mapping: JxlToneMapping,
    pub alpha: AlphaMode,
    pub extra_channels: Vec<JxlExtraChannel>,
    pub preview: Option<JxlPreviewDescriptor>,
    pub animation: JxlAnimation,
    pub frames: Vec<JxlFrameDescriptor>,
    pub jpeg_reconstruction: JxlJpegReconstruction,
}

impl JxlHeader {
    #[must_use]
    pub fn output_dimensions(&self) -> ImageDimensions {
        self.orientation.output_dimensions(self.dimensions)
    }

    #[must_use]
    pub const fn layout(&self) -> ChannelLayout {
        match (self.color.color_space(), self.alpha) {
            (JxlColorSpace::Gray, AlphaMode::None) => ChannelLayout::Gray,
            (JxlColorSpace::Gray, _) => ChannelLayout::GrayA,
            (JxlColorSpace::Rgb, AlphaMode::None) => ChannelLayout::Rgb,
            (JxlColorSpace::Rgb, _) => ChannelLayout::Rgba,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JxlDecodeMode {
    Header,
    Full,
    Region(Roi),
    Thumbnail { max_width: u32, max_height: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JxlDecodeLimits {
    pub max_source_bytes: u64,
    pub max_width: u32,
    pub max_height: u32,
    pub max_pixels: u64,
    pub max_decoded_bytes: u64,
    pub max_backend_alloc_bytes: u64,
    pub max_metadata_bytes: u64,
    pub max_boxes: u32,
    pub max_frames: u32,
    pub max_extra_channels: u32,
    pub max_previews: u32,
    pub max_name_bytes: u32,
}

impl JxlDecodeLimits {
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_source_bytes: 4 * 1024 * 1024 * 1024,
            max_width: 65_535,
            max_height: 65_535,
            max_pixels: 250_000_000,
            max_decoded_bytes: 4_000_000_000,
            max_backend_alloc_bytes: 4_000_000_000,
            max_metadata_bytes: 16 * 1024 * 1024,
            max_boxes: 4_096,
            max_frames: 1_024,
            max_extra_channels: 64,
            max_previews: 1,
            max_name_bytes: 1_024,
        }
    }

    #[must_use]
    pub const fn from_common(common: DecodeLimits) -> Self {
        let f32_bytes = common.max_decoded_bytes().saturating_mul(4);
        Self {
            max_source_bytes: common.max_source_bytes(),
            max_width: common.max_width(),
            max_height: common.max_height(),
            max_pixels: common.max_pixel_count(),
            max_decoded_bytes: f32_bytes,
            max_backend_alloc_bytes: f32_bytes.saturating_mul(8),
            max_metadata_bytes: 16 * 1024 * 1024,
            max_boxes: 4_096,
            max_frames: 1_024,
            max_extra_channels: 64,
            max_previews: 1,
            max_name_bytes: 1_024,
        }
    }
}

impl Default for JxlDecodeLimits {
    fn default() -> Self {
        Self::standard()
    }
}

#[derive(Debug, Clone)]
pub struct JxlDecodeRequest {
    pub limits: JxlDecodeLimits,
    pub mode: JxlDecodeMode,
    pub cancellation: RawCancellationToken,
}

impl JxlDecodeRequest {
    #[must_use]
    pub fn new(limits: JxlDecodeLimits) -> Self {
        Self {
            limits,
            mode: JxlDecodeMode::Full,
            cancellation: RawCancellationToken::new(),
        }
    }

    #[must_use]
    pub const fn header(mut self) -> Self {
        self.mode = JxlDecodeMode::Header;
        self
    }

    #[must_use]
    pub const fn region(mut self, region: Roi) -> Self {
        self.mode = JxlDecodeMode::Region(region);
        self
    }

    #[must_use]
    pub const fn thumbnail(mut self, max_width: u32, max_height: u32) -> Self {
        self.mode = JxlDecodeMode::Thumbnail {
            max_width,
            max_height,
        };
        self
    }

    #[must_use]
    pub fn with_cancellation(mut self, cancellation: RawCancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JxlPixelData {
    pub dimensions: ImageDimensions,
    pub layout: ChannelLayout,
    pub alpha: AlphaMode,
    pub samples: Vec<f32>,
}

impl JxlPixelData {
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.samples.len().saturating_mul(size_of::<f32>())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JxlRoiBehavior {
    NotRequested,
    FullDecodeThenCrop {
        source: ImageDimensions,
        region: Roi,
    },
    FullDecodeThenScale {
        source: ImageDimensions,
        output: ImageDimensions,
        preview_declared: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct JxlDecodeReceipt {
    pub backend: String,
    pub source_bytes: u64,
    pub source_sha256: [u8; 32],
    pub bytes_read: u64,
    pub output_bytes: u64,
    pub container: JxlContainerInventory,
    pub coding: JxlCodingMode,
    pub color: JxlColorEncoding,
    pub orientation: Orientation,
    pub orientation_applied: bool,
    pub alpha: AlphaMode,
    pub extra_channels: Vec<JxlExtraChannel>,
    pub frame_count: usize,
    pub displayed_frame_count: usize,
    pub single_frame_animation: bool,
    pub roi_behavior: JxlRoiBehavior,
    pub header_only: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct JxlDecodeResult {
    pub header: JxlHeader,
    pub pixels: Option<JxlPixelData>,
    pub receipt: JxlDecodeReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JxlDecodeError {
    Source(RawSourceError),
    NotJpegXl,
    UnsupportedAnimation {
        displayed_frames: usize,
    },
    UnsupportedColorSpace,
    UnsupportedBlackChannel,
    UnsupportedEssentialBox([u8; 4]),
    UnsupportedBoxCompression([u8; 4]),
    Limit {
        kind: &'static str,
        actual: u64,
        limit: u64,
    },
    Malformed(String),
    InvalidRegion,
    InvalidThumbnail,
    InvalidIcc(String),
    Cancelled,
    Backend(String),
    NonFiniteSamples,
    ArithmeticOverflow,
    AllocationFailure,
}

impl fmt::Display for JxlDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Source(error) => write!(formatter, "JPEG XL source failed: {error:?}"),
            Self::NotJpegXl => formatter.write_str("JPEG XL signature is missing"),
            Self::UnsupportedAnimation { displayed_frames } => write!(
                formatter,
                "JPEG XL animation has {displayed_frames} displayed frames"
            ),
            Self::UnsupportedColorSpace => {
                formatter.write_str("JPEG XL color space is unsupported")
            }
            Self::UnsupportedBlackChannel => {
                formatter.write_str("JPEG XL CMYK/black output is unsupported")
            }
            Self::UnsupportedEssentialBox(box_type) => {
                write!(formatter, "unsupported essential JPEG XL box {box_type:?}")
            }
            Self::UnsupportedBoxCompression(box_type) => {
                write!(
                    formatter,
                    "compressed JPEG XL box {box_type:?} is unsupported"
                )
            }
            Self::Limit {
                kind,
                actual,
                limit,
            } => write!(formatter, "JPEG XL {kind} {actual} exceeds limit {limit}"),
            Self::Malformed(message) => write!(formatter, "malformed JPEG XL: {message}"),
            Self::InvalidRegion => formatter.write_str("JPEG XL region is empty or out of bounds"),
            Self::InvalidThumbnail => {
                formatter.write_str("JPEG XL thumbnail bounds must be nonzero")
            }
            Self::InvalidIcc(message) => {
                write!(formatter, "invalid JPEG XL ICC profile: {message}")
            }
            Self::Cancelled => formatter.write_str("JPEG XL decode cancelled"),
            Self::Backend(message) => write!(formatter, "JPEG XL backend failed: {message}"),
            Self::NonFiniteSamples => {
                formatter.write_str("JPEG XL backend produced non-finite samples")
            }
            Self::ArithmeticOverflow => formatter.write_str("JPEG XL arithmetic overflowed"),
            Self::AllocationFailure => formatter.write_str("JPEG XL allocation failed"),
        }
    }
}

impl std::error::Error for JxlDecodeError {}
