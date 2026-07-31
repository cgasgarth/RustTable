//! Tone Equalizer radial basis coefficients and source-shaped Cholesky solve.

#![forbid(unsafe_code)]
#![allow(
    clippy::large_stack_arrays,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    reason = "the f32 equations retain the native source's matrix layout"
)]

use super::parameters::{
    CHANNELS, CONTRAST_FULCRUM, LUT_ENTRIES, LUT_RESOLUTION, MAX_EV, MIN_EV, PIXEL_CHANNELS,
};

pub const CENTERS_OPS: [f32; PIXEL_CHANNELS] = [
    -8.0,
    -48.0 / 7.0,
    -40.0 / 7.0,
    -32.0 / 7.0,
    -24.0 / 7.0,
    -16.0 / 7.0,
    -8.0 / 7.0,
    0.0,
];
pub const CENTERS_PARAMS: [f32; CHANNELS] = [-8.0, -7.0, -6.0, -5.0, -4.0, -3.0, -2.0, -1.0, 0.0];

#[must_use]
pub fn gaussian_denom(sigma: f32) -> f32 {
    2.0 * sigma * sigma
}

#[must_use]
pub fn gaussian_func(radius: f32, denominator: f32) -> f32 {
    (-radius * radius / denominator).exp()
}

#[must_use]
pub fn pixel_correction(exposure: f32, factors: &[f32; PIXEL_CHANNELS], sigma: f32) -> f32 {
    let denominator = gaussian_denom(sigma);
    let exposure = exposure.clamp(MIN_EV, MAX_EV);
    let mut result = 0.0;
    for index in 0..PIXEL_CHANNELS {
        result += gaussian_func(exposure - CENTERS_OPS[index], denominator) * factors[index];
    }
    result.clamp(0.25, 4.0)
}

#[must_use]
pub fn build_interpolation_matrix(sigma: f32) -> [f32; CHANNELS * PIXEL_CHANNELS] {
    let denominator = gaussian_denom(sigma);
    let mut matrix = [0.0; CHANNELS * PIXEL_CHANNELS];
    for row in 0..CHANNELS {
        for column in 0..PIXEL_CHANNELS {
            matrix[row * PIXEL_CHANNELS + column] =
                gaussian_func(CENTERS_PARAMS[row] - CENTERS_OPS[column], denominator);
        }
    }
    matrix
}

/// Solves the native over-constrained `9 x 8` system `A x = y` by forming
/// `A' A`, then using the same lower-triangular Cholesky passes as
/// `src/iop/choleski.h`.
#[must_use]
pub fn pseudo_solve(
    matrix: &[f32; CHANNELS * PIXEL_CHANNELS],
    y: &[f32; CHANNELS],
) -> Option<[f32; PIXEL_CHANNELS]> {
    let mut square = [[0.0_f32; PIXEL_CHANNELS]; PIXEL_CHANNELS];
    let mut square_y = [0.0_f32; PIXEL_CHANNELS];
    for i in 0..PIXEL_CHANNELS {
        for j in 0..=i {
            let mut sum = 0.0;
            for row in 0..CHANNELS {
                sum += matrix[row * PIXEL_CHANNELS + i] * matrix[row * PIXEL_CHANNELS + j];
            }
            square[i][j] = sum;
        }
        let mut sum = 0.0;
        for row in 0..CHANNELS {
            sum += matrix[row * PIXEL_CHANNELS + i] * y[row];
        }
        square_y[i] = sum;
    }

    let mut lower = [[0.0_f32; PIXEL_CHANNELS]; PIXEL_CHANNELS];
    if square[0][0] <= 0.0 {
        return None;
    }
    for i in 0..PIXEL_CHANNELS {
        for j in 0..=i {
            let mut sum = 0.0;
            for k in 0..j {
                sum += lower[i][k] * lower[j][k];
            }
            if i == j {
                let value = square[i][i] - sum;
                if value < 0.0 {
                    return None;
                }
                lower[i][j] = value.sqrt();
            } else {
                let diagonal = lower[j][j];
                if diagonal == 0.0 {
                    return None;
                }
                lower[i][j] = (square[i][j] - sum) / diagonal;
            }
        }
    }

    let mut descended = [0.0_f32; PIXEL_CHANNELS];
    for i in 0..PIXEL_CHANNELS {
        let mut sum = square_y[i];
        for j in 0..i {
            sum -= lower[i][j] * descended[j];
        }
        let diagonal = lower[i][i];
        if diagonal == 0.0 {
            return None;
        }
        descended[i] = sum / diagonal;
    }

    let mut result = [0.0_f32; PIXEL_CHANNELS];
    for i in (0..PIXEL_CHANNELS).rev() {
        let mut sum = descended[i];
        for j in ((i + 1)..PIXEL_CHANNELS).rev() {
            sum -= lower[j][i] * result[j];
        }
        let diagonal = lower[i][i];
        if diagonal == 0.0 {
            return None;
        }
        result[i] = sum / diagonal;
    }
    Some(result)
}

#[must_use]
pub fn user_gains(parameters: &super::parameters::ToneEqualizerParametersV2) -> [f32; CHANNELS] {
    parameters.exposures().map(f32::exp2)
}

#[must_use]
pub fn compute_factors(
    parameters: &super::parameters::ToneEqualizerParametersV2,
) -> Option<[f32; PIXEL_CHANNELS]> {
    let matrix = build_interpolation_matrix(parameters.smoothing);
    let gains = user_gains(parameters);
    pseudo_solve(&matrix, &gains)
}

#[must_use]
pub fn compute_channel_gains(factors: &[f32; PIXEL_CHANNELS], sigma: f32) -> [f32; CHANNELS] {
    CENTERS_PARAMS.map(|center| pixel_correction(center, factors, sigma))
}

#[must_use]
pub fn compute_correction_lut(
    factors: &[f32; PIXEL_CHANNELS],
    sigma: f32,
) -> Box<[f32; LUT_ENTRIES]> {
    let denominator = gaussian_denom(sigma);
    let mut lut = Box::new([0.0_f32; LUT_ENTRIES]);
    for (index, value) in lut.iter_mut().enumerate() {
        let exposure = index as f32 / LUT_RESOLUTION as f32 + MIN_EV;
        let mut result = 0.0;
        for channel in 0..PIXEL_CHANNELS {
            result +=
                gaussian_func(exposure - CENTERS_OPS[channel], denominator) * factors[channel];
        }
        *value = result.clamp(0.25, 4.0);
    }
    lut
}

#[must_use]
pub fn luminance_linear_contrast(pixel: f32, fulcrum: f32, contrast: f32) -> f32 {
    let value = (pixel - fulcrum) * contrast + fulcrum;
    if value < super::parameters::MIN_FLOAT {
        super::parameters::MIN_FLOAT
    } else {
        value
    }
}

#[must_use]
pub fn transform_luminance(pixel: f32, exposure_boost: f32, fulcrum: f32, contrast: f32) -> f32 {
    luminance_linear_contrast(pixel * exposure_boost, fulcrum, contrast)
}

#[must_use]
pub fn default_contrast_fulcrum() -> f32 {
    CONTRAST_FULCRUM
}
