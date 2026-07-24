//! Darktable-compatible bloom glow ported from retained `src/iop/bloom.c`.

#![allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use std::fmt;

use crate::common::box_filters::{BOX_ITERATIONS, BoxFilterError, box_mean};
use crate::{FiniteF32, RasterDimensions, RgbChannel};

use super::common::{OperationExecutionError, ReconstructionBudget, checked_bytes};

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

/// Four-channel D50 Lab sample in Darktable's native scale: L in 0..100,
/// a/b in -128..128, and an alpha/spare channel in 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomPixel {
    channels: [f32; 4],
}

impl BloomPixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; 4]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
        self.channels
    }

    #[must_use]
    pub const fn lightness(self) -> f32 {
        self.channels[0]
    }

    #[must_use]
    pub const fn a(self) -> f32 {
        self.channels[1]
    }

    #[must_use]
    pub const fn b(self) -> f32 {
        self.channels[2]
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloomPlan {
    config: BloomConfig,
    dimensions: RasterDimensions,
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
        Ok(Self {
            config,
            dimensions,
            radius,
        })
    }

    #[must_use]
    pub const fn radius(self) -> u32 {
        self.radius
    }

    /// Thresholds and blurs D50 Lab L, then screen-blends only L while
    /// preserving a, b, and alpha exactly.
    pub fn execute_lab<F: FnMut() -> bool>(
        &self,
        input: &[BloomPixel],
        mask: Option<&[f32]>,
        opacity: f32,
        mut cancelled: F,
    ) -> Result<Vec<BloomPixel>, OperationExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            }
        })?;
        if expected != input.len() {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        validate_mask(mask, expected)?;
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(OperationExecutionError::NonFiniteResult {
                pixel: 0,
                channel: RgbChannel::Red,
            });
        }
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        if opacity.to_bits() == 0.0f32.to_bits() {
            return Ok(input.to_vec());
        }
        for (pixel_index, pixel) in input.iter().enumerate() {
            for (channel_index, channel) in pixel.channels().into_iter().enumerate() {
                if !channel.is_finite() {
                    return Err(OperationExecutionError::NonFiniteResult {
                        pixel: pixel_index,
                        channel: lab_channel(channel_index),
                    });
                }
            }
        }
        let threshold = self.config.threshold();
        let strength_exponent = (self.config.strength() + 1.0).min(100.0) / 100.0;
        let scale = 1.0 / (-strength_exponent).exp2();
        let mut blurred = input
            .iter()
            .map(|pixel| {
                let lightness = pixel.lightness() * scale;
                if lightness > threshold {
                    lightness
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>();
        let width = usize::try_from(self.dimensions.width()).expect("validated width fits usize");
        let height =
            usize::try_from(self.dimensions.height()).expect("validated height fits usize");
        let radius = usize::try_from(self.radius).expect("bloom radius is at most 256");
        box_mean(&mut blurred, height, width, 1, radius, BOX_ITERATIONS)
            .map_err(box_filter_error)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        let mut output = Vec::with_capacity(expected);
        for (index, (pixel, glow)) in input.iter().zip(blurred).enumerate() {
            if index % width == 0 && cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let candidate = 100.0 - ((100.0 - pixel.lightness()) * (100.0 - glow) / 100.0);
            let coverage = mask.map_or(opacity, |values| values[index] * opacity);
            let lightness = pixel.lightness() + (candidate - pixel.lightness()) * coverage;
            if !lightness.is_finite() {
                return Err(OperationExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: RgbChannel::Red,
                });
            }
            output.push(BloomPixel::new(
                lightness,
                pixel.a(),
                pixel.b(),
                pixel.alpha(),
            ));
        }
        Ok(output)
    }

    /// Executes the native Lab operation without cancellation.
    pub fn execute(
        &self,
        input: &[BloomPixel],
    ) -> Result<Vec<BloomPixel>, OperationExecutionError> {
        self.execute_lab(input, None, 1.0, || false)
    }
}

fn validate_mask(mask: Option<&[f32]>, expected: usize) -> Result<(), OperationExecutionError> {
    if let Some(mask) = mask
        && (mask.len() != expected
            || mask
                .iter()
                .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value)))
    {
        return Err(OperationExecutionError::DimensionsMismatch {
            expected,
            actual: mask.len(),
        });
    }
    Ok(())
}

const fn lab_channel(channel: usize) -> RgbChannel {
    match channel {
        0 => RgbChannel::Red,
        1 => RgbChannel::Green,
        _ => RgbChannel::Blue,
    }
}

fn box_filter_error(error: BoxFilterError) -> OperationExecutionError {
    match error {
        BoxFilterError::AllocationFailed { required_bytes } => {
            OperationExecutionError::AllocationFailed {
                required: required_bytes,
            }
        }
        BoxFilterError::SizeOverflow => OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: ReconstructionBudget::default().maximum_bytes(),
        },
        BoxFilterError::BufferShape { expected, actual } => {
            OperationExecutionError::DimensionsMismatch { expected, actual }
        }
        BoxFilterError::InvalidDimensions { .. }
        | BoxFilterError::UnsupportedChannels { .. }
        | BoxFilterError::ScratchShape { .. }
        | BoxFilterError::NonFiniteInput { .. } => OperationExecutionError::UnsupportedCapability(
            "box mean rejected a validated bloom buffer",
        ),
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
    (256.0 * ((size + 1.0).min(100.0) / 100.0)).min(256.0) as u32
}
