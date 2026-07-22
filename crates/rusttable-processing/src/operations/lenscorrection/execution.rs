use super::geometry::{LensCorrectionCoordinateError, LensCorrectionPlan};
use crate::{FiniteF32, LinearRgb};
use sha2::{Digest, Sha256};
use std::fmt;

impl LensCorrectionPlan {
    /// Executes a tightly packed linear-RGB image with deterministic bilinear
    /// inverse resampling.
    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<LensCorrectionExecution, LensCorrectionExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    /// Executes a tightly packed linear-RGB image with a cancellation check
    /// at every output row.
    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<LensCorrectionExecution, LensCorrectionExecutionError> {
        let packed = input
            .iter()
            .flat_map(|pixel| [pixel.red().get(), pixel.green().get(), pixel.blue().get()])
            .collect::<Vec<_>>();
        let stride = usize::try_from(self.source_dimensions().width())
            .map_err(|_| LensCorrectionExecutionError::ArithmeticOverflow)?
            .checked_mul(3)
            .ok_or(LensCorrectionExecutionError::ArithmeticOverflow)?;
        let values = self.execute_interleaved_with_cancel(&packed, 3, stride, cancelled)?;
        let mut pixels = Vec::with_capacity(values.len() / 3);
        for channels in values.as_chunks::<3>().0 {
            pixels.push(LinearRgb::new(
                FiniteF32::new(channels[0])
                    .map_err(|_| LensCorrectionExecutionError::NonFiniteOutput)?,
                FiniteF32::new(channels[1])
                    .map_err(|_| LensCorrectionExecutionError::NonFiniteOutput)?,
                FiniteF32::new(channels[2])
                    .map_err(|_| LensCorrectionExecutionError::NonFiniteOutput)?,
            ));
        }
        Ok(LensCorrectionExecution {
            pixels,
            dimensions: self.output_dimensions(),
            receipt: LensCorrectionReceipt {
                plan_identity: self.identity(),
                input_digest: digest_f32(&packed),
                output_digest: digest_f32(&values),
            },
        })
    }

    /// Executes an interleaved 1–4 channel plane.  The input and output are
    /// full-image buffers; ROI planning remains explicit on the plan.
    pub fn execute_interleaved(
        &self,
        input: &[f32],
        channels: usize,
        stride: usize,
    ) -> Result<Vec<f32>, LensCorrectionExecutionError> {
        self.execute_interleaved_with_cancel(input, channels, stride, || false)
    }

    /// As above, with a cancellation boundary checked once per output row.
    pub fn execute_interleaved_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[f32],
        channels: usize,
        stride: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, LensCorrectionExecutionError> {
        if !(1..=4).contains(&channels) {
            return Err(LensCorrectionExecutionError::UnsupportedChannels(channels));
        }
        let width = usize::try_from(self.source_dimensions().width())
            .map_err(|_| LensCorrectionExecutionError::ArithmeticOverflow)?;
        let height = usize::try_from(self.source_dimensions().height())
            .map_err(|_| LensCorrectionExecutionError::ArithmeticOverflow)?;
        let minimum_stride = width
            .checked_mul(channels)
            .ok_or(LensCorrectionExecutionError::ArithmeticOverflow)?;
        if stride < minimum_stride {
            return Err(LensCorrectionExecutionError::InvalidStride {
                minimum: minimum_stride,
                actual: stride,
            });
        }
        let expected = stride
            .checked_mul(height)
            .ok_or(LensCorrectionExecutionError::ArithmeticOverflow)?;
        if input.len() < expected {
            return Err(LensCorrectionExecutionError::InvalidShape {
                minimum: expected,
                actual: input.len(),
            });
        }
        if input.iter().any(|value| !value.is_finite()) {
            return Err(LensCorrectionExecutionError::NonFiniteInput);
        }
        let output_len = minimum_stride
            .checked_mul(height)
            .ok_or(LensCorrectionExecutionError::ArithmeticOverflow)?;
        let mut output = vec![0.0; output_len];
        for y in 0..height {
            if cancelled() {
                return Err(LensCorrectionExecutionError::Cancelled);
            }
            for x in 0..width {
                let x_f32 = x as f32;
                let y_f32 = y as f32;
                let destination = y
                    .checked_mul(minimum_stride)
                    .and_then(|row| row.checked_add(x.checked_mul(channels)?))
                    .ok_or(LensCorrectionExecutionError::ArithmeticOverflow)?;
                for channel in 0..channels {
                    let coordinate = self
                        .back_channel_point([x_f32, y_f32], channel)
                        .map_err(LensCorrectionExecutionError::Coordinate)?;
                    let sample = bilinear_sample(input, stride, channels, coordinate, channel)?;
                    output[destination + channel] = if channel < 3 {
                        sample * self.vignetting_gain([x_f32, y_f32])?
                    } else {
                        sample
                    };
                }
            }
        }
        Ok(output)
    }

    /// Executes a single-channel mask using the same geometric transform but
    /// without TCA or colour gain.
    pub fn execute_plane(
        &self,
        input: &[f32],
        stride: usize,
    ) -> Result<Vec<f32>, LensCorrectionExecutionError> {
        self.execute_plane_with_cancel(input, stride, || false)
    }

    /// Executes a single-channel mask with the same inverse mapping and a
    /// cancellation check at every output row.
    pub fn execute_plane_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[f32],
        stride: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, LensCorrectionExecutionError> {
        self.execute_interleaved_with_cancel(input, 1, stride, cancelled)
    }

    fn vignetting_gain(&self, point: [f32; 2]) -> Result<f32, LensCorrectionExecutionError> {
        let Some(calibration) = self.vignetting_calibration() else {
            return Ok(1.0);
        };
        let width = f64::from(self.source_dimensions().width());
        let height = f64::from(self.source_dimensions().height());
        let radius = width.min(height) * 0.5;
        let x = (f64::from(point[0]) - (width - 1.0) * 0.5) / radius;
        let y = (f64::from(point[1]) - (height - 1.0) * 0.5) / radius;
        let radius_squared = x.mul_add(x, y * y);
        let value = 1.0
            + f64::from(calibration.k1) * radius_squared
            + f64::from(calibration.k2) * radius_squared.powi(2)
            + f64::from(calibration.k3) * radius_squared.powi(3);
        if !value.is_finite() || value <= 0.0 {
            return Err(LensCorrectionExecutionError::NonFiniteGain);
        }
        let gain =
            if self.config().parameters().mode == super::parameters::LensCorrectionMode::Correct {
                1.0 / value
            } else {
                value
            };
        gain.is_finite()
            .then_some(gain as f32)
            .ok_or(LensCorrectionExecutionError::NonFiniteGain)
    }
}

fn bilinear_sample(
    input: &[f32],
    stride: usize,
    channels: usize,
    coordinate: [f32; 2],
    channel: usize,
) -> Result<f32, LensCorrectionExecutionError> {
    let width = stride / channels;
    let height = input.len() / stride;
    let width_u32 =
        u32::try_from(width).map_err(|_| LensCorrectionExecutionError::ArithmeticOverflow)?;
    let height_u32 =
        u32::try_from(height).map_err(|_| LensCorrectionExecutionError::ArithmeticOverflow)?;
    let x = f64::from(coordinate[0]).clamp(0.0, f64::from(width_u32.saturating_sub(1)));
    let y = f64::from(coordinate[1]).clamp(0.0, f64::from(height_u32.saturating_sub(1)));
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(width.saturating_sub(1));
    let y1 = (y0 + 1).min(height.saturating_sub(1));
    let fx = x - x.floor();
    let fy = y - y.floor();
    let sample = |column: usize, row: usize| -> Result<f64, LensCorrectionExecutionError> {
        let offset = row
            .checked_mul(stride)
            .and_then(|value| value.checked_add(column.checked_mul(channels)?))
            .and_then(|value| value.checked_add(channel))
            .ok_or(LensCorrectionExecutionError::ArithmeticOverflow)?;
        input.get(offset).copied().map(f64::from).ok_or(
            LensCorrectionExecutionError::InvalidShape {
                minimum: offset + 1,
                actual: input.len(),
            },
        )
    };
    let top = sample(x0, y0)? * (1.0 - fx) + sample(x1, y0)? * fx;
    let bottom = sample(x0, y1)? * (1.0 - fx) + sample(x1, y1)? * fx;
    let value = top * (1.0 - fy) + bottom * fy;
    value
        .is_finite()
        .then_some(value as f32)
        .ok_or(LensCorrectionExecutionError::NonFiniteOutput)
}

fn digest_f32(values: &[f32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(super::LENS_CORRECTION_COMPATIBILITY_ID.as_bytes());
    for value in values {
        hasher.update(value.to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}

#[derive(Debug, Clone, PartialEq)]
pub struct LensCorrectionExecution {
    pixels: Vec<LinearRgb>,
    dimensions: crate::RasterDimensions,
    receipt: LensCorrectionReceipt,
}

impl LensCorrectionExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }

    #[must_use]
    pub const fn dimensions(&self) -> crate::RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn receipt(&self) -> &LensCorrectionReceipt {
        &self.receipt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LensCorrectionReceipt {
    plan_identity: [u8; 32],
    input_digest: [u8; 32],
    output_digest: [u8; 32],
}

impl LensCorrectionReceipt {
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
pub enum LensCorrectionExecutionError {
    InvalidShape { minimum: usize, actual: usize },
    InvalidStride { minimum: usize, actual: usize },
    UnsupportedChannels(usize),
    NonFiniteInput,
    NonFiniteOutput,
    NonFiniteGain,
    Coordinate(LensCorrectionCoordinateError),
    ArithmeticOverflow,
    Cancelled,
}

impl fmt::Display for LensCorrectionExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidShape { minimum, actual } => {
                write!(
                    formatter,
                    "lens correction input has {actual} values; minimum is {minimum}"
                )
            }
            Self::InvalidStride { minimum, actual } => {
                write!(
                    formatter,
                    "lens correction stride {actual}; minimum is {minimum}"
                )
            }
            Self::UnsupportedChannels(channels) => {
                write!(
                    formatter,
                    "lens correction does not support {channels} channels"
                )
            }
            Self::NonFiniteInput => formatter.write_str("lens correction input is non-finite"),
            Self::NonFiniteOutput => formatter.write_str("lens correction output is non-finite"),
            Self::NonFiniteGain => formatter.write_str("lens correction gain is non-finite"),
            Self::Coordinate(error) => error.fmt(formatter),
            Self::ArithmeticOverflow => {
                formatter.write_str("lens correction arithmetic overflowed")
            }
            Self::Cancelled => formatter.write_str("lens correction execution was cancelled"),
        }
    }
}

impl std::error::Error for LensCorrectionExecutionError {}
