use super::{
    LAB_MAXIMUM, LAB_MINIMUM, ShadhiAlgorithm, ShadhiConfig, UNBOUND_BILATERAL, UNBOUND_GAUSSIAN,
    UNBOUND_HIGHLIGHTS_A, UNBOUND_HIGHLIGHTS_B, UNBOUND_HIGHLIGHTS_L, UNBOUND_SHADOWS_A,
    UNBOUND_SHADOWS_B, UNBOUND_SHADOWS_L,
};
use sha2::{Digest, Sha256};
use std::mem::size_of;

use crate::common::bilateral::{BilateralError, BilateralGeometry, BilateralGrid};
use crate::operations::common::{OperationExecutionError, ReconstructionBudget, validate_shape};
use crate::operations::convolution::{BoundedGaussianError, bounded_gaussian_4c_order};
use crate::{FiniteF32, LinearRgb, RasterDimensions};

const SHADHI_WORKING_BUFFERS: usize = 8;
const EXTERNAL_BILATERAL_PEAK_LAB_BUFFERS: usize = 3;
const EXTERNAL_BILATERAL_BACKEND_LIVE_LAB_BUFFERS: usize = 2;

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

/// Canonical input to an alternative bilateral-grid executor.
///
/// The geometry is resolved by the shared CPU-derived formula before this
/// request crosses a backend boundary. The guide is four-channel D50 Lab,
/// `detail` is `-1` for Darktable's shadows/highlights base layer, and the
/// remaining operation-admission budget is carried explicitly for the backend.
#[derive(Debug, Clone, Copy)]
pub struct ShadhiBilateralRequest<'a> {
    geometry: BilateralGeometry,
    guide: &'a [[f32; 4]],
    detail: f32,
    transient_memory_budget_bytes: u64,
}

impl<'a> ShadhiBilateralRequest<'a> {
    #[must_use]
    pub const fn geometry(self) -> BilateralGeometry {
        self.geometry
    }

    #[must_use]
    pub const fn guide(self) -> &'a [[f32; 4]] {
        self.guide
    }

    #[must_use]
    pub const fn detail(self) -> f32 {
        self.detail
    }

    /// Operation-admission budget remaining after the processing allocations
    /// that stay live across the delegated backend call.
    #[must_use]
    pub const fn transient_memory_budget_bytes(self) -> u64 {
        self.transient_memory_budget_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShadhiMemoryModel {
    Cpu,
    ExternalBilateral,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShadhiPlan {
    config: ShadhiConfig,
    dimensions: RasterDimensions,
    sigma: f32,
    overlap: u32,
    memory_estimate_bytes: usize,
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
        Self::new_with_memory_model(config, dimensions, ShadhiMemoryModel::Cpu)
    }

    /// Builds the metadata and processing-memory plan used when an external
    /// backend supplies the bilateral base layer.
    ///
    /// The processing peak is one immutable [`LinearRgb`] input plus three
    /// 16-byte Lab/RGBA buffers per pixel. Three such buffers coexist both
    /// when the backend returns (Lab source, guide, filtered base) and while
    /// mixing (Lab source, filtered base, Lab output). The delegated backend
    /// receives only the remainder of this same operation budget after the
    /// input, Lab source, and guide that cross its call boundary are reserved.
    pub(crate) fn new_external_bilateral(
        config: ShadhiConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        if config.shadhi_algo() != ShadhiAlgorithm::Bilateral {
            return Err(OperationExecutionError::UnsupportedCapability(
                "external bilateral base requires bilateral shadhi",
            ));
        }
        Self::new_with_memory_model(config, dimensions, ShadhiMemoryModel::ExternalBilateral)
    }

    fn new_with_memory_model(
        config: ShadhiConfig,
        dimensions: RasterDimensions,
        memory_model: ShadhiMemoryModel,
    ) -> Result<Self, OperationExecutionError> {
        let sigma = config.radius().max(0.1);
        let overlap = (4.0 * sigma).ceil().min(256.0) as u32;
        let budget = ReconstructionBudget::default();
        let pixel_count = usize::try_from(dimensions.pixel_count()).map_err(|_| {
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: budget.maximum_bytes(),
            }
        })?;
        let memory_estimate_bytes = match memory_model {
            ShadhiMemoryModel::Cpu => {
                let base_bytes = shadhi_base_memory_bytes(pixel_count, budget)?;
                if config.shadhi_algo() == ShadhiAlgorithm::Bilateral {
                    let width = usize::try_from(dimensions.width()).expect("validated width");
                    let height = usize::try_from(dimensions.height()).expect("validated height");
                    let grid_bytes =
                        BilateralGrid::required_memory_bytes(width, height, sigma, 100.0)
                            .map_err(map_bilateral_error)?;
                    base_bytes.checked_add(grid_bytes).ok_or(
                        OperationExecutionError::MemoryBudgetExceeded {
                            required: usize::MAX,
                            budget: budget.maximum_bytes(),
                        },
                    )?
                } else {
                    base_bytes
                }
            }
            ShadhiMemoryModel::ExternalBilateral => {
                let width = usize::try_from(dimensions.width()).expect("validated width");
                let height = usize::try_from(dimensions.height()).expect("validated height");
                BilateralGeometry::new(width, height, sigma, 100.0).map_err(map_bilateral_error)?;
                external_bilateral_peak_memory_bytes(pixel_count, budget)?
            }
        };
        if memory_estimate_bytes > budget.maximum_bytes() {
            return Err(OperationExecutionError::MemoryBudgetExceeded {
                required: memory_estimate_bytes,
                budget: budget.maximum_bytes(),
            });
        }
        let analysis_identity = digest_plan(config, dimensions, sigma, overlap);
        Ok(Self {
            config,
            dimensions,
            sigma,
            overlap,
            memory_estimate_bytes,
            analysis_identity,
        })
    }

    #[must_use]
    pub const fn memory_estimate_bytes(&self) -> usize {
        self.memory_estimate_bytes
    }

    pub(crate) fn validate_opacity(opacity: f32) -> Result<(), OperationExecutionError> {
        validate_opacity(opacity)
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
        let expected = validate_execution_input(self.dimensions, input, mask, opacity)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        if opacity == 0.0 {
            return Ok(input.to_vec());
        }
        let filtered = self.filter(input, &mut cancelled)?;
        self.finish_lab(input, &filtered, mask, opacity, expected, &mut cancelled)
    }

    /// Completes bilateral Shadhi from a backend-produced sliced base layer.
    ///
    /// The caller supplies the result of the canonical bilateral grid with
    /// `detail = -1`. Shape and filtered lightness are validated before the
    /// shared Darktable Lab mix and opacity path runs. The filtered a/b/spare
    /// channels are not consumed by Darktable's bilateral Shadhi mix.
    pub fn execute_lab_with_filtered_base<F: FnMut() -> bool>(
        &self,
        input: &[ShadhiPixel],
        filtered_base: &[[f32; 4]],
        mask: Option<&[f32]>,
        opacity: f32,
        mut cancelled: F,
    ) -> Result<Vec<ShadhiPixel>, OperationExecutionError> {
        if self.config.shadhi_algo() != ShadhiAlgorithm::Bilateral {
            return Err(OperationExecutionError::UnsupportedCapability(
                "external bilateral base requires bilateral shadhi",
            ));
        }
        let expected = validate_execution_input(self.dimensions, input, mask, opacity)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        if opacity == 0.0 {
            return Ok(input.to_vec());
        }
        validate_filtered_base(filtered_base, expected)?;
        self.finish_lab(
            input,
            filtered_base,
            mask,
            opacity,
            expected,
            &mut cancelled,
        )
    }

    /// Returns the source-derived geometry and guide consumed by a bilateral
    /// backend. No grid buffer is allocated.
    pub(crate) fn bilateral_request<'a>(
        &self,
        guide: &'a [[f32; 4]],
    ) -> Result<ShadhiBilateralRequest<'a>, OperationExecutionError> {
        if self.config.shadhi_algo() != ShadhiAlgorithm::Bilateral {
            return Err(OperationExecutionError::UnsupportedCapability(
                "external bilateral base requires bilateral shadhi",
            ));
        }
        let expected = usize::try_from(self.dimensions.pixel_count()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: guide.len(),
            }
        })?;
        validate_filtered_base(guide, expected)?;
        let width = usize::try_from(self.dimensions.width()).expect("validated width");
        let height = usize::try_from(self.dimensions.height()).expect("validated height");
        let geometry = BilateralGeometry::new(width, height, self.sigma, 100.0)
            .map_err(map_bilateral_error)?;
        let budget = ReconstructionBudget::default();
        let processing_bytes_live_across_backend =
            external_bilateral_backend_live_memory_bytes(expected, budget)?;
        let backend_budget_bytes = budget
            .maximum_bytes()
            .checked_sub(processing_bytes_live_across_backend)
            .ok_or(OperationExecutionError::MemoryBudgetExceeded {
                required: processing_bytes_live_across_backend,
                budget: budget.maximum_bytes(),
            })?;
        let transient_memory_budget_bytes = u64::try_from(backend_budget_bytes).map_err(|_| {
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: budget.maximum_bytes(),
            }
        })?;
        Ok(ShadhiBilateralRequest {
            geometry,
            guide,
            detail: -1.0,
            transient_memory_budget_bytes,
        })
    }

    fn finish_lab<F: FnMut() -> bool>(
        &self,
        input: &[ShadhiPixel],
        filtered: &[[f32; 4]],
        mask: Option<&[f32]>,
        opacity: f32,
        expected: usize,
        cancelled: &mut F,
    ) -> Result<Vec<ShadhiPixel>, OperationExecutionError> {
        let mut output = Vec::with_capacity(expected);
        for (index, (source, base)) in input.iter().zip(filtered).enumerate() {
            if index % usize::try_from(self.dimensions.width()).expect("validated width") == 0
                && cancelled()
            {
                return Err(OperationExecutionError::Cancelled);
            }
            let candidate = mix_lab(*source, ShadhiPixel::from_channels(*base), self.config);
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
    ) -> Result<Vec<[f32; 4]>, OperationExecutionError> {
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
                .map_err(map_filter_error)
            }
            ShadhiAlgorithm::Bilateral => {
                bilateral_filter(&channels, self.dimensions, self.sigma, cancelled)
            }
        }
    }
}

fn shadhi_base_memory_bytes(
    pixel_count: usize,
    budget: ReconstructionBudget,
) -> Result<usize, OperationExecutionError> {
    pixel_count
        .checked_mul(SHADHI_WORKING_BUFFERS)
        .and_then(|value| value.checked_mul(size_of::<LinearRgb>()))
        .and_then(|value| value.checked_add(pixel_count.saturating_mul(16)))
        .ok_or(OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: budget.maximum_bytes(),
        })
}

fn external_bilateral_peak_memory_bytes(
    pixel_count: usize,
    budget: ReconstructionBudget,
) -> Result<usize, OperationExecutionError> {
    let input_bytes = pixel_count.checked_mul(size_of::<LinearRgb>()).ok_or(
        OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: budget.maximum_bytes(),
        },
    )?;
    let lab_bytes = pixel_count
        .checked_mul(EXTERNAL_BILATERAL_PEAK_LAB_BUFFERS)
        .and_then(|value| value.checked_mul(size_of::<[f32; 4]>()))
        .ok_or(OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: budget.maximum_bytes(),
        })?;
    input_bytes
        .checked_add(lab_bytes)
        .ok_or(OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: budget.maximum_bytes(),
        })
}

fn external_bilateral_backend_live_memory_bytes(
    pixel_count: usize,
    budget: ReconstructionBudget,
) -> Result<usize, OperationExecutionError> {
    pixel_count
        .checked_mul(size_of::<LinearRgb>())
        .and_then(|input| {
            pixel_count
                .checked_mul(EXTERNAL_BILATERAL_BACKEND_LIVE_LAB_BUFFERS)
                .and_then(|pixels| pixels.checked_mul(size_of::<[f32; 4]>()))
                .and_then(|lab| input.checked_add(lab))
        })
        .ok_or(OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: budget.maximum_bytes(),
        })
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
    source[0] *= 100.0;
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
        let chroma = chroma_factor(value[0], lightness, config, true, sign(-amount));
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
        let chroma = chroma_factor(value[0], lightness, config, false, sign(amount));
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

fn chroma_factor(
    lightness: f32,
    reference_lightness: f32,
    config: ShadhiConfig,
    highlights: bool,
    direction: f32,
) -> f32 {
    let lref = reciprocal(reference_lightness, config.low_approximation());
    let href = reciprocal(1.0 - reference_lightness, config.low_approximation());
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
    cancelled: &mut F,
) -> Result<Vec<[f32; 4]>, OperationExecutionError> {
    let width = usize::try_from(dimensions.width()).expect("validated width");
    let height = usize::try_from(dimensions.height()).expect("validated height");
    let mut grid = BilateralGrid::new(width, height, sigma, 100.0).map_err(map_bilateral_error)?;
    grid.splat_with_cancel(input, cancelled)
        .map_err(map_bilateral_error)?;
    grid.blur_with_cancel(cancelled)
        .map_err(map_bilateral_error)?;
    grid.slice_with_cancel(input, -1.0, cancelled)
        .map_err(map_bilateral_error)
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

fn validate_execution_input(
    dimensions: RasterDimensions,
    input: &[ShadhiPixel],
    mask: Option<&[f32]>,
    opacity: f32,
) -> Result<usize, OperationExecutionError> {
    let expected = validate_lab_shape(dimensions, input)?;
    validate_mask(mask, expected)?;
    validate_opacity(opacity)?;
    Ok(expected)
}

fn validate_opacity(opacity: f32) -> Result<(), OperationExecutionError> {
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        return Err(OperationExecutionError::NonFiniteResult {
            pixel: 0,
            channel: crate::RgbChannel::Red,
        });
    }
    Ok(())
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

fn validate_filtered_base(
    filtered: &[[f32; 4]],
    expected: usize,
) -> Result<(), OperationExecutionError> {
    if filtered.len() != expected {
        return Err(OperationExecutionError::DimensionsMismatch {
            expected,
            actual: filtered.len(),
        });
    }
    for (pixel, channels) in filtered.iter().enumerate() {
        if !channels[0].is_finite() {
            return Err(OperationExecutionError::NonFiniteResult {
                pixel,
                channel: crate::RgbChannel::Red,
            });
        }
    }
    Ok(())
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

fn map_bilateral_error(error: BilateralError) -> OperationExecutionError {
    match error {
        BilateralError::Cancelled => OperationExecutionError::Cancelled,
        BilateralError::BufferShape { expected, actual } => {
            OperationExecutionError::DimensionsMismatch { expected, actual }
        }
        BilateralError::AllocationFailed { required_bytes } => {
            OperationExecutionError::AllocationFailed {
                required: required_bytes,
            }
        }
        BilateralError::SizeOverflow => OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: ReconstructionBudget::default().maximum_bytes(),
        },
        BilateralError::InvalidDimensions => OperationExecutionError::DimensionsMismatch {
            expected: 1,
            actual: 0,
        },
        BilateralError::InvalidParameter(_) => OperationExecutionError::NonFiniteResult {
            pixel: 0,
            channel: crate::RgbChannel::Red,
        },
        BilateralError::NonFiniteLightness { pixel }
        | BilateralError::NonFiniteOutput { pixel } => OperationExecutionError::NonFiniteResult {
            pixel,
            channel: crate::RgbChannel::Red,
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
    hasher.update(b"rusttable.shadhi.lab-plan.v2");
    hasher.update(config.order().to_le_bytes());
    hasher.update(config.radius().to_bits().to_le_bytes());
    hasher.update(config.shadows().to_bits().to_le_bytes());
    hasher.update(config.whitepoint().to_bits().to_le_bytes());
    hasher.update(config.highlights().to_bits().to_le_bytes());
    hasher.update(config.compress().to_bits().to_le_bytes());
    hasher.update(config.shadows_ccorrect().to_bits().to_le_bytes());
    hasher.update(config.highlights_ccorrect().to_bits().to_le_bytes());
    hasher.update(config.flags().to_le_bytes());
    hasher.update(config.low_approximation().to_bits().to_le_bytes());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operations::shadhi::ShadhiParametersV5;

    #[test]
    fn external_bilateral_plan_accounts_for_processing_peak_without_cpu_grid() {
        let dimensions = RasterDimensions::new(2_000, 2_000).expect("dimensions");
        let config = ShadhiConfig::new(ShadhiParametersV5 {
            radius: 0.1,
            ..ShadhiParametersV5::defaults()
        })
        .expect("bilateral config");

        assert!(matches!(
            ShadhiPlan::new(config, dimensions),
            Err(OperationExecutionError::MemoryBudgetExceeded { .. })
        ));

        let external =
            ShadhiPlan::new_external_bilateral(config, dimensions).expect("external plan");
        let pixels = usize::try_from(dimensions.pixel_count()).expect("pixel count");
        let expected_peak = pixels
            * (size_of::<LinearRgb>()
                + EXTERNAL_BILATERAL_PEAK_LAB_BUFFERS * size_of::<[f32; 4]>());
        assert_eq!(external.memory_estimate_bytes(), expected_peak);
        assert!(expected_peak < ReconstructionBudget::default().maximum_bytes());
    }

    #[test]
    fn external_bilateral_request_carries_only_the_remaining_operation_budget() {
        let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
        let config = ShadhiConfig::new(ShadhiParametersV5::defaults()).expect("bilateral config");
        let plan = ShadhiPlan::new_external_bilateral(config, dimensions).expect("external plan");
        let guide = [[50.0, 0.0, 0.0, 1.0]];
        let request = plan.bilateral_request(&guide).expect("backend request");
        let processing_bytes_live_across_backend = size_of::<LinearRgb>()
            + EXTERNAL_BILATERAL_BACKEND_LIVE_LAB_BUFFERS * size_of::<[f32; 4]>();
        let expected_backend_budget = ReconstructionBudget::default()
            .maximum_bytes()
            .checked_sub(processing_bytes_live_across_backend)
            .expect("one-pixel processing reservation fits");

        assert_eq!(
            request.transient_memory_budget_bytes(),
            u64::try_from(expected_backend_budget).expect("operation budget fits WGPU accounting"),
        );
        assert_eq!(
            usize::try_from(request.transient_memory_budget_bytes())
                .expect("backend budget fits usize")
                + processing_bytes_live_across_backend,
            ReconstructionBudget::default().maximum_bytes(),
        );
    }
}
