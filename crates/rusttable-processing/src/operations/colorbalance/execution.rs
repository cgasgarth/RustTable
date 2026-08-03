//! Immutable Color Balance commit plan, scalar CPU execution, and NORMAL blend.
//!
//! Direct source lineage: `src/iop/colorbalance.c`, `commit_params`,
//! `process`, `_process_legacy`, `_process_lgg`, `_process_sop`, and
//! `src/develop/blends/blendif_lab.c::_blend_normal_unbounded`.

#![expect(
    clippy::suboptimal_flops,
    reason = "Native Color Balance equations preserve source evaluation order and IEEE-754 parity."
)]

use std::fmt;

use super::codec::{
    CHANNEL_BLUE, CHANNEL_FACTOR, CHANNEL_GREEN, CHANNEL_RED, CHANNEL_SIZE, ColorBalanceMode,
    ColorBalanceParametersV3,
};
use super::math;

const MILLION: f32 = 1_000_000.0;
const RGB_GAMMA: f32 = 1.0 / 2.2;
const CPU_STAGE_EPSILON: f32 = 1e-6;
// Native `process` chunks for cache locality, but `flags()` does not advertise
// tile-wise scheduling. Keep this private interval bounded for cancellation.
const MAX_CPU_WORK_PIXELS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FiniteValue(u32);

impl FiniteValue {
    const fn new(value: f32) -> Result<Self, ()> {
        if value.is_finite() {
            Ok(Self(value.to_bits()))
        } else {
            Err(())
        }
    }

    #[must_use]
    const fn get(self) -> f32 {
        f32::from_bits(self.0)
    }
}

/// Checked persisted v3 values. Finite values outside native metadata ranges
/// are retained because `commit_params` does not clamp persisted history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorBalanceConfig {
    mode: ColorBalanceMode,
    lift: [FiniteValue; CHANNEL_SIZE],
    gamma: [FiniteValue; CHANNEL_SIZE],
    gain: [FiniteValue; CHANNEL_SIZE],
    saturation: FiniteValue,
    contrast: FiniteValue,
    grey: FiniteValue,
    saturation_out: FiniteValue,
}

impl ColorBalanceConfig {
    pub fn new(parameters: ColorBalanceParametersV3) -> Result<Self, ColorBalanceParameterError> {
        Self::try_from(parameters)
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(ColorBalanceParametersV3::defaults())
            .expect("native Color Balance defaults are finite")
    }

    #[must_use]
    pub const fn mode(self) -> ColorBalanceMode {
        self.mode
    }

    #[must_use]
    pub fn lift(self) -> [f32; CHANNEL_SIZE] {
        self.lift.map(FiniteValue::get)
    }

    #[must_use]
    pub fn gamma(self) -> [f32; CHANNEL_SIZE] {
        self.gamma.map(FiniteValue::get)
    }

    #[must_use]
    pub fn gain(self) -> [f32; CHANNEL_SIZE] {
        self.gain.map(FiniteValue::get)
    }

    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.saturation.get()
    }

    #[must_use]
    pub const fn contrast(self) -> f32 {
        self.contrast.get()
    }

    #[must_use]
    pub const fn grey(self) -> f32 {
        self.grey.get()
    }

    #[must_use]
    pub const fn saturation_out(self) -> f32 {
        self.saturation_out.get()
    }

    #[must_use]
    pub fn parameters(self) -> ColorBalanceParametersV3 {
        ColorBalanceParametersV3::new(
            self.mode,
            self.lift(),
            self.gamma(),
            self.gain(),
            self.saturation(),
            self.contrast(),
            self.grey(),
            self.saturation_out(),
        )
    }
}

impl TryFrom<ColorBalanceParametersV3> for ColorBalanceConfig {
    type Error = ColorBalanceParameterError;

    fn try_from(parameters: ColorBalanceParametersV3) -> Result<Self, Self::Error> {
        Ok(Self {
            mode: parameters.mode,
            lift: finite_array(parameters.lift, "lift")?,
            gamma: finite_array(parameters.gamma, "gamma")?,
            gain: finite_array(parameters.gain, "gain")?,
            saturation: finite(parameters.saturation, "saturation", None)?,
            contrast: finite(parameters.contrast, "contrast", None)?,
            grey: finite(parameters.grey, "grey", None)?,
            saturation_out: finite(parameters.saturation_out, "saturation_out", None)?,
        })
    }
}

fn finite_array(
    values: [f32; CHANNEL_SIZE],
    field: &'static str,
) -> Result<[FiniteValue; CHANNEL_SIZE], ColorBalanceParameterError> {
    let mut checked = [FiniteValue(0); CHANNEL_SIZE];
    for (index, value) in values.into_iter().enumerate() {
        checked[index] = finite(value, field, Some(index))?;
    }
    Ok(checked)
}

fn finite(
    value: f32,
    field: &'static str,
    index: Option<usize>,
) -> Result<FiniteValue, ColorBalanceParameterError> {
    FiniteValue::new(value).map_err(|()| ColorBalanceParameterError::NonFinite { field, index })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBalanceParameterError {
    NonFinite {
        field: &'static str,
        index: Option<usize>,
    },
}

impl fmt::Display for ColorBalanceParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite {
                field,
                index: Some(index),
            } => {
                write!(formatter, "Color Balance {field}[{index}] is non-finite")
            }
            Self::NonFinite { field, index: None } => {
                write!(formatter, "Color Balance {field} is non-finite")
            }
        }
    }
}

impl std::error::Error for ColorBalanceParameterError {}

/// Native committed arrays after the mode-dependent luminance correction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceCommitted {
    mode: ColorBalanceMode,
    lift: [f32; CHANNEL_SIZE],
    gamma: [f32; CHANNEL_SIZE],
    gain: [f32; CHANNEL_SIZE],
    saturation: f32,
    contrast: f32,
    grey: f32,
    saturation_out: f32,
}

impl ColorBalanceCommitted {
    #[must_use]
    pub const fn mode(self) -> ColorBalanceMode {
        self.mode
    }

    #[must_use]
    pub const fn lift(self) -> [f32; CHANNEL_SIZE] {
        self.lift
    }

    #[must_use]
    pub const fn gamma(self) -> [f32; CHANNEL_SIZE] {
        self.gamma
    }

    #[must_use]
    pub const fn gain(self) -> [f32; CHANNEL_SIZE] {
        self.gain
    }

    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.saturation
    }

    #[must_use]
    pub const fn contrast(self) -> f32 {
        self.contrast
    }

    #[must_use]
    pub const fn grey(self) -> f32 {
        self.grey
    }

    #[must_use]
    pub const fn saturation_out(self) -> f32 {
        self.saturation_out
    }
}

/// Per-pixel coefficients derived by native `process` in its source order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceCoefficients {
    pub committed: ColorBalanceCommitted,
    pub contrast_power: f32,
    pub grey: f32,
    pub lgg_lift: [f32; 4],
    pub lgg_gamma: [f32; 4],
    pub legacy_gamma_inv: [f32; 4],
    pub lgg_gamma_inv: [f32; 4],
    pub lgg_gain: [f32; 4],
    pub sop_lift: [f32; 4],
    pub sop_gamma: [f32; 4],
    pub sop_gain: [f32; 4],
}

/// Immutable native Color Balance processing plan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalancePlan {
    config: ColorBalanceConfig,
    coefficients: ColorBalanceCoefficients,
}

impl ColorBalancePlan {
    #[must_use]
    pub fn new(config: ColorBalanceConfig) -> Self {
        let committed = commit(config);
        let contrast_power = if committed.contrast == 0.0 {
            MILLION
        } else {
            1.0 / committed.contrast
        };
        let grey = committed.grey / 100.0;

        let mut lgg_lift = [0.0; 4];
        let mut lgg_gamma = [0.0; 4];
        let mut legacy_gamma_inv = [0.0; 4];
        let mut lgg_gamma_inv = [0.0; 4];
        let mut lgg_gain = [0.0; 4];
        let mut sop_lift = [0.0; 4];
        let mut sop_gamma = [0.0; 4];
        let mut sop_gain = [0.0; 4];
        for channel in 0..3 {
            lgg_lift[channel] = 2.0 - committed.lift[channel] * committed.lift[CHANNEL_FACTOR];
            lgg_gamma[channel] = committed.gamma[channel] * committed.gamma[CHANNEL_FACTOR];
            legacy_gamma_inv[channel] = if lgg_gamma[channel] == 0.0 {
                MILLION
            } else {
                1.0 / lgg_gamma[channel]
            };
            lgg_gamma_inv[channel] = 2.2 * legacy_gamma_inv[channel];
            lgg_gain[channel] = committed.gain[channel] * committed.gain[CHANNEL_FACTOR];
            sop_lift[channel] = committed.lift[channel] + committed.lift[CHANNEL_FACTOR] - 2.0;
            sop_gamma[channel] =
                (2.0 - committed.gamma[channel]) * (2.0 - committed.gamma[CHANNEL_FACTOR]);
            sop_gain[channel] = lgg_gain[channel];
        }
        lgg_lift[3] = 0.0;
        lgg_gamma[3] = 1.0;
        legacy_gamma_inv[3] = 1.0;
        lgg_gamma_inv[3] = 1.0;
        lgg_gain[3] = 1.0;
        sop_lift[3] = 0.0;
        sop_gamma[3] = 1.0;
        sop_gain[3] = 1.0;

        Self {
            config,
            coefficients: ColorBalanceCoefficients {
                committed,
                contrast_power,
                grey,
                lgg_lift,
                lgg_gamma,
                legacy_gamma_inv,
                lgg_gamma_inv,
                lgg_gain,
                sop_lift,
                sop_gamma,
                sop_gain,
            },
        }
    }

    #[must_use]
    pub const fn config(self) -> ColorBalanceConfig {
        self.config
    }

    #[must_use]
    pub const fn coefficients(self) -> ColorBalanceCoefficients {
        self.coefficients
    }

    /// Applies the native scalar operation to a single four-float Lab sample.
    /// The fourth lane is zeroed by native Color Balance conversion and is not
    /// an external-image-alpha lane.
    #[must_use]
    pub fn execute_pixel(&self, input: ColorBalancePixel) -> ColorBalancePixel {
        let input = input.channels;
        let output = match self.config.mode {
            ColorBalanceMode::Legacy => self.process_legacy(input),
            ColorBalanceMode::LiftGammaGain => self.process_lgg(input),
            ColorBalanceMode::SlopeOffsetPower => self.process_sop(input),
        };
        ColorBalancePixel::from_channels(output)
    }

    /// Executes pointwise in row-major order.
    #[must_use]
    pub fn execute_lab(&self, input: &[ColorBalancePixel]) -> Vec<ColorBalancePixel> {
        input
            .iter()
            .map(|pixel| self.execute_pixel(*pixel))
            .collect()
    }

    /// Executes row-major work chunks with a bounded cancellation interval.
    /// Caller-provided tile sizes are only partitioning hints; an error discards
    /// the private partial output and never publishes it.
    pub fn execute_lab_tiled(
        &self,
        input: &[ColorBalancePixel],
        tile_size: usize,
        cancellation: Option<&dyn Fn(usize) -> bool>,
    ) -> Result<Vec<ColorBalancePixel>, ColorBalanceExecutionError> {
        if tile_size == 0 {
            return Err(ColorBalanceExecutionError::InvalidTileSize);
        }
        let bounded_tile_size = tile_size.min(MAX_CPU_WORK_PIXELS);
        let mut output = Vec::with_capacity(input.len());
        for (start, tile) in input.chunks(bounded_tile_size).enumerate() {
            let offset = start * bounded_tile_size;
            if cancellation.is_some_and(|check| check(offset)) {
                return Err(ColorBalanceExecutionError::Cancelled { processed: offset });
            }
            output.extend(tile.iter().map(|pixel| self.execute_pixel(*pixel)));
        }
        if cancellation.is_some_and(|check| check(input.len())) {
            return Err(ColorBalanceExecutionError::Cancelled {
                processed: input.len(),
            });
        }
        Ok(output)
    }

    /// Preserves straight alpha from the frame boundary separately from the
    /// native four-float Lab operation buffer.
    pub fn execute_lab_with_external_alpha(
        &self,
        input: &[ColorBalancePixel],
        external_alpha: &[f32],
    ) -> Result<Vec<ColorBalancePixel>, ColorBalanceExecutionError> {
        if input.len() != external_alpha.len() {
            return Err(ColorBalanceExecutionError::ExternalAlphaLengthMismatch {
                pixels: input.len(),
                alpha: external_alpha.len(),
            });
        }
        let mut output = self.execute_lab(input);
        for (pixel, alpha) in output.iter_mut().zip(external_alpha) {
            pixel.channels[3] = *alpha;
        }
        Ok(output)
    }

    /// Applies generic NORMAL blending with source/candidate Lab values and a
    /// separate local coverage lane. Global opacity is clamped before it is
    /// combined with the mask, matching `blendif_lab.c`.
    pub fn execute_lab_normal_blend(
        &self,
        input: &[ColorBalancePixel],
        mask: Option<&[f32]>,
        opacity: f32,
    ) -> Result<Vec<ColorBalancePixel>, ColorBalanceExecutionError> {
        self.execute_lab_normal_blend_tiled(input, mask, opacity, input.len().max(1), None)
    }

    /// Tiled NORMAL blend variant with the same no-partial-publication rule as
    /// the CPU operation.
    pub fn execute_lab_normal_blend_tiled(
        &self,
        input: &[ColorBalancePixel],
        mask: Option<&[f32]>,
        opacity: f32,
        tile_size: usize,
        cancellation: Option<&dyn Fn(usize) -> bool>,
    ) -> Result<Vec<ColorBalancePixel>, ColorBalanceExecutionError> {
        if let Some(mask) = mask
            && mask.len() != input.len()
        {
            return Err(ColorBalanceExecutionError::MaskLengthMismatch {
                pixels: input.len(),
                mask: mask.len(),
            });
        }
        #[expect(
            clippy::manual_clamp,
            reason = "Native fminf(fmaxf(opacity, 0), 1) ordering is required, including NaN handling."
        )]
        let global_opacity = f32::min(1.0, f32::max(opacity, 0.0));
        let candidates = self.execute_lab_tiled(input, tile_size, cancellation)?;
        let mut output = Vec::with_capacity(input.len());
        for (index, (source, candidate)) in input.iter().zip(candidates).enumerate() {
            if cancellation.is_some_and(|check| check(index)) {
                return Err(ColorBalanceExecutionError::Cancelled { processed: index });
            }
            let coverage = mask.map_or(global_opacity, |values| values[index] * global_opacity);
            output.push(blend_lab_normal_pixel(*source, candidate, coverage));
        }
        Ok(output)
    }

    fn process_legacy(&self, input: [f32; 4]) -> [f32; 4] {
        let xyz = math::lab_to_xyz(input);
        let mut rgb = math::xyz_to_srgb(xyz);
        for channel in 0..3 {
            rgb[channel] = (((rgb[channel] - 1.0) * self.coefficients.lgg_lift[channel]) + 1.0)
                * self.coefficients.lgg_gain[channel];
        }
        for channel in 0..3 {
            rgb[channel] = math::max_zero(rgb[channel]);
            rgb[channel] =
                math::approximate_powf(rgb[channel], self.coefficients.legacy_gamma_inv[channel]);
        }
        math::xyz_to_lab(math::srgb_to_xyz(rgb))
    }

    fn process_lgg(&self, input: [f32; 4]) -> [f32; 4] {
        let xyz = math::lab_to_xyz(input);
        let mut rgb = math::xyz_to_prophoto(xyz);
        if (self.coefficients.committed.saturation - 1.0).abs() > CPU_STAGE_EPSILON {
            let luma = xyz[1];
            for channel in 0..3 {
                rgb[channel] =
                    luma + self.coefficients.committed.saturation * (rgb[channel] - luma);
            }
        }
        for channel in 0..3 {
            rgb[channel] = math::max_zero(rgb[channel]);
            rgb[channel] = math::approximate_powf(rgb[channel], RGB_GAMMA);
            rgb[channel] = ((rgb[channel] - 1.0) * self.coefficients.lgg_lift[channel] + 1.0)
                * self.coefficients.lgg_gain[channel];
            rgb[channel] = math::max_zero(rgb[channel]);
            rgb[channel] =
                math::approximate_powf(rgb[channel], self.coefficients.lgg_gamma_inv[channel]);
        }
        if (self.coefficients.committed.saturation_out - 1.0).abs() > CPU_STAGE_EPSILON {
            let luma = math::prophoto_luma(rgb);
            for channel in 0..3 {
                rgb[channel] =
                    luma + self.coefficients.committed.saturation_out * (rgb[channel] - luma);
            }
        }
        if (self.coefficients.contrast_power - 1.0).abs() > CPU_STAGE_EPSILON {
            for channel in 0..3 {
                rgb[channel] = math::max_zero(rgb[channel]);
                rgb[channel] = math::approximate_powf(
                    rgb[channel] / self.coefficients.grey,
                    self.coefficients.contrast_power,
                ) * self.coefficients.grey;
            }
        }
        math::prophoto_to_lab(rgb)
    }

    fn process_sop(&self, input: [f32; 4]) -> [f32; 4] {
        let xyz = math::lab_to_xyz(input);
        let mut rgb = math::xyz_to_prophoto(xyz);
        if (self.coefficients.committed.saturation - 1.0).abs() > CPU_STAGE_EPSILON {
            let luma = xyz[1];
            for channel in 0..3 {
                rgb[channel] =
                    luma + self.coefficients.committed.saturation * (rgb[channel] - luma);
            }
        }
        for channel in 0..3 {
            rgb[channel] = rgb[channel] * self.coefficients.sop_gain[channel]
                + self.coefficients.sop_lift[channel];
            rgb[channel] = math::max_zero(rgb[channel]);
            rgb[channel] =
                math::approximate_powf(rgb[channel], self.coefficients.sop_gamma[channel]);
        }
        if (self.coefficients.committed.saturation_out - 1.0).abs() > CPU_STAGE_EPSILON {
            let luma = math::prophoto_luma(rgb);
            for channel in 0..3 {
                rgb[channel] =
                    luma + self.coefficients.committed.saturation_out * (rgb[channel] - luma);
            }
        }
        if (self.coefficients.committed.contrast - 1.0).abs() > CPU_STAGE_EPSILON {
            for channel in 0..3 {
                rgb[channel] = math::max_zero(rgb[channel]);
                rgb[channel] = math::approximate_powf(
                    rgb[channel] / self.coefficients.grey,
                    self.coefficients.contrast_power,
                ) * self.coefficients.grey;
            }
        }
        math::prophoto_to_lab(rgb)
    }
}

pub fn blend_lab_normal_pixel_for_test(
    source: ColorBalancePixel,
    candidate: ColorBalancePixel,
    coverage: f32,
) -> ColorBalancePixel {
    blend_lab_normal_pixel(source, candidate, coverage)
}

fn commit(config: ColorBalanceConfig) -> ColorBalanceCommitted {
    let persisted_lift = config.lift();
    let persisted_gamma = config.gamma();
    let persisted_gain = config.gain();
    let (lift, gamma, gain) = match config.mode {
        ColorBalanceMode::Legacy => (persisted_lift, persisted_gamma, persisted_gain),
        ColorBalanceMode::LiftGammaGain | ColorBalanceMode::SlopeOffsetPower => (
            corrected_curve(persisted_lift),
            corrected_curve(persisted_gamma),
            corrected_curve(persisted_gain),
        ),
    };
    ColorBalanceCommitted {
        mode: config.mode,
        lift,
        gamma,
        gain,
        saturation: config.saturation(),
        contrast: config.contrast(),
        grey: config.grey(),
        saturation_out: config.saturation_out(),
    }
}

fn corrected_curve(persisted: [f32; CHANNEL_SIZE]) -> [f32; CHANNEL_SIZE] {
    let xyz = math::prophoto_to_xyz([
        persisted[CHANNEL_RED],
        persisted[CHANNEL_GREEN],
        persisted[CHANNEL_BLUE],
        0.0,
    ]);
    let y = xyz[1];
    [
        persisted[CHANNEL_FACTOR],
        (persisted[CHANNEL_RED] - y) + 1.0,
        (persisted[CHANNEL_GREEN] - y) + 1.0,
        (persisted[CHANNEL_BLUE] - y) + 1.0,
    ]
}

const LAB_BLEND_SCALE: [f32; 4] = [1.0 / 100.0, 1.0 / 128.0, 1.0 / 128.0, 1.0];
const LAB_BLEND_RESCALE: [f32; 4] = [100.0, 128.0, 128.0, 1.0];

/// Native `blendif_lab.c::_blend_normal_unbounded` arithmetic.
///
/// Lab channels are normalized before interpolation and rescaled afterwards.
/// The coverage lane is published after rescaling rather than treated as an
/// image-alpha channel.
#[must_use]
fn blend_lab_normal_pixel(
    source: ColorBalancePixel,
    candidate: ColorBalancePixel,
    coverage: f32,
) -> ColorBalancePixel {
    let source = source.channels;
    let candidate = candidate.channels;
    let mut source_scaled = [0.0; 4];
    let mut candidate_scaled = [0.0; 4];
    for channel in 0..4 {
        source_scaled[channel] = source[channel] * LAB_BLEND_SCALE[channel];
        candidate_scaled[channel] = candidate[channel] * LAB_BLEND_SCALE[channel];
    }
    for channel in 0..4 {
        candidate_scaled[channel] =
            source_scaled[channel] * (1.0 - coverage) + candidate_scaled[channel] * coverage;
    }
    let mut output = [0.0; 4];
    for channel in 0..4 {
        output[channel] = candidate_scaled[channel] * LAB_BLEND_RESCALE[channel];
    }
    output[3] = coverage;
    ColorBalancePixel::from_channels(output)
}

/// Four-float native Lab sample. The fourth native CPU lane is scratch/zero;
/// straight image alpha belongs to the external frame boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalancePixel {
    channels: [f32; 4],
}

impl ColorBalancePixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha_or_spare: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha_or_spare],
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
    pub const fn alpha_or_spare(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBalanceExecutionError {
    InvalidTileSize,
    Cancelled { processed: usize },
    MaskLengthMismatch { pixels: usize, mask: usize },
    ExternalAlphaLengthMismatch { pixels: usize, alpha: usize },
}

impl fmt::Display for ColorBalanceExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTileSize => formatter.write_str("Color Balance tile size must be nonzero"),
            Self::Cancelled { processed } => {
                write!(
                    formatter,
                    "Color Balance execution cancelled after {processed} pixels"
                )
            }
            Self::MaskLengthMismatch { pixels, mask } => {
                write!(
                    formatter,
                    "Color Balance mask has {mask} values for {pixels} pixels"
                )
            }
            Self::ExternalAlphaLengthMismatch { pixels, alpha } => write!(
                formatter,
                "Color Balance external alpha has {alpha} values for {pixels} pixels"
            ),
        }
    }
}

impl std::error::Error for ColorBalanceExecutionError {}
