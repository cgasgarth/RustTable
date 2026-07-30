use std::fmt;

/// The scalar representation used by an image plane.
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

/// The logical channels carried by a pixel or plane set.
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
    pub const fn is_mosaic(self) -> bool {
        matches!(self, Self::Bayer | Self::XTrans)
    }
}

/// How alpha values are represented at the image boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AlphaMode {
    None,
    Straight,
    Premultiplied,
}

/// Byte order for multi-byte samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ByteOrder {
    Native,
    Little,
    Big,
}

/// Whether channels share a row or occupy separate planes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StorageLayout {
    Interleaved,
    Planar,
}

/// A complete, storage-level pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PixelFormat {
    sample_type: SampleType,
    channels: ChannelLayout,
    alpha: AlphaMode,
    byte_order: ByteOrder,
    storage: StorageLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormatError {
    InvalidAlpha,
    MosaicAlpha,
}

impl PixelFormat {
    /// Creates a validated format descriptor. Planar formats expose one plane
    /// per channel; interleaved formats expose one plane containing all channels.
    ///
    /// # Errors
    ///
    /// Returns an error when alpha is incompatible with the channel layout.
    pub const fn new(
        sample_type: SampleType,
        channels: ChannelLayout,
        alpha: AlphaMode,
        byte_order: ByteOrder,
        storage: StorageLayout,
    ) -> Result<Self, PixelFormatError> {
        let alpha_allowed = matches!(channels, ChannelLayout::GrayA | ChannelLayout::Rgba);
        if alpha_allowed == matches!(alpha, AlphaMode::None) {
            return Err(PixelFormatError::InvalidAlpha);
        }
        if channels.is_mosaic() && !matches!(alpha, AlphaMode::None) {
            return Err(PixelFormatError::MosaicAlpha);
        }
        Ok(Self {
            sample_type,
            channels,
            alpha,
            byte_order,
            storage,
        })
    }

    /// Canonical linear-light RGBA processing format.
    #[must_use]
    pub const fn canonical_rgba_f32() -> Self {
        Self {
            sample_type: SampleType::F32,
            channels: ChannelLayout::Rgba,
            alpha: AlphaMode::Straight,
            byte_order: ByteOrder::Native,
            storage: StorageLayout::Interleaved,
        }
    }

    /// The legacy decoded format used by the current file decoders.
    #[must_use]
    pub const fn rgba8() -> Self {
        Self {
            sample_type: SampleType::U8,
            channels: ChannelLayout::Rgba,
            alpha: AlphaMode::Straight,
            byte_order: ByteOrder::Native,
            storage: StorageLayout::Interleaved,
        }
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

    #[must_use]
    pub const fn storage(self) -> StorageLayout {
        self.storage
    }

    #[must_use]
    pub const fn plane_count(self) -> usize {
        match self.storage {
            StorageLayout::Interleaved => 1,
            StorageLayout::Planar => self.channels.channels(),
        }
    }

    #[must_use]
    pub const fn bytes_per_pixel(self) -> usize {
        match self.storage {
            StorageLayout::Interleaved => self.sample_type.bytes() * self.channels.channels(),
            StorageLayout::Planar => self.sample_type.bytes(),
        }
    }

    #[must_use]
    pub const fn bytes_per_sample(self) -> usize {
        self.sample_type.bytes()
    }
}

impl fmt::Display for PixelFormatError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidAlpha => "alpha mode does not match channel layout",
            Self::MosaicAlpha => "mosaic formats cannot carry alpha",
        })
    }
}

impl std::error::Error for PixelFormatError {}
