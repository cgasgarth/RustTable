#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use rusttable_core::FiniteF64;
use std::fmt;

pub const CLAHE_COMPATIBILITY_ID: &str = "clahe";
pub const CLAHE_ALIAS: &str = "old local contrast";
pub const CLAHE_SCHEMA_VERSION: u16 = 1;
pub const CLAHE_PARAMETER_BYTES: usize = 16;
pub const CLAHE_RADIUS_MIN: f64 = 0.0;
pub const CLAHE_RADIUS_MAX: f64 = 256.0;
pub const CLAHE_RADIUS_DEFAULT: f64 = 64.0;
pub const CLAHE_SLOPE_MIN: f64 = 1.0;
pub const CLAHE_SLOPE_MAX: f64 = 3.0;
pub const CLAHE_SLOPE_DEFAULT: f64 = 1.25;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClaheParametersV1 {
    pub radius: f64,
    pub slope: f64,
}

impl ClaheParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            radius: CLAHE_RADIUS_DEFAULT,
            slope: CLAHE_SLOPE_DEFAULT,
        }
    }

    #[must_use]
    pub const fn new(radius: f64, slope: f64) -> Self {
        Self { radius, slope }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; CLAHE_PARAMETER_BYTES] {
        let mut bytes = [0; CLAHE_PARAMETER_BYTES];
        bytes[0..8].copy_from_slice(&self.radius.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.slope.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ClaheCodecError> {
        if bytes.len() != CLAHE_PARAMETER_BYTES {
            return Err(ClaheCodecError::InvalidLength {
                expected: CLAHE_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let radius = f64::from_le_bytes(bytes[0..8].try_into().expect("checked range"));
        let slope = f64::from_le_bytes(bytes[8..16].try_into().expect("checked range"));
        let value = Self::new(radius, slope);
        ClaheConfig::try_from(value).map_err(ClaheCodecError::Parameters)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ClaheHistory {
    V1(ClaheParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ClaheHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ClaheCodecError> {
        if version == CLAHE_SCHEMA_VERSION {
            Ok(Self::V1(ClaheParametersV1::from_bytes(bytes)?))
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
            Self::V1(_) => CLAHE_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub const fn executable(&self) -> bool {
        matches!(self, Self::V1(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaheCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameters(ClaheParameterError),
}

impl fmt::Display for ClaheCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "clahe payload has {actual} bytes; expected {expected}"
                )
            }
            Self::Parameters(error) => write!(formatter, "invalid clahe parameters: {error}"),
        }
    }
}

impl std::error::Error for ClaheCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaheParameterError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
}

impl fmt::Display for ClaheParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "clahe {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "clahe {name} is outside its range"),
        }
    }
}

impl std::error::Error for ClaheParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClaheConfig {
    radius: FiniteF64,
    slope: FiniteF64,
}

impl TryFrom<ClaheParametersV1> for ClaheConfig {
    type Error = ClaheParameterError;

    fn try_from(value: ClaheParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            radius: bounded("radius", value.radius, CLAHE_RADIUS_MIN, CLAHE_RADIUS_MAX)?,
            slope: bounded("slope", value.slope, CLAHE_SLOPE_MIN, CLAHE_SLOPE_MAX)?,
        })
    }
}

impl ClaheConfig {
    pub fn new(radius: f64, slope: f64) -> Result<Self, ClaheParameterError> {
        Self::try_from(ClaheParametersV1::new(radius, slope))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(ClaheParametersV1::defaults()).expect("clahe defaults")
    }

    #[must_use]
    pub const fn radius(self) -> f64 {
        self.radius.get()
    }

    #[must_use]
    pub const fn slope(self) -> f64 {
        self.slope.get()
    }

    #[must_use]
    pub const fn parameters(self) -> ClaheParametersV1 {
        ClaheParametersV1::new(self.radius(), self.slope())
    }
}

fn bounded(
    name: &'static str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<FiniteF64, ClaheParameterError> {
    let value = FiniteF64::new(value).map_err(|_| ClaheParameterError::NonFinite(name))?;
    if !(minimum..=maximum).contains(&value.get()) {
        return Err(ClaheParameterError::OutOfRange(name));
    }
    Ok(value)
}
