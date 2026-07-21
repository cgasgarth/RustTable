#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_midpoint,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    reason = "the operation's f32 image boundary and explicit color branches are compatibility contracts"
)]

use crate::{FiniteF32, LinearRgb, RasterDimensions, RgbChannel};

use super::noise::grain_noise;
use super::parameters::{GrainChannel, GrainConfig};
use crate::operations::common::{OperationExecutionError, validate_shape};

const LUT_SIZE: usize = 128;
const LIGHTNESS_STRENGTH_SCALE: f32 = 0.15;
const COLOR_STRENGTH_SCALE: f32 = 0.25;

#[derive(Debug, Clone, PartialEq)]
pub struct GrainPlan {
    config: GrainConfig,
    dimensions: RasterDimensions,
    grain_lut: Vec<f32>,
    zoom: f32,
}

/// Immutable scalar data consumed by the concrete WGPU grain dispatcher.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GrainGpuParameters {
    pub channel: GrainChannel,
    pub seed: u64,
    pub zoom: f32,
    pub strength: f32,
}

impl GrainPlan {
    pub fn new(
        config: GrainConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        let shortest = dimensions.width().min(dimensions.height()) as f32;
        let zoom = (1.0 + 8.0 * config.scale().get() / 100.0) / 800.0;
        if !zoom.is_finite() || zoom <= 0.0 || shortest <= 0.0 {
            return Err(OperationExecutionError::UnsupportedCapability(
                "grain scale is not representable",
            ));
        }
        Ok(Self {
            grain_lut: evaluate_grain_lut(config.midtones_bias().get()),
            config,
            dimensions,
            zoom: zoom * shortest,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn config(&self) -> GrainConfig {
        self.config
    }

    #[must_use]
    pub const fn gpu_parameters(&self) -> GrainGpuParameters {
        GrainGpuParameters {
            channel: self.config.channel(),
            seed: self.config.seed(),
            zoom: self.zoom,
            strength: self.config.strength().get(),
        }
    }

    #[must_use]
    pub fn gpu_lut(&self) -> &[f32] {
        &self.grain_lut
    }

    pub fn execute(&self, input: &[LinearRgb]) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        validate_shape(self.dimensions, input)?;
        self.execute_window_with_cancel(input, 0, || false)
    }

    pub fn execute_window(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        self.execute_window_with_cancel(input, pixel_index_offset, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        validate_shape(self.dimensions, input)?;
        self.execute_window_with_cancel(input, 0, cancelled)
    }

    /// Runs the reflected point-kernel contract using the same immutable plan.
    pub fn execute_wgpu<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        self.execute_with_cancel(input, cancelled)
    }

    pub fn execute_window_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
        cancelled: F,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        let total = usize::try_from(self.dimensions.pixel_count()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            }
        })?;
        let end = pixel_index_offset.checked_add(input.len()).ok_or(
            OperationExecutionError::DimensionsMismatch {
                expected: total,
                actual: input.len(),
            },
        )?;
        if end > total {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected: total,
                actual: end,
            });
        }
        if self.config.strength().get().to_bits() == 0.0_f32.to_bits() {
            return Ok(input.to_vec());
        }
        let width = usize::try_from(self.dimensions.width()).expect("width fits usize");
        let mut output = Vec::with_capacity(input.len());
        for (local_index, pixel) in input.iter().copied().enumerate() {
            let absolute = pixel_index_offset + local_index;
            if absolute.is_multiple_of(width) && cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            output.push(self.transform(pixel, absolute, width)?);
        }
        Ok(output)
    }

    fn transform(
        &self,
        pixel: LinearRgb,
        absolute_index: usize,
        width: usize,
    ) -> Result<LinearRgb, OperationExecutionError> {
        let x = (absolute_index % width) as f32 + 0.5;
        let y = (absolute_index / width) as f32 + 0.5;
        let noise_channel = match self.config.channel() {
            GrainChannel::Rgb => 3,
            GrainChannel::Hue => 0,
            GrainChannel::Saturation => 1,
            GrainChannel::Lightness => 2,
        };
        let noise = |channel| grain_noise(self.config.seed(), x, y, self.zoom, channel);
        let luminance =
            0.2126 * pixel.red().get() + 0.7152 * pixel.green().get() + 0.0722 * pixel.blue().get();
        let luminance = luminance.clamp(0.0, 1.0);
        let response = |sample, scale| {
            self.lookup(sample * self.config.strength().get() * scale, luminance) / 100.0
        };
        let output = match self.config.channel() {
            GrainChannel::Lightness => {
                let delta = response(noise(noise_channel), LIGHTNESS_STRENGTH_SCALE);
                [
                    pixel.red().get() + delta,
                    pixel.green().get() + delta,
                    pixel.blue().get() + delta,
                ]
            }
            GrainChannel::Rgb => [
                pixel.red().get() + response(noise(0), COLOR_STRENGTH_SCALE),
                pixel.green().get() + response(noise(1), COLOR_STRENGTH_SCALE),
                pixel.blue().get() + response(noise(2), COLOR_STRENGTH_SCALE),
            ],
            GrainChannel::Hue | GrainChannel::Saturation => {
                let (mut hue, mut saturation, lightness) = rgb_to_hsl(pixel);
                let delta = response(noise(noise_channel), COLOR_STRENGTH_SCALE) * 0.01;
                if matches!(self.config.channel(), GrainChannel::Hue) {
                    hue = (hue + delta).rem_euclid(1.0);
                } else {
                    saturation += delta;
                }
                hsl_to_rgb(hue, saturation, lightness)
            }
        };
        Ok(LinearRgb::new(
            finite(output[0], absolute_index, RgbChannel::Red)?,
            finite(output[1], absolute_index, RgbChannel::Green)?,
            finite(output[2], absolute_index, RgbChannel::Blue)?,
        ))
    }

    fn lookup(&self, noise: f32, luminance: f32) -> f32 {
        let x = ((noise + 0.5) * (LUT_SIZE - 1) as f32).clamp(0.0, (LUT_SIZE - 1) as f32);
        let y = (luminance * (LUT_SIZE - 1) as f32).clamp(0.0, (LUT_SIZE - 1) as f32);
        let x0 = (x as usize).min(LUT_SIZE - 2);
        let y0 = (y as usize).min(LUT_SIZE - 2);
        let x1 = x0 + 1;
        let y1 = y0 + 1;
        let xd = x - x0 as f32;
        let yd = y - y0 as f32;
        let at = |xx, yy| self.grain_lut[yy * LUT_SIZE + xx];
        let low = at(x0, y0) * (1.0 - xd) + at(x1, y0) * xd;
        let high = at(x0, y1) * (1.0 - xd) + at(x1, y1) * xd;
        low * (1.0 - yd) + high * yd
    }
}

fn evaluate_grain_lut(midtones_bias: f32) -> Vec<f32> {
    let mut lut = vec![0.0; LUT_SIZE * LUT_SIZE];
    for y in 0..LUT_SIZE {
        let luminance = y as f32 / (LUT_SIZE - 1) as f32;
        let inverse = paper_resp_inverse(luminance, midtones_bias);
        for x in 0..LUT_SIZE {
            let grain = x as f32 / (LUT_SIZE - 1) as f32 - 0.5;
            lut[y * LUT_SIZE + x] =
                100.0 * (paper_response(grain + inverse, midtones_bias) - luminance);
        }
    }
    lut
}

fn paper_response(exposure: f32, midtones_bias: f32) -> f32 {
    let delta = 2.0 * (midtones_bias / 100.0 * 0.0001_f32.ln()).exp();
    (1.0 + 2.0 * delta) / (1.0 + (4.0 * (0.5 - exposure) / (1.0 + 2.0 * delta)).exp()) - delta
}

fn paper_resp_inverse(luminance: f32, midtones_bias: f32) -> f32 {
    let delta = 2.0 * (midtones_bias / 100.0 * 0.0001_f32.ln()).exp();
    let ratio = ((1.0 + 2.0 * delta) / (luminance + delta) - 1.0).max(0.000_001);
    -ratio.ln() * (1.0 + 2.0 * delta) / 4.0 + 0.5
}

fn rgb_to_hsl(pixel: LinearRgb) -> (f32, f32, f32) {
    let [red, green, blue] = [pixel.red().get(), pixel.green().get(), pixel.blue().get()];
    let maximum = red.max(green).max(blue);
    let minimum = red.min(green).min(blue);
    let lightness = (maximum + minimum) * 0.5;
    let delta = maximum - minimum;
    if delta.to_bits() == 0.0_f32.to_bits() {
        return (0.0, 0.0, lightness);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs()).max(0.000_001);
    let hue = if maximum.to_bits() == red.to_bits() {
        ((green - blue) / delta).rem_euclid(6.0) / 6.0
    } else if maximum.to_bits() == green.to_bits() {
        ((blue - red) / delta + 2.0) / 6.0
    } else {
        ((red - green) / delta + 4.0) / 6.0
    };
    (hue, saturation, lightness)
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [f32; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let x = chroma * (1.0 - ((hue * 6.0).rem_euclid(2.0) - 1.0).abs());
    let m = lightness - chroma * 0.5;
    let rgb = match (hue * 6.0).floor() as i32 {
        0 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        _ => [chroma, 0.0, x],
    };
    rgb.map(|value| value + m)
}

fn finite(
    value: f32,
    pixel: usize,
    channel: RgbChannel,
) -> Result<FiniteF32, OperationExecutionError> {
    FiniteF32::new(value).map_err(|_| OperationExecutionError::NonFiniteResult { pixel, channel })
}

#[must_use]
pub const fn wgpu_passes() -> [&'static str; 1] {
    ["grain.point"]
}
