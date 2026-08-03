#![expect(
    clippy::suboptimal_flops,
    reason = "Native Levels arithmetic order is preserved for IEEE-754 parity."
)]

//! Histogram resolution, LUT compilation, and the source-shaped CPU pixel leaf.

use std::fmt;

use crate::{
    RasterDimensions, RgbChannel,
    operations::{OperationExecutionError, ReconstructionBudget},
};

use super::{LEVELS_CHANNELS, LevelsConfig, LevelsMode};

/// Native automatic histogram resolution.
pub const LEVELS_AUTO_HISTOGRAM_BINS: u32 = 16_384;
/// Native manual histogram resolution.
pub const LEVELS_MANUAL_HISTOGRAM_BINS: usize = 256;
/// Native LUT has one 16-bit fractional cell for each index.
pub const LEVELS_LUT_ENTRIES: usize = 0x10000;
pub const LEVELS_MAXIMUM_LUT_BYTES: usize = LEVELS_LUT_ENTRIES * std::mem::size_of::<f32>();
/// Native `default_colorspace` and description boundary.
pub const LEVELS_INPUT_PROFILE: &str = "display-referred Lab D50";
/// L is channel zero in the retained four-float Lab pixel.
pub const LEVELS_LUMINANCE_CHANNEL: usize = 0;
/// Native Lab lightness is stored in the [0, 100] scale.
pub const LEVELS_LUMINANCE_SCALE: f32 = 100.0;

/// Native automatic-level state uses `-FLT_MAX` until a percentile resolves.
const LEVELS_UNINITIALIZED: f32 = -f32::MAX;

/// Histogram data in the retained four-channel layout.
///
/// `bins` contains the channel-interleaved histogram, so the L bin for source
/// bin `i` is `bins[4 * i]`. This is the layout consumed by both native helper
/// routines, not a generic one-channel histogram substitute.
#[derive(Debug, Clone, Copy)]
pub struct LevelsHistogram<'a> {
    bins: &'a [u32],
    bins_count: u32,
    pixels: u32,
}

impl<'a> LevelsHistogram<'a> {
    pub fn new(
        bins: &'a [u32],
        bins_count: u32,
        pixels: u32,
    ) -> Result<Self, LevelsHistogramError> {
        if bins_count < 2 {
            return Err(LevelsHistogramError::InvalidBinCount(bins_count));
        }
        let required = usize::try_from(bins_count)
            .ok()
            .and_then(|count| count.checked_mul(LEVELS_CHANNELS))
            .ok_or(LevelsHistogramError::InvalidBinCount(bins_count))?;
        if bins.len() < required {
            return Err(LevelsHistogramError::InsufficientBins {
                expected: required,
                actual: bins.len(),
            });
        }
        Ok(Self {
            bins,
            bins_count,
            pixels,
        })
    }

    #[must_use]
    pub const fn bins_count(self) -> u32 {
        self.bins_count
    }

    #[must_use]
    pub const fn pixels(self) -> u32 {
        self.pixels
    }

    #[must_use]
    pub const fn bins(self) -> &'a [u32] {
        self.bins
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelsHistogramError {
    InvalidBinCount(u32),
    InsufficientBins { expected: usize, actual: usize },
}

impl fmt::Display for LevelsHistogramError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBinCount(count) => {
                write!(formatter, "levels bin count {count} is invalid")
            }
            Self::InsufficientBins { expected, actual } => {
                write!(
                    formatter,
                    "levels histogram has {actual} bins; expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for LevelsHistogramError {}

/// Direct port of `dt_iop_levels_compute_levels_manual`.
///
/// A null native histogram leaves the existing levels unchanged. The safe
/// equivalent is `None`; a short non-null histogram is rejected rather than
/// indexing past its storage.
pub fn compute_manual_levels(
    histogram: Option<&[u32]>,
    levels: &mut [f32; 3],
) -> Result<(), LevelsHistogramError> {
    let Some(histogram) = histogram else {
        return Ok(());
    };
    let required = LEVELS_MANUAL_HISTOGRAM_BINS * LEVELS_CHANNELS;
    if histogram.len() < required {
        return Err(LevelsHistogramError::InsufficientBins {
            expected: required,
            actual: histogram.len(),
        });
    }

    // Native scans k = 0, 4, ..., 4 * 255 and divides by 4 * 256.
    for k in (0..=(4 * 255)).step_by(4) {
        if histogram[k] > 1 {
            levels[0] = k as f32 / (4 * 256) as f32;
            break;
        }
    }
    for k in (0..=(4 * 255)).rev().step_by(4) {
        if histogram[k] > 1 {
            levels[2] = k as f32 / (4 * 256) as f32;
            break;
        }
    }
    levels[1] = levels[0] / 2.0_f32 + levels[2] / 2.0_f32;
    Ok(())
}

/// Direct port of `dt_iop_levels_compute_levels_automatic`.
///
/// The native marker is `-FLT_MAX`, not NaN. It is deliberately retained here
/// so a missing preview histogram cannot be mistaken for a valid zero level.
#[must_use]
pub fn compute_automatic_levels(
    histogram: Option<LevelsHistogram<'_>>,
    percentiles: [f32; 3],
) -> [f32; 3] {
    let mut levels = [LEVELS_UNINITIALIZED; 3];
    let Some(histogram) = histogram else {
        return levels;
    };

    let thresholds = percentiles.map(|percentile| histogram.pixels as f32 * percentile / 100.0_f32);
    let mut cumulative = 0_usize;
    for index in 0..histogram.bins_count {
        let bin = usize::try_from(index).expect("histogram index fits usize");
        cumulative = cumulative.saturating_add(histogram.bins[4 * bin] as usize);
        for (channel, level) in levels.iter_mut().enumerate() {
            if *level == LEVELS_UNINITIALIZED && cumulative as f32 >= thresholds[channel] {
                *level = index as f32 / (histogram.bins_count - 1) as f32;
            }
        }
    }

    // Native code repairs only the upper marker for a sharp floating threshold.
    if levels[2] == LEVELS_UNINITIALIZED {
        levels[2] = 1.0;
    }
    let center = percentiles[1] / 100.0_f32;
    if levels[0] != LEVELS_UNINITIALIZED && levels[2] != LEVELS_UNINITIALIZED {
        levels[1] = (1.0_f32 - center) * levels[0] + center * levels[2];
    }
    levels
}

/// Native four-channel Lab sample.
///
/// The fourth channel is an alpha/spare value owned by the surrounding
/// pixelpipe and is passed through untouched by this operation-local leaf; it
/// is never used in the luminance equation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LevelsPixel {
    channels: [f32; LEVELS_CHANNELS],
}

impl LevelsPixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; LEVELS_CHANNELS]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; LEVELS_CHANNELS] {
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

/// Pointwise execution has no native neighborhood overlap or scratch tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelsTiling {
    pub overlap_pixels: u32,
    pub alignment_pixels: usize,
    pub temporary_multiplier_milli: u32,
    pub input_multiplier_milli: u32,
    pub output_multiplier_milli: u32,
}

impl Default for LevelsTiling {
    fn default() -> Self {
        Self {
            overlap_pixels: 0,
            alignment_pixels: 1,
            temporary_multiplier_milli: 0,
            input_multiplier_milli: 1_000,
            output_multiplier_milli: 1_000,
        }
    }
}

/// Immutable source-shaped LUT and pointwise CPU execution plan.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelsPlan {
    config: LevelsConfig,
    dimensions: RasterDimensions,
    levels: [f32; 3],
    in_inv_gamma: f32,
    lut: Vec<f32>,
    budget: ReconstructionBudget,
}

impl LevelsPlan {
    pub fn new(
        config: LevelsConfig,
        dimensions: RasterDimensions,
        histogram: Option<LevelsHistogram<'_>>,
    ) -> Result<Self, OperationExecutionError> {
        Self::new_with_budget(
            config,
            dimensions,
            histogram,
            ReconstructionBudget::default(),
        )
    }

    pub fn new_with_budget(
        config: LevelsConfig,
        dimensions: RasterDimensions,
        histogram: Option<LevelsHistogram<'_>>,
        budget: ReconstructionBudget,
    ) -> Result<Self, OperationExecutionError> {
        let levels = match config.mode() {
            LevelsMode::Manual => config.levels(),
            LevelsMode::Automatic => {
                compute_automatic_levels(histogram, [config.black(), config.gray(), config.white()])
            }
        };
        if levels
            .iter()
            .any(|value| !value.is_finite() || is_unresolved_level(*value))
        {
            return Err(OperationExecutionError::UnsupportedCapability(
                "levels requires resolved black, gray, and white levels",
            ));
        }
        if levels[2] <= levels[0] {
            return Err(OperationExecutionError::UnsupportedCapability(
                "levels requires white level above black level",
            ));
        }

        let delta = (levels[2] - levels[0]) / 2.0_f32;
        let middle = levels[0] + delta;
        let tmp = (levels[1] - middle) / delta;
        let in_inv_gamma = (10.0_f64).powf(f64::from(tmp)) as f32;
        if !in_inv_gamma.is_finite() || in_inv_gamma <= 0.0 {
            return Err(OperationExecutionError::UnsupportedCapability(
                "levels inverse gamma is not finite and positive",
            ));
        }

        Self::check_budget(dimensions, budget)?;
        let lut = build_lut(in_inv_gamma)?;
        Ok(Self {
            config,
            dimensions,
            levels,
            in_inv_gamma,
            lut,
            budget,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn config(&self) -> LevelsConfig {
        self.config
    }

    #[must_use]
    pub const fn levels(&self) -> [f32; 3] {
        self.levels
    }

    #[must_use]
    pub const fn in_inv_gamma(&self) -> f32 {
        self.in_inv_gamma
    }

    #[must_use]
    pub fn lut(&self) -> &[f32] {
        &self.lut
    }

    #[must_use]
    pub const fn tiling(&self) -> LevelsTiling {
        LevelsTiling {
            overlap_pixels: 0,
            alignment_pixels: 1,
            temporary_multiplier_milli: 0,
            input_multiplier_milli: 1_000,
            output_multiplier_milli: 1_000,
        }
    }

    pub fn execute(
        &self,
        input: &[LevelsPixel],
    ) -> Result<Vec<LevelsPixel>, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[LevelsPixel],
        mut cancelled: F,
    ) -> Result<Vec<LevelsPixel>, OperationExecutionError> {
        self.execute_with_input_dimensions_with_cancel(input, self.dimensions, &mut cancelled)
    }

    /// Executes an independently shaped pointwise tile with the same committed LUT.
    pub fn execute_with_input_dimensions(
        &self,
        input: &[LevelsPixel],
        dimensions: RasterDimensions,
    ) -> Result<Vec<LevelsPixel>, OperationExecutionError> {
        self.execute_with_input_dimensions_with_cancel(input, dimensions, || false)
    }

    pub fn execute_with_input_dimensions_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[LevelsPixel],
        dimensions: RasterDimensions,
        mut cancelled: F,
    ) -> Result<Vec<LevelsPixel>, OperationExecutionError> {
        self.execute_with_input_dimensions_inner(input, dimensions, &mut cancelled)
    }

    fn execute_with_input_dimensions_inner<F: FnMut() -> bool>(
        &self,
        input: &[LevelsPixel],
        dimensions: RasterDimensions,
        cancelled: &mut F,
    ) -> Result<Vec<LevelsPixel>, OperationExecutionError> {
        let expected = dimensions_pixel_count(dimensions, input.len())?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        Self::check_budget(dimensions, self.budget)?;

        let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
        let mut output = Vec::new();
        output.try_reserve_exact(expected).map_err(|_| {
            OperationExecutionError::AllocationFailed {
                required: expected.saturating_mul(std::mem::size_of::<LevelsPixel>()),
            }
        })?;
        for (index, pixel) in input.iter().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            output.push(self.process_pixel(*pixel, index)?);
        }
        Ok(output)
    }

    fn process_pixel(
        &self,
        input: LevelsPixel,
        index: usize,
    ) -> Result<LevelsPixel, OperationExecutionError> {
        let channels = input.channels();
        for (channel, value) in channels[..3].iter().enumerate() {
            if !value.is_finite() {
                return Err(OperationExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: lab_channel(channel),
                });
            }
        }

        let level_black = self.levels[0];
        let level_range = self.levels[2] - level_black;
        let lightness_in = channels[LEVELS_LUMINANCE_CHANNEL] / LEVELS_LUMINANCE_SCALE;
        let lightness_out = if lightness_in <= level_black {
            // Native CPU: anything below the lower threshold clips to zero.
            0.0_f32
        } else {
            let percentage = (lightness_in - level_black) / level_range;
            // Preserve the native LUT boundary and fallback ordering. The LUT
            // intentionally has no 1.0 endpoint; percentage == 1 uses powf.
            if percentage < 1.0_f32 {
                let lut_index = (percentage * LEVELS_LUT_ENTRIES as f32) as usize;
                self.lut[lut_index]
            } else {
                100.0_f32 * percentage.powf(self.in_inv_gamma)
            }
        };
        if !lightness_out.is_finite() {
            return Err(OperationExecutionError::NonFiniteResult {
                pixel: index,
                channel: RgbChannel::Red,
            });
        }

        // Keep native left-to-right f32 arithmetic and the 0.01 denominator
        // guard. The fourth channel is intentionally not included in this
        // contrast-preserving Lab-lightness rescale.
        let denominator = if channels[0] > 0.01_f32 {
            channels[0]
        } else {
            0.01_f32
        };
        let a = channels[1] * lightness_out / denominator;
        let b = channels[2] * lightness_out / denominator;
        if !a.is_finite() {
            return Err(OperationExecutionError::NonFiniteResult {
                pixel: index,
                channel: RgbChannel::Green,
            });
        }
        if !b.is_finite() {
            return Err(OperationExecutionError::NonFiniteResult {
                pixel: index,
                channel: RgbChannel::Blue,
            });
        }
        Ok(LevelsPixel::new(lightness_out, a, b, channels[3]))
    }

    fn check_budget(
        dimensions: RasterDimensions,
        budget: ReconstructionBudget,
    ) -> Result<(), OperationExecutionError> {
        let pixels = usize::try_from(dimensions.pixel_count()).map_err(|_| {
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: budget.maximum_bytes(),
            }
        })?;
        let output_bytes = pixels
            .checked_mul(std::mem::size_of::<LevelsPixel>())
            .ok_or_else(|| OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: budget.maximum_bytes(),
            })?;
        let required = output_bytes
            .checked_add(LEVELS_MAXIMUM_LUT_BYTES)
            .ok_or_else(|| OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: budget.maximum_bytes(),
            })?;
        if required > budget.maximum_bytes() {
            return Err(OperationExecutionError::MemoryBudgetExceeded {
                required,
                budget: budget.maximum_bytes(),
            });
        }
        Ok(())
    }
}

#[must_use]
fn is_unresolved_level(value: f32) -> bool {
    value == f32::MAX || value == -f32::MAX
}

fn build_lut(in_inv_gamma: f32) -> Result<Vec<f32>, OperationExecutionError> {
    let mut lut = Vec::new();
    lut.try_reserve_exact(LEVELS_LUT_ENTRIES).map_err(|_| {
        OperationExecutionError::AllocationFailed {
            required: LEVELS_MAXIMUM_LUT_BYTES,
        }
    })?;
    for index in 0..LEVELS_LUT_ENTRIES {
        // Native computes the percentage as f32 before powf and stores a f32.
        let percentage = index as f32 / LEVELS_LUT_ENTRIES as f32;
        lut.push(100.0_f32 * percentage.powf(in_inv_gamma));
    }
    Ok(lut)
}

fn dimensions_pixel_count(
    dimensions: RasterDimensions,
    actual: usize,
) -> Result<usize, OperationExecutionError> {
    let expected = usize::try_from(dimensions.pixel_count()).map_err(|_| {
        OperationExecutionError::DimensionsMismatch {
            expected: usize::MAX,
            actual,
        }
    })?;
    if expected == actual {
        Ok(expected)
    } else {
        Err(OperationExecutionError::DimensionsMismatch { expected, actual })
    }
}

const fn lab_channel(channel: usize) -> RgbChannel {
    match channel {
        0 => RgbChannel::Red,
        1 => RgbChannel::Green,
        _ => RgbChannel::Blue,
    }
}
