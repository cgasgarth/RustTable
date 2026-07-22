use super::{
    BILATERAL_RANGE_SIGMA, LAB_MAXIMUM, LAB_MINIMUM, MAX_BILATERAL_SUPPORT, ShadhiAlgorithm,
    ShadhiConfig, UNBOUND_BILATERAL, UNBOUND_GAUSSIAN, UNBOUND_HIGHLIGHTS_A, UNBOUND_HIGHLIGHTS_B,
    UNBOUND_HIGHLIGHTS_L, UNBOUND_L, UNBOUND_SHADOWS_A, UNBOUND_SHADOWS_B, UNBOUND_SHADOWS_L,
};
use sha2::{Digest, Sha256};

use crate::operations::common::{
    OperationExecutionError, ReconstructionBudget, checked_bytes, validate_shape,
};
use crate::operations::convolution::{BoundedGaussianError, bounded_gaussian_4c_order};
use crate::{FiniteF32, LinearRgb, RasterDimensions};

/// Four-channel D50 Lab sample in Darktable's native scale: L in 0..100,
/// a/b in -128..128, and an opaque spare/alpha channel in 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadhiPixel {
    channels: [f32; 4],
}

impl ShadhiPixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
        }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
        self.channels
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; 4]) -> Self {
        Self { channels }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShadhiPlan {
    config: ShadhiConfig,
    dimensions: RasterDimensions,
    sigma: f32,
    overlap: u32,
    analysis_identity: [u8; 32],
}

/// Frozen execution evidence carried by preview/export receipts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadhiReceipt {
    plan: [u8; 32],
    input: [u8; 32],
    output: [u8; 32],
    mask: [u8; 32],
}

impl ShadhiReceipt {
    #[must_use]
    pub const fn plan_identity(self) -> [u8; 32] {
        self.plan
    }

    #[must_use]
    pub const fn input_identity(self) -> [u8; 32] {
        self.input
    }

    #[must_use]
    pub const fn output_identity(self) -> [u8; 32] {
        self.output
    }

    #[must_use]
    pub const fn mask_identity(self) -> [u8; 32] {
        self.mask
    }
}

impl ShadhiPlan {
    pub fn new(
        config: ShadhiConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        let sigma = config.radius().max(0.1);
        let overlap = (4.0 * sigma).ceil().min(256.0) as u32;
        checked_bytes(
            usize::try_from(dimensions.pixel_count()).map_err(|_| {
                OperationExecutionError::MemoryBudgetExceeded {
                    required: usize::MAX,
                    budget: ReconstructionBudget::default().maximum_bytes(),
                }
            })?,
            8,
            ReconstructionBudget::default(),
        )?;
        let analysis_identity = digest_plan(config, dimensions, sigma, overlap);
        Ok(Self {
            config,
            dimensions,
            sigma,
            overlap,
            analysis_identity,
        })
    }

    #[must_use]
    pub const fn radius_pixels(&self) -> u32 {
        self.overlap
    }

    #[must_use]
    pub const fn cache_identity(&self) -> [u8; 32] {
        self.analysis_identity
    }

    /// Executes the native four-channel Lab operation with optional mask and
    /// blend opacity. The same full-image plan is used for preview and export.
    pub fn execute_lab<F: FnMut() -> bool>(
        &self,
        input: &[ShadhiPixel],
        mask: Option<&[f32]>,
        opacity: f32,
        mut cancelled: F,
    ) -> Result<Vec<ShadhiPixel>, OperationExecutionError> {
        let expected = validate_lab_shape(self.dimensions, input)?;
        validate_mask(mask, expected)?;
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(OperationExecutionError::NonFiniteResult {
                pixel: 0,
                channel: crate::RgbChannel::Red,
            });
        }
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        let filtered = self.filter(input, &mut cancelled)?;
        let mut output = Vec::with_capacity(expected);
        for (index, (source, base)) in input.iter().zip(filtered).enumerate() {
            if index % usize::try_from(self.dimensions.width()).expect("validated width") == 0
                && cancelled()
            {
                return Err(OperationExecutionError::Cancelled);
            }
            let candidate = mix_lab(*source, base, self.config);
            let coverage = mask.map_or(1.0, |values| values[index]) * opacity;
            output.push(blend_pixel(*source, candidate, coverage, index)?);
        }
        Ok(output)
    }

    /// Executes and records the frozen plan, input, output, and mask identity.
    pub fn execute_with_receipt<F: FnMut() -> bool>(
        &self,
        input: &[ShadhiPixel],
        mask: Option<&[f32]>,
        opacity: f32,
        cancelled: F,
    ) -> Result<(Vec<ShadhiPixel>, ShadhiReceipt), OperationExecutionError> {
        let output = self.execute_lab(input, mask, opacity, cancelled)?;
        let receipt = ShadhiReceipt {
            plan: self.analysis_identity,
            input: digest_pixels(input),
            output: digest_pixels(&output),
            mask: digest_mask(mask),
        };
        Ok((output, receipt))
    }

    /// Runs the frozen full-image plan for a tiled request. Filtering remains
    /// whole-image so overlap and bilateral evidence cannot vary by tile.
    pub fn execute_tiled<F: FnMut() -> bool>(
        &self,
        input: &[ShadhiPixel],
        mask: Option<&[f32]>,
        opacity: f32,
        cancelled: F,
    ) -> Result<Vec<ShadhiPixel>, OperationExecutionError> {
        self.execute_lab(input, mask, opacity, cancelled)
    }

    /// Compatibility transport for callers that have not yet adopted the
    /// public Lab sample type. Its three channels are interpreted as
    /// normalized Lab coordinates, never as an RGB approximation.
    pub fn execute(&self, input: &[LinearRgb]) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        validate_shape(self.dimensions, input)?;
        let lab = input
            .iter()
            .map(|pixel| {
                ShadhiPixel::new(
                    pixel.red().get() * 100.0,
                    pixel.green().get() * 128.0,
                    pixel.blue().get() * 128.0,
                    1.0,
                )
            })
            .collect::<Vec<_>>();
        self.execute_lab(&lab, None, 1.0, || false).map(|output| {
            output
                .into_iter()
                .map(|pixel| {
                    let [l, a, b, _] = pixel.channels();
                    LinearRgb::new(
                        finite_rgb(l / 100.0),
                        finite_rgb(a / 128.0),
                        finite_rgb(b / 128.0),
                    )
                })
                .collect()
        })
    }

    fn filter<F: FnMut() -> bool>(
        &self,
        input: &[ShadhiPixel],
        cancelled: &mut F,
    ) -> Result<Vec<ShadhiPixel>, OperationExecutionError> {
        let channels = input
            .iter()
            .map(|pixel| pixel.channels())
            .collect::<Vec<_>>();
        match self.config.shadhi_algo() {
            ShadhiAlgorithm::Gaussian => {
                let (minimum, maximum) = bounds(self.config.flags(), true);
                bounded_gaussian_4c_order(
                    &channels,
                    self.dimensions,
                    self.sigma,
                    minimum,
                    maximum,
                    self.config.order(),
                    cancelled,
                )
                .map(|values| values.into_iter().map(ShadhiPixel::from_channels).collect())
                .map_err(map_filter_error)
            }
            ShadhiAlgorithm::Bilateral => bilateral_filter(
                &channels,
                self.dimensions,
                self.sigma,
                self.config.flags(),
                cancelled,
            )
            .map(|values| values.into_iter().map(ShadhiPixel::from_channels).collect()),
        }
    }
}

fn mix_lab(source: ShadhiPixel, base: ShadhiPixel, config: ShadhiConfig) -> ShadhiPixel {
    let mut source = source.channels();
    let mut base = base.channels();
    let unbound = ((config.shadhi_algo() == ShadhiAlgorithm::Bilateral)
        && config.flags() & UNBOUND_BILATERAL != 0)
        || ((config.shadhi_algo() == ShadhiAlgorithm::Gaussian)
            && config.flags() & UNBOUND_GAUSSIAN != 0);
    source[0] /= 100.0;
    base[0] = 1.0 - base[0] / 100.0;
    source[1] /= 128.0;
    source[2] /= 128.0;
    base[1] = 0.0;
    base[2] = 0.0;
    let whitepoint = (1.0 - config.whitepoint() / 100.0).max(0.01);
    let compress = (config.compress() / 100.0).clamp(0.0, 0.99);
    let shadows = 2.0 * (config.shadows() / 100.0).clamp(-1.0, 1.0);
    let highlights = 2.0 * (config.highlights() / 100.0).clamp(-1.0, 1.0);
    base[0] = if base[0] > 0.0 {
        base[0] / whitepoint
    } else {
        base[0]
    };
    source[0] = if source[0] > 0.0 {
        source[0] / whitepoint
    } else {
        source[0]
    };
    overlay_highlights(&mut source, base, highlights, compress, config, unbound);
    overlay_shadows(&mut source, base, shadows, compress, config, unbound);
    source[0] = if config.flags() & UNBOUND_L != 0 {
        source[0]
    } else {
        source[0].clamp(0.0, 1.0)
    };
    source[0] *= whitepoint * 100.0;
    source[1] *= 128.0;
    source[2] *= 128.0;
    source[3] = source[3].clamp(0.0, 1.0);
    ShadhiPixel::from_channels(source)
}

fn overlay_highlights(
    value: &mut [f32; 4],
    base: [f32; 4],
    amount: f32,
    compress: f32,
    config: ShadhiConfig,
    unbound_mask: bool,
) {
    let mut remaining = amount * amount;
    let transform = (1.0 - base[0] / (1.0 - compress)).clamp(0.0, 1.0);
    while remaining > 0.0 {
        let lightness = if config.flags() & UNBOUND_HIGHLIGHTS_L != 0 {
            value[0]
        } else {
            value[0].clamp(0.0, 1.0)
        };
        let mut reference = (base[0] - 0.5) * sign(-amount) * sign(1.0 - lightness) + 0.5;
        if !unbound_mask {
            reference = reference.clamp(0.0, 1.0);
        }
        let chunk = remaining.min(1.0);
        let transition = chunk * transform;
        value[0] =
            lightness * (1.0 - transition) + overlay_value(lightness, reference) * transition;
        if config.flags() & UNBOUND_HIGHLIGHTS_L == 0 {
            value[0] = value[0].clamp(0.0, 1.0);
        }
        let chroma = chroma_factor(value[0], config, true, sign(-amount));
        value[1] = value[1] * (1.0 - transition) + (value[1] + base[1]) * chroma * transition;
        value[2] = value[2] * (1.0 - transition) + (value[2] + base[2]) * chroma * transition;
        if config.flags() & UNBOUND_HIGHLIGHTS_A == 0 {
            value[1] = value[1].clamp(-1.0, 1.0);
        }
        if config.flags() & UNBOUND_HIGHLIGHTS_B == 0 {
            value[2] = value[2].clamp(-1.0, 1.0);
        }
        remaining -= 1.0;
    }
}

fn overlay_shadows(
    value: &mut [f32; 4],
    base: [f32; 4],
    amount: f32,
    compress: f32,
    config: ShadhiConfig,
    unbound_mask: bool,
) {
    let mut remaining = amount * amount;
    let transform = (base[0] / (1.0 - compress) - compress / (1.0 - compress)).clamp(0.0, 1.0);
    while remaining > 0.0 {
        let lightness = if config.flags() & UNBOUND_HIGHLIGHTS_L != 0 {
            value[0]
        } else {
            value[0].clamp(0.0, 1.0)
        };
        let mut reference = (base[0] - 0.5) * sign(amount) * sign(1.0 - lightness) + 0.5;
        if !unbound_mask {
            reference = reference.clamp(0.0, 1.0);
        }
        let chunk = remaining.min(1.0);
        let transition = chunk * transform;
        value[0] =
            lightness * (1.0 - transition) + overlay_value(lightness, reference) * transition;
        if config.flags() & UNBOUND_SHADOWS_L == 0 {
            value[0] = value[0].clamp(0.0, 1.0);
        }
        let chroma = chroma_factor(value[0], config, false, sign(amount));
        value[1] = value[1] * (1.0 - transition) + (value[1] + base[1]) * chroma * transition;
        value[2] = value[2] * (1.0 - transition) + (value[2] + base[2]) * chroma * transition;
        if config.flags() & UNBOUND_SHADOWS_A == 0 {
            value[1] = value[1].clamp(-1.0, 1.0);
        }
        if config.flags() & UNBOUND_SHADOWS_B == 0 {
            value[2] = value[2].clamp(-1.0, 1.0);
        }
        remaining -= 1.0;
    }
}

fn chroma_factor(lightness: f32, config: ShadhiConfig, highlights: bool, direction: f32) -> f32 {
    let lref = reciprocal(lightness, config.low_approximation());
    let href = reciprocal(1.0 - lightness, config.low_approximation());
    if highlights {
        lightness * lref * (1.0 - corrected(config.highlights_ccorrect(), direction))
            + (1.0 - lightness) * href * corrected(config.highlights_ccorrect(), direction)
    } else {
        lightness * lref * corrected(config.shadows_ccorrect(), direction)
            + (1.0 - lightness) * href * (1.0 - corrected(config.shadows_ccorrect(), direction))
    }
}

fn corrected(value: f32, direction: f32) -> f32 {
    ((value / 100.0 - 0.5) * direction + 0.5).clamp(0.0, 1.0)
}

fn reciprocal(value: f32, epsilon: f32) -> f32 {
    let magnitude = value.abs().max(epsilon);
    value.signum() / magnitude
}

fn overlay_value(value: f32, reference: f32) -> f32 {
    if value > 0.5 {
        1.0 - (1.0 - 2.0 * (value - 0.5)) * (1.0 - reference)
    } else {
        2.0 * value * reference
    }
}

fn sign(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

fn bounds(flags: u32, gaussian: bool) -> ([f32; 4], [f32; 4]) {
    let unbound = flags
        & if gaussian {
            UNBOUND_GAUSSIAN
        } else {
            UNBOUND_BILATERAL
        }
        != 0;
    if unbound {
        ([-f32::MAX; 4], [f32::MAX; 4])
    } else {
        (LAB_MINIMUM, LAB_MAXIMUM)
    }
}

fn bilateral_filter<F: FnMut() -> bool>(
    input: &[[f32; 4]],
    dimensions: RasterDimensions,
    sigma: f32,
    flags: u32,
    cancelled: &mut F,
) -> Result<Vec<[f32; 4]>, OperationExecutionError> {
    let width = usize::try_from(dimensions.width()).expect("validated width");
    let height = usize::try_from(dimensions.height()).expect("validated height");
    let support = (4.0 * sigma).ceil() as i32;
    let support = support.clamp(1, MAX_BILATERAL_SUPPORT);
    let (minimum, maximum) = bounds(flags, false);
    let mut output = Vec::with_capacity(input.len());
    let width_i64 = i64::try_from(width).expect("width fits signed coordinate");
    let height_i64 = i64::try_from(height).expect("height fits signed coordinate");
    for y in 0..height {
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        for x in 0..width {
            let center = input[y * width + x];
            let mut total = [0.0; 4];
            let mut weights = 0.0;
            for dy in -support..=support {
                for dx in -support..=support {
                    let x_i64 = i64::try_from(x).expect("x fits signed coordinate");
                    let y_i64 = i64::try_from(y).expect("y fits signed coordinate");
                    let sample_x = (x_i64 + i64::from(dx)).clamp(0, width_i64 - 1) as usize;
                    let sample_y = (y_i64 + i64::from(dy)).clamp(0, height_i64 - 1) as usize;
                    let sample = input[sample_y * width + sample_x];
                    let spatial = ((dx * dx + dy * dy) as f32 / (2.0 * sigma * sigma)).exp();
                    let range = ((0..3)
                        .map(|channel| {
                            let delta = sample[channel] - center[channel];
                            delta * delta
                        })
                        .sum::<f32>()
                        / (2.0 * BILATERAL_RANGE_SIGMA * BILATERAL_RANGE_SIGMA))
                        .exp();
                    let weight = spatial * range;
                    for channel in 0..4 {
                        total[channel] +=
                            sample[channel].clamp(minimum[channel], maximum[channel]) * weight;
                    }
                    weights += weight;
                }
            }
            output.push(std::array::from_fn(|channel| total[channel] / weights));
        }
    }
    Ok(output)
}

fn blend_pixel(
    source: ShadhiPixel,
    candidate: ShadhiPixel,
    coverage: f32,
    pixel: usize,
) -> Result<ShadhiPixel, OperationExecutionError> {
    let mut output = [0.0; 4];
    for (channel, value) in output.iter_mut().enumerate() {
        *value = source.channels[channel]
            + (candidate.channels[channel] - source.channels[channel]) * coverage;
        if !value.is_finite() {
            return Err(OperationExecutionError::NonFiniteResult {
                pixel,
                channel: crate::RgbChannel::Red,
            });
        }
    }
    Ok(ShadhiPixel::from_channels(output))
}

fn validate_lab_shape(
    dimensions: RasterDimensions,
    input: &[ShadhiPixel],
) -> Result<usize, OperationExecutionError> {
    let expected = usize::try_from(dimensions.pixel_count()).map_err(|_| {
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
    if input
        .iter()
        .any(|pixel| pixel.channels().iter().any(|value| !value.is_finite()))
    {
        return Err(OperationExecutionError::NonFiniteResult {
            pixel: 0,
            channel: crate::RgbChannel::Red,
        });
    }
    Ok(expected)
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

fn map_filter_error(error: BoundedGaussianError) -> OperationExecutionError {
    match error {
        BoundedGaussianError::Cancelled => OperationExecutionError::Cancelled,
        BoundedGaussianError::InvalidSigma => OperationExecutionError::NonFiniteResult {
            pixel: 0,
            channel: crate::RgbChannel::Red,
        },
        BoundedGaussianError::Dimensions => OperationExecutionError::DimensionsMismatch {
            expected: 0,
            actual: 0,
        },
    }
}

fn digest_plan(
    config: ShadhiConfig,
    dimensions: RasterDimensions,
    sigma: f32,
    overlap: u32,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.shadhi.lab-plan.v1");
    hasher.update(config.order().to_le_bytes());
    hasher.update(config.radius().to_bits().to_le_bytes());
    hasher.update(config.shadows().to_bits().to_le_bytes());
    hasher.update(config.whitepoint().to_bits().to_le_bytes());
    hasher.update(config.highlights().to_bits().to_le_bytes());
    hasher.update(config.compress().to_bits().to_le_bytes());
    hasher.update(config.flags().to_le_bytes());
    hasher.update(config.shadhi_algo().id().to_le_bytes());
    hasher.update(dimensions.width().to_le_bytes());
    hasher.update(dimensions.height().to_le_bytes());
    hasher.update(sigma.to_bits().to_le_bytes());
    hasher.update(overlap.to_le_bytes());
    hasher.finalize().into()
}

fn digest_pixels(input: &[ShadhiPixel]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.shadhi.lab-pixels.v1");
    for pixel in input {
        for value in pixel.channels() {
            hasher.update(value.to_bits().to_le_bytes());
        }
    }
    hasher.finalize().into()
}

fn digest_mask(mask: Option<&[f32]>) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.shadhi.mask.v1");
    if let Some(mask) = mask {
        for value in mask {
            hasher.update(value.to_bits().to_le_bytes());
        }
    }
    hasher.finalize().into()
}

fn finite_rgb(value: f32) -> FiniteF32 {
    FiniteF32::new(value).unwrap_or_else(|_| FiniteF32::new(0.0).expect("zero is finite"))
}
