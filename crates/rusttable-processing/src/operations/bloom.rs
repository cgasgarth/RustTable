//! Darktable-compatible bloom glow with an immutable extraction layer.

#![allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use std::fmt;

use crate::{FiniteF32, LinearRgb, RasterDimensions};

use super::common::{
    OperationExecutionError, ReconstructionBudget, checked_bytes, luma, validate_shape,
};
use super::convolution::BoxKernel;

pub const BLOOM_COMPATIBILITY_ID: &str = "bloom";
pub const BLOOM_SCHEMA_VERSION: u16 = 1;
pub const BLOOM_PARAMETER_BYTES: usize = 12;
pub const BLOOM_DEFAULT_SIZE: f32 = 20.0;
pub const BLOOM_DEFAULT_THRESHOLD: f32 = 90.0;
pub const BLOOM_DEFAULT_STRENGTH: f32 = 25.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomParametersV1 {
    pub size: f32,
    pub threshold: f32,
    pub strength: f32,
}

impl BloomParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            size: BLOOM_DEFAULT_SIZE,
            threshold: BLOOM_DEFAULT_THRESHOLD,
            strength: BLOOM_DEFAULT_STRENGTH,
        }
    }

    #[must_use]
    pub const fn new(size: f32, threshold: f32, strength: f32) -> Self {
        Self {
            size,
            threshold,
            strength,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; BLOOM_PARAMETER_BYTES] {
        let mut bytes = [0; BLOOM_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.size.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.threshold.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.strength.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BloomCodecError> {
        if bytes.len() != BLOOM_PARAMETER_BYTES {
            return Err(BloomCodecError::InvalidLength {
                expected: BLOOM_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read = |range: std::ops::Range<usize>| {
            f32::from_le_bytes(bytes[range].try_into().expect("checked range"))
        };
        let parameters = Self::new(read(0..4), read(4..8), read(8..12));
        BloomConfig::try_from(parameters).map_err(BloomCodecError::Parameters)?;
        Ok(parameters)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum BloomHistory {
    V1(BloomParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BloomCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameters(BloomParameterError),
}

impl fmt::Display for BloomCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "bloom payload has {actual} bytes; expected {expected}"
                )
            }
            Self::Parameters(error) => write!(formatter, "invalid bloom parameters: {error}"),
        }
    }
}

impl std::error::Error for BloomCodecError {}

impl BloomHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, BloomCodecError> {
        if version == BLOOM_SCHEMA_VERSION {
            Ok(Self::V1(BloomParametersV1::from_bytes(bytes)?))
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
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => BLOOM_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BloomParameterError {
    NonFinite(&'static str),
    OutOfRange { name: &'static str, value: u32 },
}

impl fmt::Display for BloomParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "bloom {name} is non-finite"),
            Self::OutOfRange { name, value } => write!(formatter, "bloom {name} is {value}%"),
        }
    }
}

impl std::error::Error for BloomParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BloomConfig {
    size: FiniteF32,
    threshold: FiniteF32,
    strength: FiniteF32,
}

impl TryFrom<BloomParametersV1> for BloomConfig {
    type Error = BloomParameterError;

    fn try_from(parameters: BloomParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            size: bounded("size", parameters.size, 0.0, 100.0)?,
            threshold: bounded("threshold", parameters.threshold, 0.0, 100.0)?,
            strength: bounded("strength", parameters.strength, 0.0, 100.0)?,
        })
    }
}

impl BloomConfig {
    pub fn new(size: f32, threshold: f32, strength: f32) -> Result<Self, BloomParameterError> {
        Self::try_from(BloomParametersV1::new(size, threshold, strength))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(BloomParametersV1::defaults()).expect("bloom defaults are valid")
    }

    #[must_use]
    pub const fn size(self) -> f32 {
        self.size.get()
    }

    #[must_use]
    pub const fn threshold(self) -> f32 {
        self.threshold.get()
    }

    #[must_use]
    pub const fn strength(self) -> f32 {
        self.strength.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloomPlan {
    config: BloomConfig,
    radius: u32,
}

impl BloomPlan {
    pub fn new(
        config: BloomConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        let radius = bloom_radius(config.size(), dimensions);
        checked_bytes(
            usize::try_from(dimensions.pixel_count()).map_err(|_| {
                OperationExecutionError::MemoryBudgetExceeded {
                    required: usize::MAX,
                    budget: ReconstructionBudget::default().maximum_bytes(),
                }
            })?,
            3,
            ReconstructionBudget::default(),
        )?;
        Ok(Self { config, radius })
    }

    #[must_use]
    pub const fn radius(self) -> u32 {
        self.radius
    }

    /// Extracts from the frozen source, blurs the extracted luminance, and
    /// screen-blends only the luminance back into RGB, preserving chroma.
    pub fn execute(
        self,
        input: &[LinearRgb],
        dimensions: RasterDimensions,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        validate_shape(dimensions, input)?;
        if self.config.strength().to_bits() == 0.0f32.to_bits() {
            return Ok(input.to_vec());
        }
        let threshold = self.config.threshold();
        let scale = 2.0f32.powf((self.config.strength() + 1.0) / 100.0);
        let extracted = input
            .iter()
            .map(|pixel| {
                let lightness = luma(*pixel) * 100.0;
                if lightness * scale > threshold {
                    lightness * scale
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>();
        let blurred = BoxKernel::new(self.radius).apply_scalar(
            &extracted,
            dimensions,
            ReconstructionBudget::default(),
        )?;
        input
            .iter()
            .zip(blurred)
            .enumerate()
            .map(|(index, (pixel, glow))| {
                let old_luma = luma(*pixel) * 100.0;
                let new_luma = 100.0 - ((100.0 - old_luma) * (100.0 - glow) / 100.0);
                let delta = (new_luma - old_luma) / 100.0;
                let result = LinearRgb::new(
                    FiniteF32::new(pixel.red().get() + delta).map_err(|_| {
                        OperationExecutionError::NonFiniteResult {
                            pixel: index,
                            channel: crate::RgbChannel::Red,
                        }
                    })?,
                    FiniteF32::new(pixel.green().get() + delta).map_err(|_| {
                        OperationExecutionError::NonFiniteResult {
                            pixel: index,
                            channel: crate::RgbChannel::Green,
                        }
                    })?,
                    FiniteF32::new(pixel.blue().get() + delta).map_err(|_| {
                        OperationExecutionError::NonFiniteResult {
                            pixel: index,
                            channel: crate::RgbChannel::Blue,
                        }
                    })?,
                );
                Ok(result)
            })
            .collect()
    }
}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, BloomParameterError> {
    if !value.is_finite() {
        return Err(BloomParameterError::NonFinite(name));
    }
    if !(minimum..=maximum).contains(&value) {
        return Err(BloomParameterError::OutOfRange {
            name,
            value: value.to_bits(),
        });
    }
    Ok(FiniteF32::new(value).expect("finite value was checked"))
}

fn bloom_radius(size: f32, _dimensions: RasterDimensions) -> u32 {
    (256.0 * ((size + 1.0).min(100.0) / 100.0))
        .ceil()
        .min(256.0) as u32
}
