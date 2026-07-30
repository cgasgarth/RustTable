#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names,
    clippy::too_many_lines
)]

use super::codec::{DEFRINGE_SCHEMA_VERSION, DefringeConfig, DefringeMode, DefringeParametersV1};
use crate::operations::convolution::{BoundedGaussianError, bounded_gaussian_4c};
use crate::{RasterDimensions, operations::ReconstructionBudget};
use sha2::{Digest, Sha256};
use std::fmt;

pub const DEFRINGE_MAGIC_THRESHOLD_COEFFICIENT: f32 = 33.0;
pub const DEFRINGE_GAUSSIAN_ORDER: u8 = 1;
const FIBONACCI: [f32; 14] = [
    0.0, 1.0, 1.0, 2.0, 3.0, 5.0, 8.0, 13.0, 21.0, 34.0, 55.0, 89.0, 144.0, 233.0,
];
const LAB_MINIMUM: [f32; 4] = [0.0, -128.0, -128.0, 0.0];
const LAB_MAXIMUM: [f32; 4] = [100.0, 128.0, 128.0, 1.0];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DefringePixel {
    channels: [f32; 4],
}

impl DefringePixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
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
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefringeBackend {
    CpuScalarReference,
}

impl DefringeBackend {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        "cpu-scalar-reference"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefringeOutcome {
    Complete,
    ImageTooSmallForKernel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefringeExecutionError {
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
    GaussianFailure,
}

impl fmt::Display for DefringeExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => formatter.write_str("defringe arithmetic overflowed"),
            Self::DimensionsMismatch { expected, actual } => {
                write!(
                    formatter,
                    "defringe expected {expected} pixels, got {actual}"
                )
            }
            Self::InvalidScale => {
                formatter.write_str("defringe scales must be finite and positive")
            }
            Self::InvalidTileWidth => formatter.write_str("defringe tile width must be nonzero"),
            Self::MemoryBudgetExceeded { required, budget } => {
                write!(
                    formatter,
                    "defringe requires {required} bytes, budget is {budget}"
                )
            }
            Self::Cancelled => formatter.write_str("defringe execution was cancelled"),
            Self::NonFiniteInput { pixel, channel } => {
                write!(
                    formatter,
                    "defringe input pixel {pixel} channel {channel} is non-finite"
                )
            }
            Self::NonFiniteResult { pixel, channel } => {
                write!(
                    formatter,
                    "defringe result pixel {pixel} channel {channel} is non-finite"
                )
            }
            Self::MaskLength { expected, actual } => {
                write!(
                    formatter,
                    "defringe mask has {actual} pixels, expected {expected}"
                )
            }
            Self::InvalidMaskValue => {
                formatter.write_str("defringe mask coverage must be finite in 0..=1")
            }
            Self::GaussianFailure => formatter.write_str("defringe Gaussian allocation failed"),
        }
    }
}

impl std::error::Error for DefringeExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefringeMask {
    identity: [u8; 32],
}

impl DefringeMask {
    pub fn new(coverage: &[f32]) -> Result<Self, DefringeExecutionError> {
        if coverage
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(DefringeExecutionError::InvalidMaskValue);
        }
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.defringe.mask.v1");
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
pub struct DefringeBlend {
    identity: [u8; 32],
}

impl DefringeBlend {
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
pub struct DefringeAnalysis {
    average_edge_chroma: f32,
    global_threshold: f32,
    identity: [u8; 32],
}

impl DefringeAnalysis {
    #[must_use]
    pub const fn average_edge_chroma(&self) -> f32 {
        self.average_edge_chroma
    }

    #[must_use]
    pub const fn global_threshold(&self) -> f32 {
        self.global_threshold
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DefringeReceipt {
    parameters: DefringeParametersV1,
    input_scale: f32,
    roi_scale: f32,
    sigma: f32,
    support_radius: usize,
    overlap: u32,
    average_sample_count: usize,
    small_sample_count: usize,
    average_edge_chroma: Option<f32>,
    threshold: f32,
    global_threshold: Option<f32>,
    outcome: DefringeOutcome,
    backend: DefringeBackend,
    memory_estimate: usize,
    mask_identity: [u8; 32],
    blend_identity: [u8; 32],
    analysis_identity: [u8; 32],
    input_identity: [u8; 32],
    output_identity: [u8; 32],
}

impl DefringeReceipt {
    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        DEFRINGE_SCHEMA_VERSION
    }
    #[must_use]
    pub const fn parameters(&self) -> DefringeParametersV1 {
        self.parameters
    }
    #[must_use]
    pub const fn sigma(&self) -> f32 {
        self.sigma
    }
    #[must_use]
    pub const fn support_radius(&self) -> usize {
        self.support_radius
    }
    #[must_use]
    pub const fn overlap(&self) -> u32 {
        self.overlap
    }
    #[must_use]
    pub const fn average_sample_count(&self) -> usize {
        self.average_sample_count
    }
    #[must_use]
    pub const fn small_sample_count(&self) -> usize {
        self.small_sample_count
    }
    #[must_use]
    pub const fn average_edge_chroma(&self) -> Option<f32> {
        self.average_edge_chroma
    }
    #[must_use]
    pub const fn threshold(&self) -> f32 {
        self.threshold
    }
    #[must_use]
    pub const fn global_threshold(&self) -> Option<f32> {
        self.global_threshold
    }
    #[must_use]
    pub const fn outcome(&self) -> DefringeOutcome {
        self.outcome
    }
    #[must_use]
    pub const fn backend(&self) -> DefringeBackend {
        self.backend
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
    pub const fn analysis_identity(&self) -> [u8; 32] {
        self.analysis_identity
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
pub struct DefringePlan {
    config: DefringeConfig,
    dimensions: RasterDimensions,
    input_scale: f32,
    roi_scale: f32,
    sigma: f32,
    support_radius: usize,
    overlap: u32,
    average_lattice: Vec<(i32, i32)>,
    small_lattice: Vec<(i32, i32)>,
    memory_estimate: usize,
    backend: DefringeBackend,
    analysis: Option<DefringeAnalysis>,
}

impl DefringePlan {
    pub fn new(
        config: DefringeConfig,
        dimensions: RasterDimensions,
        input_scale: f32,
        roi_scale: f32,
    ) -> Result<Self, DefringeExecutionError> {
        Self::with_budget(
            config,
            dimensions,
            input_scale,
            roi_scale,
            ReconstructionBudget::default().maximum_bytes(),
        )
    }

    pub fn with_budget(
        config: DefringeConfig,
        dimensions: RasterDimensions,
        input_scale: f32,
        roi_scale: f32,
        budget: usize,
    ) -> Result<Self, DefringeExecutionError> {
        if !input_scale.is_finite()
            || !roi_scale.is_finite()
            || input_scale <= 0.0
            || roi_scale <= 0.0
        {
            return Err(DefringeExecutionError::InvalidScale);
        }
        let sigma = config.radius().abs().max(0.1) * roi_scale / input_scale;
        if !sigma.is_finite() {
            return Err(DefringeExecutionError::ArithmeticOverflow);
        }
        let support_f = 2.0 * sigma.ceil();
        if !support_f.is_finite() || support_f < 0.0 || support_f > usize::MAX as f32 {
            return Err(DefringeExecutionError::ArithmeticOverflow);
        }
        let support_radius = support_f as usize;
        let samples_wish = support_radius
            .checked_mul(support_radius)
            .ok_or(DefringeExecutionError::ArithmeticOverflow)?;
        let sample_index = sample_index_for(samples_wish);
        let small_index = sample_index - 1;
        let average_count = fib_count(sample_index);
        let small_count = fib_count(small_index);
        let small_radius = support_radius.max(3);
        let average_radius = 24usize
            .checked_add(
                support_radius
                    .checked_mul(4)
                    .ok_or(DefringeExecutionError::ArithmeticOverflow)?,
            )
            .ok_or(DefringeExecutionError::ArithmeticOverflow)?;
        let average_lattice = make_lattice(average_radius, sample_index, average_count)?;
        let small_lattice = make_lattice(small_radius, small_index, small_count)?;
        let pixels = usize::try_from(dimensions.pixel_count())
            .map_err(|_| DefringeExecutionError::ArithmeticOverflow)?;
        let image_bytes = pixels
            .checked_mul(16)
            .ok_or(DefringeExecutionError::ArithmeticOverflow)?;
        let lattice_bytes = average_lattice
            .len()
            .checked_add(small_lattice.len())
            .and_then(|count| count.checked_mul(8))
            .ok_or(DefringeExecutionError::ArithmeticOverflow)?;
        let memory_estimate = image_bytes
            .checked_mul(4)
            .and_then(|value| value.checked_add(lattice_bytes))
            .ok_or(DefringeExecutionError::ArithmeticOverflow)?;
        if memory_estimate > budget {
            return Err(DefringeExecutionError::MemoryBudgetExceeded {
                required: memory_estimate,
                budget,
            });
        }
        let overlap = (config.radius() * 2.0) as u32;
        Ok(Self {
            config,
            dimensions,
            input_scale,
            roi_scale,
            sigma,
            support_radius,
            overlap,
            average_lattice,
            small_lattice,
            memory_estimate,
            backend: DefringeBackend::CpuScalarReference,
            analysis: None,
        })
    }

    #[must_use]
    pub const fn config(&self) -> DefringeConfig {
        self.config
    }
    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn sigma(&self) -> f32 {
        self.sigma
    }
    #[must_use]
    pub const fn support_radius(&self) -> usize {
        self.support_radius
    }
    #[must_use]
    pub const fn overlap(&self) -> u32 {
        self.overlap
    }
    #[must_use]
    pub fn average_lattice(&self) -> &[(i32, i32)] {
        &self.average_lattice
    }
    #[must_use]
    pub fn small_lattice(&self) -> &[(i32, i32)] {
        &self.small_lattice
    }
    #[must_use]
    pub const fn average_sample_count(&self) -> usize {
        self.average_lattice.len()
    }
    #[must_use]
    pub const fn small_sample_count(&self) -> usize {
        self.small_lattice.len()
    }
    #[must_use]
    pub const fn memory_estimate(&self) -> usize {
        self.memory_estimate
    }
    #[must_use]
    pub const fn backend(&self) -> DefringeBackend {
        self.backend
    }
    #[must_use]
    pub const fn analysis(&self) -> Option<&DefringeAnalysis> {
        self.analysis.as_ref()
    }

    /// Resolves global analysis once and returns a new immutable plan carrying it.
    pub fn with_global_analysis<F: FnMut() -> bool>(
        &self,
        input: &[DefringePixel],
        cancelled: F,
    ) -> Result<Self, DefringeExecutionError> {
        let analysis = self.analyze(input, cancelled)?;
        let mut plan = self.clone();
        plan.analysis = Some(analysis);
        Ok(plan)
    }

    pub fn analyze<F: FnMut() -> bool>(
        &self,
        input: &[DefringePixel],
        mut cancelled: F,
    ) -> Result<DefringeAnalysis, DefringeExecutionError> {
        let channels = validate_input(input, self.dimensions)?;
        if cancelled() {
            return Err(DefringeExecutionError::Cancelled);
        }
        let edge = self.edge_layer(&channels, &mut cancelled)?;
        make_analysis(self.config, &edge)
    }

    pub fn execute<F: FnMut() -> bool>(
        &self,
        input: &[DefringePixel],
        cancelled: F,
    ) -> Result<Vec<DefringePixel>, DefringeExecutionError> {
        self.execute_with_mask(input, None, 1.0, cancelled)
    }

    pub fn execute_with_mask<F: FnMut() -> bool>(
        &self,
        input: &[DefringePixel],
        mask: Option<&[f32]>,
        opacity: f32,
        mut cancelled: F,
    ) -> Result<Vec<DefringePixel>, DefringeExecutionError> {
        let (output, _) = self.execute_internal(input, mask, opacity, &mut cancelled)?;
        Ok(output)
    }

    pub fn execute_with_receipt<F: FnMut() -> bool>(
        &self,
        input: &[DefringePixel],
        mask: Option<&[f32]>,
        opacity: f32,
        mut cancelled: F,
    ) -> Result<(Vec<DefringePixel>, DefringeReceipt), DefringeExecutionError> {
        let (output, details) = self.execute_internal(input, mask, opacity, &mut cancelled)?;
        let input_identity = digest_pixels(input);
        let output_identity = digest_pixels(&output);
        let mask_identity = mask.map_or([0; 32], |values| {
            DefringeMask::new(values)
                .expect("validated mask")
                .identity()
        });
        let receipt = DefringeReceipt {
            parameters: self.config.parameters(),
            input_scale: self.input_scale,
            roi_scale: self.roi_scale,
            sigma: self.sigma,
            support_radius: self.support_radius,
            overlap: self.overlap,
            average_sample_count: self.average_lattice.len(),
            small_sample_count: self.small_lattice.len(),
            average_edge_chroma: details.average_edge_chroma,
            threshold: details.threshold,
            global_threshold: details.global_threshold,
            outcome: details.outcome,
            backend: self.backend,
            memory_estimate: self.memory_estimate,
            mask_identity,
            blend_identity: DefringeBlend::normal().identity(),
            analysis_identity: details.analysis_identity,
            input_identity,
            output_identity,
        };
        Ok((output, receipt))
    }

    /// Executes the same frozen full-frame plan for a tiled request. The
    /// caller's tile width is validated for planner accounting, while global
    /// analysis and final publication remain whole-frame operations.
    pub fn execute_tiled<F: FnMut() -> bool>(
        &self,
        input: &[DefringePixel],
        tile_width: u32,
        cancelled: F,
    ) -> Result<Vec<DefringePixel>, DefringeExecutionError> {
        if tile_width == 0 {
            return Err(DefringeExecutionError::InvalidTileWidth);
        }
        self.execute(input, cancelled)
    }

    #[must_use]
    pub fn cache_identity(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.defringe.plan.v1");
        hasher.update(self.config.parameters().to_bytes());
        hasher.update(self.dimensions.width().to_le_bytes());
        hasher.update(self.dimensions.height().to_le_bytes());
        hasher.update(self.input_scale.to_bits().to_le_bytes());
        hasher.update(self.roi_scale.to_bits().to_le_bytes());
        hasher.update(self.sigma.to_bits().to_le_bytes());
        hasher.update((self.support_radius as u64).to_le_bytes());
        hasher.update(self.overlap.to_le_bytes());
        for point in self.average_lattice.iter().chain(&self.small_lattice) {
            hasher.update(point.0.to_le_bytes());
            hasher.update(point.1.to_le_bytes());
        }
        if let Some(analysis) = &self.analysis {
            hasher.update([1]);
            hasher.update(analysis.identity);
        } else {
            hasher.update([0]);
        }
        hasher.finalize().into()
    }

    fn edge_layer<F: FnMut() -> bool>(
        &self,
        input: &[[f32; 4]],
        cancelled: &mut F,
    ) -> Result<Vec<f32>, DefringeExecutionError> {
        let blurred = bounded_gaussian_4c(
            input,
            self.dimensions,
            self.sigma,
            LAB_MINIMUM,
            LAB_MAXIMUM,
            &mut *cancelled,
        )
        .map_err(|error| match error {
            BoundedGaussianError::Cancelled => DefringeExecutionError::Cancelled,
            BoundedGaussianError::InvalidSigma | BoundedGaussianError::Dimensions => {
                DefringeExecutionError::GaussianFailure
            }
        })?;
        let mut edge = Vec::with_capacity(input.len());
        for (index, (source, blur)) in input.iter().zip(blurred).enumerate() {
            if index % usize::try_from(self.dimensions.width()).expect("validated width") == 0
                && cancelled()
            {
                return Err(DefringeExecutionError::Cancelled);
            }
            let da = source[1] - blur[1];
            let db = source[2] - blur[2];
            let value = da.mul_add(da, db * db);
            if !value.is_finite() {
                return Err(DefringeExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: 1,
                });
            }
            edge.push(value);
        }
        Ok(edge)
    }

    fn execute_internal<F: FnMut() -> bool>(
        &self,
        input: &[DefringePixel],
        mask: Option<&[f32]>,
        opacity: f32,
        cancelled: &mut F,
    ) -> Result<(Vec<DefringePixel>, ExecutionDetails), DefringeExecutionError> {
        let channels = validate_input(input, self.dimensions)?;
        let expected = channels.len();
        let mask_identity = if let Some(values) = mask {
            if values.len() != expected {
                return Err(DefringeExecutionError::MaskLength {
                    expected,
                    actual: values.len(),
                });
            }
            DefringeMask::new(values)?.identity()
        } else {
            [0; 32]
        };
        let _ = mask_identity;
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(DefringeExecutionError::InvalidMaskValue);
        }
        if cancelled() {
            return Err(DefringeExecutionError::Cancelled);
        }
        let minimum_edge = u64::try_from(self.support_radius)
            .ok()
            .and_then(|value| value.checked_mul(2))
            .and_then(|value| value.checked_add(1))
            .ok_or(DefringeExecutionError::ArithmeticOverflow)?;
        let too_small = u64::from(self.dimensions.width()) < minimum_edge
            || u64::from(self.dimensions.height()) < minimum_edge;
        if too_small {
            return Ok((
                blend(
                    input,
                    input,
                    mask,
                    opacity,
                    width_for(self.dimensions),
                    cancelled,
                )?,
                ExecutionDetails {
                    average_edge_chroma: None,
                    threshold: threshold_for(self.config, None),
                    global_threshold: None,
                    analysis_identity: [0; 32],
                    outcome: DefringeOutcome::ImageTooSmallForKernel,
                },
            ));
        }
        let edge = self.edge_layer(&channels, cancelled)?;
        let analysis = match (&self.analysis, self.config.mode()) {
            (Some(analysis), DefringeMode::GlobalAverage) => Some(analysis.clone()),
            (None, DefringeMode::GlobalAverage) => Some(make_analysis(self.config, &edge)?),
            _ => None,
        };
        let average_edge = analysis
            .as_ref()
            .map_or(33.0, DefringeAnalysis::average_edge_chroma);
        let base_threshold = match analysis.as_ref() {
            Some(value) => value.global_threshold(),
            None => threshold_for(self.config, None),
        };
        let width = usize::try_from(self.dimensions.width()).expect("validated width");
        let height = usize::try_from(self.dimensions.height()).expect("validated height");
        let mut output = input.to_vec();
        for y in 0..height {
            if cancelled() {
                return Err(DefringeExecutionError::Cancelled);
            }
            for x in 0..width {
                let index = y * width + x;
                let (local_threshold, weight_average) = if self.config.mode()
                    == DefringeMode::LocalAverage
                    && edge[index] > base_threshold
                {
                    let local = lattice_average(&edge, width, height, x, y, &self.average_lattice)
                        .max(0.01);
                    (threshold_for(self.config, Some(local)), local)
                } else {
                    (base_threshold, average_edge)
                };
                if grows_from_neighbor(&edge, width, height, x, y, local_threshold) {
                    let mut a_total = 0.0;
                    let mut b_total = 0.0;
                    let mut norm = 0.0;
                    for &(dx, dy) in &self.small_lattice {
                        let sample_x = clamp_coordinate(x, dx, width);
                        let sample_y = clamp_coordinate(y, dy, height);
                        let sample = sample_y * width + sample_x;
                        let weight = 1.0 / (edge[sample] + weight_average);
                        a_total += weight * channels[sample][1];
                        b_total += weight * channels[sample][2];
                        norm += weight;
                    }
                    if norm.is_finite() && norm > 0.0 {
                        output[index] = DefringePixel::new(
                            channels[index][0],
                            a_total / norm,
                            b_total / norm,
                            channels[index][3],
                        );
                    }
                }
            }
        }
        let output = blend(input, &output, mask, opacity, width, cancelled)?;
        validate_output(&output)?;
        Ok((
            output,
            ExecutionDetails {
                average_edge_chroma: analysis.as_ref().map(DefringeAnalysis::average_edge_chroma),
                threshold: base_threshold,
                global_threshold: analysis.as_ref().map(DefringeAnalysis::global_threshold),
                analysis_identity: analysis.map_or([0; 32], |value| value.identity()),
                outcome: DefringeOutcome::Complete,
            },
        ))
    }
}

#[derive(Debug, Clone, Copy)]
struct ExecutionDetails {
    average_edge_chroma: Option<f32>,
    threshold: f32,
    global_threshold: Option<f32>,
    analysis_identity: [u8; 32],
    outcome: DefringeOutcome,
}

fn validate_input(
    input: &[DefringePixel],
    dimensions: RasterDimensions,
) -> Result<Vec<[f32; 4]>, DefringeExecutionError> {
    let expected = usize::try_from(dimensions.pixel_count())
        .map_err(|_| DefringeExecutionError::ArithmeticOverflow)?;
    if input.len() != expected {
        return Err(DefringeExecutionError::DimensionsMismatch {
            expected,
            actual: input.len(),
        });
    }
    for (pixel, value) in input.iter().enumerate() {
        for (channel, sample) in value.channels().into_iter().enumerate() {
            if !sample.is_finite() {
                return Err(DefringeExecutionError::NonFiniteInput { pixel, channel });
            }
        }
    }
    Ok(input.iter().map(|pixel| pixel.channels()).collect())
}

fn validate_output(input: &[DefringePixel]) -> Result<(), DefringeExecutionError> {
    for (pixel, value) in input.iter().enumerate() {
        for (channel, sample) in value.channels().into_iter().enumerate() {
            if !sample.is_finite() {
                return Err(DefringeExecutionError::NonFiniteResult { pixel, channel });
            }
        }
    }
    Ok(())
}

fn make_analysis(
    config: DefringeConfig,
    edge: &[f32],
) -> Result<DefringeAnalysis, DefringeExecutionError> {
    let sum = edge.iter().try_fold(0.0f32, |sum, value| {
        let next = sum + *value;
        next.is_finite()
            .then_some(next)
            .ok_or(DefringeExecutionError::GaussianFailure)
    })?;
    let average = sum / edge.len() as f32 + 10.0 * f32::EPSILON;
    let global_threshold = threshold_for(config, Some(average));
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.defringe.analysis.v1");
    hasher.update(average.to_bits().to_le_bytes());
    hasher.update(global_threshold.to_bits().to_le_bytes());
    Ok(DefringeAnalysis {
        average_edge_chroma: average,
        global_threshold,
        identity: hasher.finalize().into(),
    })
}

fn threshold_for(config: DefringeConfig, average: Option<f32>) -> f32 {
    match average {
        Some(value) => {
            (4.0 * config.threshold() * value / DEFRINGE_MAGIC_THRESHOLD_COEFFICIENT).max(0.1)
        }
        None => config.threshold().max(0.1),
    }
}

fn sample_index_for(samples_wish: usize) -> usize {
    if samples_wish > 89 {
        12
    } else if samples_wish > 55 {
        11
    } else if samples_wish > 34 {
        10
    } else if samples_wish > 21 {
        9
    } else if samples_wish > 13 {
        8
    } else {
        7
    }
}

fn fib_count(index: usize) -> usize {
    FIBONACCI[index] as usize
}

fn make_lattice(
    radius: usize,
    index: usize,
    count: usize,
) -> Result<Vec<(i32, i32)>, DefringeExecutionError> {
    let step = radius as f32;
    let denominator = FIBONACCI[index];
    let ratio = FIBONACCI[index + 1] / denominator;
    let mut lattice = Vec::with_capacity(count);
    for sample in 0..count {
        let px = sample as f32 / denominator;
        let mut py = sample as f32 * ratio;
        py -= py as i32 as f32;
        let x = (px * step - step / 2.0).round();
        let y = (py * step - step / 2.0).round();
        if !x.is_finite()
            || !y.is_finite()
            || x < i32::MIN as f32
            || x > i32::MAX as f32
            || y < i32::MIN as f32
            || y > i32::MAX as f32
        {
            return Err(DefringeExecutionError::ArithmeticOverflow);
        }
        lattice.push((x as i32, y as i32));
    }
    Ok(lattice)
}

fn clamp_coordinate(base: usize, offset: i32, limit: usize) -> usize {
    let value = base as i128 + i128::from(offset);
    usize::try_from(value.clamp(0, limit.saturating_sub(1) as i128))
        .expect("clamped coordinate fits usize")
}

fn lattice_average(
    edge: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    lattice: &[(i32, i32)],
) -> f32 {
    let sum = lattice.iter().fold(0.0, |sum, &(dx, dy)| {
        sum + edge[clamp_coordinate(y, dy, height) * width + clamp_coordinate(x, dx, width)]
    });
    sum / lattice.len() as f32
}

fn grows_from_neighbor(
    edge: &[f32],
    width: usize,
    height: usize,
    x: usize,
    y: usize,
    threshold: f32,
) -> bool {
    let x0 = x.saturating_sub(1);
    let x1 = x.saturating_add(1).min(width - 1);
    let y0 = y.saturating_sub(1);
    let y1 = y.saturating_add(1).min(height - 1);
    (y0..=y1).any(|row| (x0..=x1).any(|column| edge[row * width + column] > threshold))
}

fn blend<F: FnMut() -> bool>(
    source: &[DefringePixel],
    candidate: &[DefringePixel],
    mask: Option<&[f32]>,
    opacity: f32,
    width: usize,
    cancelled: &mut F,
) -> Result<Vec<DefringePixel>, DefringeExecutionError> {
    let mut output = Vec::with_capacity(source.len());
    for (index, (left, right)) in source.iter().zip(candidate).enumerate() {
        if mask.is_some() && index % width == 0 && cancelled() {
            return Err(DefringeExecutionError::Cancelled);
        }
        let coverage = mask.map_or(1.0, |values| values[index]);
        let amount = opacity * coverage;
        let left = left.channels();
        let right = right.channels();
        output.push(DefringePixel::from_channels([
            left[0] + (right[0] - left[0]) * amount,
            left[1] + (right[1] - left[1]) * amount,
            left[2] + (right[2] - left[2]) * amount,
            left[3],
        ]));
    }
    Ok(output)
}

fn width_for(dimensions: RasterDimensions) -> usize {
    usize::try_from(dimensions.width()).expect("validated width")
}

fn digest_pixels(pixels: &[DefringePixel]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.defringe.output.v1");
    for pixel in pixels {
        for value in pixel.channels() {
            hasher.update(value.to_bits().to_le_bytes());
        }
    }
    hasher.finalize().into()
}
