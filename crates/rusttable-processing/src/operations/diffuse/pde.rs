//! Direct CPU helpers from `src/iop/diffuse.c` and `data/kernels/diffuse.cl`.

#![allow(
    clippy::float_cmp,
    clippy::needless_range_loop,
    clippy::too_many_arguments
)]

use super::wavelets::dwt_interleave_rows;
use super::{DiffuseDimensions, DiffuseExecutionError, DiffusePixel, IsotropyMode};

const HALF: f32 = 0.5;

/// Computes the source's centered finite differences for a 3x3 neighborhood.
fn find_gradients(pixels: &[DiffusePixel; 9]) -> [[f32; 4]; 2] {
    let mut gradients = [[0.0; 4]; 2];
    for channel in 0..4 {
        gradients[0][channel] =
            (pixels[7].channels()[channel] - pixels[1].channels()[channel]) * HALF;
        gradients[1][channel] =
            (pixels[5].channels()[channel] - pixels[3].channels()[channel]) * HALF;
    }
    gradients
}

fn rotation_matrix_isophote(
    c2: [f32; 4],
    cos_theta_sin_theta: [f32; 4],
    cos_theta2: [f32; 4],
    sin_theta2: [f32; 4],
) -> [[[f32; 4]; 2]; 2] {
    let mut matrix = [[[0.0; 4]; 2]; 2];
    for channel in 0..4 {
        matrix[0][0][channel] = cos_theta2[channel] + c2[channel] * sin_theta2[channel];
        matrix[1][1][channel] = c2[channel] * cos_theta2[channel] + sin_theta2[channel];
        matrix[0][1][channel] = (c2[channel] - 1.0) * cos_theta_sin_theta[channel];
        matrix[1][0][channel] = matrix[0][1][channel];
    }
    matrix
}

fn rotation_matrix_gradient(
    c2: [f32; 4],
    cos_theta_sin_theta: [f32; 4],
    cos_theta2: [f32; 4],
    sin_theta2: [f32; 4],
) -> [[[f32; 4]; 2]; 2] {
    let mut matrix = [[[0.0; 4]; 2]; 2];
    for channel in 0..4 {
        matrix[0][0][channel] = c2[channel] * cos_theta2[channel] + sin_theta2[channel];
        matrix[1][1][channel] = cos_theta2[channel] + c2[channel] * sin_theta2[channel];
        matrix[0][1][channel] = (1.0 - c2[channel]) * cos_theta_sin_theta[channel];
        matrix[1][0][channel] = matrix[0][1][channel];
    }
    matrix
}

fn build_matrix(matrix: [[[f32; 4]; 2]; 2]) -> [[f32; 4]; 9] {
    let mut kernel = [[0.0; 4]; 9];
    for channel in 0..4 {
        let b11 = matrix[0][1][channel] * HALF;
        let b13 = -b11;
        let b22 = -2.0 * (matrix[0][0][channel] + matrix[1][1][channel]);
        kernel[0][channel] = b11;
        kernel[1][channel] = matrix[1][1][channel];
        kernel[2][channel] = b13;
        kernel[3][channel] = matrix[0][0][channel];
        kernel[4][channel] = b22;
        kernel[5][channel] = matrix[0][0][channel];
        kernel[6][channel] = b13;
        kernel[7][channel] = matrix[1][1][channel];
        kernel[8][channel] = b11;
    }
    kernel
}

fn isotropic_laplacian() -> [[f32; 4]; 9] {
    [
        [0.25; 4], [0.5; 4], [0.25; 4], [0.5; 4], [-3.0; 4], [0.5; 4], [0.25; 4], [0.5; 4],
        [0.25; 4],
    ]
}

fn compute_kernel(
    c2: [f32; 4],
    cos_theta_sin_theta: [f32; 4],
    cos_theta2: [f32; 4],
    sin_theta2: [f32; 4],
    isotropy: IsotropyMode,
) -> [[f32; 4]; 9] {
    match isotropy {
        IsotropyMode::Isotropic => isotropic_laplacian(),
        IsotropyMode::Isophote => build_matrix(rotation_matrix_isophote(
            c2,
            cos_theta_sin_theta,
            cos_theta2,
            sin_theta2,
        )),
        IsotropyMode::Gradient => build_matrix(rotation_matrix_gradient(
            c2,
            cos_theta_sin_theta,
            cos_theta2,
            sin_theta2,
        )),
    }
}

/// Runs one native heat-PDE update over one wavelet scale.
pub fn heat_pde_diffusion<F: FnMut() -> bool>(
    high_frequency: &[DiffusePixel],
    low_frequency: &[DiffusePixel],
    mask: Option<&[u8]>,
    output: &mut [DiffusePixel],
    dimensions: DiffuseDimensions,
    anisotropy: [f32; 4],
    isotropy: [IsotropyMode; 4],
    regularization: f32,
    variance_threshold: f32,
    current_radius_square: f32,
    multiplier: usize,
    abcd: [f32; 4],
    strength: f32,
    mut cancelled: F,
) -> Result<(), DiffuseExecutionError> {
    let width = dimensions.width();
    let height = dimensions.height();
    let expected = dimensions.pixel_count()?;
    if high_frequency.len() != expected
        || low_frequency.len() != expected
        || output.len() != expected
    {
        return Err(DiffuseExecutionError::DimensionsMismatch {
            expected,
            actual: output.len(),
        });
    }
    if mask.is_some_and(|values| values.len() != expected) {
        return Err(DiffuseExecutionError::DimensionsMismatch {
            expected,
            actual: mask.map_or(0, <[u8]>::len),
        });
    }
    let regularization_factor = regularization * current_radius_square / 9.0;
    for row in 0..height {
        if cancelled() {
            return Err(DiffuseExecutionError::Cancelled);
        }
        let image_row = dwt_interleave_rows(row, height, multiplier);
        let rows = [
            image_row.saturating_sub(multiplier),
            image_row,
            (image_row + multiplier).min(height - 1),
        ];
        for column in 0..width {
            let opacity = mask.is_none_or(|values| values[image_row * width + column] != 0);
            let index = image_row * width + column;
            if !opacity {
                let mut channels = [0.0; 4];
                for channel in 0..4 {
                    channels[channel] = high_frequency[index].channels()[channel]
                        + low_frequency[index].channels()[channel];
                }
                output[index] = DiffusePixel::from_channels(channels);
                continue;
            }

            let columns = [
                column.saturating_sub(multiplier),
                column,
                (column + multiplier).min(width - 1),
            ];
            let mut neighbors_hf = [DiffusePixel::from_channels([0.0; 4]); 9];
            let mut neighbors_lf = [DiffusePixel::from_channels([0.0; 4]); 9];
            for ii in 0..3 {
                for jj in 0..3 {
                    let neighbor = rows[ii] * width + columns[jj];
                    neighbors_hf[3 * ii + jj] = high_frequency[neighbor];
                    neighbors_lf[3 * ii + jj] = low_frequency[neighbor];
                }
            }

            let gradient = find_gradients(&neighbors_lf);
            let laplacian = find_gradients(&neighbors_hf);
            let mut gradient_direction = [[0.0; 4]; 2];
            let mut laplacian_direction = [[0.0; 4]; 2];
            for channel in 0..4 {
                let gradient_magnitude =
                    (gradient[0][channel].powi(2) + gradient[1][channel].powi(2)).sqrt();
                gradient_direction[0][channel] = if gradient_magnitude == 0.0 {
                    1.0
                } else {
                    gradient[0][channel] / gradient_magnitude
                };
                gradient_direction[1][channel] = if gradient_magnitude == 0.0 {
                    0.0
                } else {
                    gradient[1][channel] / gradient_magnitude
                };
                let laplacian_magnitude =
                    (laplacian[0][channel].powi(2) + laplacian[1][channel].powi(2)).sqrt();
                laplacian_direction[0][channel] = if laplacian_magnitude == 0.0 {
                    1.0
                } else {
                    laplacian[0][channel] / laplacian_magnitude
                };
                laplacian_direction[1][channel] = if laplacian_magnitude == 0.0 {
                    0.0
                } else {
                    laplacian[1][channel] / laplacian_magnitude
                };
            }
            // c² is stored per derivative order, with one value for each pixel channel.
            // Keep this separate from the direction arrays so the CPU and OpenCL layouts
            // remain visibly equivalent.
            let mut c2_by_order = [[0.0; 4]; 4];
            for channel in 0..4 {
                let gradient_magnitude =
                    (gradient[0][channel].powi(2) + gradient[1][channel].powi(2)).sqrt();
                let laplacian_magnitude =
                    (laplacian[0][channel].powi(2) + laplacian[1][channel].powi(2)).sqrt();
                c2_by_order[0][channel] = (-gradient_magnitude * anisotropy[0]).exp();
                c2_by_order[1][channel] = (-laplacian_magnitude * anisotropy[1]).exp();
                c2_by_order[2][channel] = (-gradient_magnitude * anisotropy[2]).exp();
                c2_by_order[3][channel] = (-laplacian_magnitude * anisotropy[3]).exp();
            }
            let cos_gradient = gradient_direction[0].map(|value| value * value);
            let sin_gradient = gradient_direction[1].map(|value| value * value);
            let cross_gradient = std::array::from_fn(|channel| {
                gradient_direction[0][channel] * gradient_direction[1][channel]
            });
            let cos_laplacian = laplacian_direction[0].map(|value| value * value);
            let sin_laplacian = laplacian_direction[1].map(|value| value * value);
            let cross_laplacian = std::array::from_fn(|channel| {
                laplacian_direction[0][channel] * laplacian_direction[1][channel]
            });
            let kernels = [
                compute_kernel(
                    c2_by_order[0],
                    cross_gradient,
                    cos_gradient,
                    sin_gradient,
                    isotropy[0],
                ),
                compute_kernel(
                    c2_by_order[1],
                    cross_laplacian,
                    cos_laplacian,
                    sin_laplacian,
                    isotropy[1],
                ),
                compute_kernel(
                    c2_by_order[2],
                    cross_gradient,
                    cos_gradient,
                    sin_gradient,
                    isotropy[2],
                ),
                compute_kernel(
                    c2_by_order[3],
                    cross_laplacian,
                    cos_laplacian,
                    sin_laplacian,
                    isotropy[3],
                ),
            ];

            let mut derivatives = [[0.0; 4]; 4];
            let mut variance = [0.0; 4];
            for k in 0..9 {
                let lf = neighbors_lf[k].channels();
                let hf = neighbors_hf[k].channels();
                for channel in 0..4 {
                    derivatives[0][channel] += kernels[0][k][channel] * lf[channel];
                    derivatives[1][channel] += kernels[1][k][channel] * lf[channel];
                    derivatives[2][channel] += kernels[2][k][channel] * hf[channel];
                    derivatives[3][channel] += kernels[3][k][channel] * hf[channel];
                    variance[channel] += hf[channel] * hf[channel];
                }
            }
            for channel in 0..4 {
                variance[channel] = variance_threshold + variance[channel] * regularization_factor;
            }
            let mut result = [0.0; 4];
            for channel in 0..4 {
                let mut acc = 0.0;
                for derivative in 0..4 {
                    acc += derivatives[derivative][channel] * abcd[derivative];
                }
                let value =
                    high_frequency[index].channels()[channel] * strength + acc / variance[channel];
                result[channel] = (value + low_frequency[index].channels()[channel]).max(0.0);
            }
            output[index] = DiffusePixel::from_channels(result);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isotropic_kernel_is_the_native_rotation_invariant_laplacian() {
        let kernel = compute_kernel(
            [0.4; 4],
            [0.0; 4],
            [1.0; 4],
            [0.0; 4],
            IsotropyMode::Isotropic,
        );
        assert_eq!(kernel[0], [0.25; 4]);
        assert_eq!(kernel[4], [-3.0; 4]);
        assert_eq!(kernel[8], [0.25; 4]);
    }
}
