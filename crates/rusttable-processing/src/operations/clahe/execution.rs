#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines
)]

use super::codec::{CLAHE_SCHEMA_VERSION, ClaheConfig, ClaheParametersV1};
use crate::{RasterDimensions, operations::ReconstructionBudget};
use sha2::{Digest, Sha256};
use std::fmt;

pub const CLAHE_BINS: usize = 256;
pub const CLAHE_HISTOGRAM_ENTRIES: usize = CLAHE_BINS + 1;
const CLAHE_HISTOGRAM_ENTRIES_I64: i64 = 257;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClahePixel {
    channels: [f32; 4],
}

impl ClahePixel {
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

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClaheBackend {
    CpuScalarReference,
}

impl ClaheBackend {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        "cpu-scalar-reference"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClaheOutcome {
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaheExecutionError {
    ArithmeticOverflow,
    DimensionsMismatch { expected: usize, actual: usize },
    InvalidScale,
    InvalidTileWidth,
    MemoryBudgetExceeded { required: usize, budget: usize },
    Cancelled,
    NonFiniteInput { pixel: usize, channel: usize },
    NonFiniteResult { pixel: usize, channel: usize },
    MaskLength { expected: usize, actual: usize },
    InvalidMaskValue,
}

impl fmt::Display for ClaheExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => formatter.write_str("clahe arithmetic overflowed"),
            Self::DimensionsMismatch { expected, actual } => {
                write!(formatter, "clahe expected {expected} pixels, got {actual}")
            }
            Self::InvalidScale => formatter.write_str("clahe scales must be finite and positive"),
            Self::InvalidTileWidth => formatter.write_str("clahe tile width must be nonzero"),
            Self::MemoryBudgetExceeded { required, budget } => {
                write!(
                    formatter,
                    "clahe requires {required} bytes, budget is {budget}"
                )
            }
            Self::Cancelled => formatter.write_str("clahe execution was cancelled"),
            Self::NonFiniteInput { pixel, channel } => {
                write!(
                    formatter,
                    "clahe input pixel {pixel} channel {channel} is non-finite"
                )
            }
            Self::NonFiniteResult { pixel, channel } => {
                write!(
                    formatter,
                    "clahe result pixel {pixel} channel {channel} is non-finite"
                )
            }
            Self::MaskLength { expected, actual } => {
                write!(
                    formatter,
                    "clahe mask has {actual} pixels, expected {expected}"
                )
            }
            Self::InvalidMaskValue => {
                formatter.write_str("clahe mask coverage must be finite in 0..=1")
            }
        }
    }
}

impl std::error::Error for ClaheExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaheMask {
    identity: [u8; 32],
}

impl ClaheMask {
    pub fn new(coverage: &[f32]) -> Result<Self, ClaheExecutionError> {
        if coverage
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(ClaheExecutionError::InvalidMaskValue);
        }
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.clahe.mask.v1");
        for value in coverage {
            hasher.update(value.to_bits().to_le_bytes());
        }
        Ok(Self {
            identity: hasher.finalize().into(),
        })
    }

    #[must_use]
    pub const fn identity(self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClaheBlend {
    identity: [u8; 32],
}

impl ClaheBlend {
    #[must_use]
    pub const fn normal() -> Self {
        Self { identity: [0; 32] }
    }

    #[must_use]
    pub const fn identity(self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClaheReceipt {
    parameters: ClaheParametersV1,
    input_scale: f32,
    roi_scale: f32,
    resolved_radius: usize,
    roi_width: u32,
    roi_height: u32,
    window_radius: usize,
    histogram_entries: usize,
    full_image: bool,
    backend: ClaheBackend,
    outcome: ClaheOutcome,
    memory_estimate: usize,
    mask_identity: [u8; 32],
    blend_identity: [u8; 32],
    input_identity: [u8; 32],
    output_identity: [u8; 32],
}

impl ClaheReceipt {
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        CLAHE_SCHEMA_VERSION
    }
    #[must_use]
    pub const fn parameters(&self) -> ClaheParametersV1 {
        self.parameters
    }
    #[must_use]
    pub const fn input_scale(&self) -> f32 {
        self.input_scale
    }
    #[must_use]
    pub const fn roi_scale(&self) -> f32 {
        self.roi_scale
    }
    #[must_use]
    pub const fn resolved_radius(&self) -> usize {
        self.resolved_radius
    }
    #[must_use]
    pub const fn roi_width(&self) -> u32 {
        self.roi_width
    }
    #[must_use]
    pub const fn roi_height(&self) -> u32 {
        self.roi_height
    }
    #[must_use]
    pub const fn window_radius(&self) -> usize {
        self.window_radius
    }
    #[must_use]
    pub const fn histogram_entries(&self) -> usize {
        self.histogram_entries
    }
    #[must_use]
    pub const fn full_image(&self) -> bool {
        self.full_image
    }
    #[must_use]
    pub const fn backend(&self) -> ClaheBackend {
        self.backend
    }
    #[must_use]
    pub const fn outcome(&self) -> ClaheOutcome {
        self.outcome
    }
    #[must_use]
    pub const fn memory_estimate(&self) -> usize {
        self.memory_estimate
    }
    #[must_use]
    pub const fn mask_identity(&self) -> [u8; 32] {
        self.mask_identity
    }
    #[must_use]
    pub const fn blend_identity(&self) -> [u8; 32] {
        self.blend_identity
    }
    #[must_use]
    pub const fn input_identity(&self) -> [u8; 32] {
        self.input_identity
    }
    #[must_use]
    pub const fn output_identity(&self) -> [u8; 32] {
        self.output_identity
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClahePlan {
    config: ClaheConfig,
    dimensions: RasterDimensions,
    input_scale: f32,
    roi_scale: f32,
    resolved_radius: usize,
    memory_estimate: usize,
    full_image: bool,
    backend: ClaheBackend,
}

impl ClahePlan {
    pub fn new(
        config: ClaheConfig,
        dimensions: RasterDimensions,
        input_scale: f32,
        roi_scale: f32,
    ) -> Result<Self, ClaheExecutionError> {
        Self::with_budget(
            config,
            dimensions,
            input_scale,
            roi_scale,
            ReconstructionBudget::default().maximum_bytes(),
        )
    }

    pub fn with_budget(
        config: ClaheConfig,
        dimensions: RasterDimensions,
        input_scale: f32,
        roi_scale: f32,
        budget: usize,
    ) -> Result<Self, ClaheExecutionError> {
        if !input_scale.is_finite()
            || !roi_scale.is_finite()
            || input_scale <= 0.0
            || roi_scale <= 0.0
        {
            return Err(ClaheExecutionError::InvalidScale);
        }
        let resolved = config.radius() * f64::from(roi_scale) / f64::from(input_scale);
        if !resolved.is_finite() || resolved < 0.0 || resolved >= usize::MAX as f64 {
            return Err(ClaheExecutionError::ArithmeticOverflow);
        }
        let resolved_radius = resolved as usize;
        let pixels = usize::try_from(dimensions.pixel_count())
            .map_err(|_| ClaheExecutionError::ArithmeticOverflow)?;
        let image_bytes = pixels
            .checked_mul(std::mem::size_of::<ClahePixel>())
            .and_then(|value| value.checked_add(pixels.checked_mul(4)?))
            .ok_or(ClaheExecutionError::ArithmeticOverflow)?;
        let row_bytes = usize::try_from(dimensions.width())
            .map_err(|_| ClaheExecutionError::ArithmeticOverflow)?
            .checked_mul(4)
            .ok_or(ClaheExecutionError::ArithmeticOverflow)?;
        let histogram_bytes = CLAHE_HISTOGRAM_ENTRIES
            .checked_mul(std::mem::size_of::<i64>())
            .and_then(|value| value.checked_mul(2))
            .ok_or(ClaheExecutionError::ArithmeticOverflow)?;
        let memory_estimate = image_bytes
            .checked_add(row_bytes)
            .and_then(|value| value.checked_add(histogram_bytes))
            .ok_or(ClaheExecutionError::ArithmeticOverflow)?;
        if memory_estimate > budget {
            return Err(ClaheExecutionError::MemoryBudgetExceeded {
                required: memory_estimate,
                budget,
            });
        }
        Ok(Self {
            config,
            dimensions,
            input_scale,
            roi_scale,
            resolved_radius,
            memory_estimate,
            full_image: true,
            backend: ClaheBackend::CpuScalarReference,
        })
    }

    #[must_use]
    pub const fn config(&self) -> ClaheConfig {
        self.config
    }
    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn input_scale(&self) -> f32 {
        self.input_scale
    }
    #[must_use]
    pub const fn roi_scale(&self) -> f32 {
        self.roi_scale
    }
    #[must_use]
    pub const fn resolved_radius(&self) -> usize {
        self.resolved_radius
    }
    #[must_use]
    pub const fn histogram_entries(&self) -> usize {
        CLAHE_HISTOGRAM_ENTRIES
    }
    #[must_use]
    pub const fn full_image(&self) -> bool {
        self.full_image
    }
    #[must_use]
    pub const fn memory_estimate(&self) -> usize {
        self.memory_estimate
    }
    #[must_use]
    pub const fn backend(&self) -> ClaheBackend {
        self.backend
    }

    pub fn execute<F: FnMut() -> bool>(
        &self,
        input: &[ClahePixel],
        cancelled: F,
    ) -> Result<Vec<ClahePixel>, ClaheExecutionError> {
        self.execute_with_mask(input, None, 1.0, cancelled)
    }

    pub fn execute_with_mask<F: FnMut() -> bool>(
        &self,
        input: &[ClahePixel],
        mask: Option<&[f32]>,
        opacity: f32,
        mut cancelled: F,
    ) -> Result<Vec<ClahePixel>, ClaheExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| ClaheExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(ClaheExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if let Some(values) = mask {
            if values.len() != expected {
                return Err(ClaheExecutionError::MaskLength {
                    expected,
                    actual: values.len(),
                });
            }
            ClaheMask::new(values)?;
        }
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(ClaheExecutionError::InvalidMaskValue);
        }
        for (pixel, value) in input.iter().enumerate() {
            for (channel, sample) in value.channels().into_iter().enumerate() {
                if !sample.is_finite() {
                    return Err(ClaheExecutionError::NonFiniteInput { pixel, channel });
                }
            }
        }
        if cancelled() {
            return Err(ClaheExecutionError::Cancelled);
        }
        let width = usize::try_from(self.dimensions.width())
            .map_err(|_| ClaheExecutionError::ArithmeticOverflow)?;
        let height = usize::try_from(self.dimensions.height())
            .map_err(|_| ClaheExecutionError::ArithmeticOverflow)?;
        let luminance = input
            .iter()
            .map(|pixel| luminance(pixel.channels()))
            .collect::<Vec<_>>();
        let mut output = Vec::with_capacity(expected);
        let mut destination = vec![0.0; width];
        for y in 0..height {
            if cancelled() {
                return Err(ClaheExecutionError::Cancelled);
            }
            self.map_row(&luminance, y, width, height, &mut destination)?;
            for (x, mapped_luminance) in destination.iter().copied().enumerate() {
                let index = y
                    .checked_mul(width)
                    .and_then(|value| value.checked_add(x))
                    .ok_or(ClaheExecutionError::ArithmeticOverflow)?;
                let source = input[index].channels();
                let (hue, saturation, _) = crate::operations::common::rgb_to_hsl(
                    source[..3].try_into().expect("three RGB channels"),
                );
                let rgb = crate::operations::common::hsl_to_rgb(hue, saturation, mapped_luminance);
                if rgb.iter().any(|value| !value.is_finite()) {
                    return Err(ClaheExecutionError::NonFiniteResult {
                        pixel: index,
                        channel: 0,
                    });
                }
                let amount = mask.map_or(1.0, |values| values[index]) * opacity;
                let result = [
                    source[0] + (rgb[0] - source[0]) * amount,
                    source[1] + (rgb[1] - source[1]) * amount,
                    source[2] + (rgb[2] - source[2]) * amount,
                    source[3],
                ];
                for (channel, sample) in result.into_iter().enumerate() {
                    if !sample.is_finite() {
                        return Err(ClaheExecutionError::NonFiniteResult {
                            pixel: index,
                            channel,
                        });
                    }
                }
                output.push(ClahePixel::from_channels(result));
            }
        }
        Ok(output)
    }

    pub fn execute_with_receipt<F: FnMut() -> bool>(
        &self,
        input: &[ClahePixel],
        mask: Option<&[f32]>,
        opacity: f32,
        mut cancelled: F,
    ) -> Result<(Vec<ClahePixel>, ClaheReceipt), ClaheExecutionError> {
        let output = self.execute_with_mask(input, mask, opacity, &mut cancelled)?;
        let input_identity = digest_pixels(input, b"rusttable.clahe.input.v1");
        let output_identity = digest_pixels(&output, b"rusttable.clahe.output.v1");
        let mask_identity = mask.map_or([0; 32], |values| {
            ClaheMask::new(values).expect("validated mask").identity()
        });
        Ok((
            output,
            ClaheReceipt {
                parameters: self.config.parameters(),
                input_scale: self.input_scale,
                roi_scale: self.roi_scale,
                resolved_radius: self.resolved_radius,
                roi_width: self.dimensions.width(),
                roi_height: self.dimensions.height(),
                window_radius: self.resolved_radius,
                histogram_entries: CLAHE_HISTOGRAM_ENTRIES,
                full_image: self.full_image,
                backend: self.backend,
                outcome: ClaheOutcome::Complete,
                memory_estimate: self.memory_estimate,
                mask_identity,
                blend_identity: ClaheBlend::normal().identity(),
                input_identity,
                output_identity,
            },
        ))
    }

    pub fn execute_tiled<F: FnMut() -> bool>(
        &self,
        input: &[ClahePixel],
        tile_width: u32,
        cancelled: F,
    ) -> Result<Vec<ClahePixel>, ClaheExecutionError> {
        if tile_width == 0 {
            return Err(ClaheExecutionError::InvalidTileWidth);
        }
        self.execute(input, cancelled)
    }

    #[must_use]
    pub fn cache_identity(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.clahe.plan.v1");
        hasher.update(self.config.parameters().to_bytes());
        hasher.update(self.dimensions.width().to_le_bytes());
        hasher.update(self.dimensions.height().to_le_bytes());
        hasher.update(self.input_scale.to_bits().to_le_bytes());
        hasher.update(self.roi_scale.to_bits().to_le_bytes());
        hasher.update((self.resolved_radius as u64).to_le_bytes());
        hasher.update((CLAHE_HISTOGRAM_ENTRIES as u32).to_le_bytes());
        hasher.update([u8::from(self.full_image)]);
        hasher.update(self.backend.tag().as_bytes());
        hasher.finalize().into()
    }

    fn map_row(
        &self,
        luminance: &[f32],
        y: usize,
        width: usize,
        height: usize,
        destination: &mut [f32],
    ) -> Result<(), ClaheExecutionError> {
        let radius = self.resolved_radius;
        let y_min = y.saturating_sub(radius);
        let y_max = y
            .checked_add(radius)
            .and_then(|value| value.checked_add(1))
            .ok_or(ClaheExecutionError::ArithmeticOverflow)?
            .min(height);
        let x_max0 = width.saturating_sub(1).min(radius);
        let mut histogram = [0_i64; CLAHE_HISTOGRAM_ENTRIES];
        let mut clipped = [0_i64; CLAHE_HISTOGRAM_ENTRIES];
        for yi in y_min..y_max {
            for xi in 0..x_max0 {
                histogram[bin(luminance[yi * width + xi])] += 1;
            }
        }
        for x in 0..width {
            let value_bin = bin(luminance[y * width + x]);
            let x_min = x.saturating_sub(radius);
            let x_max = x
                .checked_add(radius)
                .and_then(|value| value.checked_add(1))
                .ok_or(ClaheExecutionError::ArithmeticOverflow)?;
            let window_width = x_max.min(width) - x_min;
            let sample_count = i64::try_from(
                (y_max - y_min)
                    .checked_mul(window_width)
                    .ok_or(ClaheExecutionError::ArithmeticOverflow)?,
            )
            .map_err(|_| ClaheExecutionError::ArithmeticOverflow)?;
            let limit =
                (self.config.slope() * sample_count as f64 / CLAHE_BINS as f64 + 0.5) as i64;
            if x_min > 0 {
                let removed = x_min - 1;
                for yi in y_min..y_max {
                    histogram[bin(luminance[yi * width + removed])] -= 1;
                }
            }
            if x_max <= width {
                let added = x_max - 1;
                for yi in y_min..y_max {
                    histogram[bin(luminance[yi * width + added])] += 1;
                }
            }
            clipped.copy_from_slice(&histogram);
            clip_histogram(&mut clipped, limit);
            let h_min = clipped
                .iter()
                .take(CLAHE_BINS)
                .position(|value| *value != 0)
                .unwrap_or(CLAHE_BINS);
            let cdf = if h_min <= value_bin {
                clipped[h_min..=value_bin].iter().sum::<i64>()
            } else {
                0
            };
            let cdf_max = cdf + clipped[(value_bin + 1)..].iter().sum::<i64>();
            let cdf_min = clipped[h_min];
            destination[x] = if cdf_max == cdf_min {
                luminance[y * width + x]
            } else {
                (cdf - cdf_min) as f32 / (cdf_max - cdf_min) as f32
            };
        }
        Ok(())
    }
}

fn clip_histogram(histogram: &mut [i64; CLAHE_HISTOGRAM_ENTRIES], limit: i64) {
    let mut previous = -1_i64;
    let mut clipped_entries = 0_i64;
    while clipped_entries != previous {
        previous = clipped_entries;
        clipped_entries = 0;
        for value in histogram.iter_mut() {
            let excess = *value - limit;
            if excess > 0 {
                clipped_entries += excess;
                *value = limit;
            }
        }
        let distributed = clipped_entries / CLAHE_HISTOGRAM_ENTRIES_I64;
        let remainder = clipped_entries % CLAHE_HISTOGRAM_ENTRIES_I64;
        for value in histogram.iter_mut() {
            *value += distributed;
        }
        if remainder != 0 {
            let step = (CLAHE_BINS as f64 / remainder as f64) as usize;
            for bin in (0..=CLAHE_BINS).step_by(step.max(1)) {
                histogram[bin] += 1;
            }
        }
    }
}

fn luminance(channels: [f32; 4]) -> f32 {
    let maximum = channels[0]
        .max(channels[1])
        .max(channels[2])
        .clamp(0.0, 1.0);
    let minimum = channels[0]
        .min(channels[1])
        .min(channels[2])
        .clamp(0.0, 1.0);
    f32::midpoint(maximum, minimum)
}

fn bin(value: f32) -> usize {
    ((f64::from(value) * CLAHE_BINS as f64 + 0.5) as usize).min(CLAHE_BINS)
}

fn digest_pixels(pixels: &[ClahePixel], domain: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for pixel in pixels {
        for value in pixel.channels() {
            hasher.update(value.to_bits().to_le_bytes());
        }
    }
    hasher.finalize().into()
}
