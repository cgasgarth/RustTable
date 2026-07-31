//! Direct CPU helpers from `src/common/bspline.h` and `src/common/dwt.h`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::missing_panics_doc
)]

use super::{DiffuseDimensions, DiffuseExecutionError, DiffusePixel};

/// Native B-spline support width.
pub const BSPLINE_FSIZE: usize = 5;
/// Native B-spline equivalent sigma.
pub const B_SPLINE_SIGMA: f32 = 1.055_365_1;
/// Native B-spline filter, in source declaration order.
const BSPLINE_FILTER: [f32; BSPLINE_FSIZE] =
    [1.0 / 16.0, 4.0 / 16.0, 6.0 / 16.0, 4.0 / 16.0, 1.0 / 16.0];

/// Returns the equivalent Gaussian sigma after the native À trous step.
#[must_use]
pub fn equivalent_sigma_at_step(sigma: f32, step: usize) -> f32 {
    if step == 0 {
        sigma
    } else {
        (equivalent_sigma_at_step(sigma, step - 1).powi(2)
            + (2_f32.powi(i32::try_from(step).expect("diffuse scale fits i32")) * sigma).powi(2))
        .sqrt()
    }
}

/// Inverse of [`equivalent_sigma_at_step`], matching the native loop and return value.
#[must_use]
pub fn num_steps_to_reach_equivalent_sigma(sigma_filter: f32, sigma_final: f32) -> usize {
    let mut step = 0_usize;
    let mut radius = sigma_filter;
    while radius < sigma_final {
        step += 1;
        let multiplier = 1_u32
            .checked_shl(u32::try_from(step).expect("diffuse scale fits u32"))
            .unwrap_or(u32::MAX) as f32;
        radius = (radius.powi(2) + (multiplier * sigma_filter).powi(2)).sqrt();
    }
    step + 1
}

/// Native row interleaving used for cache locality in wavelet and PDE passes.
#[must_use]
pub fn dwt_interleave_rows(row_id: usize, height: usize, stride: usize) -> usize {
    if height <= stride {
        return row_id;
    }
    let per_pass = height.div_ceil(stride);
    let long_passes = height % stride;
    if long_passes == 0 || row_id < long_passes * per_pass {
        return row_id / per_pass + stride * (row_id % per_pass);
    }
    let row_id2 = row_id - long_passes * per_pass;
    long_passes + row_id2 / (per_pass - 1) + stride * (row_id2 % (per_pass - 1))
}

/// Decomposes one image into a clipped B-spline low-frequency plane and detail plane.
///
/// The source always clips the low-frequency result before calculating `HF = input - LF`.
pub fn decompose_2d_bspline<F: FnMut() -> bool>(
    input: &[DiffusePixel],
    high_frequency: &mut [DiffusePixel],
    low_frequency: &mut [DiffusePixel],
    dimensions: DiffuseDimensions,
    multiplier: usize,
    mut cancelled: F,
) -> Result<(), DiffuseExecutionError> {
    let width = dimensions.width();
    let height = dimensions.height();
    let expected = dimensions.pixel_count()?;
    if input.len() != expected
        || high_frequency.len() != expected
        || low_frequency.len() != expected
    {
        return Err(DiffuseExecutionError::DimensionsMismatch {
            expected,
            actual: input.len(),
        });
    }
    let mut vertical = Vec::new();
    let row_bytes = width
        .checked_mul(std::mem::size_of::<DiffusePixel>())
        .ok_or(DiffuseExecutionError::DimensionsTooLarge)?;
    vertical
        .try_reserve_exact(width)
        .map_err(|_| DiffuseExecutionError::AllocationFailed {
            required: row_bytes,
        })?;
    vertical.resize(width, [0.0; 4]);
    for row in 0..height {
        if cancelled() {
            return Err(DiffuseExecutionError::Cancelled);
        }
        let image_row = dwt_interleave_rows(row, height, multiplier);
        for column in 0..width {
            let mut value = [0.0; 4];
            for (tap, weight) in BSPLINE_FILTER.into_iter().enumerate() {
                let source_row = clamp_offset(image_row, tap, multiplier, height);
                let pixel = input[source_row * width + column].channels();
                for channel in 0..4 {
                    value[channel] += weight * pixel[channel];
                }
            }
            for channel in 0..4 {
                // `_bspline_vertical_pass` receives `clip_negatives = TRUE`.
                vertical[column][channel] = value[channel].max(0.0);
            }
        }
        for column in 0..width {
            let mut blur = [0.0; 4];
            for (tap, weight) in BSPLINE_FILTER.into_iter().enumerate() {
                let source_column = clamp_offset(column, tap, multiplier, width);
                let pixel = vertical[source_column];
                for channel in 0..4 {
                    blur[channel] += weight * pixel[channel];
                }
            }
            let index = image_row * width + column;
            for channel in 0..4 {
                let clipped = blur[channel].max(0.0);
                low_frequency[index].channels_mut()[channel] = clipped;
                high_frequency[index].channels_mut()[channel] =
                    input[index].channels()[channel] - clipped;
            }
        }
    }
    Ok(())
}

fn clamp_offset(center: usize, tap: usize, multiplier: usize, limit: usize) -> usize {
    let signed_center = isize::try_from(center).expect("raster dimension fits isize");
    let signed_tap = isize::try_from(tap).expect("B-spline tap fits isize") - 2;
    let signed_multiplier = isize::try_from(multiplier).expect("diffuse scale fits isize");
    let coordinate = signed_center + signed_tap * signed_multiplier;
    coordinate.clamp(0, isize::try_from(limit - 1).expect("nonempty raster")) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_interleaving_matches_native_examples() {
        assert_eq!(dwt_interleave_rows(0, 10, 4), 0);
        assert_eq!(dwt_interleave_rows(1, 10, 4), 4);
        assert_eq!(dwt_interleave_rows(2, 10, 4), 8);
        assert_eq!(dwt_interleave_rows(3, 10, 4), 1);
        assert_eq!(dwt_interleave_rows(9, 10, 4), 7);
    }

    #[test]
    fn detail_uses_clipped_low_frequency_plane() {
        let dimensions = DiffuseDimensions::new(1, 1).expect("dimensions");
        let input = vec![DiffusePixel::from_channels([-1.0, 2.0, -3.0, 4.0])];
        let mut high = vec![DiffusePixel::from_channels([0.0; 4])];
        let mut low = vec![DiffusePixel::from_channels([0.0; 4])];
        decompose_2d_bspline(&input, &mut high, &mut low, dimensions, 1, || false)
            .expect("decompose");
        assert_eq!(low[0].channels(), [0.0, 2.0, 0.0, 4.0]);
        assert_eq!(high[0].channels(), [-1.0, 0.0, -3.0, 0.0]);
    }
}
