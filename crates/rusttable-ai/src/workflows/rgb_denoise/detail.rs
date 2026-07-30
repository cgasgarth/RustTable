#![expect(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "image dimensions are checked before the fixed DWT arithmetic and float conversion"
)]

use super::{DETAIL_BAND_MULTIPLIERS, Strength};

const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

pub fn apply_detail_recovery(
    original: &[[f32; 4]],
    denoised: &mut [[f32; 4]],
    width: u32,
    height: u32,
    strength: Strength,
) -> Result<(), DetailError> {
    if original.len() != denoised.len() {
        return Err(DetailError::PixelCountMismatch);
    }
    if strength.get() == 100 {
        return Ok(());
    }
    let width = usize::try_from(width).map_err(|_| DetailError::DimensionOverflow)?;
    let height = usize::try_from(height).map_err(|_| DetailError::DimensionOverflow)?;
    let expected = width
        .checked_mul(height)
        .ok_or(DetailError::DimensionOverflow)?;
    if original.len() != expected {
        return Err(DetailError::PixelCountMismatch);
    }
    let mut residual = Vec::with_capacity(expected);
    for (source, output) in original.iter().zip(denoised.iter()) {
        let source_luma = luma(source);
        let output_luma = luma(output);
        let value = source_luma - output_luma;
        if !value.is_finite() {
            return Err(DetailError::NonFiniteResidual);
        }
        residual.push(value);
    }
    let sigma = standard_deviation(&residual);
    let noise = DETAIL_BAND_MULTIPLIERS.map(|multiplier| sigma * multiplier);
    dwt_denoise(&mut residual, width, height, noise);
    let alpha = f32::from(strength.detail_recovery_strength()) / 100.0;
    for (pixel, detail) in denoised.iter_mut().zip(residual) {
        let delta = alpha * detail;
        pixel[0] += delta;
        pixel[1] += delta;
        pixel[2] += delta;
        if !pixel[..3].iter().all(|value| value.is_finite()) {
            return Err(DetailError::NonFiniteOutput);
        }
    }
    Ok(())
}

fn luma(pixel: &[f32; 4]) -> f32 {
    LUMA[0] * pixel[0] + LUMA[1] * pixel[1] + LUMA[2] * pixel[2]
}

fn standard_deviation(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().copied().map(f64::from).sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| {
            let delta = f64::from(*value) - mean;
            delta * delta
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt() as f32
}

fn dwt_denoise(image: &mut [f32], width: usize, height: usize, noise: [f32; 5]) {
    let mut details = vec![0.0; image.len()];
    let mut intermediate = vec![0.0; image.len()];
    for (level, threshold) in noise.into_iter().enumerate() {
        let scale = (1usize << level).min(width.saturating_sub(1));
        let vertical_scale = (1usize << level).min(height.saturating_sub(1));
        for row in 0..height {
            let above = reflect(row as isize - vertical_scale as isize, height);
            let below = reflect(row as isize + vertical_scale as isize, height);
            for column in 0..width {
                let center = image[row * width + column];
                intermediate[row * width + column] = if vertical_scale == 0 {
                    center
                } else {
                    2.0 * center + image[above * width + column] + image[below * width + column]
                };
            }
        }
        for row in 0..height {
            for column in 0..width {
                let center = intermediate[row * width + column];
                let hat = if scale == 0 {
                    center
                } else {
                    let left = reflect(column as isize - scale as isize, width);
                    let right = reflect(column as isize + scale as isize, width);
                    (2.0 * center
                        + intermediate[row * width + left]
                        + intermediate[row * width + right])
                        / 16.0
                };
                let index = row * width + column;
                let difference = image[index] - hat;
                image[index] = hat;
                details[index] += if difference > threshold {
                    difference - threshold
                } else if difference < -threshold {
                    difference + threshold
                } else {
                    0.0
                };
                if level == 4 {
                    image[index] += details[index];
                }
            }
        }
    }
}

fn reflect(index: isize, length: usize) -> usize {
    if length <= 1 {
        return 0;
    }
    let period = 2 * (length as isize - 1);
    let normalized = index.rem_euclid(period);
    if normalized < length as isize {
        normalized as usize
    } else {
        (period - normalized) as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailError {
    PixelCountMismatch,
    DimensionOverflow,
    NonFiniteResidual,
    NonFiniteOutput,
}

impl std::fmt::Display for DetailError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::PixelCountMismatch => "detail recovery pixel count mismatch",
            Self::DimensionOverflow => "detail recovery dimensions overflow",
            Self::NonFiniteResidual => "detail recovery residual is non-finite",
            Self::NonFiniteOutput => "detail recovery output is non-finite",
        })
    }
}

impl std::error::Error for DetailError {}
