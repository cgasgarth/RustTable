#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::fmt;

use crate::FiniteF32;

pub const CENSORIZE_COMPATIBILITY_ID: &str = "censorize";
pub const CENSORIZE_SCHEMA_VERSION: u16 = 1;
pub const CENSORIZE_PARAMETER_BYTES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CensorizeParametersV1 {
    pub radius_1: f32,
    pub pixelate: f32,
    pub radius_2: f32,
    pub noise: f32,
}

impl CensorizeParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            radius_1: 0.0,
            pixelate: 0.0,
            radius_2: 0.0,
            noise: 0.0,
        }
    }

    #[must_use]
    pub const fn new(radius_1: f32, pixelate: f32, radius_2: f32, noise: f32) -> Self {
        Self {
            radius_1,
            pixelate,
            radius_2,
            noise,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; CENSORIZE_PARAMETER_BYTES] {
        let mut bytes = [0; CENSORIZE_PARAMETER_BYTES];
        for (index, value) in [self.radius_1, self.pixelate, self.radius_2, self.noise]
            .into_iter()
            .enumerate()
        {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CensorizeCodecError> {
        if bytes.len() != CENSORIZE_PARAMETER_BYTES {
            return Err(CensorizeCodecError::InvalidLength {
                expected: CENSORIZE_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read =
            |start| f32::from_le_bytes(bytes[start..start + 4].try_into().expect("checked range"));
        let value = Self::new(read(0), read(4), read(8), read(12));
        CensorizeConfig::try_from(value).map_err(CensorizeCodecError::Parameters)?;
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CensorizeHistory {
    V1(CensorizeParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl CensorizeHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, CensorizeCodecError> {
        if version == CENSORIZE_SCHEMA_VERSION {
            Ok(Self::V1(CensorizeParametersV1::from_bytes(bytes)?))
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
            Self::V1(_) => CENSORIZE_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub const fn executable(&self) -> bool {
        matches!(self, Self::V1(_))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CensorizeCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameters(CensorizeParameterError),
}

impl fmt::Display for CensorizeCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                f,
                "censorize payload has {actual} bytes; expected {expected}"
            ),
            Self::Parameters(error) => write!(f, "invalid censorize parameters: {error}"),
        }
    }
}
impl std::error::Error for CensorizeCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CensorizeParameterError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
}

impl fmt::Display for CensorizeParameterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(f, "censorize {name} is non-finite"),
            Self::OutOfRange(name) => write!(f, "censorize {name} is outside its range"),
        }
    }
}
impl std::error::Error for CensorizeParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CensorizeConfig {
    radius_1: FiniteF32,
    pixelate: FiniteF32,
    radius_2: FiniteF32,
    noise: FiniteF32,
}

impl TryFrom<CensorizeParametersV1> for CensorizeConfig {
    type Error = CensorizeParameterError;
    fn try_from(value: CensorizeParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            radius_1: bounded("radius_1", value.radius_1, 0.0, 500.0)?,
            pixelate: bounded("pixelate", value.pixelate, 0.0, 500.0)?,
            radius_2: bounded("radius_2", value.radius_2, 0.0, 500.0)?,
            noise: bounded("noise", value.noise, 0.0, 1.0)?,
        })
    }
}

impl CensorizeConfig {
    pub fn new(
        radius_1: f32,
        pixelate: f32,
        radius_2: f32,
        noise: f32,
    ) -> Result<Self, CensorizeParameterError> {
        Self::try_from(CensorizeParametersV1::new(
            radius_1, pixelate, radius_2, noise,
        ))
    }
    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(CensorizeParametersV1::defaults()).expect("censorize defaults")
    }
    #[must_use]
    pub const fn radius_1(self) -> f32 {
        self.radius_1.get()
    }
    #[must_use]
    pub const fn pixelate(self) -> f32 {
        self.pixelate.get()
    }
    #[must_use]
    pub const fn radius_2(self) -> f32 {
        self.radius_2.get()
    }
    #[must_use]
    pub const fn noise(self) -> f32 {
        self.noise.get()
    }
    #[must_use]
    pub const fn parameters(self) -> CensorizeParametersV1 {
        CensorizeParametersV1::new(
            self.radius_1(),
            self.pixelate(),
            self.radius_2(),
            self.noise(),
        )
    }
}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, CensorizeParameterError> {
    let value = FiniteF32::new(value).map_err(|_| CensorizeParameterError::NonFinite(name))?;
    if !(minimum..=maximum).contains(&value.get()) {
        return Err(CensorizeParameterError::OutOfRange(name));
    }
    Ok(value)
}
