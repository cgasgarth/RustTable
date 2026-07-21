#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::fmt;

use crate::FiniteF32;

pub const DEFRINGE_COMPATIBILITY_ID: &str = "defringe";
pub const DEFRINGE_ALIAS: &str = "chromatic aberrations";
pub const DEFRINGE_SCHEMA_VERSION: u16 = 1;
pub const DEFRINGE_PARAMETER_BYTES: usize = 12;

pub const DEFRINGE_RADIUS_MIN: f32 = 0.5;
pub const DEFRINGE_RADIUS_MAX: f32 = 20.0;
pub const DEFRINGE_RADIUS_DEFAULT: f32 = 4.0;
pub const DEFRINGE_THRESHOLD_MIN: f32 = 0.5;
pub const DEFRINGE_THRESHOLD_MAX: f32 = 128.0;
pub const DEFRINGE_THRESHOLD_DEFAULT: f32 = 20.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum DefringeMode {
    GlobalAverage = 0,
    LocalAverage = 1,
    Static = 2,
}

impl TryFrom<u32> for DefringeMode {
    type Error = DefringeParameterError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::GlobalAverage),
            1 => Ok(Self::LocalAverage),
            2 => Ok(Self::Static),
            _ => Err(DefringeParameterError::UnknownMode(value)),
        }
    }
}

impl From<DefringeMode> for u32 {
    fn from(value: DefringeMode) -> Self {
        value as Self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefringeParametersV1 {
    pub radius: f32,
    pub threshold: f32,
    pub mode: DefringeMode,
}

impl DefringeParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            radius: DEFRINGE_RADIUS_DEFAULT,
            threshold: DEFRINGE_THRESHOLD_DEFAULT,
            mode: DefringeMode::GlobalAverage,
        }
    }

    #[must_use]
    pub const fn new(radius: f32, threshold: f32, mode: DefringeMode) -> Self {
        Self {
            radius,
            threshold,
            mode,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; DEFRINGE_PARAMETER_BYTES] {
        let mut bytes = [0; DEFRINGE_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.radius.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.threshold.to_le_bytes());
        bytes[8..12].copy_from_slice(&u32::from(self.mode).to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DefringeCodecError> {
        if bytes.len() != DEFRINGE_PARAMETER_BYTES {
            return Err(DefringeCodecError::InvalidLength {
                expected: DEFRINGE_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read_f32 = |offset| {
            f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("checked range"))
        };
        let mode = u32::from_le_bytes(bytes[8..12].try_into().expect("checked range"));
        let mode = mode.try_into().map_err(DefringeCodecError::Parameters)?;
        let value = Self::new(read_f32(0), read_f32(4), mode);
        DefringeConfig::try_from(value).map_err(DefringeCodecError::Parameters)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DefringeHistory {
    V1(DefringeParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl DefringeHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, DefringeCodecError> {
        if version == DEFRINGE_SCHEMA_VERSION {
            Ok(Self::V1(DefringeParametersV1::from_bytes(bytes)?))
        } else {
            Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            })
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(value) => value.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => DEFRINGE_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub const fn executable(&self) -> bool {
        matches!(self, Self::V1(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefringeCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameters(DefringeParameterError),
}

impl fmt::Display for DefringeCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "defringe payload has {actual} bytes; expected {expected}"
            ),
            Self::Parameters(error) => write!(formatter, "invalid defringe parameters: {error}"),
        }
    }
}

impl std::error::Error for DefringeCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefringeParameterError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
    UnknownMode(u32),
}

impl fmt::Display for DefringeParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "defringe {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "defringe {name} is outside its range"),
            Self::UnknownMode(value) => write!(formatter, "defringe mode {value} is unknown"),
        }
    }
}

impl std::error::Error for DefringeParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefringeConfig {
    radius: FiniteF32,
    threshold: FiniteF32,
    mode: DefringeMode,
}

impl TryFrom<DefringeParametersV1> for DefringeConfig {
    type Error = DefringeParameterError;

    fn try_from(value: DefringeParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            radius: bounded(
                "radius",
                value.radius,
                DEFRINGE_RADIUS_MIN,
                DEFRINGE_RADIUS_MAX,
            )?,
            threshold: bounded(
                "threshold",
                value.threshold,
                DEFRINGE_THRESHOLD_MIN,
                DEFRINGE_THRESHOLD_MAX,
            )?,
            mode: value.mode,
        })
    }
}

impl DefringeConfig {
    pub fn new(
        radius: f32,
        threshold: f32,
        mode: DefringeMode,
    ) -> Result<Self, DefringeParameterError> {
        Self::try_from(DefringeParametersV1::new(radius, threshold, mode))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(DefringeParametersV1::defaults()).expect("defringe defaults")
    }

    #[must_use]
    pub const fn radius(self) -> f32 {
        self.radius.get()
    }

    #[must_use]
    pub const fn threshold(self) -> f32 {
        self.threshold.get()
    }

    #[must_use]
    pub const fn mode(self) -> DefringeMode {
        self.mode
    }

    #[must_use]
    pub const fn parameters(self) -> DefringeParametersV1 {
        DefringeParametersV1::new(self.radius(), self.threshold(), self.mode())
    }
}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, DefringeParameterError> {
    let value = FiniteF32::new(value).map_err(|_| DefringeParameterError::NonFinite(name))?;
    if !(minimum..=maximum).contains(&value.get()) {
        return Err(DefringeParameterError::OutOfRange(name));
    }
    Ok(value)
}
