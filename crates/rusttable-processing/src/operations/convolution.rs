//! Shared deterministic separable convolution primitives for neighborhood effects.

#![allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::missing_errors_doc,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::too_many_lines
)]

use super::common::{OperationExecutionError, ReconstructionBudget, checked_bytes, validate_shape};
use crate::{FiniteF32, LinearRgb, RasterDimensions};

/// Error from the bounded four-channel recursive Gaussian used by Lab
/// compatibility operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BoundedGaussianError {
    InvalidSigma,
    Dimensions,
    Cancelled,
}

/// Shared order-zero Gaussian with explicit per-channel bounds and clamped
/// edges. The reduction order is fixed by the row/column traversal.
pub(crate) fn bounded_gaussian_4c<F: FnMut() -> bool>(
    input: &[[f32; 4]],
    dimensions: RasterDimensions,
    sigma: f32,
    minimum: [f32; 4],
    maximum: [f32; 4],
    cancelled: F,
) -> Result<Vec<[f32; 4]>, BoundedGaussianError> {
    bounded_gaussian_4c_order(input, dimensions, sigma, minimum, maximum, 0, cancelled)
}

/// Darktable-compatible bounded Gaussian with the persisted order selector.
pub(crate) fn bounded_gaussian_4c_order<F: FnMut() -> bool>(
    input: &[[f32; 4]],
    dimensions: RasterDimensions,
    sigma: f32,
    minimum: [f32; 4],
    maximum: [f32; 4],
    order: u32,
    mut cancelled: F,
) -> Result<Vec<[f32; 4]>, BoundedGaussianError> {
    if !sigma.is_finite() || sigma <= 0.0 {
        return Err(BoundedGaussianError::InvalidSigma);
    }
    let width =
        usize::try_from(dimensions.width()).map_err(|_| BoundedGaussianError::Dimensions)?;
    let height =
        usize::try_from(dimensions.height()).map_err(|_| BoundedGaussianError::Dimensions)?;
    let expected = width
        .checked_mul(height)
        .ok_or(BoundedGaussianError::Dimensions)?;
    if input.len() != expected {
        return Err(BoundedGaussianError::Dimensions);
    }
    let (a0, a1, a2, a3, b1, b2, coefp, coefn) = gaussian_parameters(sigma, order);
    let mut temp = vec![[0.0; 4]; expected];
    let mut output = vec![[0.0; 4]; expected];

    for x in 0..width {
        if cancelled() {
            return Err(BoundedGaussianError::Cancelled);
        }
        let first = clamp_channels(input[x], minimum, maximum);
        let mut xp = first;
        let mut yb = [0.0; 4];
        let mut yp = [0.0; 4];
        for channel in 0..4 {
            yb[channel] = first[channel] * coefp;
            yp[channel] = yb[channel];
        }
        for y in 0..height {
            let index = y * width + x;
            let sample = input[index];
            let mut value = [0.0; 4];
            for channel in 0..4 {
                let current = sample[channel].clamp(minimum[channel], maximum[channel]);
                value[channel] =
                    a0 * current + a1 * xp[channel] - b1 * yp[channel] - b2 * yb[channel];
                xp[channel] = current;
                yb[channel] = yp[channel];
                yp[channel] = value[channel];
            }
            temp[index] = value;
        }

        let last = clamp_channels(input[(height - 1) * width + x], minimum, maximum);
        let mut xn = last;
        let mut xa = last;
        let mut yn = last.map(|value| value * coefn);
        let mut ya = yn;
        for y in (0..height).rev() {
            let index = y * width + x;
            let sample = input[index];
            let mut value = [0.0; 4];
            for channel in 0..4 {
                let current = sample[channel].clamp(minimum[channel], maximum[channel]);
                value[channel] =
                    a2 * xn[channel] + a3 * xa[channel] - b1 * yn[channel] - b2 * ya[channel];
                xa[channel] = xn[channel];
                xn[channel] = current;
                ya[channel] = yn[channel];
                yn[channel] = value[channel];
                temp[index][channel] += value[channel];
            }
        }
    }

    for y in 0..height {
        if cancelled() {
            return Err(BoundedGaussianError::Cancelled);
        }
        let row = y * width;
        let first = clamp_channels(temp[row], minimum, maximum);
        let mut xp = first;
        let mut yb = first.map(|value| value * coefp);
        let mut yp = yb;
        for x in 0..width {
            let index = row + x;
            let sample = temp[index];
            let mut value = [0.0; 4];
            for channel in 0..4 {
                let current = sample[channel].clamp(minimum[channel], maximum[channel]);
                value[channel] =
                    a0 * current + a1 * xp[channel] - b1 * yp[channel] - b2 * yb[channel];
                xp[channel] = current;
                yb[channel] = yp[channel];
                yp[channel] = value[channel];
            }
            output[index] = value;
        }

        let last = clamp_channels(temp[row + width - 1], minimum, maximum);
        let mut xn = last;
        let mut xa = last;
        let mut yn = last.map(|value| value * coefn);
        let mut ya = yn;
        for x in (0..width).rev() {
            let index = row + x;
            let sample = temp[index];
            let mut value = [0.0; 4];
            for channel in 0..4 {
                let current = sample[channel].clamp(minimum[channel], maximum[channel]);
                value[channel] =
                    a2 * xn[channel] + a3 * xa[channel] - b1 * yn[channel] - b2 * ya[channel];
                xa[channel] = xn[channel];
                xn[channel] = current;
                ya[channel] = yn[channel];
                yn[channel] = value[channel];
                output[index][channel] += value[channel];
            }
        }
    }
    Ok(output)
}

fn gaussian_parameters(sigma: f32, order: u32) -> (f32, f32, f32, f32, f32, f32, f32, f32) {
    let alpha = 1.695 / sigma;
    let ema = (-alpha).exp();
    let ema2 = (-2.0 * alpha).exp();
    let b1 = -2.0 * ema;
    let b2 = ema2;
    let (a0, a1, a2, a3) = match order {
        1 => {
            let a0 = (1.0 - ema) * (1.0 - ema);
            (a0, 0.0, -a0, 0.0)
        }
        2 => {
            let k = -(ema2 - 1.0) / (2.0 * alpha * ema);
            let mut kn = -2.0 * (-1.0 + 3.0 * ema - 3.0 * ema2 + ema * ema * ema);
            kn /= 3.0 * ema + 1.0 + 3.0 * ema2 + ema * ema * ema;
            (
                kn,
                -kn * (1.0 + k * alpha) * ema,
                kn * (1.0 - k * alpha) * ema,
                -kn * ema2,
            )
        }
        _ => {
            let k = (1.0 - ema) * (1.0 - ema) / (1.0 + 2.0 * alpha * ema - ema2);
            (
                k,
                k * (alpha - 1.0) * ema,
                k * (alpha + 1.0) * ema,
                -k * ema2,
            )
        }
    };
    let denominator = 1.0 + b1 + b2;
    let coefp = (a0 + a1) / denominator;
    let coefn = (a2 + a3) / denominator;
    (a0, a1, a2, a3, b1, b2, coefp, coefn)
}

fn clamp_channels(value: [f32; 4], minimum: [f32; 4], maximum: [f32; 4]) -> [f32; 4] {
    std::array::from_fn(|channel| value[channel].clamp(minimum[channel], maximum[channel]))
}

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
