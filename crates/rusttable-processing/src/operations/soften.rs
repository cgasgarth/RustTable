//! Darktable-compatible RGB Orton soft focus ported from retained
//! `src/iop/soften.c`.

#![allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc
)]

use std::fmt;

use crate::common::box_filters::{BOX_ITERATIONS, BoxFilterError, box_mean};
use crate::{FiniteF32, LinearRgb, RasterDimensions};

use super::common::{
    OperationExecutionError, ReconstructionBudget, checked_bytes, hsl_to_rgb, rgb_to_hsl,
    validate_shape,
};

pub const SOFTEN_COMPATIBILITY_ID: &str = "soften";
pub const SOFTEN_SCHEMA_VERSION: u16 = 1;
pub const SOFTEN_PARAMETER_BYTES: usize = 16;
pub const SOFTEN_DEFAULT_SIZE: f32 = 50.0;
pub const SOFTEN_DEFAULT_SATURATION: f32 = 100.0;
pub const SOFTEN_DEFAULT_BRIGHTNESS: f32 = 0.33;
pub const SOFTEN_DEFAULT_AMOUNT: f32 = 50.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenParametersV1 {
    pub size: f32,
    pub saturation: f32,
    pub brightness: f32,
    pub amount: f32,
}

impl SoftenParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            size: SOFTEN_DEFAULT_SIZE,
            saturation: SOFTEN_DEFAULT_SATURATION,
            brightness: SOFTEN_DEFAULT_BRIGHTNESS,
            amount: SOFTEN_DEFAULT_AMOUNT,
        }
    }

    #[must_use]
    pub const fn new(size: f32, saturation: f32, brightness: f32, amount: f32) -> Self {
        Self {
            size,
            saturation,
            brightness,
            amount,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; SOFTEN_PARAMETER_BYTES] {
        let mut bytes = [0; SOFTEN_PARAMETER_BYTES];
        for (index, value) in [self.size, self.saturation, self.brightness, self.amount]
            .into_iter()
            .enumerate()
        {
            let start = index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SoftenCodecError> {
        if bytes.len() != SOFTEN_PARAMETER_BYTES {
            return Err(SoftenCodecError::InvalidLength {
                expected: SOFTEN_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read = |start| f32::from_le_bytes(bytes[start..start + 4].try_into().expect("range"));
        let parameters = Self::new(read(0), read(4), read(8), read(12));
        SoftenConfig::try_from(parameters).map_err(SoftenCodecError::Parameters)?;
        Ok(parameters)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SoftenHistory {
    V1(SoftenParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftenCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameters(SoftenParameterError),
}

impl fmt::Display for SoftenCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "soften payload has {actual} bytes; expected {expected}"
                )
            }
            Self::Parameters(error) => write!(formatter, "invalid soften parameters: {error}"),
        }
    }
}

impl std::error::Error for SoftenCodecError {}

impl SoftenHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, SoftenCodecError> {
        if version == SOFTEN_SCHEMA_VERSION {
            Ok(Self::V1(SoftenParametersV1::from_bytes(bytes)?))
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
            Self::V1(_) => SOFTEN_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftenParameterError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
}

impl fmt::Display for SoftenParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "soften {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "soften {name} is outside its range"),
        }
    }
}

impl std::error::Error for SoftenParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoftenConfig {
    size: FiniteF32,
    saturation: FiniteF32,
    brightness: FiniteF32,
    amount: FiniteF32,
}

impl TryFrom<SoftenParametersV1> for SoftenConfig {
    type Error = SoftenParameterError;

    fn try_from(parameters: SoftenParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            size: bounded("size", parameters.size, 0.0, 100.0)?,
            saturation: bounded("saturation", parameters.saturation, 0.0, 100.0)?,
            brightness: bounded("brightness", parameters.brightness, -2.0, 2.0)?,
            amount: bounded("amount", parameters.amount, 0.0, 100.0)?,
        })
    }
}

impl SoftenConfig {
    pub fn new(
        size: f32,
        saturation: f32,
        brightness: f32,
        amount: f32,
    ) -> Result<Self, SoftenParameterError> {
        Self::try_from(SoftenParametersV1::new(
            size, saturation, brightness, amount,
        ))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(SoftenParametersV1::defaults()).expect("soften defaults are valid")
    }

    #[must_use]
    pub const fn size(self) -> f32 {
        self.size.get()
    }

    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.saturation.get()
    }

    #[must_use]
    pub const fn brightness(self) -> f32 {
        self.brightness.get()
    }

    #[must_use]
    pub const fn amount(self) -> f32 {
        self.amount.get()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoftenPlan {
    config: SoftenConfig,
    radius: u32,
}

impl SoftenPlan {
    pub fn new(
        config: SoftenConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        let radius = soften_radius(config.size(), dimensions);
        checked_bytes(
            usize::try_from(dimensions.pixel_count()).map_err(|_| {
                OperationExecutionError::MemoryBudgetExceeded {
                    required: usize::MAX,
                    budget: ReconstructionBudget::default().maximum_bytes(),
                }
            })?,
            4,
            ReconstructionBudget::default(),
        )?;
        Ok(Self { config, radius })
    }

    #[must_use]
    pub const fn radius(&self) -> u32 {
        self.radius
    }

    /// Pre-adjusts a frozen source, applies Darktable's eight-pass, four-channel
    /// box mean, and mixes it with the original source. The adjusted layer is
    /// never reused as the next operation's source.
    pub fn execute(
        &self,
        input: &[LinearRgb],
        dimensions: RasterDimensions,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        validate_shape(dimensions, input)?;
        if self.config.amount().to_bits() == 0.0f32.to_bits() {
            return Ok(input.to_vec());
        }
        let saturation = self.config.saturation() / 100.0;
        let brightness = 2.0f32.powf(self.config.brightness());
        let float_count =
            input
                .len()
                .checked_mul(4)
                .ok_or(OperationExecutionError::MemoryBudgetExceeded {
                    required: usize::MAX,
                    budget: ReconstructionBudget::default().maximum_bytes(),
                })?;
        let required_bytes = float_count.checked_mul(std::mem::size_of::<f32>()).ok_or(
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: ReconstructionBudget::default().maximum_bytes(),
            },
        )?;
        let mut blurred = Vec::new();
        blurred.try_reserve_exact(float_count).map_err(|_| {
            OperationExecutionError::AllocationFailed {
                required: required_bytes,
            }
        })?;
        for (index, pixel) in input.iter().enumerate() {
            let adjusted = adjust(*pixel, saturation, brightness, index)?;
            blurred.extend_from_slice(&[
                adjusted.red().get(),
                adjusted.green().get(),
                adjusted.blue().get(),
                0.0,
            ]);
        }
        let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
        let height = usize::try_from(dimensions.height()).expect("validated height fits usize");
        let radius = usize::try_from(self.radius).expect("soften radius fits usize");
        box_mean(&mut blurred, height, width, 4, radius, BOX_ITERATIONS)
            .map_err(box_filter_error)?;
        let amount = self.config.amount() / 100.0;
        let (blurred_pixels, remainder) = blurred.as_chunks::<4>();
        debug_assert!(remainder.is_empty());
        input
            .iter()
            .zip(blurred_pixels)
            .enumerate()
            .map(|(index, (original, processed))| {
                let red = original.red().get()
                    + (processed[0].clamp(0.0, 1.0) - original.red().get()) * amount;
                let green = original.green().get()
                    + (processed[1].clamp(0.0, 1.0) - original.green().get()) * amount;
                let blue = original.blue().get()
                    + (processed[2].clamp(0.0, 1.0) - original.blue().get()) * amount;
                Ok(LinearRgb::new(
                    finite(red, index, crate::RgbChannel::Red)?,
                    finite(green, index, crate::RgbChannel::Green)?,
                    finite(blue, index, crate::RgbChannel::Blue)?,
                ))
            })
            .collect()
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
            "box mean rejected a validated soften buffer",
        ),
    }
}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, SoftenParameterError> {
    if !value.is_finite() {
        return Err(SoftenParameterError::NonFinite(name));
    }
    if !(minimum..=maximum).contains(&value) {
        return Err(SoftenParameterError::OutOfRange(name));
    }
    Ok(FiniteF32::new(value).expect("finite value was checked"))
}

fn soften_radius(size: f32, dimensions: RasterDimensions) -> u32 {
    let width = dimensions.width() as f32;
    let height = dimensions.height() as f32;
    let maximum = (width.mul_add(width, height * height)).sqrt() * 0.01;
    let base = maximum as u32;
    let requested = (base as f32 * ((size + 1.0).min(100.0) / 100.0)) as u32;
    requested.min(base)
}

fn adjust(
    pixel: LinearRgb,
    saturation: f32,
    brightness: f32,
    index: usize,
) -> Result<LinearRgb, OperationExecutionError> {
    let (hue, mut saturation_value, mut lightness) =
        rgb_to_hsl([pixel.red().get(), pixel.green().get(), pixel.blue().get()]);
    saturation_value = (saturation_value * saturation).clamp(0.0, 1.0);
    lightness = (lightness * brightness).clamp(0.0, 1.0);
    let [red, green, blue] = hsl_to_rgb(hue, saturation_value, lightness);
    Ok(LinearRgb::new(
        finite(red, index, crate::RgbChannel::Red)?,
        finite(green, index, crate::RgbChannel::Green)?,
        finite(blue, index, crate::RgbChannel::Blue)?,
    ))
}

fn finite(
    value: f32,
    pixel: usize,
    channel: crate::RgbChannel,
) -> Result<FiniteF32, OperationExecutionError> {
    FiniteF32::new(value).map_err(|_| OperationExecutionError::NonFiniteResult { pixel, channel })
}
