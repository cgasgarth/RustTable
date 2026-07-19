use std::fmt;

use crate::ImageDimensions;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ImageRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageRectError {
    ZeroWidth,
    ZeroHeight,
    OutsideImage,
    ArithmeticOverflow,
}

impl ImageRect {
    /// Creates a nonempty rectangle wholly contained by image bounds.
    ///
    /// # Errors
    ///
    /// Returns an error for zero dimensions, overflow, or an out-of-bounds
    /// rectangle.
    pub fn new(
        x: u32,
        y: u32,
        width: u32,
        height: u32,
        bounds: ImageDimensions,
    ) -> Result<Self, ImageRectError> {
        if width == 0 {
            return Err(ImageRectError::ZeroWidth);
        }
        if height == 0 {
            return Err(ImageRectError::ZeroHeight);
        }
        let right = x
            .checked_add(width)
            .ok_or(ImageRectError::ArithmeticOverflow)?;
        let bottom = y
            .checked_add(height)
            .ok_or(ImageRectError::ArithmeticOverflow)?;
        if right > bounds.width() || bottom > bounds.height() {
            return Err(ImageRectError::OutsideImage);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
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
    pub fn dimensions(self) -> ImageDimensions {
        ImageDimensions::new(self.width, self.height)
            .unwrap_or_else(|_| ImageDimensions::new(1, 1).unwrap_or_else(|_| unreachable!()))
    }
}

impl fmt::Display for ImageRectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroWidth => "ROI width must be nonzero",
            Self::ZeroHeight => "ROI height must be nonzero",
            Self::OutsideImage => "ROI is outside image bounds",
            Self::ArithmeticOverflow => "ROI arithmetic overflowed",
        })
    }
}

impl std::error::Error for ImageRectError {}
