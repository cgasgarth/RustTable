#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::chunks_exact_to_as_chunks,
    clippy::missing_errors_doc,
    clippy::needless_raw_string_hashes,
    clippy::range_minus_one,
    clippy::too_many_arguments
)]

use crate::{FiniteF32, LinearRgb};
use sha2::{Digest, Sha256};

use super::{ClippingInterpolation, ClippingPlan, TransformPointError};

#[derive(Debug, Clone, PartialEq)]
pub struct ClippingExecution {
    pixels: Vec<LinearRgb>,
    dimensions: crate::RasterDimensions,
    receipt: ClippingReceipt,
}

impl ClippingExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    #[must_use]
    pub const fn dimensions(&self) -> crate::RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn receipt(&self) -> &ClippingReceipt {
        &self.receipt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClippingReceipt {
    plan_identity: [u8; 32],
    input_digest: [u8; 32],
    output_digest: [u8; 32],
}

impl ClippingReceipt {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClippingExecutionError {
    InvalidShape { expected: usize, actual: usize },
    InvalidStride { minimum: usize, actual: usize },
    NonFiniteInput,
    NonFiniteCoordinate,
    ArithmeticOverflow,
    Cancelled,
}

impl std::fmt::Display for ClippingExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidShape { expected, actual } => {
                write!(f, "clipping expected {expected} values, got {actual}")
            }
            Self::InvalidStride { minimum, actual } => {
                write!(f, "clipping stride {actual} is smaller than {minimum}")
            }
            Self::NonFiniteInput => f.write_str("clipping input is non-finite"),
            Self::NonFiniteCoordinate => f.write_str("clipping sample coordinate is non-finite"),
            Self::ArithmeticOverflow => f.write_str("clipping execution arithmetic overflowed"),
            Self::Cancelled => f.write_str("clipping execution was cancelled"),
        }
    }
}
impl std::error::Error for ClippingExecutionError {}

impl ClippingPlan {
    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<ClippingExecution, ClippingExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<ClippingExecution, ClippingExecutionError> {
        let expected = usize::try_from(self.source_dimensions().pixel_count())
            .map_err(|_| ClippingExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(ClippingExecutionError::InvalidShape {
                expected,
                actual: input.len(),
            });
        }
        if input.iter().any(|pixel| {
            ![pixel.red().get(), pixel.green().get(), pixel.blue().get()]
                .iter()
                .all(|value| value.is_finite())
        }) {
            return Err(ClippingExecutionError::NonFiniteInput);
        }
        let packed = input
            .iter()
            .flat_map(|pixel| [pixel.red().get(), pixel.green().get(), pixel.blue().get()])
            .collect::<Vec<_>>();
        let output = self.execute_interleaved_with_cancel(
            &packed,
            3,
            self.source_dimensions().width() as usize * 3,
            cancelled,
        )?;
        let mut pixels = Vec::with_capacity(output.len() / 3);
        for channels in output.chunks_exact(3) {
            pixels.push(LinearRgb::new(
                FiniteF32::new(channels[0]).map_err(|_| ClippingExecutionError::NonFiniteInput)?,
                FiniteF32::new(channels[1]).map_err(|_| ClippingExecutionError::NonFiniteInput)?,
                FiniteF32::new(channels[2]).map_err(|_| ClippingExecutionError::NonFiniteInput)?,
            ));
        }
        Ok(ClippingExecution {
            pixels,
            dimensions: self.output_dimensions(),
            receipt: ClippingReceipt {
                plan_identity: self.identity(),
                input_digest: digest(&packed),
                output_digest: digest(&output),
            },
        })
    }

    pub fn execute_plane(
        &self,
        input: &[f32],
        stride: usize,
    ) -> Result<Vec<f32>, ClippingExecutionError> {
        self.execute_plane_with_cancel(input, stride, || false)
    }

    pub fn execute_plane_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[f32],
        stride: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, ClippingExecutionError> {
        let width = self.source_dimensions().width() as usize;
        let height = self.source_dimensions().height() as usize;
        if stride < width {
            return Err(ClippingExecutionError::InvalidStride {
                minimum: width,
                actual: stride,
            });
        }
        let expected = stride
            .checked_mul(height)
            .ok_or(ClippingExecutionError::ArithmeticOverflow)?;
        if input.len() < expected {
            return Err(ClippingExecutionError::InvalidShape {
                expected,
                actual: input.len(),
            });
        }
        if input[..expected].iter().any(|value| !value.is_finite()) {
            return Err(ClippingExecutionError::NonFiniteInput);
        }
        self.resample(input, stride, 1, cancelled)
    }

    fn execute_interleaved_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[f32],
        channels: usize,
        stride: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, ClippingExecutionError> {
        let width = self.source_dimensions().width() as usize;
        let height = self.source_dimensions().height() as usize;
        let minimum_stride = width
            .checked_mul(channels)
            .ok_or(ClippingExecutionError::ArithmeticOverflow)?;
        if stride < minimum_stride {
            return Err(ClippingExecutionError::InvalidStride {
                minimum: minimum_stride,
                actual: stride,
            });
        }
        let expected = stride
            .checked_mul(height)
            .ok_or(ClippingExecutionError::ArithmeticOverflow)?;
        if input.len() < expected {
            return Err(ClippingExecutionError::InvalidShape {
                expected,
                actual: input.len(),
            });
        }
        self.resample(input, stride, channels, cancelled)
    }

    fn resample<F: Fn() -> bool>(
        &self,
        input: &[f32],
        stride: usize,
        channels: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, ClippingExecutionError> {
        let out_width = self.output_dimensions().width() as usize;
        let out_height = self.output_dimensions().height() as usize;
        let count = out_width
            .checked_mul(out_height)
            .and_then(|value| value.checked_mul(channels))
            .ok_or(ClippingExecutionError::ArithmeticOverflow)?;
        let mut output = vec![0.0; count];
        for y in 0..out_height {
            if cancelled() {
                return Err(ClippingExecutionError::Cancelled);
            }
            for x in 0..out_width {
                let point = self
                    .back_point(crate::operations::perspective::Point::new(
                        x as f64 + 0.5,
                        y as f64 + 0.5,
                    ))
                    .map_err(|error| match error {
                        TransformPointError::NonFinite | TransformPointError::AtInfinity => {
                            ClippingExecutionError::NonFiniteCoordinate
                        }
                    })?;
                let sx = point.x() - 0.5;
                let sy = point.y() - 0.5;
                for channel in 0..channels {
                    let index = (y * out_width + x) * channels + channel;
                    output[index] = sample(
                        input,
                        stride,
                        channels,
                        channel,
                        sx,
                        sy,
                        self.interpolation(),
                    );
                }
            }
        }
        Ok(output)
    }
}

fn sample(
    input: &[f32],
    stride: usize,
    channels: usize,
    channel: usize,
    x: f64,
    y: f64,
    interpolation: ClippingInterpolation,
) -> f32 {
    match interpolation {
        ClippingInterpolation::Nearest => sample_at(
            input,
            stride,
            channels,
            channel,
            x.round() as isize,
            y.round() as isize,
        ),
        ClippingInterpolation::Bilinear => {
            weighted_grid(input, stride, channels, channel, x, y, 1, bilinear_weight)
        }
        ClippingInterpolation::Bicubic => {
            weighted_grid(input, stride, channels, channel, x, y, 2, cubic_weight)
        }
        ClippingInterpolation::Lanczos => {
            weighted_grid(input, stride, channels, channel, x, y, 3, lanczos_weight)
        }
    }
}

fn weighted_grid(
    input: &[f32],
    stride: usize,
    channels: usize,
    channel: usize,
    x: f64,
    y: f64,
    radius: isize,
    weight: fn(f64) -> f64,
) -> f32 {
    let start_x = x.floor() as isize - radius + 1;
    let start_y = y.floor() as isize - radius + 1;
    let mut total = 0.0;
    let mut weight_total = 0.0;
    for iy in start_y..=start_y + radius * 2 - 1 {
        for ix in start_x..=start_x + radius * 2 - 1 {
            let w = weight(x - ix as f64) * weight(y - iy as f64);
            total += f64::from(sample_at(input, stride, channels, channel, ix, iy)) * w;
            weight_total += w;
        }
    }
    if weight_total.abs() < f64::EPSILON {
        0.0
    } else {
        (total / weight_total) as f32
    }
}

fn sample_at(
    input: &[f32],
    stride: usize,
    channels: usize,
    channel: usize,
    x: isize,
    y: isize,
) -> f32 {
    if x < 0 || y < 0 {
        return 0.0;
    }
    let x = x as usize;
    let y = y as usize;
    let width = stride / channels;
    if x >= width {
        return 0.0;
    }
    let Some(index) = y
        .checked_mul(stride)
        .and_then(|row| row.checked_add(x * channels + channel))
    else {
        return 0.0;
    };
    input.get(index).copied().unwrap_or(0.0)
}

fn bilinear_weight(distance: f64) -> f64 {
    (1.0 - distance.abs()).max(0.0)
}

fn cubic_weight(distance: f64) -> f64 {
    let x = distance.abs();
    if x < 1.0 {
        1.5 * x * x * x - 2.5 * x * x + 1.0
    } else if x < 2.0 {
        -0.5 * x * x * x + 2.5 * x * x - 4.0 * x + 2.0
    } else {
        0.0
    }
}

fn lanczos_weight(distance: f64) -> f64 {
    let x = distance.abs();
    if x >= 3.0 {
        0.0
    } else if x < 1.0e-12 {
        1.0
    } else {
        let p = std::f64::consts::PI * x;
        (p.sin() / p) * (p / 3.0).sin() / (p / 3.0)
    }
}

fn digest(values: &[f32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(super::codec::CLIPPING_COMPATIBILITY_ID.as_bytes());
    for value in values {
        hasher.update(value.to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClippingWgpuDispatch {
    pub workgroups_x: u32,
    pub workgroups_y: u32,
    pub channels: u32,
}

#[must_use]
pub fn wgpu_dispatch(plan: &ClippingPlan, channels: u32) -> ClippingWgpuDispatch {
    ClippingWgpuDispatch {
        workgroups_x: plan.output_dimensions().width().saturating_add(7) / 8,
        workgroups_y: plan.output_dimensions().height().saturating_add(7) / 8,
        channels,
    }
}

#[must_use]
pub const fn wgpu_passes() -> [&'static str; 2] {
    ["clipping.image", "clipping.mask"]
}

pub const CLIPPING_WGSL: &str = r#"
fn clipping_resample(source: texture_2d<f32>, output: texture_storage_2d<rgba32float, write>, inverse: mat3x3<f32>) {
  let xy = vec2<f32>(textureDimensions(output));
  let p = (vec2<f32>(global_invocation_id.xy) + vec2<f32>(0.5)) / xy;
  let q = inverse * vec3<f32>(p, 1.0);
  let source_xy = q.xy / q.z;
  textureStore(output, vec2<i32>(global_invocation_id.xy), textureSampleLevel(source, source_sampler, source_xy, 0.0));
}
"#;
