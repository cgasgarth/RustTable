use std::fmt;

use crate::ImageDimensions;

/// A checked, half-open image rectangle. Zero-area rectangles are valid for
/// operation propagation but cannot produce a non-empty image view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Roi {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoiError {
    Overflow,
    OutOfBounds,
}

impl Roi {
    /// Creates a checked half-open rectangle.
    ///
    /// # Errors
    ///
    /// Returns [`RoiError::Overflow`] when an end coordinate overflows.
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, RoiError> {
        if x.checked_add(width).is_none() || y.checked_add(height).is_none() {
            return Err(RoiError::Overflow);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    /// Validates that this rectangle is inside an image.
    ///
    /// # Errors
    ///
    /// Returns [`RoiError::OutOfBounds`] when an endpoint exceeds the image.
    pub fn within(self, dimensions: ImageDimensions) -> Result<Self, RoiError> {
        if self.right() > dimensions.width() || self.bottom() > dimensions.height() {
            return Err(RoiError::OutOfBounds);
        }
        Ok(self)
    }

    #[must_use]
    pub const fn full(dimensions: ImageDimensions) -> Self {
        Self {
            x: 0,
            y: 0,
            width: dimensions.width(),
            height: dimensions.height(),
        }
    }

    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
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
    pub const fn right(self) -> u32 {
        self.x + self.width
    }

    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.y + self.height
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    #[must_use]
    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        (x <= right && y <= bottom).then(|| Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

impl fmt::Display for RoiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Overflow => "ROI coordinate arithmetic overflowed",
            Self::OutOfBounds => "ROI is outside image dimensions",
        })
    }
}

impl std::error::Error for RoiError {}

/// The eight EXIF orientation transforms. Pixels are never rotated by a
/// descriptor; consumers apply this logical mapping when requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum Orientation {
    Normal = 1,
    FlipHorizontal = 2,
    Rotate180 = 3,
    FlipVertical = 4,
    Transpose = 5,
    Rotate90 = 6,
    Transverse = 7,
    Rotate270 = 8,
}

impl Orientation {
    #[must_use]
    pub fn output_dimensions(self, source: ImageDimensions) -> ImageDimensions {
        match self {
            Self::Transpose | Self::Rotate90 | Self::Transverse | Self::Rotate270 => {
                ImageDimensions::from_nonzero(source.height(), source.width())
            }
            Self::Normal | Self::FlipHorizontal | Self::Rotate180 | Self::FlipVertical => source,
        }
    }

    #[must_use]
    pub const fn map_source_to_output(self, source: ImageDimensions, x: u32, y: u32) -> (u32, u32) {
        let w = source.width() - 1;
        let h = source.height() - 1;
        match self {
            Self::Normal => (x, y),
            Self::FlipHorizontal => (w - x, y),
            Self::Rotate180 => (w - x, h - y),
            Self::FlipVertical => (x, h - y),
            Self::Transpose => (y, x),
            Self::Rotate90 => (h - y, x),
            Self::Transverse => (h - y, w - x),
            Self::Rotate270 => (y, w - x),
        }
    }

    #[must_use]
    pub const fn inverse(self) -> Self {
        match self {
            Self::Rotate90 => Self::Rotate270,
            Self::Rotate270 => Self::Rotate90,
            other => other,
        }
    }
}

impl ImageDimensions {
    pub(crate) fn from_nonzero(width: u32, height: u32) -> Self {
        Self::from_nonzero_parts(width, height)
    }
}
