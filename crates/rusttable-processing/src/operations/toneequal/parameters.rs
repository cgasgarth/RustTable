//! ABI-shaped parameters from `src/iop/toneequal.c`.

#![forbid(unsafe_code)]
#![allow(clippy::excessive_precision)]

use std::fmt;

pub const PARAMETER_VERSION: u16 = 2;
pub const PARAMETER_BYTES: usize = 72;
pub const LEGACY_V1_BYTES: usize = 64;
pub const CHANNELS: usize = 9;
pub const PIXEL_CHANNELS: usize = 8;
pub const LUT_RESOLUTION: usize = 10_000;
pub const LUT_ENTRIES: usize = PIXEL_CHANNELS * LUT_RESOLUTION + 1;
pub const MIN_EV: f32 = -8.0;
pub const MAX_EV: f32 = 0.0;
pub const MIN_FLOAT: f32 = 0.000_015_258_789_062_5;
pub const CONTRAST_FULCRUM: f32 = 0.0625;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum DetailsFilter {
    None = 0,
    AveragedGuided = 1,
    Guided = 2,
    AveragedEigf = 3,
    Eigf = 4,
}

impl DetailsFilter {
    pub const fn from_raw(value: i32) -> Result<Self, ParameterError> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::AveragedGuided),
            2 => Ok(Self::Guided),
            3 => Ok(Self::AveragedEigf),
            4 => Ok(Self::Eigf),
            other => Err(ParameterError::InvalidDetails(other)),
        }
    }

    pub const fn raw(self) -> i32 {
        self as i32
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum LuminanceMethod {
    Mean = 0,
    Lightness = 1,
    Value = 2,
    Norm1 = 3,
    Norm2 = 4,
    NormPower = 5,
    Geomean = 6,
}

impl LuminanceMethod {
    pub const fn from_raw(value: i32) -> Result<Self, ParameterError> {
        match value {
            0 => Ok(Self::Mean),
            1 => Ok(Self::Lightness),
            2 => Ok(Self::Value),
            3 => Ok(Self::Norm1),
            4 => Ok(Self::Norm2),
            5 => Ok(Self::NormPower),
            6 => Ok(Self::Geomean),
            other => Err(ParameterError::InvalidMethod(other)),
        }
    }

    pub const fn raw(self) -> i32 {
        self as i32
    }
}

/// Current native v2 parameter ordering, including enum fields as four-byte
/// C integers. The order is intentionally the declaration order in
/// `dt_iop_toneequalizer_params_t`, not a convenient Rust grouping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneEqualizerParametersV2 {
    pub noise: f32,
    pub ultra_deep_blacks: f32,
    pub deep_blacks: f32,
    pub blacks: f32,
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub whites: f32,
    pub speculars: f32,
    pub blending: f32,
    pub smoothing: f32,
    pub feathering: f32,
    pub quantization: f32,
    pub contrast_boost: f32,
    pub exposure_boost: f32,
    pub details: DetailsFilter,
    pub method: LuminanceMethod,
    pub iterations: i32,
}

impl Default for ToneEqualizerParametersV2 {
    fn default() -> Self {
        Self {
            noise: 0.0,
            ultra_deep_blacks: 0.0,
            deep_blacks: 0.0,
            blacks: 0.0,
            shadows: 0.0,
            midtones: 0.0,
            highlights: 0.0,
            whites: 0.0,
            speculars: 0.0,
            // No `$DEFAULT` is present for blending in the native source;
            // C introspection therefore supplies its zero value.
            blending: 0.0,
            smoothing: std::f32::consts::SQRT_2,
            feathering: 1.0,
            quantization: 0.0,
            contrast_boost: 0.0,
            exposure_boost: 0.0,
            details: DetailsFilter::Eigf,
            method: LuminanceMethod::Norm2,
            iterations: 1,
        }
    }
}

impl ToneEqualizerParametersV2 {
    pub const fn from_values(
        exposures: [f32; CHANNELS],
        blending: f32,
        smoothing: f32,
        feathering: f32,
        quantization: f32,
        contrast_boost: f32,
        exposure_boost: f32,
        details: DetailsFilter,
        method: LuminanceMethod,
        iterations: i32,
    ) -> Self {
        Self {
            noise: exposures[0],
            ultra_deep_blacks: exposures[1],
            deep_blacks: exposures[2],
            blacks: exposures[3],
            shadows: exposures[4],
            midtones: exposures[5],
            highlights: exposures[6],
            whites: exposures[7],
            speculars: exposures[8],
            blending,
            smoothing,
            feathering,
            quantization,
            contrast_boost,
            exposure_boost,
            details,
            method,
            iterations,
        }
    }

    #[must_use]
    pub const fn exposures(self) -> [f32; CHANNELS] {
        [
            self.noise,
            self.ultra_deep_blacks,
            self.deep_blacks,
            self.blacks,
            self.shadows,
            self.midtones,
            self.highlights,
            self.whites,
            self.speculars,
        ]
    }

    pub fn validate(self) -> Result<(), ParameterError> {
        if !self.smoothing.is_finite() || self.smoothing <= 0.0 {
            return Err(ParameterError::InvalidSmoothing(self.smoothing));
        }
        if !self.feathering.is_finite() || self.feathering <= 0.0 {
            return Err(ParameterError::InvalidFeathering(self.feathering));
        }
        if !(1..=20).contains(&self.iterations) {
            return Err(ParameterError::InvalidIterations(self.iterations));
        }
        if !self.quantization.is_finite() || self.quantization < 0.0 {
            return Err(ParameterError::InvalidQuantization(self.quantization));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParameterError {
    InvalidDetails(i32),
    InvalidMethod(i32),
    InvalidSmoothing(f32),
    InvalidFeathering(f32),
    InvalidQuantization(f32),
    InvalidIterations(i32),
}

impl fmt::Display for ParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDetails(value) => {
                write!(formatter, "invalid Tone Equalizer details value {value}")
            }
            Self::InvalidMethod(value) => {
                write!(formatter, "invalid Tone Equalizer luminance method {value}")
            }
            Self::InvalidSmoothing(value) => {
                write!(formatter, "invalid Tone Equalizer smoothing {value}")
            }
            Self::InvalidFeathering(value) => {
                write!(formatter, "invalid Tone Equalizer feathering {value}")
            }
            Self::InvalidQuantization(value) => {
                write!(formatter, "invalid Tone Equalizer quantization {value}")
            }
            Self::InvalidIterations(value) => {
                write!(formatter, "invalid Tone Equalizer iteration count {value}")
            }
        }
    }
}

impl std::error::Error for ParameterError {}
