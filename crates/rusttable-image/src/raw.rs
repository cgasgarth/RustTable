use std::fmt;

use crate::{ImageDimensions, Orientation, Roi};

/// Bayer color at one sensor-site position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CfaColor {
    Red,
    Green,
    Blue,
    Clear,
}

/// A periodic Bayer or X-Trans sensor pattern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CfaPattern {
    Bayer([[CfaColor; 2]; 2]),
    XTrans([[CfaColor; 6]; 6]),
}

/// A CFA pattern and its phase are one inseparable interpretation contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CfaDescriptor {
    pattern: CfaPattern,
    phase: CfaPhase,
}

impl CfaDescriptor {
    #[must_use]
    pub const fn new(pattern: CfaPattern, phase: CfaPhase) -> Self {
        Self { pattern, phase }
    }

    #[must_use]
    pub const fn pattern(self) -> CfaPattern {
        self.pattern
    }

    #[must_use]
    pub const fn phase(self) -> CfaPhase {
        self.phase
    }

    #[must_use]
    pub const fn after_crop(self, roi: Roi) -> Self {
        Self {
            pattern: self.pattern,
            phase: self.pattern.phase_after_crop(self.phase, roi),
        }
    }

    #[must_use]
    pub fn after_orientation(self, source: ImageDimensions, orientation: Orientation) -> Self {
        Self {
            pattern: self.pattern,
            phase: self
                .pattern
                .phase_after_orientation(self.phase, source, orientation),
        }
    }
}

impl CfaPattern {
    #[must_use]
    pub const fn bayer_rggb() -> Self {
        Self::Bayer([
            [CfaColor::Red, CfaColor::Green],
            [CfaColor::Green, CfaColor::Blue],
        ])
    }

    #[must_use]
    pub const fn period(self) -> (u32, u32) {
        match self {
            Self::Bayer(_) => (2, 2),
            Self::XTrans(_) => (6, 6),
        }
    }

    #[must_use]
    pub const fn color_at(self, x: u32, y: u32, phase: CfaPhase) -> CfaColor {
        match self {
            Self::Bayer(pattern) => {
                pattern[((y % 2 + phase.y) % 2) as usize][((x % 2 + phase.x) % 2) as usize]
            }
            Self::XTrans(pattern) => {
                pattern[((y % 6 + phase.y) % 6) as usize][((x % 6 + phase.x) % 6) as usize]
            }
        }
    }

    #[must_use]
    pub const fn phase_after_crop(self, phase: CfaPhase, roi: Roi) -> CfaPhase {
        let (width, height) = self.period();
        CfaPhase {
            x: (phase.x + roi.x()) % width,
            y: (phase.y + roi.y()) % height,
        }
    }

    #[must_use]
    pub fn phase_after_orientation(
        self,
        phase: CfaPhase,
        source: ImageDimensions,
        orientation: Orientation,
    ) -> CfaPhase {
        let output = orientation.output_dimensions(source);
        let (source_x, source_y) = inverse_coordinate(orientation, source, output, 0, 0);
        let (width, height) = self.period();
        CfaPhase {
            x: (phase.x + source_x) % width,
            y: (phase.y + source_y) % height,
        }
    }
}

const fn inverse_coordinate(
    orientation: Orientation,
    source: ImageDimensions,
    _output: ImageDimensions,
    x: u32,
    y: u32,
) -> (u32, u32) {
    let w = source.width() - 1;
    let h = source.height() - 1;
    match orientation {
        Orientation::Normal => (x, y),
        Orientation::FlipHorizontal => (w - x, y),
        Orientation::Rotate180 => (w - x, h - y),
        Orientation::FlipVertical => (x, h - y),
        Orientation::Transpose => (y, x),
        Orientation::Rotate90 => (y, h - x),
        Orientation::Transverse => (w - y, h - x),
        Orientation::Rotate270 => (w - y, x),
    }
}

/// The phase of the periodic CFA at output coordinate `(0, 0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CfaPhase {
    x: u32,
    y: u32,
}

impl CfaPhase {
    #[must_use]
    pub const fn new(x: u32, y: u32, pattern: CfaPattern) -> Self {
        let (width, height) = pattern.period();
        Self {
            x: x % width,
            y: y % height,
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
}

/// Validated black and white points for a RAW mosaic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlackWhiteLevels {
    black: u16,
    white: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlackWhiteLevelsError {
    WhiteNotAboveBlack,
}

impl BlackWhiteLevels {
    /// Creates validated RAW black and white points.
    ///
    /// # Errors
    ///
    /// Returns an error when white is not strictly above black.
    pub const fn new(black: u16, white: u16) -> Result<Self, BlackWhiteLevelsError> {
        if white <= black {
            return Err(BlackWhiteLevelsError::WhiteNotAboveBlack);
        }
        Ok(Self { black, white })
    }

    #[must_use]
    pub const fn black(self) -> u16 {
        self.black
    }

    #[must_use]
    pub const fn white(self) -> u16 {
        self.white
    }
}

/// Owned single-plane u16 sensor data and the metadata needed to interpret it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMosaic {
    dimensions: ImageDimensions,
    row_stride_samples: usize,
    samples: Vec<u16>,
    pattern: CfaPattern,
    phase: CfaPhase,
    levels: BlackWhiteLevels,
    orientation: Orientation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawMosaicError {
    ArithmeticOverflow,
    StrideTooSmall,
    SampleLengthMismatch { expected: usize, actual: usize },
    SampleOutsideWhitePoint { index: usize, value: u16 },
    RoiOutOfBounds,
    EmptyRoi,
}

impl RawMosaic {
    /// Creates a checked, owned single-plane RAW mosaic.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid stride, sample length, arithmetic, or
    /// sample values above the declared white point.
    pub fn new(
        dimensions: ImageDimensions,
        row_stride_samples: usize,
        samples: Vec<u16>,
        pattern: CfaPattern,
        phase: CfaPhase,
        levels: BlackWhiteLevels,
        orientation: Orientation,
    ) -> Result<Self, RawMosaicError> {
        let width =
            usize::try_from(dimensions.width()).map_err(|_| RawMosaicError::ArithmeticOverflow)?;
        if row_stride_samples < width {
            return Err(RawMosaicError::StrideTooSmall);
        }
        let expected = row_stride_samples
            .checked_mul(
                usize::try_from(dimensions.height())
                    .map_err(|_| RawMosaicError::ArithmeticOverflow)?,
            )
            .ok_or(RawMosaicError::ArithmeticOverflow)?;
        if samples.len() != expected {
            return Err(RawMosaicError::SampleLengthMismatch {
                expected,
                actual: samples.len(),
            });
        }
        if let Some((index, value)) = samples
            .iter()
            .copied()
            .enumerate()
            .find(|(_, value)| *value > levels.white())
        {
            return Err(RawMosaicError::SampleOutsideWhitePoint { index, value });
        }
        Ok(Self {
            dimensions,
            row_stride_samples,
            samples,
            pattern,
            phase,
            levels,
            orientation,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn row_stride_samples(&self) -> usize {
        self.row_stride_samples
    }

    #[must_use]
    pub fn samples(&self) -> &[u16] {
        &self.samples
    }

    #[must_use]
    pub const fn pattern(&self) -> CfaPattern {
        self.pattern
    }

    #[must_use]
    pub const fn phase(&self) -> CfaPhase {
        self.phase
    }

    #[must_use]
    pub const fn cfa(&self) -> CfaDescriptor {
        CfaDescriptor::new(self.pattern, self.phase)
    }

    #[must_use]
    pub const fn levels(&self) -> BlackWhiteLevels {
        self.levels
    }

    #[must_use]
    pub const fn orientation(&self) -> Orientation {
        self.orientation
    }

    /// Returns an owned cropped mosaic and updates the periodic CFA phase.
    ///
    /// # Errors
    ///
    /// Returns an error when the ROI is out of bounds, empty, or arithmetic
    /// cannot be represented.
    pub fn crop(&self, roi: Roi) -> Result<Self, RawMosaicError> {
        if roi.is_empty() {
            return Err(RawMosaicError::EmptyRoi);
        }
        roi.within(self.dimensions)
            .map_err(|_| RawMosaicError::RoiOutOfBounds)?;
        let width = usize::try_from(roi.width()).map_err(|_| RawMosaicError::ArithmeticOverflow)?;
        let mut cropped = Vec::with_capacity(
            width
                .checked_mul(usize::try_from(roi.height()).unwrap_or(usize::MAX))
                .ok_or(RawMosaicError::ArithmeticOverflow)?,
        );
        let row_stride = self.row_stride_samples;
        for row in roi.y()..roi.bottom() {
            let start = usize::try_from(row)
                .map_err(|_| RawMosaicError::ArithmeticOverflow)?
                .checked_mul(row_stride)
                .and_then(|offset| offset.checked_add(usize::try_from(roi.x()).ok()?))
                .ok_or(RawMosaicError::ArithmeticOverflow)?;
            cropped.extend_from_slice(&self.samples[start..start + width]);
        }
        let dimensions = ImageDimensions::new(roi.width(), roi.height()).map_err(|_| {
            RawMosaicError::SampleLengthMismatch {
                expected: 1,
                actual: 0,
            }
        })?;
        let phase = self.pattern.phase_after_crop(self.phase, roi);
        Self::new(
            dimensions,
            width,
            cropped,
            self.pattern,
            phase,
            self.levels,
            self.orientation,
        )
    }
}

impl fmt::Display for BlackWhiteLevelsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RAW white point must be above black level")
    }
}

impl std::error::Error for BlackWhiteLevelsError {}

impl fmt::Display for RawMosaicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => formatter.write_str("RAW mosaic arithmetic overflowed"),
            Self::StrideTooSmall => formatter.write_str("RAW mosaic stride is too small"),
            Self::SampleLengthMismatch { expected, actual } => {
                write!(
                    formatter,
                    "RAW mosaic has {actual} samples, expected {expected}"
                )
            }
            Self::SampleOutsideWhitePoint { index, value } => {
                write!(
                    formatter,
                    "RAW sample {index} has value {value} above white point"
                )
            }
            Self::RoiOutOfBounds => formatter.write_str("RAW crop ROI is out of bounds"),
            Self::EmptyRoi => formatter.write_str("RAW crop ROI must be nonempty"),
        }
    }
}

impl std::error::Error for RawMosaicError {}
