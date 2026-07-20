use crate::RasterDimensions;
use rusttable_image::Roi;
use std::f64::consts::PI;

use super::codec::RotatePixelsInterpolation;
use super::execution::RotatePixelsExecutionError;
use super::geometry::f64_to_f32;

pub(crate) fn validate_buffer(
    input: &[f32],
    roi: Roi,
    channels: usize,
    stride: usize,
) -> Result<(), RotatePixelsExecutionError> {
    let width =
        usize::try_from(roi.width()).map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)?;
    let height = usize::try_from(roi.height())
        .map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)?;
    let minimum_stride = width
        .checked_mul(channels)
        .ok_or(RotatePixelsExecutionError::ArithmeticOverflow)?;
    if stride < minimum_stride {
        return Err(RotatePixelsExecutionError::InvalidStride {
            minimum: minimum_stride,
            actual: stride,
        });
    }
    let required = stride
        .checked_mul(height)
        .ok_or(RotatePixelsExecutionError::ArithmeticOverflow)?;
    if input.len() < required {
        return Err(RotatePixelsExecutionError::InvalidShape {
            expected: required,
            actual: input.len(),
        });
    }
    if input[..required].iter().any(|value| !value.is_finite()) {
        return Err(RotatePixelsExecutionError::NonFiniteInput);
    }
    Ok(())
}

pub(crate) fn checked_output_index(
    x: u32,
    y: u32,
    width: u32,
    channels: usize,
) -> Result<usize, RotatePixelsExecutionError> {
    let row = usize::try_from(y)
        .map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)?
        .checked_mul(
            usize::try_from(width).map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)?,
        )
        .ok_or(RotatePixelsExecutionError::ArithmeticOverflow)?;
    row.checked_add(usize::try_from(x).map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)?)
        .and_then(|index| index.checked_mul(channels))
        .ok_or(RotatePixelsExecutionError::ArithmeticOverflow)
}

pub(crate) fn pixel_count(
    dimensions: RasterDimensions,
) -> Result<usize, RotatePixelsExecutionError> {
    usize::try_from(dimensions.pixel_count())
        .map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)
}

#[allow(clippy::too_many_lines)]
pub(crate) fn sample_pixel(
    input: &[f32],
    roi: Roi,
    channels: usize,
    stride: usize,
    x: f64,
    y: f64,
    interpolation: RotatePixelsInterpolation,
) -> Result<Vec<f32>, RotatePixelsExecutionError> {
    let width =
        i32::try_from(roi.width()).map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)?;
    let height =
        i32::try_from(roi.height()).map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)?;
    let mut sample = vec![0.0; channels];
    match interpolation {
        RotatePixelsInterpolation::Nearest => {
            let ix = reflect_index(round_to_i32(x)?, width);
            let iy = reflect_index(round_to_i32(y)?, height);
            read_pixel(input, stride, channels, ix, iy, &mut sample)?;
        }
        RotatePixelsInterpolation::Bilinear => {
            let x0 = floor_to_i32(x)?;
            let y0 = floor_to_i32(y)?;
            let fx = x - f64::from(x0);
            let fy = y - f64::from(y0);
            accumulate_tap(
                input,
                stride,
                channels,
                width,
                height,
                x0,
                y0,
                (1.0 - fx) * (1.0 - fy),
                &mut sample,
            )?;
            accumulate_tap(
                input,
                stride,
                channels,
                width,
                height,
                x0 + 1,
                y0,
                fx * (1.0 - fy),
                &mut sample,
            )?;
            accumulate_tap(
                input,
                stride,
                channels,
                width,
                height,
                x0,
                y0 + 1,
                (1.0 - fx) * fy,
                &mut sample,
            )?;
            accumulate_tap(
                input,
                stride,
                channels,
                width,
                height,
                x0 + 1,
                y0 + 1,
                fx * fy,
                &mut sample,
            )?;
        }
        RotatePixelsInterpolation::Bicubic => {
            let x0 = floor_to_i32(x)?;
            let y0 = floor_to_i32(y)?;
            for j in -1..=2 {
                for i in -1..=2 {
                    let weight =
                        cubic_weight(x - f64::from(x0 + i)) * cubic_weight(y - f64::from(y0 + j));
                    accumulate_tap(
                        input,
                        stride,
                        channels,
                        width,
                        height,
                        x0 + i,
                        y0 + j,
                        weight,
                        &mut sample,
                    )?;
                }
            }
        }
        RotatePixelsInterpolation::Lanczos => {
            let x0 = floor_to_i32(x)?;
            let y0 = floor_to_i32(y)?;
            for j in -3..=3 {
                for i in -3..=3 {
                    let weight = lanczos_weight(x - f64::from(x0 + i))
                        * lanczos_weight(y - f64::from(y0 + j));
                    accumulate_tap(
                        input,
                        stride,
                        channels,
                        width,
                        height,
                        x0 + i,
                        y0 + j,
                        weight,
                        &mut sample,
                    )?;
                }
            }
        }
    }
    Ok(sample)
}

#[allow(clippy::too_many_arguments)]
fn accumulate_tap(
    input: &[f32],
    stride: usize,
    channels: usize,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    weight: f64,
    output: &mut [f32],
) -> Result<(), RotatePixelsExecutionError> {
    if weight == 0.0 {
        return Ok(());
    }
    let ix = reflect_index(x, width);
    let iy = reflect_index(y, height);
    let mut pixel = vec![0.0; channels];
    read_pixel(input, stride, channels, ix, iy, &mut pixel)?;
    for (destination, value) in output.iter_mut().zip(pixel) {
        *destination += f64_to_f32(f64::from(value) * weight);
    }
    Ok(())
}

fn read_pixel(
    input: &[f32],
    stride: usize,
    channels: usize,
    x: i32,
    y: i32,
    output: &mut [f32],
) -> Result<(), RotatePixelsExecutionError> {
    let row = usize::try_from(y)
        .map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)?
        .checked_mul(stride)
        .ok_or(RotatePixelsExecutionError::ArithmeticOverflow)?;
    let column = usize::try_from(x)
        .map_err(|_| RotatePixelsExecutionError::ArithmeticOverflow)?
        .checked_mul(channels)
        .ok_or(RotatePixelsExecutionError::ArithmeticOverflow)?;
    let start = row
        .checked_add(column)
        .ok_or(RotatePixelsExecutionError::ArithmeticOverflow)?;
    let end = start
        .checked_add(channels)
        .ok_or(RotatePixelsExecutionError::ArithmeticOverflow)?;
    output.copy_from_slice(input.get(start..end).ok_or(
        RotatePixelsExecutionError::InvalidShape {
            expected: end,
            actual: input.len(),
        },
    )?);
    Ok(())
}

fn reflect_index(value: i32, extent: i32) -> i32 {
    if extent <= 1 {
        return 0;
    }
    let period = 2 * extent - 2;
    let wrapped = value.rem_euclid(period);
    if wrapped >= extent {
        period - wrapped
    } else {
        wrapped
    }
}

fn round_to_i32(value: f64) -> Result<i32, RotatePixelsExecutionError> {
    checked_i32(value.round())
}

fn floor_to_i32(value: f64) -> Result<i32, RotatePixelsExecutionError> {
    checked_i32(value.floor())
}

fn checked_i32(value: f64) -> Result<i32, RotatePixelsExecutionError> {
    if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
        return Err(RotatePixelsExecutionError::NonFiniteCoordinate);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(value as i32)
}

fn cubic_weight(value: f64) -> f64 {
    let x = value.abs();
    if x < 1.0 {
        (1.5 * x - 2.5) * x * x + 1.0
    } else if x < 2.0 {
        (2.5 - 1.5 * x) * x * x - 4.0 * x + 2.0
    } else {
        0.0
    }
}

fn lanczos_weight(value: f64) -> f64 {
    let x = value.abs();
    if x >= 3.0 {
        0.0
    } else if x < 1.0e-12 {
        1.0
    } else {
        let pi_x = PI * x;
        (pi_x.sin() / pi_x) * ((pi_x / 3.0).sin() / (pi_x / 3.0))
    }
}
