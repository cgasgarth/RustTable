use std::fmt;

use crate::ColorEncoding;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SampleType {
    U8,
    U16,
    F16,
    F32,
}

impl SampleType {
    #[must_use]
    pub const fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 | Self::F16 => 2,
            Self::F32 => 4,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChannelLayout {
    Gray,
    GrayA,
    Rgb,
    Rgba,
    Bayer,
    XTrans,
}

impl ChannelLayout {
    #[must_use]
    pub const fn channels(self) -> usize {
        match self {
            Self::Gray | Self::Bayer | Self::XTrans => 1,
            Self::GrayA => 2,
            Self::Rgb => 3,
            Self::Rgba => 4,
        }
    }

    #[must_use]
    pub const fn has_alpha(self) -> bool {
        matches!(self, Self::GrayA | Self::Rgba)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AlphaMode {
    None,
    Straight,
    Premultiplied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ByteOrder {
    Native,
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StorageLayout {
    Interleaved,
    Planar { plane: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ColorEncodingReference {
    Unspecified,
    Srgb,
    LinearSrgb,
    DisplayP3,
    CameraNative,
    IccProfile([u8; 32]),
}

pub type ColorEncodingRef = ColorEncodingReference;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormatError {
    AlphaNotAllowed {
        layout: ChannelLayout,
        alpha: AlphaMode,
    },
    MissingAlpha {
        layout: ChannelLayout,
    },
    InvalidPlane {
        plane: u8,
        channels: usize,
    },
    ArithmeticOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PixelFormat {
    sample_type: SampleType,
    channels: ChannelLayout,
    alpha: AlphaMode,
    byte_order: ByteOrder,
}

impl PixelFormat {
    /// Creates a descriptor whose channel and alpha facts are internally consistent.
    ///
    /// # Errors
    ///
    /// Returns an error when alpha or planar channel facts are inconsistent.
    pub fn new(
        sample_type: SampleType,
        channels: ChannelLayout,
        alpha: AlphaMode,
        byte_order: ByteOrder,
    ) -> Result<Self, PixelFormatError> {
        if channels.has_alpha() && alpha == AlphaMode::None {
            return Err(PixelFormatError::MissingAlpha { layout: channels });
        }
        if !channels.has_alpha() && alpha != AlphaMode::None {
            return Err(PixelFormatError::AlphaNotAllowed {
                layout: channels,
                alpha,
            });
        }
        Ok(Self {
            sample_type,
            channels,
            alpha,
            byte_order,
        })
    }

    #[must_use]
    pub const fn sample_type(self) -> SampleType {
        self.sample_type
    }

    #[must_use]
    pub const fn channels(self) -> ChannelLayout {
        self.channels
    }

    #[must_use]
    pub const fn alpha(self) -> AlphaMode {
        self.alpha
    }

    #[must_use]
    pub const fn byte_order(self) -> ByteOrder {
        self.byte_order
    }

    /// Returns the checked interleaved byte width of one pixel.
    ///
    /// # Errors
    ///
    /// Returns an error when the multiplication overflows.
    pub fn bytes_per_pixel(self) -> Result<usize, PixelFormatError> {
        self.sample_type
            .bytes()
            .checked_mul(self.channels.channels())
            .ok_or(PixelFormatError::ArithmeticOverflow)
    }

    #[must_use]
    pub const fn bytes_per_sample(self) -> usize {
        self.sample_type.bytes()
    }

    /// Validates a planar or interleaved storage descriptor.
    ///
    /// # Errors
    ///
    /// Returns an error when a planar index exceeds the channel count.
    pub fn validate_storage(self, storage: StorageLayout) -> Result<(), PixelFormatError> {
        if let StorageLayout::Planar { plane } = storage
            && usize::from(plane) >= self.channels.channels()
        {
            return Err(PixelFormatError::InvalidPlane {
                plane,
                channels: self.channels.channels(),
            });
        }
        Ok(())
    }

    #[must_use]
    pub const fn canonical_processing() -> Self {
        Self {
            sample_type: SampleType::F32,
            channels: ChannelLayout::Rgba,
            alpha: AlphaMode::Straight,
            byte_order: ByteOrder::Native,
        }
    }

    #[must_use]
    pub const fn with_color_encoding(self, _encoding: ColorEncoding) -> Self {
        self
    }
}

impl fmt::Display for PixelFormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlphaNotAllowed { layout, alpha } => {
                write!(formatter, "{alpha:?} alpha is invalid for {layout:?}")
            }
            Self::MissingAlpha { layout } => write!(formatter, "{layout:?} requires alpha mode"),
            Self::InvalidPlane { plane, channels } => {
                write!(formatter, "plane {plane} is outside {channels} channels")
            }
            Self::ArithmeticOverflow => formatter.write_str("pixel format arithmetic overflowed"),
        }
    }
}

impl std::error::Error for PixelFormatError {}
