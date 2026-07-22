use std::fmt;

use rusttable_image::{ImageDimensions, Orientation};

/// Maximum number of bytes scanned by the JPEG header parser when it is used
/// as a registry probe.
pub const JPEG_PROBE_BUDGET_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegSof {
    BaselineDct,
    ExtendedSequentialDct,
    ProgressiveDct,
    Differential,
    Lossless,
    Arithmetic,
    Unknown(u8),
}

impl JpegSof {
    #[must_use]
    pub const fn marker(self) -> u8 {
        match self {
            Self::BaselineDct => 0xc0,
            Self::ExtendedSequentialDct => 0xc1,
            Self::ProgressiveDct => 0xc2,
            Self::Differential => 0xc5,
            Self::Lossless => 0xc3,
            Self::Arithmetic => 0xc9,
            Self::Unknown(marker) => marker,
        }
    }

    #[must_use]
    pub const fn is_dct(self) -> bool {
        matches!(
            self,
            Self::BaselineDct | Self::ExtendedSequentialDct | Self::ProgressiveDct
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegCodingProcess {
    Baseline,
    ExtendedSequential,
    Progressive,
    Lossless,
    Arithmetic,
    Unsupported(JpegSof),
}

impl fmt::Display for JpegCodingProcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Baseline => "baseline",
            Self::ExtendedSequential => "extended sequential",
            Self::Progressive => "progressive",
            Self::Lossless => "lossless",
            Self::Arithmetic => "arithmetic",
            Self::Unsupported(_) => "unsupported",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegComponentModel {
    Gray,
    Ycbcr,
    Rgb,
    Cmyk,
    Ycck,
    Unknown(u8),
}

impl JpegComponentModel {
    #[must_use]
    pub const fn channels(self) -> u8 {
        match self {
            Self::Gray => 1,
            Self::Ycbcr | Self::Rgb => 3,
            Self::Cmyk | Self::Ycck => 4,
            Self::Unknown(channels) => channels,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JpegSampling {
    pub component_id: u8,
    pub horizontal: u8,
    pub vertical: u8,
    pub quantization_table: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegMetadataSegment {
    pub marker: u8,
    pub offset: u64,
    pub byte_length: u32,
    pub data: Vec<u8>,
}

impl JpegMetadataSegment {
    #[must_use]
    pub fn is_profile(&self) -> bool {
        self.marker == 0xe2 && self.data.starts_with(b"ICC_PROFILE\0")
    }

    #[must_use]
    pub fn is_exif(&self) -> bool {
        self.marker == 0xe1 && self.data.starts_with(b"Exif\0\0")
    }

    #[must_use]
    pub fn is_xmp(&self) -> bool {
        self.marker == 0xe1 && self.data.starts_with(b"http://ns.adobe.com/xap/1.0/\0")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegHeader {
    pub dimensions: ImageDimensions,
    pub precision: u8,
    pub components: JpegComponentModel,
    pub coding_process: JpegCodingProcess,
    pub sof: JpegSof,
    pub sampling: Vec<JpegSampling>,
    pub scans: u16,
    pub restart_interval: u16,
    pub orientation: Orientation,
    pub adobe_color_transform: Option<u8>,
    pub metadata: Vec<JpegMetadataSegment>,
}

impl JpegHeader {
    #[must_use]
    pub const fn channels(&self) -> u8 {
        self.components.channels()
    }

    #[must_use]
    pub fn output_dimensions(&self) -> ImageDimensions {
        self.orientation.output_dimensions(self.dimensions)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JpegPixelData {
    GrayU8(Vec<u8>),
    RgbU8(Vec<u8>),
    CmykU8(Vec<u8>),
    GrayU16(Vec<u16>),
    RgbU16(Vec<u16>),
    CmykU16(Vec<u16>),
}

impl JpegPixelData {
    #[must_use]
    pub const fn bytes_per_sample(&self) -> usize {
        match self {
            Self::GrayU8(_) | Self::RgbU8(_) | Self::CmykU8(_) => 1,
            Self::GrayU16(_) | Self::RgbU16(_) | Self::CmykU16(_) => 2,
        }
    }

    #[must_use]
    pub fn byte_len(&self) -> usize {
        match self {
            Self::GrayU8(values) | Self::RgbU8(values) | Self::CmykU8(values) => values.len(),
            Self::GrayU16(values) | Self::RgbU16(values) | Self::CmykU16(values) => {
                values.len().saturating_mul(2)
            }
        }
    }

    #[must_use]
    pub const fn channels(&self) -> usize {
        match self {
            Self::GrayU8(_) | Self::GrayU16(_) => 1,
            Self::RgbU8(_) | Self::RgbU16(_) => 3,
            Self::CmykU8(_) | Self::CmykU16(_) => 4,
        }
    }
}

impl fmt::Display for JpegSof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BaselineDct => "baseline DCT",
            Self::ExtendedSequentialDct => "extended sequential DCT",
            Self::ProgressiveDct => "progressive DCT",
            Self::Differential => "differential JPEG",
            Self::Lossless => "lossless JPEG",
            Self::Arithmetic => "arithmetic JPEG",
            Self::Unknown(_) => "unknown JPEG SOF",
        })
    }
}
