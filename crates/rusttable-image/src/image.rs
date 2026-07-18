use std::fmt;
use std::num::NonZeroU32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDimensionsError {
    ZeroWidth,
    ZeroHeight,
    ArithmeticOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageDimensions {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl ImageDimensions {
    /// Creates dimensions after rejecting either zero axis.
    ///
    /// # Errors
    ///
    /// Returns an error when either axis is zero.
    pub fn new(width: u32, height: u32) -> Result<Self, ImageDimensionsError> {
        let width = NonZeroU32::new(width).ok_or(ImageDimensionsError::ZeroWidth)?;
        let height = NonZeroU32::new(height).ok_or(ImageDimensionsError::ZeroHeight)?;
        Ok(Self { width, height })
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width.get()
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height.get()
    }

    /// Returns the checked number of pixels represented by these dimensions.
    ///
    /// # Errors
    ///
    /// Returns an error if the multiplication cannot be represented.
    pub fn pixel_count(self) -> Result<u64, ImageDimensionsError> {
        u64::from(self.width())
            .checked_mul(u64::from(self.height()))
            .ok_or(ImageDimensionsError::ArithmeticOverflow)
    }

    /// Returns the checked number of packed RGBA8 bytes represented by these dimensions.
    ///
    /// # Errors
    ///
    /// Returns an error if the multiplication cannot be represented.
    pub fn decoded_byte_count(self) -> Result<u64, ImageDimensionsError> {
        self.pixel_count()?
            .checked_mul(4)
            .ok_or(ImageDimensionsError::ArithmeticOverflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodedImageError {
    ArithmeticOverflow,
    ByteLengthMismatch { expected: u64, actual: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelLayout {
    Rgba8StraightAlpha,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorEncoding {
    Unspecified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    dimensions: ImageDimensions,
    layout: PixelLayout,
    color_encoding: ColorEncoding,
    pixels: Vec<u8>,
}

impl DecodedImage {
    /// Creates an immutable packed RGBA8 image with an exact buffer length.
    ///
    /// # Errors
    ///
    /// Returns an error when the expected byte count overflows or the buffer
    /// length differs from the checked dimensions.
    pub fn new(dimensions: ImageDimensions, pixels: Vec<u8>) -> Result<Self, DecodedImageError> {
        let expected = dimensions
            .decoded_byte_count()
            .map_err(|_| DecodedImageError::ArithmeticOverflow)?;
        let actual =
            u64::try_from(pixels.len()).map_err(|_| DecodedImageError::ArithmeticOverflow)?;
        if actual != expected {
            return Err(DecodedImageError::ByteLengthMismatch { expected, actual });
        }
        Ok(Self {
            dimensions,
            layout: PixelLayout::Rgba8StraightAlpha,
            color_encoding: ColorEncoding::Unspecified,
            pixels,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn layout(&self) -> PixelLayout {
        self.layout
    }

    #[must_use]
    pub const fn color_encoding(&self) -> ColorEncoding {
        self.color_encoding
    }

    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

impl fmt::Display for ImageDimensionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroWidth => "image width must be nonzero",
            Self::ZeroHeight => "image height must be nonzero",
            Self::ArithmeticOverflow => "image dimension arithmetic overflowed",
        })
    }
}

impl std::error::Error for ImageDimensionsError {}
