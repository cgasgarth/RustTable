use std::fmt;

use rusttable_processing::RasterDimensions;
use sha2::{Digest, Sha256};

/// A CPU pixelpipe raster channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbaF32Channel {
    Red,
    Green,
    Blue,
    Alpha,
}

/// Immutable evidence for the decoded source raster supplied to a pixelpipe.
///
/// The identity is a SHA-256 digest of the exact, ordered RGBA f32 bit pattern.
/// It therefore binds a request to the decoded source raster rather than a
/// mutable path or decoder buffer.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceRasterIdentity([u8; 32]);

impl SourceRasterIdentity {
    #[must_use]
    pub(crate) fn from_components(
        representation: RgbaF32SourceRepresentation,
        components: impl IntoIterator<Item = f32>,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update([representation.tag()]);
        for component in components {
            hasher.update(component.to_bits().to_le_bytes());
        }
        Self(hasher.finalize().into())
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for SourceRasterIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

/// The color interpretation of a packed RGBA f32 raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbaF32ColorEncoding {
    SrgbD65,
    LinearSrgbD65,
    LabD50,
}

/// Native representation retained after the typed decode-to-f32 bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RgbaF32SourceRepresentation {
    U8,
    U16,
    F16,
    F32,
}

impl RgbaF32SourceRepresentation {
    const fn tag(self) -> u8 {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::F16 => 3,
            Self::F32 => 4,
        }
    }
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
    source_representation: RgbaF32SourceRepresentation,
}

impl RgbaF32Descriptor {
    #[must_use]
    pub const fn new(dimensions: RasterDimensions, color_encoding: RgbaF32ColorEncoding) -> Self {
        Self {
            dimensions,
            color_encoding,
            alpha_mode: RgbaF32AlphaMode::Straight,
            source_representation: RgbaF32SourceRepresentation::F32,
        }
    }

    #[must_use]
    pub const fn with_source_representation(
        dimensions: RasterDimensions,
        color_encoding: RgbaF32ColorEncoding,
        source_representation: RgbaF32SourceRepresentation,
    ) -> Self {
        Self {
            dimensions,
            color_encoding,
            alpha_mode: RgbaF32AlphaMode::Straight,
            source_representation,
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

    #[must_use]
    pub const fn source_representation(self) -> RgbaF32SourceRepresentation {
        self.source_representation
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
    SourceIdentityMismatch {
        expected: SourceRasterIdentity,
        actual: SourceRasterIdentity,
    },
}

/// An immutable, tightly packed RGBA f32 image with validated descriptor semantics.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbaF32Image {
    descriptor: RgbaF32Descriptor,
    pixels: Vec<RgbaF32Pixel>,
    source_identity: SourceRasterIdentity,
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
        let source_identity = SourceRasterIdentity::from_components(
            descriptor.source_representation(),
            pixels
                .iter()
                .flat_map(|pixel| [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]),
        );
        Ok(Self {
            descriptor,
            pixels,
            source_identity,
        })
    }

    /// Validates an immutable source-raster identity while constructing input.
    ///
    /// # Errors
    ///
    /// Returns an error when the image is invalid or its supplied identity does
    /// not describe the exact packed pixel bits.
    pub fn new_with_source_identity(
        descriptor: RgbaF32Descriptor,
        pixels: Vec<RgbaF32Pixel>,
        expected_source_identity: SourceRasterIdentity,
    ) -> Result<Self, RgbaF32ImageError> {
        let image = Self::new(descriptor, pixels)?;
        if image.source_identity != expected_source_identity {
            return Err(RgbaF32ImageError::SourceIdentityMismatch {
                expected: expected_source_identity,
                actual: image.source_identity,
            });
        }
        Ok(image)
    }

    #[must_use]
    pub const fn descriptor(&self) -> RgbaF32Descriptor {
        self.descriptor
    }

    #[must_use]
    pub fn pixels(&self) -> &[RgbaF32Pixel] {
        &self.pixels
    }

    /// Returns immutable evidence for the validated decoded source raster.
    #[must_use]
    pub const fn source_identity(&self) -> SourceRasterIdentity {
        self.source_identity
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
            Self::SourceIdentityMismatch { expected, actual } => write!(
                formatter,
                "RGBA f32 image source identity mismatch: expected {expected:?}, got {actual:?}"
            ),
        }
    }
}

impl std::error::Error for RgbaF32ImageError {}
