//! Direct CPU helpers from `src/develop/noise_generator.h` and diffuse.c masks.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp
)]

use super::{DiffuseDimensions, DiffuseExecutionError, DiffusePixel};

const TWO_PI: f32 = 2.0 * std::f32::consts::PI;

/// Source `splitmix32`, including its unsigned 64-bit wrapping arithmetic.
#[must_use]
pub const fn splitmix32(seed: u64) -> u32 {
    let mut result = (seed ^ (seed >> 33)).wrapping_mul(0x062a_9d9e_d799_705f);
    result = (result ^ (result >> 28)).wrapping_mul(0xcb24_d0a5_c88c_35b3);
    (result >> 32) as u32
}

const fn rotate_left_32(value: u32, amount: u32) -> u32 {
    value.rotate_left(amount)
}

/// Source `xoshiro128plus` output/state transition.
pub fn xoshiro128plus(state: &mut [u32; 4]) -> f32 {
    let result = state[0].wrapping_add(state[3]);
    let t = state[1] << 9;
    state[2] ^= state[0];
    state[3] ^= state[1];
    state[1] ^= state[2];
    state[0] ^= state[3];
    state[2] ^= t;
    state[3] = rotate_left_32(state[3], 11);
    (result >> 8) as f32 * 2_f32.powi(-24)
}

/// Source Box-Muller Gaussian noise helper.
#[must_use]
pub fn gaussian_noise(mu: f32, sigma: f32, flip: bool, state: &mut [u32; 4]) -> f32 {
    let u1 = xoshiro128plus(state).max(f32::MIN_POSITIVE);
    let u2 = xoshiro128plus(state);
    let magnitude = (-2.0 * u1.ln()).sqrt();
    let noise = if flip {
        magnitude * (TWO_PI * u2).cos()
    } else {
        magnitude * (TWO_PI * u2).sin()
    };
    noise * sigma + mu
}

/// Builds the native RGB-only luminance mask (`mask` is one byte per pixel).
pub fn build_mask(
    input: &[DiffusePixel],
    mask: &mut [u8],
    threshold: f32,
    dimensions: DiffuseDimensions,
) -> Result<(), DiffuseExecutionError> {
    let expected = dimensions.pixel_count()?;
    if input.len() != expected || mask.len() != expected {
        return Err(DiffuseExecutionError::DimensionsMismatch {
            expected,
            actual: input.len(),
        });
    }
    for (pixel, value) in input.iter().zip(mask.iter_mut()) {
        let channels = pixel.channels();
        *value =
            u8::from(channels[0] > threshold || channels[1] > threshold || channels[2] > threshold);
    }
    Ok(())
}

/// Initializes masked pixels with the native deterministic Gaussian noise.
///
/// The CPU oracle's coordinates intentionally use the byte offset `k` before
/// dividing by width. This preserves its exact `inpaint_mask` behavior rather
/// than silently substituting the corrected `OpenCL` coordinate calculation.
pub fn inpaint_mask<F: FnMut() -> bool>(
    input: &[DiffusePixel],
    mask: &[u8],
    output: &mut [DiffusePixel],
    dimensions: DiffuseDimensions,
    mut cancelled: F,
) -> Result<(), DiffuseExecutionError> {
    let expected = dimensions.pixel_count()?;
    if input.len() != expected || mask.len() != expected || output.len() != expected {
        return Err(DiffuseExecutionError::DimensionsMismatch {
            expected,
            actual: input.len(),
        });
    }
    let width = dimensions.width();
    let width_u32 = u32::try_from(width).map_err(|_| DiffuseExecutionError::DimensionsTooLarge)?;
    for index in 0..expected {
        if index % width == 0 && cancelled() {
            return Err(DiffuseExecutionError::Cancelled);
        }
        if mask[index] == 0 {
            output[index] = input[index];
            continue;
        }
        // Native code increments k by four and then derives i/j from k.
        let byte_offset = index
            .checked_mul(4)
            .ok_or(DiffuseExecutionError::DimensionsTooLarge)?;
        let byte_offset =
            u32::try_from(byte_offset).map_err(|_| DiffuseExecutionError::DimensionsTooLarge)?;
        let row = byte_offset / width_u32;
        let column = byte_offset - row;
        let mut state = [
            splitmix32(u64::from(column) + 1),
            splitmix32((u64::from(column) + 1).wrapping_mul(u64::from(row) + 3)),
            splitmix32(1337),
            splitmix32(666),
        ];
        for _ in 0..4 {
            xoshiro128plus(&mut state);
        }
        let flip = row % 2 != 0 || column % 2 != 0;
        let original = input[index].channels();
        let mut reconstructed = [0.0; 4];
        for channel in 0..4 {
            reconstructed[channel] =
                gaussian_noise(original[channel], original[channel], flip, &mut state).abs();
        }
        output[index] = DiffusePixel::from_channels(reconstructed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_stream_matches_known_splitmix_and_xoshiro_values() {
        let mut state = [
            splitmix32(1),
            splitmix32(6),
            splitmix32(1337),
            splitmix32(666),
        ];
        assert_eq!(state, [0xa80d_2c65, 0x07e4_0cd6, 0x0c06_227d, 0xfc12_2d99]);
        let first = xoshiro128plus(&mut state);
        assert!(first.is_finite());
        assert!((0.0..1.0).contains(&first));
    }

    #[test]
    fn mask_ignores_alpha() {
        let dimensions = DiffuseDimensions::new(1, 1).expect("dimensions");
        let input = [DiffusePixel::from_channels([0.0, 0.0, 0.0, 100.0])];
        let mut mask = [0];
        build_mask(&input, &mut mask, 0.5, dimensions).expect("mask");
        assert_eq!(mask, [0]);
    }
}
