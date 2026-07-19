use std::fmt;

use crate::ImageDimensions;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExifOrientation {
    Identity,
    MirrorHorizontal,
    Rotate180,
    MirrorVertical,
    Transpose,
    Rotate90,
    Transverse,
    Rotate270,
}

pub type OrientationTransform = ExifOrientation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrientationError {
    OutOfBounds,
    ArithmeticOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Coordinate {
    x: u32,
    y: u32,
}

impl Coordinate {
    #[must_use]
    pub const fn new(x: u32, y: u32) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }
}

impl ExifOrientation {
    pub const ALL: [Self; 8] = [
        Self::Identity,
        Self::MirrorHorizontal,
        Self::Rotate180,
        Self::MirrorVertical,
        Self::Transpose,
        Self::Rotate90,
        Self::Transverse,
        Self::Rotate270,
    ];

    #[must_use]
    /// Returns the dimensions after applying this logical transform.
    pub fn output_dimensions(self, source: ImageDimensions) -> ImageDimensions {
        match self {
            Self::Transpose | Self::Rotate90 | Self::Transverse | Self::Rotate270 => {
                match ImageDimensions::new(source.height(), source.width()) {
                    Ok(dimensions) => dimensions,
                    Err(_) => source,
                }
            }
            Self::Identity | Self::MirrorHorizontal | Self::Rotate180 | Self::MirrorVertical => {
                source
            }
        }
    }

    /// Maps one source coordinate to its transformed output coordinate.
    ///
    /// # Errors
    ///
    /// Returns an error when the source coordinate is outside the image.
    pub fn source_to_output(
        self,
        source: ImageDimensions,
        coordinate: Coordinate,
    ) -> Result<Coordinate, OrientationError> {
        check_coordinate(source, coordinate)?;
        let x = coordinate.x();
        let y = coordinate.y();
        Ok(match self {
            Self::Identity => Coordinate::new(x, y),
            Self::MirrorHorizontal => Coordinate::new(source.width() - 1 - x, y),
            Self::Rotate180 => Coordinate::new(source.width() - 1 - x, source.height() - 1 - y),
            Self::MirrorVertical => Coordinate::new(x, source.height() - 1 - y),
            Self::Transpose => Coordinate::new(y, x),
            Self::Rotate90 => Coordinate::new(source.height() - 1 - y, x),
            Self::Transverse => Coordinate::new(source.height() - 1 - y, source.width() - 1 - x),
            Self::Rotate270 => Coordinate::new(y, source.width() - 1 - x),
        })
    }

    /// Maps one transformed output coordinate back to source coordinates.
    ///
    /// # Errors
    ///
    /// Returns an error when the output coordinate is outside the transformed image.
    pub fn output_to_source(
        self,
        source: ImageDimensions,
        coordinate: Coordinate,
    ) -> Result<Coordinate, OrientationError> {
        let output = self.output_dimensions(source);
        check_coordinate(output, coordinate)?;
        let x = coordinate.x();
        let y = coordinate.y();
        Ok(match self {
            Self::Identity => Coordinate::new(x, y),
            Self::MirrorHorizontal => Coordinate::new(source.width() - 1 - x, y),
            Self::Rotate180 => Coordinate::new(source.width() - 1 - x, source.height() - 1 - y),
            Self::MirrorVertical => Coordinate::new(x, source.height() - 1 - y),
            Self::Transpose => Coordinate::new(y, x),
            Self::Rotate90 => Coordinate::new(y, source.height() - 1 - x),
            Self::Transverse => Coordinate::new(source.width() - 1 - y, source.height() - 1 - x),
            Self::Rotate270 => Coordinate::new(source.width() - 1 - y, x),
        })
    }
}

fn check_coordinate(
    dimensions: ImageDimensions,
    coordinate: Coordinate,
) -> Result<(), OrientationError> {
    if coordinate.x() >= dimensions.width() || coordinate.y() >= dimensions.height() {
        return Err(OrientationError::OutOfBounds);
    }
    Ok(())
}

impl fmt::Display for OrientationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OutOfBounds => "orientation coordinate is outside image bounds",
            Self::ArithmeticOverflow => "orientation arithmetic overflowed",
        })
    }
}

impl std::error::Error for OrientationError {}
