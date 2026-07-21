use crate::{FiniteF32, FiniteF32Error, LinearRgb};
use std::fmt;

use super::{ENLARGECANVAS_PARAMETER_BYTES, ENLARGECANVAS_PARAMETER_VERSION};

/// The five colors exposed by Darktable's v1 enlarge-canvas operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u32)]
pub enum CanvasColor {
    Green = 0,
    Red = 1,
    Blue = 2,
    Black = 3,
    White = 4,
}

impl CanvasColor {
    #[must_use]
    pub fn fill(self) -> CanvasFill {
        match self {
            Self::Green => CanvasFill::rgb(0.0, 1.0, 0.0),
            Self::Red => CanvasFill::rgb(1.0, 0.0, 0.0),
            Self::Blue => CanvasFill::rgb(0.0, 0.0, 1.0),
            Self::Black => CanvasFill::rgb(0.0, 0.0, 0.0),
            Self::White => CanvasFill::rgb(1.0, 1.0, 1.0),
        }
    }
}

impl TryFrom<u32> for CanvasColor {
    type Error = EnlargeCanvasParameterError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Green),
            1 => Ok(Self::Red),
            2 => Ok(Self::Blue),
            3 => Ok(Self::Black),
            4 => Ok(Self::White),
            value => Err(Self::Error::UnknownColor(value)),
        }
    }
}

/// A finite linear-light RGB fill and alpha value for image-contract output.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanvasFill {
    red: FiniteF32,
    green: FiniteF32,
    blue: FiniteF32,
    alpha: FiniteF32,
}

impl CanvasFill {
    #[must_use]
    pub fn rgb(red: f32, green: f32, blue: f32) -> Self {
        Self::new(red, green, blue, 1.0).expect("built-in canvas fill is finite")
    }

    pub fn new(
        red: f32,
        green: f32,
        blue: f32,
        alpha: f32,
    ) -> Result<Self, EnlargeCanvasParameterError> {
        Ok(Self {
            red: finite(red)?,
            green: finite(green)?,
            blue: finite(blue)?,
            alpha: finite(alpha)?,
        })
    }

    #[must_use]
    pub const fn red(self) -> FiniteF32 {
        self.red
    }
    #[must_use]
    pub const fn green(self) -> FiniteF32 {
        self.green
    }
    #[must_use]
    pub const fn blue(self) -> FiniteF32 {
        self.blue
    }
    #[must_use]
    pub const fn alpha(self) -> FiniteF32 {
        self.alpha
    }

    #[must_use]
    pub const fn rgb_pixel(self) -> LinearRgb {
        LinearRgb::new(self.red, self.green, self.blue)
    }
}

/// Current normalized parameters. Percentages use the source image's width
/// or height, and are retained as typed finite values until planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnlargeCanvasConfig {
    percent_left: FiniteF32,
    percent_right: FiniteF32,
    percent_top: FiniteF32,
    percent_bottom: FiniteF32,
    color: CanvasColor,
}

impl EnlargeCanvasConfig {
    pub fn new(
        percent_left: f32,
        percent_right: f32,
        percent_top: f32,
        percent_bottom: f32,
        color: CanvasColor,
    ) -> Result<Self, EnlargeCanvasParameterError> {
        let values = [
            finite_percent(percent_left)?,
            finite_percent(percent_right)?,
            finite_percent(percent_top)?,
            finite_percent(percent_bottom)?,
        ];
        Ok(Self {
            percent_left: values[0],
            percent_right: values[1],
            percent_top: values[2],
            percent_bottom: values[3],
            color,
        })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0, CanvasColor::Green).expect("enlargecanvas defaults are valid")
    }

    #[must_use]
    pub const fn percent_left(self) -> FiniteF32 {
        self.percent_left
    }
    #[must_use]
    pub const fn percent_right(self) -> FiniteF32 {
        self.percent_right
    }
    #[must_use]
    pub const fn percent_top(self) -> FiniteF32 {
        self.percent_top
    }
    #[must_use]
    pub const fn percent_bottom(self) -> FiniteF32 {
        self.percent_bottom
    }
    #[must_use]
    pub const fn color(self) -> CanvasColor {
        self.color
    }

    #[must_use]
    pub fn fill(self) -> CanvasFill {
        self.color.fill()
    }
}

impl Default for EnlargeCanvasConfig {
    fn default() -> Self {
        Self::defaults()
    }
}

/// Explicit, little-endian semantic representation of Darktable v1 history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnlargeCanvasParametersV1 {
    config: EnlargeCanvasConfig,
}

impl EnlargeCanvasParametersV1 {
    #[must_use]
    pub const fn new(config: EnlargeCanvasConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(self) -> EnlargeCanvasConfig {
        self.config
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; ENLARGECANVAS_PARAMETER_BYTES] {
        let mut bytes = [0; ENLARGECANVAS_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.config.percent_left().get().to_le_bytes());
        bytes[4..8].copy_from_slice(&self.config.percent_right().get().to_le_bytes());
        bytes[8..12].copy_from_slice(&self.config.percent_top().get().to_le_bytes());
        bytes[12..16].copy_from_slice(&self.config.percent_bottom().get().to_le_bytes());
        bytes[16..20].copy_from_slice(&(self.config.color() as u32).to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, EnlargeCanvasCodecError> {
        if bytes.len() != ENLARGECANVAS_PARAMETER_BYTES {
            return Err(EnlargeCanvasCodecError::InvalidLength {
                expected: ENLARGECANVAS_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read = |range: std::ops::Range<usize>| {
            f32::from_le_bytes(bytes[range].try_into().expect("checked payload range"))
        };
        let color = u32::from_le_bytes(bytes[16..20].try_into().expect("checked payload range"));
        let color = color
            .try_into()
            .map_err(EnlargeCanvasCodecError::Parameter)?;
        let config =
            EnlargeCanvasConfig::new(read(0..4), read(4..8), read(8..12), read(12..16), color)
                .map_err(EnlargeCanvasCodecError::Parameter)?;
        Ok(Self::new(config))
    }
}

/// Unknown history is retained without attempting a nonportable decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnlargeCanvasHistoryParameters {
    V1(EnlargeCanvasParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

pub fn decode_history(
    version: u16,
    bytes: &[u8],
) -> Result<EnlargeCanvasHistoryParameters, EnlargeCanvasCodecError> {
    match version {
        ENLARGECANVAS_PARAMETER_VERSION => Ok(EnlargeCanvasHistoryParameters::V1(
            EnlargeCanvasParametersV1::from_bytes(bytes)?,
        )),
        _ => Ok(EnlargeCanvasHistoryParameters::Opaque {
            version,
            bytes: bytes.to_vec(),
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnlargeCanvasCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameter(EnlargeCanvasParameterError),
}

impl fmt::Display for EnlargeCanvasCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "enlargecanvas payload has {actual} bytes; expected {expected}"
                )
            }
            Self::Parameter(error) => write!(formatter, "invalid enlargecanvas payload: {error}"),
        }
    }
}

impl std::error::Error for EnlargeCanvasCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnlargeCanvasParameterError {
    NonFinite,
    Negative,
    AboveMaximum,
    UnknownColor(u32),
}

impl fmt::Display for EnlargeCanvasParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "enlargecanvas values must be finite",
            Self::Negative => "enlargecanvas percentages must not be negative",
            Self::AboveMaximum => "enlargecanvas percentages must not exceed 100",
            Self::UnknownColor(_) => "enlargecanvas color is unknown",
        })
    }
}

impl std::error::Error for EnlargeCanvasParameterError {}

fn finite(value: f32) -> Result<FiniteF32, EnlargeCanvasParameterError> {
    FiniteF32::new(value).map_err(|_: FiniteF32Error| EnlargeCanvasParameterError::NonFinite)
}

fn finite_percent(value: f32) -> Result<FiniteF32, EnlargeCanvasParameterError> {
    let value = finite(value)?;
    if value.get() < 0.0 {
        return Err(EnlargeCanvasParameterError::Negative);
    }
    if value.get() > 100.0 {
        return Err(EnlargeCanvasParameterError::AboveMaximum);
    }
    Ok(value)
}
