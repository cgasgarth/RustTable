use std::fmt;

use rusttable_color::ColorEncoding;
use rusttable_image::ImageDimensions;
use sha2::{Digest, Sha256};

use crate::{RgbaF32ColorEncoding, RgbaF32Image, RgbaF32Pixel};

/// Borrowed packed pixels plus exact identity. Construction validates shape but deliberately
/// permits non-finite and extended-range components so request policy can skip or reject them.
#[derive(Debug, Clone, Copy)]
pub struct AnalysisRaster<'a> {
    dimensions: ImageDimensions,
    source_color_space: ColorEncoding,
    pixels: &'a [RgbaF32Pixel],
    identity: [u8; 32],
}

impl<'a> AnalysisRaster<'a> {
    /// Borrows packed pixels without retaining or copying pipeline storage.
    ///
    /// # Errors
    ///
    /// Rejects a shape mismatch, size overflow, or unspecified color space.
    pub fn new(
        dimensions: ImageDimensions,
        source_color_space: ColorEncoding,
        pixels: &'a [RgbaF32Pixel],
    ) -> Result<Self, AnalysisRasterError> {
        let expected = dimensions
            .pixel_count()
            .map_err(|_| AnalysisRasterError::SizeOverflow)?;
        if u64::try_from(pixels.len()) != Ok(expected) {
            return Err(AnalysisRasterError::PixelCount {
                expected,
                actual: pixels.len(),
            });
        }
        if !source_color_space.is_explicit() {
            return Err(AnalysisRasterError::UnspecifiedColorSpace);
        }
        let identity = raster_identity(dimensions, source_color_space, pixels, None);
        Ok(Self {
            dimensions,
            source_color_space,
            pixels,
            identity,
        })
    }

    /// Convenience bridge for the established pixelpipe descriptor color enum.
    ///
    /// # Errors
    ///
    /// Rejects a shape mismatch or size overflow.
    pub fn from_rgba(
        dimensions: ImageDimensions,
        source_color_space: RgbaF32ColorEncoding,
        pixels: &'a [RgbaF32Pixel],
    ) -> Result<Self, AnalysisRasterError> {
        Self::new(dimensions, color_encoding(source_color_space), pixels)
    }

    /// Borrows an existing immutable pixelpipe image and incorporates its established source
    /// identity into analysis provenance.
    ///
    /// # Errors
    ///
    /// Returns a size error if established raster dimensions cannot cross the image boundary.
    pub fn from_image(image: &'a RgbaF32Image) -> Result<Self, AnalysisRasterError> {
        let descriptor = image.descriptor();
        let raster_dimensions = descriptor.dimensions();
        let dimensions =
            ImageDimensions::new(raster_dimensions.width(), raster_dimensions.height())
                .map_err(|_| AnalysisRasterError::SizeOverflow)?;
        let source_color_space = color_encoding(descriptor.color_encoding());
        let identity = raster_identity(
            dimensions,
            source_color_space,
            image.pixels(),
            Some(image.source_identity().as_bytes()),
        );
        Ok(Self {
            dimensions,
            source_color_space,
            pixels: image.pixels(),
            identity,
        })
    }

    #[must_use]
    pub const fn dimensions(self) -> ImageDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn source_color_space(self) -> ColorEncoding {
        self.source_color_space
    }
    #[must_use]
    pub const fn pixels(self) -> &'a [RgbaF32Pixel] {
        self.pixels
    }
    #[must_use]
    pub const fn identity(self) -> [u8; 32] {
        self.identity
    }
}

/// Borrowed, already-evaluated one-value-per-pixel mask. This is a consumption boundary only;
/// mask graph evaluation and mask algorithms remain owned by `rusttable-masks`.
#[derive(Debug, Clone, Copy)]
pub struct AnalysisMask<'a> {
    values: &'a [f32],
    identity: [u8; 32],
}

impl<'a> AnalysisMask<'a> {
    /// Borrows one mask value per source pixel and computes its exact identity.
    ///
    /// # Errors
    ///
    /// Rejects a shape mismatch or size overflow.
    pub fn new(
        dimensions: ImageDimensions,
        values: &'a [f32],
    ) -> Result<Self, AnalysisRasterError> {
        let expected = dimensions
            .pixel_count()
            .map_err(|_| AnalysisRasterError::SizeOverflow)?;
        if u64::try_from(values.len()) != Ok(expected) {
            return Err(AnalysisRasterError::MaskCount {
                expected,
                actual: values.len(),
            });
        }
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.analysis.mask.v1");
        hasher.update(dimensions.width().to_le_bytes());
        hasher.update(dimensions.height().to_le_bytes());
        for value in values {
            hasher.update(value.to_bits().to_le_bytes());
        }
        Ok(Self {
            values,
            identity: hasher.finalize().into(),
        })
    }

    #[must_use]
    pub const fn values(self) -> &'a [f32] {
        self.values
    }
    #[must_use]
    pub const fn identity(self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisRasterError {
    SizeOverflow,
    PixelCount { expected: u64, actual: usize },
    MaskCount { expected: u64, actual: usize },
    UnspecifiedColorSpace,
}

fn raster_identity(
    dimensions: ImageDimensions,
    color: ColorEncoding,
    pixels: &[RgbaF32Pixel],
    established: Option<[u8; 32]>,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.analysis.raster.v1");
    hasher.update(dimensions.width().to_le_bytes());
    hasher.update(dimensions.height().to_le_bytes());
    hasher.update(postcard::to_allocvec(&color).expect("closed color encoding serializes"));
    if let Some(identity) = established {
        hasher.update([1]);
        hasher.update(identity);
    } else {
        hasher.update([0]);
        for pixel in pixels {
            for value in [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()] {
                hasher.update(value.to_bits().to_le_bytes());
            }
        }
    }
    hasher.finalize().into()
}

const fn color_encoding(value: RgbaF32ColorEncoding) -> ColorEncoding {
    match value {
        RgbaF32ColorEncoding::SrgbD65 => ColorEncoding::SrgbD65,
        RgbaF32ColorEncoding::LinearSrgbD65 => ColorEncoding::LinearSrgbD65,
        RgbaF32ColorEncoding::DisplayP3D65 => ColorEncoding::DisplayP3D65,
        RgbaF32ColorEncoding::LinearDisplayP3D65 => ColorEncoding::LinearDisplayP3D65,
        RgbaF32ColorEncoding::Rec2020D65 => ColorEncoding::Rec2020D65,
        RgbaF32ColorEncoding::LinearRec2020D65 => ColorEncoding::LinearRec2020D65,
        RgbaF32ColorEncoding::AcesCgD60 => ColorEncoding::AcesCgD60,
        RgbaF32ColorEncoding::External(profile) => ColorEncoding::External(profile),
        RgbaF32ColorEncoding::LabD50 => ColorEncoding::LabD50,
    }
}

impl fmt::Display for AnalysisRasterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeOverflow => formatter.write_str("analysis raster size overflowed"),
            Self::PixelCount { expected, actual } => write!(
                formatter,
                "analysis raster has {actual} pixels, expected {expected}"
            ),
            Self::MaskCount { expected, actual } => write!(
                formatter,
                "analysis mask has {actual} values, expected {expected}"
            ),
            Self::UnspecifiedColorSpace => {
                formatter.write_str("analysis raster color space is unspecified")
            }
        }
    }
}

impl std::error::Error for AnalysisRasterError {}
