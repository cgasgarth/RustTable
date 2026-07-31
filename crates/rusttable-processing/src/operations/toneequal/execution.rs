//! Bounded CPU Tone Equalizer execution ported from `process()`,
//! `modify_roi_in()`, and the operation-local numerical helpers in
//! `src/iop/toneequal.c`.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::float_cmp,
    clippy::large_types_passed_by_value,
    clippy::manual_midpoint,
    clippy::struct_excessive_bools,
    clippy::unused_self,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::too_many_arguments,
    reason = "source-shaped f32 raster loops preserve native processing order"
)]

use std::fmt;

use super::filters::{FilterBlend, FilterError, alloc_f32, eigf_surface_blur, guided_surface_blur};
use super::math::{compute_correction_lut, compute_factors};
use super::parameters::{
    CONTRAST_FULCRUM, DetailsFilter, LuminanceMethod, MAX_EV, MIN_EV, MIN_FLOAT, ParameterError,
    ToneEqualizerParametersV2,
};

pub const OPERATION_NAME: &str = "tone equalizer";
pub const DEFAULT_COLORSPACE: &str = "RGB";
pub const DESCRIPTION: &str = "relight the scene as if the lighting was done directly on the scene";
pub const SUPPORTS_BLENDING: bool = true;
pub const GPU_SUPPORTED: bool = false;
pub const GTK_SUPPORTED: bool = false;
pub const TILING_SUPPORTED: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToneEqualizerCapabilities {
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub gtk_supported: bool,
    pub masks_consumed: bool,
    pub outer_blend_deferred: bool,
    pub profile_transform: bool,
    pub independent_tiles_supported: bool,
}

#[must_use]
pub const fn capabilities() -> ToneEqualizerCapabilities {
    ToneEqualizerCapabilities {
        cpu_supported: true,
        gpu_supported: GPU_SUPPORTED,
        gtk_supported: GTK_SUPPORTED,
        masks_consumed: false,
        outer_blend_deferred: true,
        profile_transform: false,
        independent_tiles_supported: TILING_SUPPORTED,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneEqualizerPixel {
    channels: [f32; 4],
}

impl ToneEqualizerPixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            channels: [red, green, blue, alpha],
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToneEqualizerTile {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    pub overlap: usize,
}

impl ToneEqualizerTile {
    #[must_use]
    pub const fn new(x: usize, y: usize, width: usize, height: usize, overlap: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
            overlap,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneEqualizerOutputMode {
    Corrected,
    LuminanceMask,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToneEqualizerExecution {
    pub pixels: Vec<ToneEqualizerPixel>,
    pub input_format_problem: bool,
}

/// The retained operation has no tiling callback. Guided box windows are
/// finite, but EIGF's recursive Gaussian has image-wide state, so independent
/// tiles would not be equivalent to native processing. The bounded leaf only
/// accepts one whole-raster tile until the pixelpipe owner supplies the native
/// ROI/overlap publication contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneEqualizerTileContract {
    WholeRasterOnly,
}

#[derive(Debug, Clone)]
pub struct ToneEqualizerPlan {
    parameters: ToneEqualizerParametersV2,
    factors: [f32; super::parameters::PIXEL_CHANNELS],
    correction_lut: Box<[f32; super::parameters::LUT_ENTRIES]>,
}

impl ToneEqualizerPlan {
    pub fn new(parameters: ToneEqualizerParametersV2) -> Result<Self, ToneEqualizerExecutionError> {
        parameters.validate()?;
        let factors = compute_factors(&parameters)
            .ok_or(ToneEqualizerExecutionError::UnstableInterpolation)?;
        let correction_lut = compute_correction_lut(&factors, parameters.smoothing);
        Ok(Self {
            parameters,
            factors,
            correction_lut,
        })
    }

    #[must_use]
    pub const fn parameters(&self) -> &ToneEqualizerParametersV2 {
        &self.parameters
    }

    #[must_use]
    pub const fn factors(&self) -> &[f32; super::parameters::PIXEL_CHANNELS] {
        &self.factors
    }

    #[must_use]
    pub fn correction_lut(&self) -> &[f32] {
        self.correction_lut.as_slice()
    }

    #[must_use]
    pub fn radius_for(&self, image_width: usize, image_height: usize, roi_scale: f32) -> usize {
        let max_size = image_width.max(image_height) as f32;
        let diameter = self.parameters.blending / 100.0 * max_size * roi_scale;
        let radius = ((diameter - 1.0) / 2.0) as i32;
        radius.max(0) as usize
    }

    #[must_use]
    pub fn tile_contract(&self) -> ToneEqualizerTileContract {
        ToneEqualizerTileContract::WholeRasterOnly
    }

    /// Executes the native RGBA path into a new publication candidate. The
    /// destination is returned only after the complete luminance mask and RGB
    /// raster have succeeded; cancellation and allocation failures cannot
    /// expose a partially filled result.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[ToneEqualizerPixel],
        width: usize,
        height: usize,
        roi_scale: f32,
        output_mode: ToneEqualizerOutputMode,
        cancelled: F,
    ) -> Result<ToneEqualizerExecution, ToneEqualizerExecutionError> {
        self.execute_required_format_with_cancel(
            input,
            width,
            height,
            4,
            roi_scale,
            output_mode,
            cancelled,
        )
    }

    /// Models native `piece->colors != 4` fail-closed behavior without
    /// publishing an uninitialized destination in the safe Rust API.
    pub fn execute_required_format_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[ToneEqualizerPixel],
        width: usize,
        height: usize,
        channels: usize,
        roi_scale: f32,
        output_mode: ToneEqualizerOutputMode,
        mut cancelled: F,
    ) -> Result<ToneEqualizerExecution, ToneEqualizerExecutionError> {
        if channels != 4 {
            return Ok(ToneEqualizerExecution {
                pixels: input.to_vec(),
                input_format_problem: true,
            });
        }
        let expected = width
            .checked_mul(height)
            .ok_or(ToneEqualizerExecutionError::InvalidDimensions)?;
        if width == 0 || height == 0 || input.len() != expected {
            return Err(ToneEqualizerExecutionError::InvalidDimensions);
        }
        if cancelled() {
            return Err(ToneEqualizerExecutionError::Cancelled);
        }

        let mut luminance = alloc_f32(expected).map_err(ToneEqualizerExecutionError::from)?;
        self.compute_luminance_mask(
            input,
            &mut luminance,
            width,
            height,
            roi_scale,
            &mut cancelled,
        )?;

        let mut output = Vec::new();
        output
            .try_reserve_exact(expected)
            .map_err(|_| ToneEqualizerExecutionError::AllocationFailed { elements: expected })?;
        output.resize(expected, ToneEqualizerPixel::from_channels([0.0; 4]));
        for index in 0..expected {
            if index % 1024 == 0 && cancelled() {
                return Err(ToneEqualizerExecutionError::Cancelled);
            }
            let source = input[index].channels;
            let channels = match output_mode {
                ToneEqualizerOutputMode::Corrected => {
                    let exposure = luminance[index].log2().clamp(MIN_EV, MAX_EV);
                    let lut_index = ((exposure - MIN_EV) * super::parameters::LUT_RESOLUTION as f32)
                        .round() as usize;
                    let correction = self.correction_lut[lut_index];
                    // Native for_each_channel uses four lanes in the normal build;
                    // correction therefore applies to RGBA, not RGB with conventionally preserved alpha.
                    [
                        source[0] * correction,
                        source[1] * correction,
                        source[2] * correction,
                        source[3] * correction,
                    ]
                }
                ToneEqualizerOutputMode::LuminanceMask => {
                    let intensity = ((luminance[index] - 0.003_906_25).max(0.0) / 0.996_093_75)
                        .min(1.0)
                        .sqrt();
                    [intensity, intensity, intensity, source[3]]
                }
            };
            output[index] = ToneEqualizerPixel::from_channels(channels);
        }
        Ok(ToneEqualizerExecution {
            pixels: output,
            input_format_problem: false,
        })
    }

    /// Validates the native bounded leaf's only currently faithful tile
    /// schedule. A caller must provide the complete image because EIGF's
    /// recursive Gaussian cannot be reproduced from a finite halo.
    pub fn execute_tiles_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[ToneEqualizerPixel],
        width: usize,
        height: usize,
        tiles: &[ToneEqualizerTile],
        roi_scale: f32,
        output_mode: ToneEqualizerOutputMode,
        cancelled: F,
    ) -> Result<ToneEqualizerExecution, ToneEqualizerExecutionError> {
        let tile = tiles
            .first()
            .copied()
            .ok_or(ToneEqualizerExecutionError::InvalidTile)?;
        if tiles.len() != 1
            || tile.x != 0
            || tile.y != 0
            || tile.width != width
            || tile.height != height
            || tile.overlap != 0
        {
            return Err(ToneEqualizerExecutionError::InvalidTile);
        }
        self.execute_with_cancel(input, width, height, roi_scale, output_mode, cancelled)
    }

    fn compute_luminance_mask(
        &self,
        input: &[ToneEqualizerPixel],
        luminance: &mut [f32],
        width: usize,
        height: usize,
        roi_scale: f32,
        cancelled: &mut impl FnMut() -> bool,
    ) -> Result<(), ToneEqualizerExecutionError> {
        let contrast = match self.parameters.details {
            DetailsFilter::Guided | DetailsFilter::Eigf => {
                2.0_f32.powf(self.parameters.contrast_boost)
            }
            DetailsFilter::None | DetailsFilter::AveragedGuided | DetailsFilter::AveragedEigf => {
                1.0
            }
        };
        let fulcrum = match self.parameters.details {
            DetailsFilter::Guided | DetailsFilter::Eigf => CONTRAST_FULCRUM,
            DetailsFilter::None | DetailsFilter::AveragedGuided | DetailsFilter::AveragedEigf => {
                0.0
            }
        };
        let exposure_boost = 2.0_f32.powf(self.parameters.exposure_boost);
        for (index, pixel) in input.iter().enumerate() {
            if index % 1024 == 0 && cancelled() {
                return Err(ToneEqualizerExecutionError::Cancelled);
            }
            let rgb = pixel.channels;
            let base = match self.parameters.method {
                LuminanceMethod::Mean => (rgb[0] + rgb[1] + rgb[2]) / 3.0,
                LuminanceMethod::Lightness => {
                    (rgb[0].max(rgb[1]).max(rgb[2]) + rgb[0].min(rgb[1]).min(rgb[2])) / 2.0
                }
                LuminanceMethod::Value => rgb[0].max(rgb[1]).max(rgb[2]),
                LuminanceMethod::Norm1 => rgb[0].abs() + rgb[1].abs() + rgb[2].abs(),
                LuminanceMethod::Norm2 => {
                    (rgb[0] * rgb[0] + rgb[1] * rgb[1] + rgb[2] * rgb[2]).sqrt()
                }
                LuminanceMethod::NormPower => {
                    let red = rgb[0].abs();
                    let green = rgb[1].abs();
                    let blue = rgb[2].abs();
                    let denominator = red * red + green * green + blue * blue;
                    if denominator == 0.0 {
                        f32::NAN
                    } else {
                        (red * red * red + green * green * green + blue * blue * blue) / denominator
                    }
                }
                LuminanceMethod::Geomean => {
                    (rgb[0].abs() * rgb[1].abs() * rgb[2].abs()).powf(1.0 / 3.0)
                }
            };
            let value = (base * exposure_boost - fulcrum) * contrast + fulcrum;
            luminance[index] = if value.is_nan() || value < MIN_FLOAT {
                MIN_FLOAT
            } else {
                value
            };
        }

        let radius = self.radius_for(width, height, roi_scale);
        match self.parameters.details {
            DetailsFilter::None => Ok(()),
            DetailsFilter::AveragedGuided => guided_surface_blur(
                luminance,
                width,
                height,
                radius,
                1.0 / self.parameters.feathering,
                self.parameters.iterations as usize,
                FilterBlend::Geomean,
                self.parameters.quantization,
                2f32.powi(-14),
                4.0,
                cancelled,
            )
            .map_err(ToneEqualizerExecutionError::from),
            DetailsFilter::Guided => guided_surface_blur(
                luminance,
                width,
                height,
                radius,
                1.0 / self.parameters.feathering,
                self.parameters.iterations as usize,
                FilterBlend::Linear,
                self.parameters.quantization,
                2f32.powi(-14),
                4.0,
                cancelled,
            )
            .map_err(ToneEqualizerExecutionError::from),
            DetailsFilter::AveragedEigf => eigf_surface_blur(
                luminance,
                width,
                height,
                radius as f32,
                1.0 / self.parameters.feathering,
                self.parameters.iterations as usize,
                FilterBlend::Geomean,
                self.parameters.quantization,
                2f32.powi(-14),
                4.0,
                cancelled,
            )
            .map_err(ToneEqualizerExecutionError::from),
            DetailsFilter::Eigf => eigf_surface_blur(
                luminance,
                width,
                height,
                radius as f32,
                1.0 / self.parameters.feathering,
                self.parameters.iterations as usize,
                FilterBlend::Linear,
                self.parameters.quantization,
                2f32.powi(-14),
                4.0,
                cancelled,
            )
            .map_err(ToneEqualizerExecutionError::from),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToneEqualizerExecutionError {
    Parameters(ParameterError),
    Filter(FilterError),
    UnstableInterpolation,
    Cancelled,
    InvalidDimensions,
    InvalidTile,
    AllocationFailed { elements: usize },
}

impl From<ParameterError> for ToneEqualizerExecutionError {
    fn from(error: ParameterError) -> Self {
        Self::Parameters(error)
    }
}

impl From<FilterError> for ToneEqualizerExecutionError {
    fn from(error: FilterError) -> Self {
        match error {
            FilterError::Cancelled => Self::Cancelled,
            FilterError::InvalidDimensions => Self::InvalidDimensions,
            FilterError::AllocationFailed { elements } => Self::AllocationFailed { elements },
        }
    }
}

impl fmt::Display for ToneEqualizerExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parameters(error) => error.fmt(formatter),
            Self::Filter(error) => write!(formatter, "Tone Equalizer filter failed: {error:?}"),
            Self::UnstableInterpolation => {
                formatter.write_str("Tone Equalizer interpolation is unstable")
            }
            Self::Cancelled => formatter.write_str("Tone Equalizer execution was cancelled"),
            Self::InvalidDimensions => {
                formatter.write_str("Tone Equalizer raster dimensions are invalid")
            }
            Self::InvalidTile => {
                formatter.write_str("Tone Equalizer requires one complete raster tile")
            }
            Self::AllocationFailed { elements } => {
                write!(
                    formatter,
                    "Tone Equalizer failed to allocate {elements} float elements"
                )
            }
        }
    }
}

impl std::error::Error for ToneEqualizerExecutionError {}

#[must_use]
pub const fn lut_resolution() -> usize {
    super::parameters::LUT_RESOLUTION
}

#[must_use]
pub const fn channel_count() -> usize {
    4
}

#[must_use]
pub const fn exposure_channel_count() -> usize {
    super::parameters::CHANNELS
}
