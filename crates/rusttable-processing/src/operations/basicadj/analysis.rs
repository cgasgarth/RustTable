//! Source-derived automatic analysis for Darktable's legacy `basicadj`.
//!
//! Source lineage: `src/iop/basicadj.c` histogram and auto-level helpers.
//!
//! The retained implementation in `src/iop/basicadj.c` builds an 8192-bin
//! histogram from the selected RGB samples, then resolves the automatic
//! controls from that histogram.  This module keeps that pass independent of
//! execution so one immutable result can be reused by every tile.

#![allow(
    clippy::approx_constant,
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::excessive_precision,
    clippy::manual_midpoint,
    clippy::needless_range_loop,
    clippy::unreadable_literal
)]

use std::fmt;

use sha2::{Digest, Sha256};

use crate::{LinearRgb, RasterDimensions};

use super::{BasicAdjAutoControls, BasicAdjConfig, BasicAdjPlanError};

/// Darktable's legacy histogram compression: `65536 >> 3` bins.
pub const BASICADJ_HISTOGRAM_BINS: usize = 65536 >> 3;
/// Native histogram lower bound. Values at or below it enter bin zero.
pub const BASICADJ_HISTOGRAM_MINIMUM: f32 = 0.0;
/// Native histogram upper bound. Values at or above it enter the last bin.
pub const BASICADJ_HISTOGRAM_MAXIMUM: f32 = 1.0;
/// Compatibility name retained for callers; native analysis does not sample.
pub const BASICADJ_MAX_ANALYSIS_PIXELS: usize = usize::MAX;

const CANCEL_POLL_INTERVAL: usize = 1024;
const HISTOGRAM_COMPRESSION: i32 = 3;

/// Checked row-major rectangle used by automatic analysis.
///
/// Coordinates are half-open at the API boundary. This is equivalent to the
/// native inclusive box after its right and bottom edges are converted to
/// `x + width - 1` and `y + height - 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BasicAdjAnalysisRoi {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl BasicAdjAnalysisRoi {
    /// Constructs a non-empty rectangle.
    ///
    /// # Errors
    ///
    /// Returns an error when either extent is zero or coordinate arithmetic
    /// overflows.
    pub fn new(x: u32, y: u32, width: u32, height: u32) -> Result<Self, BasicAdjAnalysisError> {
        if width == 0 || height == 0 {
            return Err(BasicAdjAnalysisError::EmptyRoi);
        }
        x.checked_add(width)
            .ok_or(BasicAdjAnalysisError::RoiOutOfBounds)?;
        y.checked_add(height)
            .ok_or(BasicAdjAnalysisError::RoiOutOfBounds)?;
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }

    fn validate(self, dimensions: RasterDimensions) -> Result<(), BasicAdjAnalysisError> {
        if self
            .x
            .checked_add(self.width)
            .is_none_or(|end| end > dimensions.width())
            || self
                .y
                .checked_add(self.height)
                .is_none_or(|end| end > dimensions.height())
        {
            return Err(BasicAdjAnalysisError::RoiOutOfBounds);
        }
        Ok(())
    }
}

/// Borrowed analysis raster with optional one-value-per-pixel selection mask.
///
/// A positive finite mask value selects a pixel, while zero, negative, and
/// non-finite values select no samples. The normal operation blend mask is a
/// separate execution concern and is not implicitly folded into this raster.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAdjAnalysisRaster<'a> {
    dimensions: RasterDimensions,
    pixels: &'a [LinearRgb],
    mask: Option<&'a [f32]>,
    roi: BasicAdjAnalysisRoi,
}

impl<'a> BasicAdjAnalysisRaster<'a> {
    /// Creates a full-frame analysis raster.
    ///
    /// # Errors
    ///
    /// Returns an error when the dimensions, pixels, or mask are invalid.
    pub fn new(
        dimensions: RasterDimensions,
        pixels: &'a [LinearRgb],
        mask: Option<&'a [f32]>,
    ) -> Result<Self, BasicAdjAnalysisError> {
        let roi = BasicAdjAnalysisRoi::new(0, 0, dimensions.width(), dimensions.height())?;
        Self::with_roi(dimensions, pixels, mask, roi)
    }

    /// Creates a masked/ROI analysis raster.
    ///
    /// # Errors
    ///
    /// Returns an error when the dimensions, pixels, mask, or ROI are invalid.
    pub fn with_roi(
        dimensions: RasterDimensions,
        pixels: &'a [LinearRgb],
        mask: Option<&'a [f32]>,
        roi: BasicAdjAnalysisRoi,
    ) -> Result<Self, BasicAdjAnalysisError> {
        let expected = usize::try_from(dimensions.pixel_count())
            .map_err(|_| BasicAdjAnalysisError::InputTooLarge)?;
        if pixels.len() != expected {
            return Err(BasicAdjAnalysisError::PixelCount {
                expected,
                actual: pixels.len(),
            });
        }
        if let Some(mask) = mask
            && mask.len() != expected
        {
            return Err(BasicAdjAnalysisError::MaskCount {
                expected,
                actual: mask.len(),
            });
        }
        roi.validate(dimensions)?;
        // Native `_get_selected_area` rejects a one-pixel-wide/high inclusive
        // box and analyzes the full frame instead. Preserve that fallback at
        // this half-open API boundary rather than silently narrowing analysis.
        let roi = if roi.width() < 2 || roi.height() < 2 {
            BasicAdjAnalysisRoi::new(0, 0, dimensions.width(), dimensions.height())?
        } else {
            roi
        };
        Ok(Self {
            dimensions,
            pixels,
            mask,
            roi,
        })
    }

    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn pixels(self) -> &'a [LinearRgb] {
        self.pixels
    }
    #[must_use]
    pub const fn mask(self) -> Option<&'a [f32]> {
        self.mask
    }
    #[must_use]
    pub const fn roi(self) -> BasicAdjAnalysisRoi {
        self.roi
    }
}

/// Values resolved by a single automatic analysis pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAdjResolvedValues {
    black_point: f32,
    exposure: f32,
    brightness: f32,
    contrast: f32,
    hlcompr: f32,
    hlcomprthresh: f32,
}

impl BasicAdjResolvedValues {
    #[must_use]
    pub const fn black_point(self) -> f32 {
        self.black_point
    }
    #[must_use]
    pub const fn exposure(self) -> f32 {
        self.exposure
    }
    #[must_use]
    pub const fn brightness(self) -> f32 {
        self.brightness
    }
    #[must_use]
    pub const fn contrast(self) -> f32 {
        self.contrast
    }
    #[must_use]
    pub const fn hlcompr(self) -> f32 {
        self.hlcompr
    }
    #[must_use]
    pub const fn hlcomprthresh(self) -> f32 {
        self.hlcomprthresh
    }
}

/// Stable output of automatic analysis. The histogram is retained for UI
/// inspection, while the immutable plan stores its digest and resolved values.
#[derive(Debug, Clone, PartialEq)]
pub struct BasicAdjAnalysisResult {
    controls: BasicAdjAutoControls,
    histogram: Vec<u64>,
    sample_count: u64,
    percentiles: [f32; 5],
    average: f32,
    resolved: BasicAdjResolvedValues,
    identity: [u8; 32],
}

impl BasicAdjAnalysisResult {
    #[must_use]
    pub const fn controls(&self) -> BasicAdjAutoControls {
        self.controls
    }
    #[must_use]
    pub fn histogram(&self) -> &[u64] {
        &self.histogram
    }
    #[must_use]
    pub const fn sample_count(&self) -> u64 {
        self.sample_count
    }
    /// Percentiles are p01, p25, p50, p75, and p99 in that order.
    #[must_use]
    pub const fn percentiles(&self) -> [f32; 5] {
        self.percentiles
    }
    /// Arithmetic mean of the selected channel values.
    #[must_use]
    pub const fn average(&self) -> f32 {
        self.average
    }
    #[must_use]
    pub const fn resolved_values(&self) -> BasicAdjResolvedValues {
        self.resolved
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

/// Stateless analysis entry point.
#[derive(Debug, Clone, Copy, Default)]
pub struct BasicAdjAnalysisPlan;

impl BasicAdjAnalysisPlan {
    /// Analyzes all selected RGB channels in stable row-major order.
    ///
    /// # Errors
    ///
    /// Returns an error when automatic controls are disabled, the selection
    /// has no usable samples, or the input cannot be represented safely.
    pub fn analyze(
        config: BasicAdjConfig,
        raster: BasicAdjAnalysisRaster<'_>,
    ) -> Result<BasicAdjAnalysisResult, BasicAdjAnalysisError> {
        Self::analyze_with_cancellation(config, raster, || false)
    }

    /// Checks cancellation at every source row, at least every 1024 samples,
    /// and immediately before immutable result publication. No partial
    /// histogram or plan is returned after cancellation.
    ///
    /// # Errors
    ///
    /// Returns an error when automatic controls are disabled, cancellation is
    /// requested, the selection has no usable samples, or the input cannot be
    /// represented safely.
    pub fn analyze_with_cancellation(
        config: BasicAdjConfig,
        raster: BasicAdjAnalysisRaster<'_>,
        should_cancel: impl Fn() -> bool,
    ) -> Result<BasicAdjAnalysisResult, BasicAdjAnalysisError> {
        let controls = config.auto_controls();
        if !controls.is_active() {
            return Err(BasicAdjAnalysisError::ControlsDisabled);
        }
        let roi = raster.roi();
        let width = usize::try_from(raster.dimensions().width())
            .map_err(|_| BasicAdjAnalysisError::InputTooLarge)?;
        let mut histogram = vec![0_u64; BASICADJ_HISTOGRAM_BINS];
        let mut sample_count = 0_u64;
        let mut raw_sum = 0.0_f32;
        let mut samples_since_poll = 0_usize;

        for y in roi.y()..roi.y() + roi.height() {
            if should_cancel() {
                return Err(BasicAdjAnalysisError::Cancelled);
            }
            for x in roi.x()..roi.x() + roi.width() {
                let index = usize::try_from(y)
                    .ok()
                    .and_then(|row| row.checked_mul(width))
                    .and_then(|row| row.checked_add(usize::try_from(x).ok()?))
                    .ok_or(BasicAdjAnalysisError::InputTooLarge)?;
                if raster.mask().is_some_and(|mask| {
                    let value = mask[index];
                    !value.is_finite() || value <= 0.0
                }) {
                    continue;
                }
                let pixel = raster.pixels()[index];
                for value in [pixel.red().get(), pixel.green().get(), pixel.blue().get()] {
                    let bin = bin_for(value);
                    histogram[bin] = histogram[bin]
                        .checked_add(1)
                        .ok_or(BasicAdjAnalysisError::CountOverflow)?;
                    sample_count = sample_count
                        .checked_add(1)
                        .ok_or(BasicAdjAnalysisError::CountOverflow)?;
                    raw_sum += value;
                    samples_since_poll += 1;
                    if samples_since_poll >= CANCEL_POLL_INTERVAL {
                        if should_cancel() {
                            return Err(BasicAdjAnalysisError::Cancelled);
                        }
                        samples_since_poll = 0;
                    }
                }
            }
        }
        if should_cancel() {
            return Err(BasicAdjAnalysisError::Cancelled);
        }
        if sample_count == 0 {
            return Err(BasicAdjAnalysisError::EmptySample);
        }

        let average = raw_sum / sample_count as f32;
        let percentiles = [0.01, 0.25, 0.50, 0.75, 0.99]
            .map(|quantile| percentile(&histogram, sample_count, quantile));
        let resolved = resolve_values(config, &histogram)?;
        let identity = analysis_identity(config, raster, &histogram, &resolved);
        if should_cancel() {
            return Err(BasicAdjAnalysisError::Cancelled);
        }
        Ok(BasicAdjAnalysisResult {
            controls,
            histogram,
            sample_count,
            percentiles,
            average,
            resolved,
            identity,
        })
    }
}

/// Failure from a bounded automatic analysis pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasicAdjAnalysisError {
    ControlsDisabled,
    EmptyRoi,
    EmptySample,
    Cancelled,
    InputTooLarge,
    PixelCount { expected: usize, actual: usize },
    MaskCount { expected: usize, actual: usize },
    RoiOutOfBounds,
    CountOverflow,
    NonFiniteResult(&'static str),
    Plan(BasicAdjPlanError),
}

impl fmt::Display for BasicAdjAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ControlsDisabled => {
                formatter.write_str("basicadj automatic controls are disabled")
            }
            Self::EmptyRoi => formatter.write_str("basicadj analysis ROI is empty"),
            Self::EmptySample => {
                formatter.write_str("basicadj analysis selected no usable samples")
            }
            Self::Cancelled => formatter.write_str("basicadj analysis was cancelled"),
            Self::InputTooLarge => formatter.write_str("basicadj analysis input is too large"),
            Self::PixelCount { expected, actual } => write!(
                formatter,
                "basicadj analysis has {actual} pixels, expected {expected}"
            ),
            Self::MaskCount { expected, actual } => write!(
                formatter,
                "basicadj analysis mask has {actual} values, expected {expected}"
            ),
            Self::RoiOutOfBounds => formatter.write_str("basicadj analysis ROI is out of bounds"),
            Self::CountOverflow => formatter.write_str("basicadj analysis count overflowed"),
            Self::NonFiniteResult(name) => {
                write!(formatter, "basicadj automatic {name} is non-finite")
            }
            Self::Plan(error) => write!(formatter, "basicadj analysis plan failed: {error}"),
        }
    }
}

impl std::error::Error for BasicAdjAnalysisError {}

fn bin_for(value: f32) -> usize {
    if value <= 0.0 {
        return 0;
    }
    if value >= 1.0 {
        return BASICADJ_HISTOGRAM_BINS - 1;
    }
    (value * BASICADJ_HISTOGRAM_BINS as f32) as usize
}

fn percentile(histogram: &[u64], count: u64, quantile: f32) -> f32 {
    let rank = ((count - 1) as f32 * quantile).floor() as u64;
    let mut cumulative = 0_u64;
    for (bin, amount) in histogram.iter().copied().enumerate() {
        cumulative = cumulative.saturating_add(amount);
        if cumulative > rank {
            return value_for_bin(bin);
        }
    }
    value_for_bin(histogram.len() - 1)
}

fn value_for_bin(bin: usize) -> f32 {
    (bin as f32 + 0.5) / BASICADJ_HISTOGRAM_BINS as f32
}

fn resolve_values(
    config: BasicAdjConfig,
    histogram: &[u64],
) -> Result<BasicAdjResolvedValues, BasicAdjAnalysisError> {
    let (sum, average) = sum_and_average(histogram);
    let mut median = 0_usize;
    let mut count = histogram[0];
    while (count as f32) < sum / 2.0 && median + 1 < histogram.len() {
        median += 1;
        count = count.saturating_add(histogram[median]);
    }

    if median == 0 || average < 1.0 {
        return Ok(neutral_values());
    }

    let imax = i32::try_from(histogram.len()).expect("histogram length fits i32");
    let mut octile = [0.0_f32; 8];
    let mut octile_count = 0_usize;
    let mut low_sum = 0.0_f32;
    let mut high_sum = 0.0_f32;
    let average_limit = (average as usize).min(histogram.len());

    for index in 0..average_limit {
        if octile_count < octile.len() {
            octile[octile_count] += histogram[index] as f32;
            if octile[octile_count] > sum / 8.0
                || (octile_count == 7 && octile[octile_count] > sum / 16.0)
            {
                octile[octile_count] = xlog(1.0 + index as f64) as f32 / 2.0_f32.ln();
                octile_count += 1;
            }
        }
        low_sum += histogram[index] as f32;
    }
    for index in average_limit..histogram.len() {
        if octile_count < octile.len() {
            octile[octile_count] += histogram[index] as f32;
            if octile[octile_count] > sum / 8.0
                || (octile_count == 7 && octile[octile_count] > sum / 16.0)
            {
                octile[octile_count] = xlog(1.0 + index as f64) as f32 / 2.0_f32.ln();
                octile_count += 1;
            }
        }
        high_sum += histogram[index] as f32;
    }

    if low_sum == 0.0 || high_sum == 0.0 {
        return Ok(neutral_values());
    }

    let threshold = (imax as f32 + 1.0).ln() / 2.0_f32.ln();
    let mut overex = 0_i32;
    if octile[6] > threshold {
        octile[6] = 1.5 * octile[5] - 0.5 * octile[4];
        overex = 2;
    }
    if octile[7] > threshold {
        octile[7] = 1.5 * octile[6] - 0.5 * octile[5];
        overex = 1;
    }
    let oct6 = octile[6];
    let oct7 = octile[7];
    for index in 1..8 {
        if octile[index] == 0.0 {
            octile[index] = octile[index - 1];
        }
    }

    let mut octile_spread = 0.0_f32;
    for index in 1..6 {
        let numerator = octile[index + 1] - octile[index];
        let denominator = 0.5_f32.max(if index > 2 {
            octile[index + 1] - octile[3]
        } else {
            octile[3] - octile[index]
        });
        octile_spread += numerator / denominator;
    }
    octile_spread /= 5.0;
    if octile_spread <= 0.0 {
        return Ok(neutral_values());
    }

    let mut raw_max = histogram.len() - 1;
    let mut clipped = 0_u64;
    while raw_max > 1 && histogram[raw_max].saturating_add(clipped) == 0 {
        clipped = clipped.saturating_add(histogram[raw_max]);
        raw_max -= 1;
    }

    let clippable = (sum * config.clip()).trunc() as i32 as u32;
    clipped = 0;
    let mut white_clip = histogram.len() - 1;
    while white_clip > 1 && histogram[white_clip].saturating_add(clipped) <= u64::from(clippable) {
        clipped = clipped.saturating_add(histogram[white_clip]);
        white_clip -= 1;
    }

    clipped = 0;
    let mut shadow_clip = 0_usize;
    while shadow_clip < white_clip - 1
        && histogram[shadow_clip].saturating_add(clipped) <= u64::from(clippable)
    {
        clipped = clipped.saturating_add(histogram[shadow_clip]);
        shadow_clip += 1;
    }

    let raw_max =
        (i32::try_from(raw_max).expect("native histogram index fits i32")) << HISTOGRAM_COMPRESSION;
    let white_clip = (i32::try_from(white_clip).expect("native histogram index fits i32"))
        << HISTOGRAM_COMPRESSION;
    let average = average * (1_i32 << HISTOGRAM_COMPRESSION) as f32;
    let median =
        (i32::try_from(median).expect("native histogram index fits i32")) << HISTOGRAM_COMPRESSION;
    let shadow_clip = (i32::try_from(shadow_clip).expect("native histogram index fits i32"))
        << HISTOGRAM_COMPRESSION;
    let midgray = config.middle_grey() / 100.0;

    let expcomp1 =
        (midgray * 65536.0 / (average - shadow_clip as f32 + midgray * shadow_clip as f32)).ln()
            / 2.0_f32.ln();
    let expcomp2 = if overex == 0 {
        0.5 * ((15.5 - HISTOGRAM_COMPRESSION as f32 - (2.0 * oct7 - oct6))
            + (65536.0 / raw_max as f32).ln() / 2.0_f32.ln())
    } else {
        0.5 * ((15.5 - HISTOGRAM_COMPRESSION as f32 - (2.0 * octile[7] - octile[6]))
            + (65536.0 / raw_max as f32).ln() / 2.0_f32.ln())
    };
    let mut exposure = if expcomp1.abs() - expcomp2.abs() > 1.0 {
        (expcomp1 * expcomp2.abs() + expcomp2 * expcomp1.abs()) / (expcomp2.abs() + expcomp1.abs())
    } else {
        (0.5_f64 * f64::from(expcomp1) + 0.5_f64 * f64::from(expcomp2)) as f32
    };

    let gain = (exposure * 2.0_f32.ln()).exp();
    let correction = (gain * 65536.0 / raw_max as f32).sqrt();
    let mut black = shadow_clip as f32 * correction;
    let mut highlight_compression =
        (gain * (white_clip as f32 / 65536.0 - 1.0) * 2.3) / (exposure.max(0.0) + 1.0);
    highlight_compression = highlight_compression.clamp(0.0, 100.0);

    let midtmp = gain * (median as f32 * average).sqrt() / 65536.0;
    let mut brightness = if midtmp < 0.1 {
        (midgray - midtmp) * 15.0 / midtmp
    } else {
        (midgray - midtmp) * 15.0 / (0.10833 - 0.0833 * midtmp)
    };
    brightness = (0.25 * brightness.max(0.0)).clamp(-100.0, 100.0);
    let mut contrast = (midgray * 100.0 * (1.1 - octile_spread)).clamp(0.0, 100.0) / 100.0;

    let mut white_clip_gamma = gamma2(white_clip as f32 * correction) as f32;
    let mut gamma_average = 0.0_f32;
    let increment = correction * (1_i32 << HISTOGRAM_COMPRESSION) as f32;
    let mut value = 0.0_f32;
    for amount in histogram {
        gamma_average = (f64::from(gamma_average) + *amount as f64 * gamma2(value)) as f32;
        value += increment;
    }
    gamma_average /= sum;
    if black < gamma_average {
        let max_white_clip = (gamma_average - black) * 4.0 / 3.0 + black;
        if white_clip_gamma < max_white_clip {
            white_clip_gamma = max_white_clip;
        }
    }
    white_clip_gamma = igamma2(white_clip_gamma) as f32;
    black /= white_clip_gamma;
    exposure = exposure.clamp(-5.0, 12.0);
    brightness = brightness.clamp(-100.0, 100.0);
    contrast = contrast.clamp(0.0, 1.0);

    let values = BasicAdjResolvedValues {
        black_point: nan_to_zero(black / 100.0),
        exposure: nan_to_zero(exposure),
        brightness: nan_to_zero(brightness / 100.0),
        contrast: nan_to_zero(contrast),
        hlcompr: nan_to_zero(highlight_compression),
        hlcomprthresh: 0.0,
    };
    for (name, value) in [
        ("black point", values.black_point),
        ("exposure", values.exposure),
        ("brightness", values.brightness),
        ("contrast", values.contrast),
        ("highlight compression", values.hlcompr),
        ("highlight threshold", values.hlcomprthresh),
    ] {
        if !value.is_finite() {
            return Err(BasicAdjAnalysisError::NonFiniteResult(name));
        }
    }
    Ok(values)
}

fn sum_and_average(histogram: &[u64]) -> (f32, f32) {
    let mut sum = 0.0_f32;
    let mut average = 0.0_f32;
    for (index, amount) in histogram.iter().copied().enumerate() {
        let value = amount as f32;
        sum += value;
        average += index as f32 * value;
    }
    (sum, average / sum)
}

fn neutral_values() -> BasicAdjResolvedValues {
    BasicAdjResolvedValues {
        black_point: 0.0,
        exposure: 0.0,
        brightness: 0.0,
        contrast: 0.0,
        hlcompr: 0.0,
        hlcomprthresh: 0.0,
    }
}

fn nan_to_zero(value: f32) -> f32 {
    if value.is_nan() { 0.0 } else { value }
}

fn gamma2(value: f32) -> f64 {
    let value = f64::from(value);
    if value <= 0.00304 {
        value * 12.92
    } else {
        1.055 * (value.ln() / 2.4).exp() - 0.055
    }
}

fn igamma2(value: f32) -> f64 {
    let value = f64::from(value);
    if value <= 0.03928 {
        value / 12.92
    } else {
        (((value + 0.055) / 1.055).ln() * 2.4).exp()
    }
}

fn xlog(value: f64) -> f64 {
    let exponent = ilogbp1(value * 0.7071);
    let mantissa = ldexpk(value, -exponent);
    let x = (mantissa - 1.0) / (mantissa + 1.0);
    let x2 = x * x;
    let mut term: f64 = 0.148197055177935105296783;
    term = term * x2 + 0.153108178020442575739679;
    term = term * x2 + 0.181837339521549679055568;
    term = term * x2 + 0.22222194152736701733275;
    term = term * x2 + 0.285714288030134544449368;
    term = term * x2 + 0.399999999989941956712869;
    term = term * x2 + 0.666666666666685503450651;
    term = term * x2 + 2.0;
    x * term + 0.693147180559945286226764 * exponent as f64
}

fn ilogbp1(mut value: f64) -> i32 {
    let minimum = value < 4.9090934652977266E-91;
    if minimum {
        value *= 2.037035976334486E90;
    }
    let exponent = ((value.to_bits() >> 52) & 0x7ff) as i32;
    if minimum {
        exponent - (300 + 0x03fe)
    } else {
        exponent - 0x03fe
    }
}

fn ldexpk(mut value: f64, mut exponent: i32) -> f64 {
    let sign = if exponent < 0 { -1 } else { 0 };
    let scale = (((sign + exponent) >> 9) - sign) << 7;
    exponent -= scale << 2;
    let unit = f64::from_bits(((scale + 0x3ff) as u64) << 52);
    let mut unit_squared = unit * unit;
    unit_squared *= unit_squared;
    value *= unit_squared;
    let final_unit = f64::from_bits(((exponent + 0x3ff) as u64) << 52);
    value * final_unit
}

fn analysis_identity(
    config: BasicAdjConfig,
    raster: BasicAdjAnalysisRaster<'_>,
    histogram: &[u64],
    resolved: &BasicAdjResolvedValues,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.basicadj.analysis.v2");
    hasher.update([config.auto_controls().bits()]);
    hasher.update(config.clip().to_bits().to_le_bytes());
    hasher.update(config.middle_grey().to_bits().to_le_bytes());
    hasher.update(raster.dimensions().width().to_le_bytes());
    hasher.update(raster.dimensions().height().to_le_bytes());
    hasher.update(raster.roi().x().to_le_bytes());
    hasher.update(raster.roi().y().to_le_bytes());
    hasher.update(raster.roi().width().to_le_bytes());
    hasher.update(raster.roi().height().to_le_bytes());
    for count in histogram {
        hasher.update(count.to_le_bytes());
    }
    for value in [
        resolved.black_point,
        resolved.exposure,
        resolved.brightness,
        resolved.contrast,
        resolved.hlcompr,
        resolved.hlcomprthresh,
    ] {
        hasher.update(value.to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BasicAdjAutoControls, BasicAdjParametersV2};

    fn pixel(value: f32) -> LinearRgb {
        LinearRgb::new(
            crate::FiniteF32::new(value).expect("finite"),
            crate::FiniteF32::new(value).expect("finite"),
            crate::FiniteF32::new(value).expect("finite"),
        )
    }

    #[test]
    fn histogram_uses_native_zero_one_bins_and_clamps_hdr() {
        let dimensions = RasterDimensions::new(3, 1).expect("dimensions");
        let pixels = [pixel(-1.0), pixel(0.5), pixel(2.0)];
        let config = BasicAdjConfig::new(BasicAdjParametersV2::defaults())
            .expect("config")
            .with_auto_controls(BasicAdjAutoControls::all());
        let raster = BasicAdjAnalysisRaster::new(dimensions, &pixels, None).expect("raster");
        let result = BasicAdjAnalysisPlan::analyze(config, raster).expect("analysis");
        assert_eq!(result.histogram()[0], 3);
        assert_eq!(result.histogram()[4096], 3);
        assert_eq!(result.histogram()[BASICADJ_HISTOGRAM_BINS - 1], 3);
    }

    #[test]
    fn cancellation_is_polled_inside_large_rows() {
        let dimensions = RasterDimensions::new(2048, 1).expect("dimensions");
        let pixels = vec![pixel(0.5); 2048];
        let config = BasicAdjConfig::defaults().with_auto_controls(BasicAdjAutoControls::all());
        let polls = std::cell::Cell::new(0_usize);
        let raster = BasicAdjAnalysisRaster::new(dimensions, &pixels, None).expect("raster");
        let result = BasicAdjAnalysisPlan::analyze_with_cancellation(config, raster, || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 2
        });
        assert_eq!(result, Err(BasicAdjAnalysisError::Cancelled));
        assert!(polls.get() >= 2);
    }
}
