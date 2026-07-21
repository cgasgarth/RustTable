//! Deterministic automatic analysis for the legacy `basicadj` operation.
//!
//! The analysis is deliberately independent from the pixelpipe.  A caller
//! supplies one immutable raster and optional mask/ROI, receives one resolved
//! result, and can reuse that result for full-frame, preview, export, and
//! tiled execution.  Histogram bins use a fixed range and lower-bound ties so
//! parallel tile aggregation has one stable answer.

use std::fmt;

use sha2::{Digest, Sha256};

use crate::{LinearRgb, RasterDimensions};

use super::basicadj::{BasicAdjAutoControls, BasicAdjConfig, BasicAdjPlanError};

/// Darktable's legacy histogram compression: 65536 values shifted by three.
pub const BASICADJ_HISTOGRAM_BINS: usize = 8192;
/// Fixed analysis range that retains negative and HDR samples without an
/// allocation proportional to image dimensions.
pub const BASICADJ_HISTOGRAM_MINIMUM: f32 = -4.0;
pub const BASICADJ_HISTOGRAM_MAXIMUM: f32 = 16.0;
/// Maximum sampled pixels per analysis pass. Sampling is deterministic when
/// an image is larger than this bound.
pub const BASICADJ_MAX_ANALYSIS_PIXELS: usize = 1_048_576;

/// Checked row-major rectangle used by automatic analysis.
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

/// Borrowed analysis raster with optional one-value-per-pixel mask.
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
/// inspection, while the immutable plan stores only its digest and resolved
/// values.
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

    /// The cancellation hook is checked once per source row and before the
    /// result is published. No partial histogram is returned.
    ///
    /// # Errors
    ///
    /// Returns an error when automatic controls are disabled, cancellation is
    /// requested, the selection has no usable samples, or the input cannot be
    /// represented safely.
    #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
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
        let area = usize::try_from(u64::from(roi.width()) * u64::from(roi.height()))
            .map_err(|_| BasicAdjAnalysisError::InputTooLarge)?;
        let stride = area.div_ceil(BASICADJ_MAX_ANALYSIS_PIXELS).max(1);
        let mut histogram = vec![0_u64; BASICADJ_HISTOGRAM_BINS];
        let mut sample_count = 0_u64;
        let mut sum = 0.0_f64;
        let width = usize::try_from(raster.dimensions().width())
            .map_err(|_| BasicAdjAnalysisError::InputTooLarge)?;
        for y in roi.y()..roi.y() + roi.height() {
            if should_cancel() {
                return Err(BasicAdjAnalysisError::Cancelled);
            }
            for x in roi.x()..roi.x() + roi.width() {
                let ordinal = usize::try_from(
                    u64::from(y - roi.y()) * u64::from(roi.width()) + u64::from(x - roi.x()),
                )
                .map_err(|_| BasicAdjAnalysisError::InputTooLarge)?;
                if !ordinal.is_multiple_of(stride) {
                    continue;
                }
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
                    sum += f64::from(value);
                    sample_count = sample_count
                        .checked_add(1)
                        .ok_or(BasicAdjAnalysisError::CountOverflow)?;
                }
            }
        }
        if should_cancel() {
            return Err(BasicAdjAnalysisError::Cancelled);
        }
        if sample_count == 0 {
            return Err(BasicAdjAnalysisError::EmptySample);
        }
        let average = (sum / sample_count as f64) as f32;
        let percentiles = [0.01, 0.25, 0.50, 0.75, 0.99]
            .map(|quantile| percentile(&histogram, sample_count, quantile));
        let resolved = resolve_values(config, average, percentiles);
        let identity = analysis_identity(config, raster, stride, &histogram, &resolved);
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
                formatter.write_str("basicadj analysis selected no finite samples")
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
            Self::Plan(error) => write!(formatter, "basicadj analysis plan failed: {error}"),
        }
    }
}

impl std::error::Error for BasicAdjAnalysisError {}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
fn bin_for(value: f32) -> usize {
    if value <= BASICADJ_HISTOGRAM_MINIMUM {
        return 0;
    }
    if value >= BASICADJ_HISTOGRAM_MAXIMUM {
        return BASICADJ_HISTOGRAM_BINS - 1;
    }
    let fraction = (value - BASICADJ_HISTOGRAM_MINIMUM)
        / (BASICADJ_HISTOGRAM_MAXIMUM - BASICADJ_HISTOGRAM_MINIMUM);
    (fraction * BASICADJ_HISTOGRAM_BINS as f32).floor() as usize
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
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

#[allow(clippy::cast_precision_loss)]
fn value_for_bin(bin: usize) -> f32 {
    BASICADJ_HISTOGRAM_MINIMUM
        + (bin as f32 + 0.5) * (BASICADJ_HISTOGRAM_MAXIMUM - BASICADJ_HISTOGRAM_MINIMUM)
            / BASICADJ_HISTOGRAM_BINS as f32
}

#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
fn resolve_values(
    config: BasicAdjConfig,
    average: f32,
    percentiles: [f32; 5],
) -> BasicAdjResolvedValues {
    let middle_grey = config.middle_grey() / 100.0;
    let low = percentiles[0];
    let median = percentiles[2];
    let high = percentiles[4];
    if median <= 0.0 || average <= 0.0 {
        return neutral_values();
    }
    let safe_median = median.max(0.000_001);
    let exp_from_average = (middle_grey / average.max(0.000_001)).log2();
    let exp_from_median = (middle_grey / safe_median).log2();
    let exposure = exp_from_average.midpoint(exp_from_median).clamp(-5.0, 12.0);
    let black_point = low.clamp(-1.0, 1.0);
    let spread = (percentiles[3] - percentiles[1]).max(0.000_001);
    let contrast = ((middle_grey * 100.0) * (1.1 - spread)).clamp(0.0, 100.0) / 100.0;
    let mid_after_gain = (safe_median * 2.0_f32.powf(exposure)).max(0.000_001);
    let brightness = ((middle_grey - mid_after_gain) / mid_after_gain * 3.75).clamp(-4.0, 4.0);
    let hlcompr = ((high * 2.0_f32.powf(exposure) - 1.0) * 230.0).clamp(0.0, 500.0);
    let hlcomprthresh = ((high - 1.0) * 800.0).clamp(0.0, 100.0);
    BasicAdjResolvedValues {
        black_point,
        exposure,
        brightness,
        contrast,
        hlcompr,
        hlcomprthresh,
    }
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

fn analysis_identity(
    config: BasicAdjConfig,
    raster: BasicAdjAnalysisRaster<'_>,
    stride: usize,
    histogram: &[u64],
    resolved: &BasicAdjResolvedValues,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.basicadj.analysis.v1");
    hasher.update([config.auto_controls().bits()]);
    hasher.update(config.clip().to_bits().to_le_bytes());
    hasher.update(config.middle_grey().to_bits().to_le_bytes());
    hasher.update(raster.dimensions().width().to_le_bytes());
    hasher.update(raster.dimensions().height().to_le_bytes());
    hasher.update(raster.roi().x().to_le_bytes());
    hasher.update(raster.roi().y().to_le_bytes());
    hasher.update(raster.roi().width().to_le_bytes());
    hasher.update(raster.roi().height().to_le_bytes());
    hasher.update(stride.to_le_bytes());
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
