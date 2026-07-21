use std::fmt;

use rusttable_color::ColorEncoding;

use crate::ImageDimensions;

use super::receipt::image_identity;

#[derive(Debug, Clone, PartialEq)]
pub struct RgbAiImage {
    dimensions: ImageDimensions,
    profile: ColorEncoding,
    pixels: Vec<[f32; 4]>,
    identity: [u8; 32],
}

impl RgbAiImage {
    pub fn new(
        dimensions: ImageDimensions,
        profile: ColorEncoding,
        pixels: Vec<[f32; 4]>,
    ) -> Result<Self, RgbAiImageError> {
        let expected = dimensions.pixels().ok_or(RgbAiImageError::Overflow)?;
        if pixels.len() != expected {
            return Err(RgbAiImageError::PixelCount {
                expected,
                actual: pixels.len(),
            });
        }
        if pixels.iter().flatten().any(|value| !value.is_finite()) {
            return Err(RgbAiImageError::NonFinite);
        }
        if !profile.is_linear() {
            return Err(RgbAiImageError::WorkingProfileMustBeLinear);
        }
        let identity = image_identity(dimensions, profile, &pixels);
        Ok(Self {
            dimensions,
            profile,
            pixels,
            identity,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn profile(&self) -> ColorEncoding {
        self.profile
    }
    #[must_use]
    pub fn pixels(&self) -> &[[f32; 4]] {
        &self.pixels
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbAiImageError {
    Overflow,
    PixelCount { expected: usize, actual: usize },
    NonFinite,
    WorkingProfileMustBeLinear,
}

impl fmt::Display for RgbAiImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid RGB AI image: {self:?}")
    }
}

impl std::error::Error for RgbAiImageError {}
