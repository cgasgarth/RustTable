use std::fmt;

use crate::{
    AlphaMode, ChannelLayout, Coordinate, ExifOrientation, ImageDimensions, ImageRect, OwnedPlane,
    PlaneError, SampleType, StorageLayout,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CfaColor {
    Red,
    Green,
    Blue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BayerPattern {
    Rggb,
    Bggr,
    Grbg,
    Gbrg,
}

impl BayerPattern {
    #[must_use]
    pub const fn color_at(self, x: u32, y: u32, phase: CfaPhase) -> CfaColor {
        let x = (x + phase.x() as u32) % 2;
        let y = (y + phase.y() as u32) % 2;
        match (self, x, y) {
            (Self::Rggb, 0, 0) | (Self::Bggr, 1, 1) | (Self::Grbg, 1, 0) | (Self::Gbrg, 0, 1) => {
                CfaColor::Red
            }
            (Self::Rggb, 1, 1) | (Self::Bggr, 0, 0) | (Self::Grbg, 0, 1) | (Self::Gbrg, 1, 0) => {
                CfaColor::Blue
            }
            _ => CfaColor::Green,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct XTransPattern {
    colors: [[CfaColor; 6]; 6],
}

impl XTransPattern {
    pub const FUJIFILM: Self = Self {
        colors: [
            [
                CfaColor::Green,
                CfaColor::Blue,
                CfaColor::Green,
                CfaColor::Green,
                CfaColor::Blue,
                CfaColor::Green,
            ],
            [
                CfaColor::Red,
                CfaColor::Green,
                CfaColor::Red,
                CfaColor::Blue,
                CfaColor::Green,
                CfaColor::Red,
            ],
            [
                CfaColor::Green,
                CfaColor::Blue,
                CfaColor::Green,
                CfaColor::Green,
                CfaColor::Blue,
                CfaColor::Green,
            ],
            [
                CfaColor::Green,
                CfaColor::Green,
                CfaColor::Blue,
                CfaColor::Green,
                CfaColor::Green,
                CfaColor::Blue,
            ],
            [
                CfaColor::Blue,
                CfaColor::Green,
                CfaColor::Red,
                CfaColor::Green,
                CfaColor::Red,
                CfaColor::Green,
            ],
            [
                CfaColor::Green,
                CfaColor::Green,
                CfaColor::Blue,
                CfaColor::Green,
                CfaColor::Green,
                CfaColor::Blue,
            ],
        ],
    };

    #[must_use]
    pub const fn color_at(self, x: u32, y: u32, phase: CfaPhase) -> CfaColor {
        self.colors[((y + phase.y() as u32) % 6) as usize][((x + phase.x() as u32) % 6) as usize]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CfaPhase {
    x: u8,
    y: u8,
}

impl CfaPhase {
    #[must_use]
    pub const fn new(x: u8, y: u8) -> Self {
        Self { x, y }
    }

    #[must_use]
    pub const fn x(self) -> u8 {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> u8 {
        self.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CfaDescriptor {
    Bayer {
        pattern: BayerPattern,
        phase: CfaPhase,
    },
    XTrans {
        pattern: XTransPattern,
        phase: CfaPhase,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CfaError {
    ArithmeticOverflow,
    NonFiniteLevel,
    WhiteLevelNotAboveBlack,
    InvalidRawPlane(PlaneError),
    InvalidRawSampleType,
    InvalidRawLayout,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlackWhiteLevels {
    black: f32,
    white: f32,
}

impl BlackWhiteLevels {
    /// Creates finite sensor black and white calibration levels.
    ///
    /// # Errors
    ///
    /// Returns an error for non-finite levels or a white level that is not
    /// strictly above black.
    pub fn new(black: f32, white: f32) -> Result<Self, CfaError> {
        if !black.is_finite() || !white.is_finite() {
            return Err(CfaError::NonFiniteLevel);
        }
        if white <= black {
            return Err(CfaError::WhiteLevelNotAboveBlack);
        }
        Ok(Self { black, white })
    }

    #[must_use]
    pub const fn black(self) -> f32 {
        self.black
    }

    #[must_use]
    pub const fn white(self) -> f32 {
        self.white
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RawMosaic {
    plane: OwnedPlane,
    cfa: CfaDescriptor,
    levels: BlackWhiteLevels,
}

impl RawMosaic {
    /// Builds a typed single-plane U16 RAW mosaic.
    ///
    /// # Errors
    ///
    /// Returns an error when the plane is not a packed U16 Bayer/X-Trans
    /// plane or when its calibration levels are invalid.
    pub fn new(
        plane: OwnedPlane,
        cfa: CfaDescriptor,
        levels: BlackWhiteLevels,
    ) -> Result<Self, CfaError> {
        let descriptor = plane.descriptor();
        let format = descriptor.format();
        if format.sample_type() != SampleType::U16 {
            return Err(CfaError::InvalidRawSampleType);
        }
        if !matches!(
            format.channels(),
            ChannelLayout::Bayer | ChannelLayout::XTrans
        ) || format.alpha() != AlphaMode::None
        {
            return Err(CfaError::InvalidRawLayout);
        }
        if descriptor.storage() != StorageLayout::Interleaved {
            return Err(CfaError::InvalidRawLayout);
        }
        Ok(Self { plane, cfa, levels })
    }

    #[must_use]
    pub const fn plane(&self) -> &OwnedPlane {
        &self.plane
    }

    #[must_use]
    pub const fn cfa(&self) -> CfaDescriptor {
        self.cfa
    }

    #[must_use]
    pub const fn levels(&self) -> BlackWhiteLevels {
        self.levels
    }
}

impl CfaDescriptor {
    #[must_use]
    pub const fn bayer(pattern: BayerPattern, phase: CfaPhase) -> Self {
        Self::Bayer { pattern, phase }
    }

    #[must_use]
    pub const fn x_trans(pattern: XTransPattern, phase: CfaPhase) -> Self {
        Self::XTrans { pattern, phase }
    }

    #[must_use]
    pub const fn phase(self) -> CfaPhase {
        match self {
            Self::Bayer { phase, .. } | Self::XTrans { phase, .. } => phase,
        }
    }

    #[must_use]
    pub const fn color_at(self, x: u32, y: u32) -> CfaColor {
        match self {
            Self::Bayer { pattern, phase } => pattern.color_at(x, y, phase),
            Self::XTrans { pattern, phase } => pattern.color_at(x, y, phase),
        }
    }

    /// Adjusts the CFA phase for a view crop without copying samples.
    ///
    /// # Errors
    ///
    /// Returns an error when the adjusted phase cannot be represented.
    pub fn crop(self, rect: ImageRect) -> Result<Self, CfaError> {
        let (period_x, period_y) = self.period();
        let phase = self.phase();
        let x = (u32::from(phase.x()) + rect.x()) % period_x;
        let y = (u32::from(phase.y()) + rect.y()) % period_y;
        let phase = CfaPhase::new(
            u8::try_from(x).map_err(|_| CfaError::ArithmeticOverflow)?,
            u8::try_from(y).map_err(|_| CfaError::ArithmeticOverflow)?,
        );
        Ok(match self {
            Self::Bayer { pattern, .. } => Self::Bayer { pattern, phase },
            Self::XTrans { pattern, .. } => Self::XTrans { pattern, phase },
        })
    }

    #[must_use]
    pub const fn oriented(self, orientation: ExifOrientation) -> OrientedCfa {
        OrientedCfa {
            descriptor: self,
            orientation,
        }
    }

    #[must_use]
    const fn period(self) -> (u32, u32) {
        match self {
            Self::Bayer { .. } => (2, 2),
            Self::XTrans { .. } => (6, 6),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrientedCfa {
    descriptor: CfaDescriptor,
    orientation: ExifOrientation,
}

impl OrientedCfa {
    #[must_use]
    pub const fn descriptor(self) -> CfaDescriptor {
        self.descriptor
    }

    #[must_use]
    pub const fn orientation(self) -> ExifOrientation {
        self.orientation
    }

    /// Resolves an output coordinate to its source CFA color.
    ///
    /// # Errors
    ///
    /// Returns an error when the orientation coordinate is invalid.
    pub fn color_at(
        self,
        source_dimensions: ImageDimensions,
        coordinate: Coordinate,
    ) -> Result<CfaColor, CfaError> {
        let source = self
            .orientation
            .output_to_source(source_dimensions, coordinate)
            .map_err(|_| CfaError::ArithmeticOverflow)?;
        Ok(self.descriptor.color_at(source.x(), source.y()))
    }
}

impl fmt::Display for CfaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => formatter.write_str("CFA arithmetic overflowed"),
            Self::NonFiniteLevel => formatter.write_str("CFA calibration level is not finite"),
            Self::WhiteLevelNotAboveBlack => {
                formatter.write_str("CFA white level must exceed black level")
            }
            Self::InvalidRawPlane(error) => error.fmt(formatter),
            Self::InvalidRawSampleType => formatter.write_str("RAW mosaic sample type is not U16"),
            Self::InvalidRawLayout => {
                formatter.write_str("RAW mosaic layout is not a single packed plane")
            }
        }
    }
}

impl std::error::Error for CfaError {}
