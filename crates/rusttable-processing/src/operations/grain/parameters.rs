#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "history codecs are explicit compatibility boundaries"
)]

use std::fmt;

use crate::FiniteF32;

pub const GRAIN_V1_PARAMETER_BYTES: usize = 12;
pub const GRAIN_V2_PARAMETER_BYTES: usize = 16;
pub const GRAIN_LEGACY_PARAMETER_BYTES: usize = 65_616;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum GrainChannel {
    Hue = 0,
    Saturation = 1,
    Lightness = 2,
    Rgb = 3,
}

impl GrainChannel {
    pub fn from_id(id: u32) -> Result<Self, GrainParameterError> {
        match id {
            0 => Ok(Self::Hue),
            1 => Ok(Self::Saturation),
            2 => Ok(Self::Lightness),
            3 => Ok(Self::Rgb),
            _ => Err(GrainParameterError::UnknownChannel(id)),
        }
    }

    #[must_use]
    pub const fn id(self) -> u32 {
        self as u32
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GrainParametersV1 {
    pub channel: GrainChannel,
    pub scale: f32,
    pub strength: f32,
    legacy_tail: Vec<u8>,
}

impl GrainParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            channel: GrainChannel::Lightness,
            scale: 1600.0 / 213.2,
            strength: 25.0,
            legacy_tail: Vec::new(),
        }
    }

    #[must_use]
    pub fn new(channel: GrainChannel, scale: f32, strength: f32) -> Self {
        Self {
            channel,
            scale,
            strength,
            legacy_tail: Vec::new(),
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, GrainCodecError> {
        if bytes.len() != GRAIN_V1_PARAMETER_BYTES && bytes.len() != GRAIN_LEGACY_PARAMETER_BYTES {
            return Err(GrainCodecError::InvalidLength {
                expected: GRAIN_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let channel =
            GrainChannel::from_id(u32::from_le_bytes(bytes[0..4].try_into().expect("channel")))
                .map_err(GrainCodecError::UnknownChannel)?;
        let mut legacy_tail = Vec::new();
        legacy_tail.extend_from_slice(&bytes[GRAIN_V1_PARAMETER_BYTES..]);
        Ok(Self {
            channel,
            scale: f32::from_le_bytes(bytes[4..8].try_into().expect("scale")),
            strength: f32::from_le_bytes(bytes[8..12].try_into().expect("strength")),
            legacy_tail,
        })
    }

    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(GRAIN_V1_PARAMETER_BYTES + self.legacy_tail.len());
        bytes.extend_from_slice(&self.channel.id().to_le_bytes());
        bytes.extend_from_slice(&self.scale.to_le_bytes());
        bytes.extend_from_slice(&self.strength.to_le_bytes());
        bytes.extend_from_slice(&self.legacy_tail);
        bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrainParametersV2 {
    pub channel: GrainChannel,
    pub scale: f32,
    pub strength: f32,
    pub midtones_bias: f32,
}

impl GrainParametersV2 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            channel: GrainChannel::Lightness,
            scale: 1600.0 / 213.2,
            strength: 25.0,
            midtones_bias: 100.0,
        }
    }

    #[must_use]
    pub const fn new(channel: GrainChannel, scale: f32, strength: f32, midtones_bias: f32) -> Self {
        Self {
            channel,
            scale,
            strength,
            midtones_bias,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, GrainCodecError> {
        if bytes.len() != GRAIN_V2_PARAMETER_BYTES {
            return Err(GrainCodecError::InvalidLength {
                expected: GRAIN_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        Ok(Self::new(
            GrainChannel::from_id(u32::from_le_bytes(bytes[0..4].try_into().expect("channel")))
                .map_err(GrainCodecError::UnknownChannel)?,
            f32::from_le_bytes(bytes[4..8].try_into().expect("scale")),
            f32::from_le_bytes(bytes[8..12].try_into().expect("strength")),
            f32::from_le_bytes(bytes[12..16].try_into().expect("midtones")),
        ))
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; GRAIN_V2_PARAMETER_BYTES] {
        let mut bytes = [0; GRAIN_V2_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.channel.id().to_le_bytes());
        bytes[4..8].copy_from_slice(&self.scale.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.strength.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.midtones_bias.to_le_bytes());
        bytes
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GrainHistory {
    V1(GrainParametersV1),
    V2(GrainParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl GrainHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, GrainCodecError> {
        match version {
            1 => match GrainParametersV1::from_bytes(bytes) {
                Ok(parameters) => Ok(Self::V1(parameters)),
                Err(GrainCodecError::UnknownChannel(_)) => Ok(Self::Opaque {
                    version,
                    bytes: bytes.to_vec(),
                }),
                Err(error) => Err(error),
            },
            2 => match GrainParametersV2::from_bytes(bytes) {
                Ok(parameters) => Ok(Self::V2(parameters)),
                Err(GrainCodecError::UnknownChannel(_)) => Ok(Self::Opaque {
                    version,
                    bytes: bytes.to_vec(),
                }),
                Err(error) => Err(error),
            },
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes(),
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn migrate_v1(&self) -> Option<GrainParametersV2> {
        match self {
            Self::V1(parameters) => Some(GrainParametersV2::new(
                parameters.channel,
                parameters.scale,
                parameters.strength,
                0.0,
            )),
            _ => None,
        }
    }

    pub fn current(&self) -> Result<GrainParametersV2, GrainCodecError> {
        match self {
            Self::V2(parameters) => Ok(*parameters),
            Self::V1(_) => Err(GrainCodecError::RequiresMigration),
            Self::Opaque { version, .. } => Err(GrainCodecError::UnsupportedVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrainCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnknownChannel(GrainParameterError),
    RequiresMigration,
    UnsupportedVersion(u16),
}

impl fmt::Display for GrainCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "grain payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnknownChannel(error) => error.fmt(formatter),
            Self::RequiresMigration => formatter.write_str("grain v1 history requires migration"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported grain version {version}")
            }
        }
    }
}

impl std::error::Error for GrainCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrainParameterError {
    UnknownChannel(u32),
    NonFinite(&'static str),
    OutOfRange(&'static str),
}

impl fmt::Display for GrainParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownChannel(id) => write!(formatter, "unknown grain channel {id}"),
            Self::NonFinite(name) => write!(formatter, "grain {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "grain {name} is out of range"),
        }
    }
}

impl std::error::Error for GrainParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GrainConfig {
    channel: GrainChannel,
    scale: FiniteF32,
    strength: FiniteF32,
    midtones_bias: FiniteF32,
    seed: u64,
}

impl TryFrom<GrainParametersV2> for GrainConfig {
    type Error = GrainParameterError;

    fn try_from(parameters: GrainParametersV2) -> Result<Self, Self::Error> {
        Ok(Self {
            channel: parameters.channel,
            scale: bounded("scale", parameters.scale, 20.0 / 213.2, 6400.0 / 213.2)?,
            strength: bounded("strength", parameters.strength, 0.0, 100.0)?,
            midtones_bias: bounded("midtones_bias", parameters.midtones_bias, 0.0, 100.0)?,
            seed: 0,
        })
    }
}

impl GrainConfig {
    pub fn new(parameters: GrainParametersV2) -> Result<Self, GrainParameterError> {
        Self::try_from(parameters)
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(GrainParametersV2::defaults()).expect("grain defaults are valid")
    }

    #[must_use]
    pub const fn channel(self) -> GrainChannel {
        self.channel
    }

    #[must_use]
    pub const fn scale(self) -> FiniteF32 {
        self.scale
    }

    #[must_use]
    pub const fn strength(self) -> FiniteF32 {
        self.strength
    }

    #[must_use]
    pub const fn midtones_bias(self) -> FiniteF32 {
        self.midtones_bias
    }

    #[must_use]
    pub const fn seed(self) -> u64 {
        self.seed
    }

    #[must_use]
    pub const fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    #[must_use]
    pub const fn parameters(self) -> GrainParametersV2 {
        GrainParametersV2::new(
            self.channel,
            self.scale.get(),
            self.strength.get(),
            self.midtones_bias.get(),
        )
    }
}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, GrainParameterError> {
    let value = FiniteF32::new(value).map_err(|_| GrainParameterError::NonFinite(name))?;
    if !(minimum..=maximum).contains(&value.get()) {
        return Err(GrainParameterError::OutOfRange(name));
    }
    Ok(value)
}
