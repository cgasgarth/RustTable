use super::{
    OperationExecutionError, ScalePixelsExecution, ScalePixelsKernel, ScalePixelsMaskError,
    ScalePixelsPlan,
};
use crate::{FiniteF32, LinearRgb};
use rusttable_image::Roi;

impl ScalePixelsPlan {
    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<ScalePixelsExecution, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<ScalePixelsExecution, OperationExecutionError> {
        let expected = pixel_count(self.source_dimensions).ok_or(
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            },
        )?;
        if input.len() != expected {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if self.is_identity() {
            return Ok(ScalePixelsExecution {
                pixels: input.to_vec(),
                dimensions: self.output_dimensions,
                identity: self.identity,
            });
        }
        let output_count = pixel_count(self.output_dimensions).ok_or(
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            },
        )?;
        let source_width = usize::try_from(self.source_dimensions.width()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            }
        })?;
        let mut pixels = Vec::with_capacity(output_count);
        for y in 0..self.output_dimensions.height() {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            for x in 0..self.output_dimensions.width() {
                let source_x = x as f32 * self.x_scale;
                let source_y = y as f32 * self.y_scale;
                let kernel = self.preferences.image();
                let red = sample_linear(input, source_width, source_x, source_y, 0, kernel);
                let green = sample_linear(input, source_width, source_x, source_y, 1, kernel);
                let blue = sample_linear(input, source_width, source_x, source_y, 2, kernel);
                let pixel_index = pixels.len();
                let red = finite_channel(red, pixel_index, crate::RgbChannel::Red)?;
                let green = finite_channel(green, pixel_index, crate::RgbChannel::Green)?;
                let blue = finite_channel(blue, pixel_index, crate::RgbChannel::Blue)?;
                pixels.push(LinearRgb::new(red, green, blue));
            }
        }
        Ok(ScalePixelsExecution {
            pixels,
            dimensions: self.output_dimensions,
            identity: self.identity,
        })
    }

    pub fn execute_mask(
        &self,
        input: &[f32],
        input_roi: Roi,
        output_roi: Roi,
    ) -> Result<Vec<f32>, ScalePixelsMaskError> {
        ensure_mask_roi(self, input_roi, output_roi)?;
        let expected = roi_len(input_roi).ok_or(ScalePixelsMaskError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(ScalePixelsMaskError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if input.iter().any(|value| !value.is_finite()) {
            return Err(ScalePixelsMaskError::NonFiniteInput);
        }
        let output_len = roi_len(output_roi).ok_or(ScalePixelsMaskError::ArithmeticOverflow)?;
        let mut output = Vec::with_capacity(output_len);
        let kernel = self.preferences.warp();
        for y in 0..output_roi.height() {
            for x in 0..output_roi.width() {
                let global_x = (output_roi.x() + x) as f32 * self.x_scale;
                let global_y = (output_roi.y() + y) as f32 * self.y_scale;
                let source_x = global_x - input_roi.x() as f32;
                let source_y = global_y - input_roi.y() as f32;
                output.push(
                    sample_scalar(
                        input,
                        input_roi.width(),
                        input_roi.height(),
                        source_x,
                        source_y,
                        kernel,
                    )
                    .clamp(0.0, 1.0),
                );
            }
        }
        Ok(output)
    }
}

fn ensure_mask_roi(
    plan: &ScalePixelsPlan,
    input: Roi,
    output: Roi,
) -> Result<(), ScalePixelsMaskError> {
    let source = rusttable_image::ImageDimensions::new(
        plan.source_dimensions.width(),
        plan.source_dimensions.height(),
    )
    .map_err(|_| ScalePixelsMaskError::ArithmeticOverflow)?;
    input
        .within(source)
        .map_err(|_| ScalePixelsMaskError::RoiOutsideSource)?;
    let destination = rusttable_image::ImageDimensions::new(
        plan.output_dimensions.width(),
        plan.output_dimensions.height(),
    )
    .map_err(|_| ScalePixelsMaskError::ArithmeticOverflow)?;
    output
        .within(destination)
        .map_err(|_| ScalePixelsMaskError::RoiOutsideOutput)?;
    Ok(())
}

fn pixel_count(dimensions: crate::RasterDimensions) -> Option<usize> {
    usize::try_from(dimensions.pixel_count()).ok()
}

fn roi_len(roi: Roi) -> Option<usize> {
    usize::try_from(roi.width())
        .ok()?
        .checked_mul(usize::try_from(roi.height()).ok()?)
}

fn finite_channel(
    value: f32,
    pixel: usize,
    channel: crate::RgbChannel,
) -> Result<FiniteF32, OperationExecutionError> {
    FiniteF32::new(value).map_err(|_| OperationExecutionError::NonFiniteResult { pixel, channel })
}

fn sample_linear(
    input: &[LinearRgb],
    width: usize,
    x: f32,
    y: f32,
    channel: usize,
    kernel: ScalePixelsKernel,
) -> f32 {
    let height = input.len() / width;
    sample_kernel(width as u32, height as u32, x, y, kernel, |sx, sy| {
        let pixel = input[sy * width + sx];
        match channel {
            0 => pixel.red().get(),
            1 => pixel.green().get(),
            _ => pixel.blue().get(),
        }
    })
}

fn sample_scalar(
    input: &[f32],
    width: u32,
    height: u32,
    x: f32,
    y: f32,
    kernel: ScalePixelsKernel,
) -> f32 {
    sample_kernel(width, height, x, y, kernel, |sx, sy| {
        input[sy * usize::try_from(width).expect("u32 fits usize") + sx]
    })
}

pub(super) fn sample_kernel<F: Fn(usize, usize) -> f32>(
    width: u32,
    height: u32,
    x: f32,
    y: f32,
    kernel: ScalePixelsKernel,
    sample: F,
) -> f32 {
    let sample_reflected =
        |x_index: i32, y_index: i32| sample(reflect(x_index, width), reflect(y_index, height));
    match kernel {
        ScalePixelsKernel::Nearest => {
            sample_reflected((x + 0.5).floor() as i32, (y + 0.5).floor() as i32)
        }
        ScalePixelsKernel::Bilinear => bilinear(x, y, &sample_reflected),
        ScalePixelsKernel::Bicubic => cubic(x, y, &sample_reflected),
        ScalePixelsKernel::Lanczos => lanczos(x, y, 3, &sample_reflected),
    }
}

fn bilinear<F: Fn(i32, i32) -> f32>(x: f32, y: f32, sample: &F) -> f32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let x_weight = x - x.floor();
    let y_weight = y - y.floor();
    let top = sample(x0, y0) * (1.0 - x_weight) + sample(x0 + 1, y0) * x_weight;
    let bottom = sample(x0, y0 + 1) * (1.0 - x_weight) + sample(x0 + 1, y0 + 1) * x_weight;
    top * (1.0 - y_weight) + bottom * y_weight
}

fn cubic<F: Fn(i32, i32) -> f32>(x: f32, y: f32, sample: &F) -> f32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let mut sum = 0.0;
    let mut weight_sum = 0.0;
    for y_offset in -1..=2 {
        for x_offset in -1..=2 {
            let weight =
                cubic_weight(x - (x0 + x_offset) as f32) * cubic_weight(y - (y0 + y_offset) as f32);
            sum += sample(x0 + x_offset, y0 + y_offset) * weight;
            weight_sum += weight;
        }
    }
    if weight_sum == 0.0 {
        0.0
    } else {
        sum / weight_sum
    }
}

fn lanczos<F: Fn(i32, i32) -> f32>(x: f32, y: f32, support: i32, sample: &F) -> f32 {
    let x0 = x.floor() as i32;
    let y0 = y.floor() as i32;
    let mut sum = 0.0;
    let mut weight_sum = 0.0;
    for y_offset in (1 - support)..=support {
        for x_offset in (1 - support)..=support {
            let weight = lanczos_weight(x - (x0 + x_offset) as f32, support)
                * lanczos_weight(y - (y0 + y_offset) as f32, support);
            sum += sample(x0 + x_offset, y0 + y_offset) * weight;
            weight_sum += weight;
        }
    }
    if weight_sum == 0.0 {
        0.0
    } else {
        sum / weight_sum
    }
}

fn cubic_weight(value: f32) -> f32 {
    let value = value.abs();
    if value <= 1.0 {
        1.5 * value * value * value - 2.5 * value * value + 1.0
    } else if value < 2.0 {
        -0.5 * value * value * value + 2.5 * value * value - 4.0 * value + 2.0
    } else {
        0.0
    }
}

fn lanczos_weight(value: f32, support: i32) -> f32 {
    let value = value.abs();
    if value == 0.0 {
        return 1.0;
    }
    if value >= support as f32 {
        return 0.0;
    }
    let pi_value = std::f32::consts::PI * value;
    (pi_value.sin() / pi_value) * ((pi_value / support as f32).sin() / (pi_value / support as f32))
}

fn reflect(index: i32, length: u32) -> usize {
    if length == 1 {
        return 0;
    }
    let period = i32::try_from(length.saturating_mul(2)).unwrap_or(i32::MAX);
    let mut value = index % period;
    if value < 0 {
        value += period;
    }
    let reflected = if value >= i32::try_from(length).unwrap_or(i32::MAX) {
        period - 1 - value
    } else {
        value
    };
    usize::try_from(reflected).expect("reflected index is nonnegative")
}

/// Reflected scalar image resampling WGSL. Host bindings provide exact scales.
pub const WGSL_IMAGE_RESAMPLER: &str = r"
fn reflect_index(index: i32, length: i32) -> i32 {
  if (length <= 1) { return 0; }
  let period = 2 * length;
  var value = index % period;
  if (value < 0) { value += period; }
  return select(value, period - 1 - value, value >= length);
}
";

/// Reflected scalar mask resampling WGSL with the required mask clamp.
pub const WGSL_MASK_RESAMPLER: &str = r"
fn clamp_mask(value: f32) -> f32 { return clamp(value, 0.0, 1.0); }
";
