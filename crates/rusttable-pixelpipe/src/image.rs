use std::fmt;

use rusttable_processing::RasterDimensions;

/// A CPU pixelpipe raster channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbaF32Channel {
    Red,
    Green,
    Blue,
    Alpha,
}

/// The color interpretation of a packed RGBA f32 raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbaF32ColorEncoding {
    SrgbD65,
    LinearSrgbD65,
}

/// The alpha representation at the pixelpipe boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbaF32AlphaMode {
    Straight,
}

/// Exact, packed raster metadata accepted by the CPU executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RgbaF32Descriptor {
    dimensions: RasterDimensions,
    color_encoding: RgbaF32ColorEncoding,
    alpha_mode: RgbaF32AlphaMode,
}

impl RgbaF32Descriptor {
    #[must_use]
    pub const fn new(dimensions: RasterDimensions, color_encoding: RgbaF32ColorEncoding) -> Self {
        Self {
            dimensions,
            color_encoding,
            alpha_mode: RgbaF32AlphaMode::Straight,
        }
    }

    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn color_encoding(self) -> RgbaF32ColorEncoding {
        self.color_encoding
    }

    #[must_use]
    pub const fn alpha_mode(self) -> RgbaF32AlphaMode {
        self.alpha_mode
    }
}

/// One straight-alpha RGBA f32 pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbaF32Pixel {
    red: f32,
    green: f32,
    blue: f32,
    alpha: f32,
}

impl RgbaF32Pixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    #[must_use]
    pub const fn red(self) -> f32 {
        self.red
    }

    #[must_use]
    pub const fn green(self) -> f32 {
        self.green
    }

    #[must_use]
    pub const fn blue(self) -> f32 {
        self.blue
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.alpha
    }
}

/// Rejection reason for an attempted immutable pixelpipe image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbaF32ImageError {
    PixelCountMismatch {
        expected: u64,
        actual: usize,
    },
    NonFiniteComponent {
        pixel_index: usize,
        channel: RgbaF32Channel,
    },
    ComponentOutsideUnitInterval {
        pixel_index: usize,
        channel: RgbaF32Channel,
    },
}

/// An immutable, tightly packed RGBA f32 image with validated descriptor semantics.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbaF32Image {
    descriptor: RgbaF32Descriptor,
    pixels: Vec<RgbaF32Pixel>,
}

impl RgbaF32Image {
    /// Validates and stores one immutable packed raster.
    ///
    /// Linear RGB may extend beyond the unit interval; transfer-encoded sRGB
    /// and straight alpha remain normalized at this boundary.
    ///
    /// # Errors
    ///
    /// Returns an error if the packed pixel count, finiteness, or descriptor
    /// range contract is violated.
    pub fn new(
        descriptor: RgbaF32Descriptor,
        pixels: Vec<RgbaF32Pixel>,
    ) -> Result<Self, RgbaF32ImageError> {
        let expected = descriptor.dimensions().pixel_count();
        if u64::try_from(pixels.len()) != Ok(expected) {
            return Err(RgbaF32ImageError::PixelCountMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        for (pixel_index, pixel) in pixels.iter().copied().enumerate() {
            validate_component(pixel_index, RgbaF32Channel::Red, pixel.red())?;
            validate_component(pixel_index, RgbaF32Channel::Green, pixel.green())?;
            validate_component(pixel_index, RgbaF32Channel::Blue, pixel.blue())?;
            validate_component(pixel_index, RgbaF32Channel::Alpha, pixel.alpha())?;
            if descriptor.color_encoding() == RgbaF32ColorEncoding::SrgbD65 {
                validate_normalized(pixel_index, RgbaF32Channel::Red, pixel.red())?;
                validate_normalized(pixel_index, RgbaF32Channel::Green, pixel.green())?;
                validate_normalized(pixel_index, RgbaF32Channel::Blue, pixel.blue())?;
            }
            validate_normalized(pixel_index, RgbaF32Channel::Alpha, pixel.alpha())?;
        }
        Ok(Self { descriptor, pixels })
    }

    #[must_use]
    pub const fn descriptor(&self) -> RgbaF32Descriptor {
        self.descriptor
    }

    #[must_use]
    pub fn pixels(&self) -> &[RgbaF32Pixel] {
        &self.pixels
    }
}

fn validate_component(
    pixel_index: usize,
    channel: RgbaF32Channel,
    component: f32,
) -> Result<(), RgbaF32ImageError> {
    if component.is_finite() {
        Ok(())
    } else {
        Err(RgbaF32ImageError::NonFiniteComponent {
            pixel_index,
            channel,
        })
    }
}

fn validate_normalized(
    pixel_index: usize,
    channel: RgbaF32Channel,
    component: f32,
) -> Result<(), RgbaF32ImageError> {
    if (0.0..=1.0).contains(&component) {
        Ok(())
    } else {
        Err(RgbaF32ImageError::ComponentOutsideUnitInterval {
            pixel_index,
            channel,
        })
    }
}

impl fmt::Display for RgbaF32ImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PixelCountMismatch { expected, actual } => {
                write!(
                    formatter,
                    "RGBA f32 image has {actual} pixels, expected {expected}"
                )
            }
            Self::NonFiniteComponent {
                pixel_index,
                channel,
            } => write!(
                formatter,
                "RGBA f32 image pixel {pixel_index} has a non-finite {channel:?} component"
            ),
            Self::ComponentOutsideUnitInterval {
                pixel_index,
                channel,
            } => write!(
                formatter,
                "RGBA f32 image pixel {pixel_index} has an out-of-range {channel:?} component"
            ),
        }
    }
}

impl std::error::Error for RgbaF32ImageError {}
