use crate::{FiniteF32, LinearRgb, RasterDimensions};
use sha2::{Digest, Sha256};
use std::fmt;

use super::geometry::{RotatePixelsPlan, f64_to_f32};
use super::sampling::{checked_output_index, pixel_count, sample_pixel, validate_buffer};

impl RotatePixelsPlan {
    /// Executes a tightly packed linear-RGB image using the canonical sampler.
    ///
    /// # Errors
    ///
    /// Returns an execution error for invalid input, cancellation, or arithmetic overflow.
    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<RotatePixelsExecution, RotatePixelsExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    /// # Errors
    ///
    /// Returns an execution error for invalid input, cancellation, or arithmetic overflow.
    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<RotatePixelsExecution, RotatePixelsExecutionError> {
        let packed = input
            .iter()
            .flat_map(|pixel| [pixel.red().get(), pixel.green().get(), pixel.blue().get()])
            .collect::<Vec<_>>();
        let source_stride = usize::try_from(self.source_dimensions.width())
            .map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)?
            .checked_mul(3)
            .ok_or(RotatePixelsExecutionError::ArithmeticOverflow)?;
        let pixels = self.execute_interleaved_with_cancel(&packed, 3, source_stride, cancelled)?;
        let mut output = Vec::with_capacity(pixels.len() / 3);
        for channels in pixels.as_chunks::<3>().0 {
            output.push(LinearRgb::new(
                FiniteF32::new(channels[0])
                    .map_err(|_| RotatePixelsExecutionError::NonFiniteInput)?,
                FiniteF32::new(channels[1])
                    .map_err(|_| RotatePixelsExecutionError::NonFiniteInput)?,
                FiniteF32::new(channels[2])
                    .map_err(|_| RotatePixelsExecutionError::NonFiniteInput)?,
            ));
        }
        Ok(RotatePixelsExecution {
            pixels: output,
            dimensions: self.output_dimensions,
            receipt: RotatePixelsReceipt {
                plan_identity: self.identity,
                input_digest: digest_f32(&packed),
                output_digest: digest_f32(&pixels),
            },
        })
    }

    /// # Errors
    ///
    /// Returns an execution error for invalid shape, stride, channels, or sampling coordinates.
    pub fn execute_interleaved(
        &self,
        input: &[f32],
        channels: usize,
        stride: usize,
    ) -> Result<Vec<f32>, RotatePixelsExecutionError> {
        self.execute_interleaved_with_cancel(input, channels, stride, || false)
    }

    /// # Errors
    ///
    /// Returns an execution error for invalid input, cancellation, or arithmetic overflow.
    pub fn execute_interleaved_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[f32],
        channels: usize,
        stride: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, RotatePixelsExecutionError> {
        let input_roi = self.source_roi;
        let output_roi = self.output_roi;
        validate_buffer(input, input_roi, channels, stride)?;
        if !(1..=4).contains(&channels) {
            return Err(RotatePixelsExecutionError::UnsupportedChannels(channels));
        }
        let output_count = pixel_count(self.output_dimensions)?
            .checked_mul(channels)
            .ok_or(RotatePixelsExecutionError::ArithmeticOverflow)?;
        let mut output = vec![0.0; output_count];
        for y in 0..output_roi.height() {
            if cancelled() {
                return Err(RotatePixelsExecutionError::Cancelled);
            }
            for x in 0..output_roi.width() {
                let point = self
                    .back_point([f64_to_f32(f64::from(x)), f64_to_f32(f64::from(y))])
                    .map_err(|_| RotatePixelsExecutionError::NonFiniteCoordinate)?;
                let sx = f64::from(point[0]) - f64::from(input_roi.x());
                let sy = f64::from(point[1]) - f64::from(input_roi.y());
                let sample = sample_pixel(
                    input,
                    input_roi,
                    channels,
                    stride,
                    sx,
                    sy,
                    self.interpolation,
                )?;
                let destination = checked_output_index(x, y, output_roi.width(), channels)?;
                output[destination..destination + channels].copy_from_slice(&sample);
            }
        }
        Ok(output)
    }

    /// # Errors
    ///
    /// Returns an execution error for invalid input, cancellation, or arithmetic overflow.
    pub fn execute_plane(
        &self,
        input: &[f32],
        stride: usize,
    ) -> Result<Vec<f32>, RotatePixelsExecutionError> {
        self.execute_interleaved(input, 1, stride)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RotatePixelsExecution {
    pixels: Vec<LinearRgb>,
    dimensions: RasterDimensions,
    receipt: RotatePixelsReceipt,
}

impl RotatePixelsExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn receipt(&self) -> &RotatePixelsReceipt {
        &self.receipt
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotatePixelsExecutionError {
    InvalidShape { expected: usize, actual: usize },
    InvalidStride { minimum: usize, actual: usize },
    UnsupportedChannels(usize),
    NonFiniteInput,
    NonFiniteCoordinate,
    ArithmeticOverflow,
    Cancelled,
    InvalidRoi,
}

impl fmt::Display for RotatePixelsExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidShape { expected, actual } => {
                write!(
                    formatter,
                    "rotatepixels expected {expected} values, got {actual}"
                )
            }
            Self::InvalidStride { minimum, actual } => {
                write!(
                    formatter,
                    "rotatepixels stride {actual} is smaller than {minimum}"
                )
            }
            Self::UnsupportedChannels(channels) => {
                write!(
                    formatter,
                    "rotatepixels does not support {channels} channels"
                )
            }
            Self::NonFiniteInput => formatter.write_str("rotatepixels input is non-finite"),
            Self::NonFiniteCoordinate => {
                formatter.write_str("rotatepixels sample coordinate is non-finite")
            }
            Self::ArithmeticOverflow => formatter.write_str("rotatepixels arithmetic overflowed"),
            Self::Cancelled => formatter.write_str("rotatepixels execution was cancelled"),
            Self::InvalidRoi => formatter.write_str("rotatepixels input ROI is invalid"),
        }
    }
}

impl std::error::Error for RotatePixelsExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RotatePixelsReceipt {
    plan_identity: [u8; 32],
    input_digest: [u8; 32],
    output_digest: [u8; 32],
}

impl RotatePixelsReceipt {
    #[must_use]
    pub const fn plan_identity(self) -> [u8; 32] {
        self.plan_identity
    }

    #[must_use]
    pub const fn input_digest(self) -> [u8; 32] {
        self.input_digest
    }

    #[must_use]
    pub const fn output_digest(self) -> [u8; 32] {
        self.output_digest
    }
}

/// WGPU dispatch information. The shared GPU worker binds the plan's f32
/// matrix and center to this fixed inverse-resampling kernel contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RotatePixelsWgpuDispatch {
    pub workgroups_x: u32,
    pub workgroups_y: u32,
    pub channels: u32,
}

#[must_use]
pub fn wgpu_dispatch(plan: &RotatePixelsPlan, channels: u32) -> RotatePixelsWgpuDispatch {
    RotatePixelsWgpuDispatch {
        workgroups_x: ceil_div(plan.output_dimensions.width(), 8),
        workgroups_y: ceil_div(plan.output_dimensions.height(), 8),
        channels,
    }
}

#[must_use]
pub const fn wgpu_passes() -> [&'static str; 2] {
    ["rotatepixels.image", "rotatepixels.mask"]
}

/// WGSL source shared by image and single-plane mask dispatches.
pub const ROTATEPIXELS_WGSL: &str = r"
fn reflect_index(value: i32, extent: i32) -> i32 {
  if (extent <= 1) { return 0; }
  let period = 2 * extent - 2;
  let wrapped = ((value % period) + period) % period;
  return select(wrapped, period - wrapped, wrapped >= extent);
}

fn rotatepixels_inverse(point: vec2<f32>, center: vec2<f32>, inverse: mat2x2<f32>) -> vec2<f32> {
  return inverse * point + center;
}

fn rotatepixels_bicubic(value: f32) -> f32 {
  let x = abs(value);
  return select(0.0, select((1.5 * x - 2.5) * x * x + 1.0, (2.5 - 1.5 * x) * x * x - 4.0 * x + 2.0, x >= 1.0), x >= 2.0);
}

fn rotatepixels_lanczos(value: f32) -> f32 {
  let x = abs(value);
  if (x >= 3.0) { return 0.0; }
  if (x < 0.000001) { return 1.0; }
  return sin(3.141592653589793 * x) * sin(3.141592653589793 * x / 3.0) / (3.141592653589793 * 3.0 * x * x);
}
";

fn digest_f32(values: &[f32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(super::codec::ROTATEPIXELS_COMPATIBILITY_ID.as_bytes());
    for value in values {
        hasher.update(value.to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}

fn ceil_div(value: u32, divisor: u32) -> u32 {
    value.saturating_add(divisor - 1) / divisor
}
