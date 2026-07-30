#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::struct_excessive_bools,
    clippy::too_many_lines
)]

use super::gaussian::ReferenceGaussian;
use super::rng::CensorizeRng;
use super::{CensorizeConfig, CensorizeParametersV1};
use crate::{RasterDimensions, operations::ReconstructionBudget};
use sha2::{Digest, Sha256};
use std::fmt;

pub const CENSORIZE_RNG_VERSION: &str = super::rng::CENSORIZE_RNG_VERSION;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CensorizePixel {
    channels: [f32; 4],
}

impl CensorizePixel {
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
    pub const fn red(self) -> f32 {
        self.channels[0]
    }
    #[must_use]
    pub const fn green(self) -> f32 {
        self.channels[1]
    }
    #[must_use]
    pub const fn blue(self) -> f32 {
        self.channels[2]
    }
    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CensorizeBackend {
    CpuScalarReference,
}

impl CensorizeBackend {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        "cpu-scalar-reference"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CensorizeStages {
    pre_blur: bool,
    pixelization: bool,
    post_blur: bool,
    noise: bool,
}
impl CensorizeStages {
    #[must_use]
    pub const fn pre_blur(self) -> bool {
        self.pre_blur
    }
    #[must_use]
    pub const fn pixelization(self) -> bool {
        self.pixelization
    }
    #[must_use]
    pub const fn post_blur(self) -> bool {
        self.post_blur
    }
    #[must_use]
    pub const fn noise(self) -> bool {
        self.noise
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CensorizeExecutionError {
    ArithmeticOverflow,
    DimensionsMismatch { expected: usize, actual: usize },
    InvalidScale,
    MemoryBudgetExceeded { required: usize, budget: usize },
    Cancelled,
    NonFiniteInput { pixel: usize, channel: usize },
    NonFiniteResult { pixel: usize, channel: usize },
    MaskLength { expected: usize, actual: usize },
    InvalidMaskValue,
}
impl fmt::Display for CensorizeExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow => f.write_str("censorize arithmetic overflowed"),
            Self::DimensionsMismatch { expected, actual } => {
                write!(f, "censorize expected {expected} pixels, got {actual}")
            }
            Self::InvalidScale => f.write_str("censorize scales must be finite and positive"),
            Self::MemoryBudgetExceeded { required, budget } => {
                write!(f, "censorize requires {required} bytes, budget is {budget}")
            }
            Self::Cancelled => f.write_str("censorize execution was cancelled"),
            Self::NonFiniteInput { pixel, channel } => {
                write!(
                    f,
                    "censorize input pixel {pixel} channel {channel} is non-finite"
                )
            }
            Self::NonFiniteResult { pixel, channel } => {
                write!(
                    f,
                    "censorize result pixel {pixel} channel {channel} is non-finite"
                )
            }
            Self::MaskLength { expected, actual } => {
                write!(f, "censorize mask has {actual} pixels, expected {expected}")
            }
            Self::InvalidMaskValue => {
                f.write_str("censorize mask coverage must be finite in 0..=1")
            }
        }
    }
}
impl std::error::Error for CensorizeExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CensorizeMask {
    identity: [u8; 32],
}
impl CensorizeMask {
    pub fn new(coverage: &[f32]) -> Result<Self, CensorizeExecutionError> {
        if coverage
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err(CensorizeExecutionError::InvalidMaskValue);
        }
        let mut hash = Sha256::new();
        hash.update(b"rusttable.censorize.mask.v1");
        for value in coverage {
            hash.update(value.to_bits().to_le_bytes());
        }
        Ok(Self {
            identity: hash.finalize().into(),
        })
    }
    #[must_use]
    pub const fn identity(self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CensorizeBlend {
    identity: [u8; 32],
}
impl CensorizeBlend {
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
pub struct CensorizeReceipt {
    parameters: CensorizeParametersV1,
    input_scale: f32,
    roi_scale: f32,
    sigma_1: f32,
    sigma_2: f32,
    pixel_radius: usize,
    effective_noise: f32,
    stages: CensorizeStages,
    noise_calls: u8,
    memory_estimate: usize,
    backend: CensorizeBackend,
    rng_version: &'static str,
    mask_identity: [u8; 32],
    blend_identity: [u8; 32],
    output_hash: [u8; 32],
}
impl CensorizeReceipt {
    #[must_use]
    pub const fn parameters(&self) -> CensorizeParametersV1 {
        self.parameters
    }
    #[must_use]
    pub const fn sigma_1(&self) -> f32 {
        self.sigma_1
    }
    #[must_use]
    pub const fn sigma_2(&self) -> f32 {
        self.sigma_2
    }
    #[must_use]
    pub const fn pixel_radius(&self) -> usize {
        self.pixel_radius
    }
    #[must_use]
    pub const fn effective_noise(&self) -> f32 {
        self.effective_noise
    }
    #[must_use]
    pub const fn stages(&self) -> CensorizeStages {
        self.stages
    }
    #[must_use]
    pub const fn noise_calls(&self) -> u8 {
        self.noise_calls
    }
    #[must_use]
    pub const fn memory_estimate(&self) -> usize {
        self.memory_estimate
    }
    #[must_use]
    pub const fn backend(&self) -> CensorizeBackend {
        self.backend
    }
    #[must_use]
    pub const fn rng_version(&self) -> &'static str {
        self.rng_version
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
    pub const fn output_hash(&self) -> [u8; 32] {
        self.output_hash
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CensorizePlan {
    config: CensorizeConfig,
    dimensions: RasterDimensions,
    input_scale: f32,
    roi_scale: f32,
    sigma_1: f32,
    sigma_2: f32,
    pixel_radius: usize,
    effective_noise: f32,
    stages: CensorizeStages,
    memory_estimate: usize,
    backend: CensorizeBackend,
}

impl CensorizePlan {
    pub fn new(
        config: CensorizeConfig,
        dimensions: RasterDimensions,
        input_scale: f32,
        roi_scale: f32,
    ) -> Result<Self, CensorizeExecutionError> {
        Self::with_budget(
            config,
            dimensions,
            input_scale,
            roi_scale,
            ReconstructionBudget::default().maximum_bytes(),
        )
    }

    pub fn with_budget(
        config: CensorizeConfig,
        dimensions: RasterDimensions,
        input_scale: f32,
        roi_scale: f32,
        budget: usize,
    ) -> Result<Self, CensorizeExecutionError> {
        if !input_scale.is_finite()
            || !roi_scale.is_finite()
            || input_scale <= 0.0
            || roi_scale <= 0.0
        {
            return Err(CensorizeExecutionError::InvalidScale);
        }
        let count = usize::try_from(dimensions.pixel_count())
            .map_err(|_| CensorizeExecutionError::ArithmeticOverflow)?;
        let scale = roi_scale / input_scale;
        let radius_1 = config.radius_1() * scale;
        let radius_2 = config.radius_2() * scale;
        let pixel_value = config.pixelate() * scale;
        if !radius_1.is_finite() || !radius_2.is_finite() || !pixel_value.is_finite() {
            return Err(CensorizeExecutionError::ArithmeticOverflow);
        }
        let pixel_radius = if pixel_value < usize::MAX as f32 {
            pixel_value as usize
        } else {
            return Err(CensorizeExecutionError::ArithmeticOverflow);
        };
        let stages = CensorizeStages {
            pre_blur: radius_1.to_bits() != 0,
            pixelization: pixel_radius != 0,
            post_blur: radius_2.to_bits() != 0,
            noise: config.noise().to_bits() != 0,
        };
        let image_bytes = count
            .checked_mul(16)
            .ok_or(CensorizeExecutionError::ArithmeticOverflow)?;
        let buffers = 1usize
            .checked_add(usize::from(stages.pixelization))
            .and_then(|value| value.checked_add(usize::from(stages.pre_blur || stages.post_blur)))
            .ok_or(CensorizeExecutionError::ArithmeticOverflow)?;
        let memory_estimate = image_bytes
            .checked_mul(buffers)
            .ok_or(CensorizeExecutionError::ArithmeticOverflow)?;
        if memory_estimate > budget {
            return Err(CensorizeExecutionError::MemoryBudgetExceeded {
                required: memory_estimate,
                budget,
            });
        }
        Ok(Self {
            config,
            dimensions,
            input_scale,
            roi_scale,
            sigma_1: radius_1,
            sigma_2: radius_2,
            pixel_radius,
            effective_noise: config.noise() / (input_scale / roi_scale).max(1.0),
            stages,
            memory_estimate,
            backend: CensorizeBackend::CpuScalarReference,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn config(&self) -> CensorizeConfig {
        self.config
    }
    #[must_use]
    pub const fn sigma_1(&self) -> f32 {
        self.sigma_1
    }
    #[must_use]
    pub const fn sigma_2(&self) -> f32 {
        self.sigma_2
    }
    #[must_use]
    pub const fn pixel_radius(&self) -> usize {
        self.pixel_radius
    }
    #[must_use]
    pub const fn effective_noise(&self) -> f32 {
        self.effective_noise
    }
    #[must_use]
    pub const fn stages(&self) -> CensorizeStages {
        self.stages
    }
    #[must_use]
    pub const fn memory_estimate(&self) -> usize {
        self.memory_estimate
    }
    #[must_use]
    pub const fn backend(&self) -> CensorizeBackend {
        self.backend
    }
    #[must_use]
    pub const fn noise_calls(&self) -> u8 {
        if !self.stages.noise {
            0
        } else if self.stages.post_blur {
            2
        } else {
            1
        }
    }

    pub fn execute<F: FnMut() -> bool>(
        &self,
        input: &[CensorizePixel],
        cancelled: F,
    ) -> Result<Vec<CensorizePixel>, CensorizeExecutionError> {
        self.execute_with_mask(input, None, 1.0, cancelled)
    }

    pub fn execute_with_mask<F: FnMut() -> bool>(
        &self,
        input: &[CensorizePixel],
        mask: Option<&[f32]>,
        opacity: f32,
        mut cancelled: F,
    ) -> Result<Vec<CensorizePixel>, CensorizeExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| CensorizeExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(CensorizeExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        for (pixel, value) in input.iter().enumerate() {
            for (channel, channel_value) in value.channels().into_iter().enumerate() {
                if !channel_value.is_finite() {
                    return Err(CensorizeExecutionError::NonFiniteInput { pixel, channel });
                }
            }
        }
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(CensorizeExecutionError::InvalidMaskValue);
        }
        let _mask_identity = if let Some(values) = mask {
            if values.len() != expected {
                return Err(CensorizeExecutionError::MaskLength {
                    expected,
                    actual: values.len(),
                });
            }
            CensorizeMask::new(values)?.identity()
        } else {
            [0; 32]
        };
        if cancelled() {
            return Err(CensorizeExecutionError::Cancelled);
        }
        let mut current = input.to_vec();
        if self.stages.pre_blur {
            current = ReferenceGaussian::new(self.sigma_1).apply(
                &current,
                self.dimensions,
                &mut cancelled,
            )?;
        }
        if self.stages.pixelization {
            current = pixelize(&current, self.dimensions, self.pixel_radius, &mut cancelled)?;
        }
        if self.stages.post_blur {
            if self.stages.noise {
                make_noise(
                    &mut current,
                    self.dimensions,
                    self.effective_noise,
                    &mut cancelled,
                )?;
            }
            current = ReferenceGaussian::new(self.sigma_2).apply(
                &current,
                self.dimensions,
                &mut cancelled,
            )?;
        }
        if self.stages.noise {
            make_noise(
                &mut current,
                self.dimensions,
                self.effective_noise,
                &mut cancelled,
            )?;
        }
        for (index, (pixel, source)) in current.iter_mut().zip(input).enumerate() {
            let mut channels = pixel.channels();
            channels[3] = source.alpha();
            *pixel = CensorizePixel::from_channels(channels);
            for (channel, channel_value) in channels.into_iter().enumerate() {
                if !channel_value.is_finite() {
                    return Err(CensorizeExecutionError::NonFiniteResult {
                        pixel: index,
                        channel,
                    });
                }
            }
        }
        if mask.is_some() || opacity.to_bits() != 1.0f32.to_bits() {
            let values = mask.unwrap_or(&[]);
            let width = usize::try_from(self.dimensions.width())
                .map_err(|_| CensorizeExecutionError::ArithmeticOverflow)?;
            for index in 0..current.len() {
                if index.is_multiple_of(width) && cancelled() {
                    return Err(CensorizeExecutionError::Cancelled);
                }
                let coverage = values.get(index).copied().unwrap_or(1.0);
                let amount = opacity * coverage;
                let source = input[index].channels();
                let candidate = current[index].channels();
                let mut blended = [0.0; 4];
                for channel in 0..4 {
                    blended[channel] =
                        source[channel] + (candidate[channel] - source[channel]) * amount;
                }
                current[index] = CensorizePixel::from_channels(blended);
            }
        }
        Ok(current)
    }

    pub fn execute_with_receipt<F: FnMut() -> bool>(
        &self,
        input: &[CensorizePixel],
        mask: Option<&[f32]>,
        opacity: f32,
        cancelled: F,
    ) -> Result<(Vec<CensorizePixel>, CensorizeReceipt), CensorizeExecutionError> {
        let output = self.execute_with_mask(input, mask, opacity, cancelled)?;
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.censorize.output.v1");
        for pixel in &output {
            for value in pixel.channels() {
                hasher.update(value.to_bits().to_le_bytes());
            }
        }
        let receipt = CensorizeReceipt {
            parameters: self.config.parameters(),
            input_scale: self.input_scale,
            roi_scale: self.roi_scale,
            sigma_1: self.sigma_1,
            sigma_2: self.sigma_2,
            pixel_radius: self.pixel_radius,
            effective_noise: self.effective_noise,
            stages: self.stages,
            noise_calls: self.noise_calls(),
            memory_estimate: self.memory_estimate,
            backend: self.backend,
            rng_version: CENSORIZE_RNG_VERSION,
            mask_identity: mask.map_or([0; 32], |values| {
                CensorizeMask::new(values)
                    .expect("validated mask")
                    .identity()
            }),
            blend_identity: CensorizeBlend::normal().identity(),
            output_hash: hasher.finalize().into(),
        };
        Ok((output, receipt))
    }
}

fn pixelize<F: FnMut() -> bool>(
    input: &[CensorizePixel],
    dimensions: RasterDimensions,
    radius: usize,
    cancelled: &mut F,
) -> Result<Vec<CensorizePixel>, CensorizeExecutionError> {
    let width = usize::try_from(dimensions.width())
        .map_err(|_| CensorizeExecutionError::ArithmeticOverflow)?;
    let height = usize::try_from(dimensions.height())
        .map_err(|_| CensorizeExecutionError::ArithmeticOverflow)?;
    let side = radius
        .checked_mul(2)
        .ok_or(CensorizeExecutionError::ArithmeticOverflow)?;
    let cells_x = width / side;
    let cells_y = height / side;
    let mut output = input.to_vec();
    for j in 0..=cells_y {
        if cancelled() {
            return Err(CensorizeExecutionError::Cancelled);
        }
        for i in 0..=cells_x {
            let tl_x = i
                .checked_mul(side)
                .ok_or(CensorizeExecutionError::ArithmeticOverflow)?
                .min(width - 1);
            let tl_y = j
                .checked_mul(side)
                .ok_or(CensorizeExecutionError::ArithmeticOverflow)?
                .min(height - 1);
            let cc_x = tl_x.saturating_add(radius).min(width - 1);
            let cc_y = tl_y.saturating_add(radius).min(height - 1);
            let br_x = cc_x.saturating_add(radius).min(width - 1);
            let br_y = cc_y.saturating_add(radius).min(height - 1);
            let points = [
                (tl_x, tl_y),
                (br_x, tl_y),
                (cc_x, cc_y),
                (tl_x, br_y),
                (br_x, br_y),
            ];
            let mut average = [0.0; 4];
            for (x, y) in points {
                let values = input[y * width + x].channels();
                for channel in 0..4 {
                    average[channel] += values[channel] / 5.0;
                }
            }
            for y in tl_y..br_y {
                for x in tl_x..br_x {
                    output[y * width + x] = CensorizePixel::from_channels(average);
                }
            }
        }
    }
    Ok(output)
}

fn make_noise<F: FnMut() -> bool>(
    pixels: &mut [CensorizePixel],
    dimensions: RasterDimensions,
    noise: f32,
    cancelled: &mut F,
) -> Result<(), CensorizeExecutionError> {
    let width = usize::try_from(dimensions.width())
        .map_err(|_| CensorizeExecutionError::ArithmeticOverflow)?;
    for (index, pixel) in pixels.iter_mut().enumerate() {
        if index.is_multiple_of(width) && cancelled() {
            return Err(CensorizeExecutionError::Cancelled);
        }
        let x = index % width;
        let y = index / width;
        let mut rng = CensorizeRng::for_pixel(x, y);
        let norm = pixel.green();
        let sampled = rng.gaussian(norm, noise * norm, y % 2 != 0 || x % 2 != 0);
        let epsilon = sampled / norm;
        let mut values = pixel.channels();
        for value in values.iter_mut().take(3) {
            let scaled = *value * epsilon;
            *value = if scaled.is_nan() {
                0.0
            } else {
                scaled.max(0.0)
            };
        }
        *pixel = CensorizePixel::from_channels(values);
    }
    Ok(())
}
