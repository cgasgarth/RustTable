use std::fmt;

use crate::{FiniteF32, FiniteF32Error};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RasterDimensions {
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RasterDimensionsError {
    ZeroWidth,
    ZeroHeight,
}

impl RasterDimensions {
    /// Creates nonzero raster dimensions.
    ///
    /// # Errors
    ///
    /// Returns a distinct error for a zero width or zero height.
    pub const fn new(width: u32, height: u32) -> Result<Self, RasterDimensionsError> {
        if width == 0 {
            return Err(RasterDimensionsError::ZeroWidth);
        }
        if height == 0 {
            return Err(RasterDimensionsError::ZeroHeight);
        }
        Ok(Self { width, height })
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn pixel_count(self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbChannel {
    Red,
    Green,
    Blue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrgbChannelError {
    NonFinite,
    BelowZero,
    AboveOne,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SrgbChannel(FiniteF32);

impl SrgbChannel {
    /// Creates a normalized transfer-encoded sRGB channel.
    ///
    /// # Errors
    ///
    /// Returns an error when the value is non-finite or outside `0.0..=1.0`.
    pub fn new(value: f32) -> Result<Self, SrgbChannelError> {
        let value =
            FiniteF32::new(value).map_err(|_: FiniteF32Error| SrgbChannelError::NonFinite)?;
        if value.get() < 0.0 {
            return Err(SrgbChannelError::BelowZero);
        }
        if value.get() > 1.0 {
            return Err(SrgbChannelError::AboveOne);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> f32 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceRgb {
    red: SrgbChannel,
    green: SrgbChannel,
    blue: SrgbChannel,
}

impl SourceRgb {
    #[must_use]
    pub const fn new(red: SrgbChannel, green: SrgbChannel, blue: SrgbChannel) -> Self {
        Self { red, green, blue }
    }

    #[must_use]
    pub const fn red(self) -> SrgbChannel {
        self.red
    }

    #[must_use]
    pub const fn green(self) -> SrgbChannel {
        self.green
    }

    #[must_use]
    pub const fn blue(self) -> SrgbChannel {
        self.blue
    }

    #[must_use]
    pub const fn channel(self, channel: RgbChannel) -> SrgbChannel {
        match channel {
            RgbChannel::Red => self.red,
            RgbChannel::Green => self.green,
            RgbChannel::Blue => self.blue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinearRgb {
    red: FiniteF32,
    green: FiniteF32,
    blue: FiniteF32,
}

impl LinearRgb {
    #[must_use]
    pub const fn new(red: FiniteF32, green: FiniteF32, blue: FiniteF32) -> Self {
        Self { red, green, blue }
    }

    #[must_use]
    pub const fn red(self) -> FiniteF32 {
        self.red
    }

    #[must_use]
    pub const fn green(self) -> FiniteF32 {
        self.green
    }

    #[must_use]
    pub const fn blue(self) -> FiniteF32 {
        self.blue
    }

    #[must_use]
    pub const fn channel(self, channel: RgbChannel) -> FiniteF32 {
        match channel {
            RgbChannel::Red => self.red,
            RgbChannel::Green => self.green,
            RgbChannel::Blue => self.blue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceColorSpace {
    Srgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkingColorSpace {
    LinearSrgb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageBuildError {
    PixelCountMismatch { expected: u64, actual: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceRgbImage {
    dimensions: RasterDimensions,
    pixels: Vec<SourceRgb>,
}

impl SourceRgbImage {
    /// Creates a row-major normalized sRGB image without allocating from dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`ImageBuildError::PixelCountMismatch`] when the supplied pixels
    /// do not exactly cover the dimensions.
    pub fn new(
        dimensions: RasterDimensions,
        pixels: Vec<SourceRgb>,
    ) -> Result<Self, ImageBuildError> {
        if u64::try_from(pixels.len()) != Ok(dimensions.pixel_count()) {
            return Err(ImageBuildError::PixelCountMismatch {
                expected: dimensions.pixel_count(),
                actual: pixels.len(),
            });
        }
        Ok(Self { dimensions, pixels })
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn space(&self) -> SourceColorSpace {
        SourceColorSpace::Srgb
    }

    #[must_use]
    pub fn pixel_slice(&self) -> &[SourceRgb] {
        &self.pixels
    }

    pub fn pixels(&self) -> impl Iterator<Item = &SourceRgb> {
        self.pixels.iter()
    }

    #[must_use]
    pub fn pixel(&self, index: usize) -> Option<&SourceRgb> {
        self.pixels.get(index)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingRgbImage {
    dimensions: RasterDimensions,
    pixels: Vec<LinearRgb>,
}

impl WorkingRgbImage {
    /// Creates a row-major linear-light sRGB working image.
    ///
    /// # Errors
    ///
    /// Returns [`ImageBuildError::PixelCountMismatch`] when the supplied pixels
    /// do not exactly cover the dimensions.
    pub fn new(
        dimensions: RasterDimensions,
        pixels: Vec<LinearRgb>,
    ) -> Result<Self, ImageBuildError> {
        if u64::try_from(pixels.len()) != Ok(dimensions.pixel_count()) {
            return Err(ImageBuildError::PixelCountMismatch {
                expected: dimensions.pixel_count(),
                actual: pixels.len(),
            });
        }
        Ok(Self { dimensions, pixels })
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn space(&self) -> WorkingColorSpace {
        WorkingColorSpace::LinearSrgb
    }

    #[must_use]
    pub fn pixel_slice(&self) -> &[LinearRgb] {
        &self.pixels
    }

    pub fn pixels(&self) -> impl Iterator<Item = &LinearRgb> {
        self.pixels.iter()
    }

    #[must_use]
    pub fn pixel(&self, index: usize) -> Option<&LinearRgb> {
        self.pixels.get(index)
    }

    pub(crate) fn from_validated_parts(
        dimensions: RasterDimensions,
        pixels: Vec<LinearRgb>,
    ) -> Self {
        Self { dimensions, pixels }
    }
}

/// Converts normalized transfer-encoded sRGB to linear-light sRGB.
///
/// This establishes a linear-light working space while making no claim that
/// decoded sRGB is scene-referred and does not perform ICC or display conversion.
#[must_use]
pub fn to_linear_srgb(source: &SourceRgbImage) -> WorkingRgbImage {
    let pixels = source
        .pixels
        .iter()
        .map(|pixel| {
            LinearRgb::new(
                decode_channel(pixel.red()),
                decode_channel(pixel.green()),
                decode_channel(pixel.blue()),
            )
        })
        .collect();
    WorkingRgbImage {
        dimensions: source.dimensions,
        pixels,
    }
}

fn decode_channel(channel: SrgbChannel) -> FiniteF32 {
    let encoded = channel.get();
    let linear = if encoded <= 0.04045 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    };
    FiniteF32::from_proven_finite(linear)
}

impl fmt::Display for RasterDimensionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWidth => formatter.write_str("raster width must be nonzero"),
            Self::ZeroHeight => formatter.write_str("raster height must be nonzero"),
        }
    }
}

impl std::error::Error for RasterDimensionsError {}

impl fmt::Display for SrgbChannelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite => formatter.write_str("sRGB channel must be finite"),
            Self::BelowZero => formatter.write_str("sRGB channel must not be below zero"),
            Self::AboveOne => formatter.write_str("sRGB channel must not be above one"),
        }
    }
}

impl std::error::Error for SrgbChannelError {}

impl fmt::Display for ImageBuildError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PixelCountMismatch { expected, actual } => write!(
                formatter,
                "raster dimensions require {expected} pixels but received {actual}"
            ),
        }
    }
}

impl std::error::Error for ImageBuildError {}
