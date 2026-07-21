//! Shared deterministic separable convolution primitives for neighborhood effects.

#![allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::missing_errors_doc,
    clippy::needless_range_loop
)]

use super::common::{OperationExecutionError, ReconstructionBudget, checked_bytes, validate_shape};
use crate::{FiniteF32, LinearRgb, RasterDimensions};

pub const BOX_ITERATIONS: u8 = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct GaussianKernel {
    support: u32,
    weights: Vec<f32>,
}

impl GaussianKernel {
    /// Builds the Gaussian approximation used by Darktable's soften `OpenCL` path.
    pub fn from_box_radius(radius: u32) -> Result<Self, ConvolutionError> {
        if radius == 0 {
            return Ok(Self {
                support: 0,
                weights: vec![1.0],
            });
        }
        let radius_f = radius as f32;
        let sigma = ((radius_f * (radius_f + 1.0) * f32::from(BOX_ITERATIONS) + 2.0) / 3.0).sqrt();
        let support = (3.0 * sigma).ceil() as u32;
        let width = support
            .checked_mul(2)
            .and_then(|value| value.checked_add(1))
            .ok_or(ConvolutionError::SupportOverflow)?;
        let mut weights = Vec::with_capacity(usize::try_from(width).unwrap_or(usize::MAX));
        let mut total = 0.0f32;
        for offset in 0..=support {
            let x = offset as f32;
            let weight = (-(x * x) / (2.0 * sigma * sigma)).exp();
            if offset == 0 {
                total += weight;
            } else {
                total += 2.0 * weight;
            }
            weights.push(weight);
        }
        for weight in &mut weights {
            *weight /= total;
        }
        Ok(Self { support, weights })
    }

    #[must_use]
    pub const fn support(&self) -> u32 {
        self.support
    }

    /// Applies a clamped-to-edge separable Gaussian to RGB values.
    pub fn apply_rgb(
        &self,
        input: &[LinearRgb],
        dimensions: RasterDimensions,
        budget: ReconstructionBudget,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        validate_shape(dimensions, input)?;
        checked_bytes(input.len(), 3, budget)?;
        if self.support == 0 {
            return Ok(input.to_vec());
        }
        let horizontal = convolve_rgb(input, dimensions, self.support, &self.weights, true)?;
        convolve_rgb(&horizontal, dimensions, self.support, &self.weights, false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxKernel {
    radius: u32,
    iterations: u8,
}

impl BoxKernel {
    #[must_use]
    pub const fn new(radius: u32) -> Self {
        Self {
            radius,
            iterations: BOX_ITERATIONS,
        }
    }

    #[must_use]
    pub const fn radius(self) -> u32 {
        self.radius
    }

    #[must_use]
    pub const fn support(self) -> u32 {
        self.radius.saturating_mul(self.iterations as u32)
    }

    pub fn apply_scalar(
        self,
        input: &[f32],
        dimensions: RasterDimensions,
        budget: ReconstructionBudget,
    ) -> Result<Vec<f32>, OperationExecutionError> {
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
        checked_bytes(input.len(), 2, budget)?;
        if self.radius == 0 {
            return Ok(input.to_vec());
        }
        let mut current = input.to_vec();
        let mut scratch = vec![0.0; input.len()];
        for _ in 0..self.iterations {
            convolve_scalar(&current, &mut scratch, dimensions, self.radius, true);
            convolve_scalar(&scratch, &mut current, dimensions, self.radius, false);
        }
        Ok(current)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvolutionError {
    SupportOverflow,
}

impl std::fmt::Display for ConvolutionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("convolution support exceeds the bounded planner")
    }
}

impl std::error::Error for ConvolutionError {}

fn convolve_rgb(
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    support: u32,
    weights: &[f32],
    horizontal: bool,
) -> Result<Vec<LinearRgb>, OperationExecutionError> {
    let mut output = Vec::with_capacity(input.len());
    let width = usize::try_from(dimensions.width()).map_err(|_| {
        OperationExecutionError::DimensionsMismatch {
            expected: usize::MAX,
            actual: input.len(),
        }
    })?;
    for index in 0..input.len() {
        let x = index % width;
        let y = index / width;
        let mut red = 0.0f32;
        let mut green = 0.0f32;
        let mut blue = 0.0f32;
        for offset in 0..=support {
            let weight = weights[usize::try_from(offset).expect("bounded support")];
            if offset == 0 {
                let pixel = input[index];
                red += pixel.red().get() * weight;
                green += pixel.green().get() * weight;
                blue += pixel.blue().get() * weight;
            } else {
                for direction in [-1i32, 1] {
                    let sample = sample_index(
                        x,
                        y,
                        direction * i32::try_from(offset).expect("bounded support"),
                        horizontal,
                        dimensions,
                    );
                    let pixel = input[sample];
                    red += pixel.red().get() * weight;
                    green += pixel.green().get() * weight;
                    blue += pixel.blue().get() * weight;
                }
            }
        }
        output.push(LinearRgb::new(
            finite(red, index, crate::RgbChannel::Red)?,
            finite(green, index, crate::RgbChannel::Green)?,
            finite(blue, index, crate::RgbChannel::Blue)?,
        ));
    }
    Ok(output)
}

fn convolve_scalar(
    input: &[f32],
    output: &mut [f32],
    dimensions: RasterDimensions,
    radius: u32,
    horizontal: bool,
) {
    let width = usize::try_from(dimensions.width()).expect("validated width");
    let radius = i32::try_from(radius).expect("bounded radius");
    let divisor = (radius * 2 + 1) as f32;
    for index in 0..input.len() {
        let x = index % width;
        let y = index / width;
        let mut sum = 0.0f32;
        for offset in -radius..=radius {
            let sample = sample_index(x, y, offset, horizontal, dimensions);
            sum += input[sample];
        }
        output[index] = sum / divisor;
    }
}

fn sample_index(
    x: usize,
    y: usize,
    offset: i32,
    horizontal: bool,
    dimensions: RasterDimensions,
) -> usize {
    let width = usize::try_from(dimensions.width()).expect("validated width");
    let height = usize::try_from(dimensions.height()).expect("validated height");
    let limit = if horizontal { width } else { height };
    let base = if horizontal { x } else { y };
    let sample = if offset.is_negative() {
        base.saturating_sub(usize::try_from(offset.unsigned_abs()).expect("bounded offset"))
    } else {
        base.saturating_add(usize::try_from(offset).expect("bounded offset"))
    }
    .min(limit - 1);
    if horizontal {
        y * width + sample
    } else {
        sample * width + x
    }
}

fn finite(
    value: f32,
    pixel: usize,
    channel: crate::RgbChannel,
) -> Result<FiniteF32, OperationExecutionError> {
    FiniteF32::new(value).map_err(|_| OperationExecutionError::NonFiniteResult { pixel, channel })
}
