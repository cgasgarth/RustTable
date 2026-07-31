//! The operation-local guided and exposure-independent guided filters.
//!
//! These are direct Rust translations of `fast_guided_filter.h`, `eigf.h`,
//! `box_filters.cc`, and the order-zero path in `gaussian.c`. The existing
//! generic Rust blur is deliberately not used: its sampling, boundary,
//! quantization, and coefficient equations are different.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::if_not_else,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    reason = "source-shaped f32 image loops preserve native equations"
)]

use super::parameters::MIN_FLOAT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FilterBlend {
    Linear,
    Geomean,
}

pub(crate) fn alloc_f32(length: usize) -> Result<Vec<f32>, FilterError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| FilterError::AllocationFailed { elements: length })?;
    values.resize(length, 0.0);
    Ok(values)
}

pub(crate) fn interpolate_bilinear(
    input: &[f32],
    width_in: usize,
    height_in: usize,
    output: &mut [f32],
    width_out: usize,
    height_out: usize,
    channels: usize,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    if width_in == 0 || height_in == 0 || width_out == 0 || height_out == 0 || channels == 0 {
        return Err(FilterError::InvalidDimensions);
    }
    if input.len() != width_in * height_in * channels
        || output.len() != width_out * height_out * channels
    {
        return Err(FilterError::InvalidDimensions);
    }
    for row in 0..height_out {
        if cancelled() {
            return Err(FilterError::Cancelled);
        }
        for column in 0..width_out {
            let x_out = column as f32 / width_out as f32;
            let y_out = row as f32 / height_out as f32;
            let x_in = x_out * width_in as f32;
            let y_in = y_out * height_in as f32;
            let x_prev = (x_in.floor() as usize).min(width_in - 1);
            let x_next = x_prev.saturating_add(1).min(width_in - 1);
            let y_prev = (y_in.floor() as usize).min(height_in - 1);
            let y_next = y_prev.saturating_add(1).min(height_in - 1);
            let dy_next = y_next as f32 - y_in;
            let dy_prev = 1.0 - dy_next;
            let dx_next = x_next as f32 - x_in;
            let dx_prev = 1.0 - dx_next;
            let nw = (y_prev * width_in + x_prev) * channels;
            let ne = (y_prev * width_in + x_next) * channels;
            let se = (y_next * width_in + x_next) * channels;
            let sw = (y_next * width_in + x_prev) * channels;
            let destination = (row * width_out + column) * channels;
            for channel in 0..channels {
                output[destination + channel] = dy_prev
                    * (input[sw + channel] * dx_next + input[se + channel] * dx_prev)
                    + dy_next * (input[nw + channel] * dx_next + input[ne + channel] * dx_prev);
            }
        }
    }
    Ok(())
}

pub(crate) fn box_mean(
    buffer: &mut [f32],
    width: usize,
    height: usize,
    channels: usize,
    radius: usize,
    iterations: usize,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    if width == 0 || height == 0 || channels == 0 || buffer.len() != width * height * channels {
        return Err(FilterError::InvalidDimensions);
    }
    for _ in 0..iterations {
        for row in 0..height {
            if cancelled() {
                return Err(FilterError::Cancelled);
            }
            let source = buffer[row * width * channels..(row + 1) * width * channels].to_vec();
            for column in 0..width {
                let start = column.saturating_sub(radius);
                let end = column.saturating_add(radius).min(width - 1);
                let count = (end - start + 1) as f32;
                for channel in 0..channels {
                    let mut sum = 0.0;
                    for source_column in start..=end {
                        sum += source[source_column * channels + channel];
                    }
                    buffer[(row * width + column) * channels + channel] = sum / count;
                }
            }
        }
        for column in 0..width {
            if cancelled() {
                return Err(FilterError::Cancelled);
            }
            let mut source = Vec::with_capacity(height * channels);
            for row in 0..height {
                source.extend_from_slice(
                    &buffer
                        [(row * width + column) * channels..(row * width + column + 1) * channels],
                );
            }
            for row in 0..height {
                let start = row.saturating_sub(radius);
                let end = row.saturating_add(radius).min(height - 1);
                let count = (end - start + 1) as f32;
                for channel in 0..channels {
                    let mut sum = 0.0;
                    for source_row in start..=end {
                        sum += source[source_row * channels + channel];
                    }
                    buffer[(row * width + column) * channels + channel] = sum / count;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn quantize(
    input: &[f32],
    output: &mut [f32],
    sampling: f32,
    clip_min: f32,
    clip_max: f32,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    if input.len() != output.len() {
        return Err(FilterError::InvalidDimensions);
    }
    if sampling == 0.0 {
        output.copy_from_slice(input);
        return Ok(());
    }
    for (index, value) in input.iter().copied().enumerate() {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        let level = (value.log2() / sampling).floor() * sampling;
        output[index] = (2.0_f32.powf(level)).clamp(clip_min, clip_max);
    }
    Ok(())
}

fn variance_analyse(
    guide: &[f32],
    mask: &[f32],
    ab: &mut [f32],
    width: usize,
    height: usize,
    radius: usize,
    feathering: f32,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    let elements = width
        .checked_mul(height)
        .ok_or(FilterError::InvalidDimensions)?;
    if guide.len() != elements || mask.len() != elements || ab.len() != elements * 2 {
        return Err(FilterError::InvalidDimensions);
    }
    let mut input = alloc_f32(elements * 4)?;
    for index in 0..elements {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        let guide_value = guide[index];
        let mask_value = mask[index];
        input[index * 4] = guide_value;
        input[index * 4 + 1] = mask_value;
        input[index * 4 + 2] = guide_value * guide_value;
        input[index * 4 + 3] = guide_value * mask_value;
    }
    box_mean(&mut input, width, height, 4, radius, 1, cancelled)?;
    for index in 0..elements {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        let denominator =
            (input[index * 4 + 2] - input[index * 4] * input[index * 4] + feathering).max(1.0e-15);
        let a = (input[index * 4 + 3] - input[index * 4] * input[index * 4 + 1]) / denominator;
        let b = input[index * 4 + 1] - a * input[index * 4];
        ab[index * 2] = a;
        ab[index * 2 + 1] = b;
    }
    Ok(())
}

fn apply_linear(
    image: &mut [f32],
    ab: &[f32],
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    for (index, value) in image.iter_mut().enumerate() {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        *value = (*value * ab[index * 2] + ab[index * 2 + 1]).max(MIN_FLOAT);
    }
    Ok(())
}

fn apply_geomean(
    image: &mut [f32],
    ab: &[f32],
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    for (index, value) in image.iter_mut().enumerate() {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        *value = (*value * (*value * ab[index * 2] + ab[index * 2 + 1]).max(MIN_FLOAT)).sqrt();
    }
    Ok(())
}

/// Source `fast_surface_blur`: downsample by four, use box-guided variance,
/// average the `a,b` model, then bilinearly upsample the model.
pub(crate) fn guided_surface_blur(
    image: &mut [f32],
    width: usize,
    height: usize,
    radius: usize,
    feathering: f32,
    iterations: usize,
    blend: FilterBlend,
    quantization: f32,
    quantize_min: f32,
    quantize_max: f32,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    let ds_width = width / 4;
    let ds_height = height / 4;
    if ds_width == 0 || ds_height == 0 {
        return Err(FilterError::InvalidDimensions);
    }
    let ds_elements = ds_width * ds_height;
    let elements = width
        .checked_mul(height)
        .ok_or(FilterError::InvalidDimensions)?;
    if image.len() != elements {
        return Err(FilterError::InvalidDimensions);
    }
    let mut ds_image = alloc_f32(ds_elements)?;
    let mut ds_mask = alloc_f32(ds_elements)?;
    let mut ds_ab = alloc_f32(ds_elements * 2)?;
    let mut ab = alloc_f32(elements * 2)?;
    interpolate_bilinear(
        image,
        width,
        height,
        &mut ds_image,
        ds_width,
        ds_height,
        1,
        cancelled,
    )?;
    let ds_radius = if radius < 4 { 1 } else { radius / 4 };
    for iteration in 0..iterations {
        quantize(
            &ds_image,
            &mut ds_mask,
            quantization,
            quantize_min,
            quantize_max,
            cancelled,
        )?;
        variance_analyse(
            &ds_mask, &ds_image, &mut ds_ab, ds_width, ds_height, ds_radius, feathering, cancelled,
        )?;
        box_mean(&mut ds_ab, ds_width, ds_height, 2, ds_radius, 1, cancelled)?;
        if iteration + 1 != iterations {
            apply_linear(&mut ds_image, &ds_ab, cancelled)?;
        }
    }
    interpolate_bilinear(
        &ds_ab, ds_width, ds_height, &mut ab, width, height, 2, cancelled,
    )?;
    match blend {
        FilterBlend::Linear => apply_linear(image, &ab, cancelled),
        FilterBlend::Geomean => apply_geomean(image, &ab, cancelled),
    }
}

fn gaussian_params(sigma: f32) -> (f32, f32, f32, f32, f32, f32, f32, f32) {
    let alpha = 1.695 / sigma;
    let ema = (-alpha).exp();
    let ema2 = (-2.0 * alpha).exp();
    let b1 = -2.0 * ema;
    let b2 = ema2;
    let k = (1.0 - ema) * (1.0 - ema) / (1.0 + 2.0 * alpha * ema - ema2);
    let a0 = k;
    let a1 = k * (alpha - 1.0) * ema;
    let a2 = k * (alpha + 1.0) * ema;
    let a3 = -k * ema2;
    let coefp = (a0 + a1) / (1.0 + b1 + b2);
    let coefn = (a2 + a3) / (1.0 + b1 + b2);
    (a0, a1, a2, a3, b1, b2, coefp, coefn)
}

fn gaussian_blur(
    input: &[f32],
    output: &mut [f32],
    width: usize,
    height: usize,
    channels: usize,
    sigma: f32,
    mins: &[f32],
    maxs: &[f32],
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    let elements = width
        .checked_mul(height)
        .ok_or(FilterError::InvalidDimensions)?;
    if input.len() != elements * channels
        || output.len() != elements * channels
        || mins.len() != channels
        || maxs.len() != channels
    {
        return Err(FilterError::InvalidDimensions);
    }
    let (a0, a1, a2, a3, b1, b2, coefp, coefn) = gaussian_params(sigma);
    let mut temporary = alloc_f32(input.len())?;
    for column in 0..width {
        if cancelled() {
            return Err(FilterError::Cancelled);
        }
        let mut xp = vec![0.0; channels];
        let mut yb = vec![0.0; channels];
        let mut yp = vec![0.0; channels];
        for channel in 0..channels {
            xp[channel] = input[column * channels + channel].clamp(mins[channel], maxs[channel]);
            yb[channel] = xp[channel] * coefp;
            yp[channel] = yb[channel];
        }
        let mut xc = vec![0.0; channels];
        let mut xn = vec![0.0; channels];
        let mut xa = vec![0.0; channels];
        let mut yn = vec![0.0; channels];
        let mut ya = vec![0.0; channels];
        for row in 0..height {
            let offset = (row * width + column) * channels;
            for channel in 0..channels {
                xc[channel] = input[offset + channel].clamp(mins[channel], maxs[channel]);
                let yc = a0 * xc[channel] + a1 * xp[channel] - b1 * yp[channel] - b2 * yb[channel];
                temporary[offset + channel] = yc;
                xp[channel] = xc[channel];
                yb[channel] = yp[channel];
                yp[channel] = yc;
            }
        }
        for channel in 0..channels {
            xn[channel] = input[((height - 1) * width + column) * channels + channel]
                .clamp(mins[channel], maxs[channel]);
            xa[channel] = xn[channel];
            yn[channel] = xn[channel] * coefn;
            ya[channel] = yn[channel];
        }
        for row in (0..height).rev() {
            let offset = (row * width + column) * channels;
            for channel in 0..channels {
                xc[channel] = input[offset + channel].clamp(mins[channel], maxs[channel]);
                let yc = a2 * xn[channel] + a3 * xa[channel] - b1 * yn[channel] - b2 * ya[channel];
                xa[channel] = xn[channel];
                xn[channel] = xc[channel];
                ya[channel] = yn[channel];
                yn[channel] = yc;
                temporary[offset + channel] += yc;
            }
        }
    }
    for row in 0..height {
        if cancelled() {
            return Err(FilterError::Cancelled);
        }
        let mut xp = vec![0.0; channels];
        let mut yb = vec![0.0; channels];
        let mut yp = vec![0.0; channels];
        for channel in 0..channels {
            xp[channel] =
                temporary[row * width * channels + channel].clamp(mins[channel], maxs[channel]);
            yb[channel] = xp[channel] * coefp;
            yp[channel] = yb[channel];
        }
        let mut xc = vec![0.0; channels];
        let mut xn = vec![0.0; channels];
        let mut xa = vec![0.0; channels];
        let mut yn = vec![0.0; channels];
        let mut ya = vec![0.0; channels];
        for column in 0..width {
            let offset = (row * width + column) * channels;
            for channel in 0..channels {
                xc[channel] = temporary[offset + channel].clamp(mins[channel], maxs[channel]);
                let yc = a0 * xc[channel] + a1 * xp[channel] - b1 * yp[channel] - b2 * yb[channel];
                output[offset + channel] = yc;
                xp[channel] = xc[channel];
                yb[channel] = yp[channel];
                yp[channel] = yc;
            }
        }
        for channel in 0..channels {
            xn[channel] = temporary[(row * width + width - 1) * channels + channel]
                .clamp(mins[channel], maxs[channel]);
            xa[channel] = xn[channel];
            yn[channel] = xn[channel] * coefn;
            ya[channel] = yn[channel];
        }
        for column in (0..width).rev() {
            let offset = (row * width + column) * channels;
            for channel in 0..channels {
                xc[channel] = temporary[offset + channel].clamp(mins[channel], maxs[channel]);
                let yc = a2 * xn[channel] + a3 * xa[channel] - b1 * yn[channel] - b2 * ya[channel];
                xa[channel] = xn[channel];
                xn[channel] = xc[channel];
                ya[channel] = yn[channel];
                yn[channel] = yc;
                output[offset + channel] += yc;
            }
        }
    }
    Ok(())
}

fn eigf_variance_analysis(
    guide: &[f32],
    mask: &[f32],
    output: &mut [f32],
    width: usize,
    height: usize,
    sigma: f32,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    let elements = width * height;
    let mut input = alloc_f32(elements * 4)?;
    let mut mins = [f32::MAX; 4];
    let mut maxs = [0.0_f32; 4];
    for index in 0..elements {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        let guide_value = guide[index];
        let mask_value = mask[index];
        let values = [
            guide_value,
            guide_value * guide_value,
            mask_value,
            guide_value * mask_value,
        ];
        for channel in 0..4 {
            input[index * 4 + channel] = values[channel];
            mins[channel] = mins[channel].min(values[channel]);
            maxs[channel] = maxs[channel].max(values[channel]);
        }
    }
    gaussian_blur(
        &input, output, width, height, 4, sigma, &mins, &maxs, cancelled,
    )?;
    for index in 0..elements {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        output[index * 4 + 1] -= output[index * 4] * output[index * 4];
        output[index * 4 + 3] -= output[index * 4] * output[index * 4 + 2];
    }
    Ok(())
}

fn eigf_variance_analysis_no_mask(
    guide: &[f32],
    output: &mut [f32],
    width: usize,
    height: usize,
    sigma: f32,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    let elements = width * height;
    let mut input = alloc_f32(elements * 2)?;
    let mut mins = [f32::MAX; 2];
    let mut maxs = [0.0_f32; 2];
    for index in 0..elements {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        let guide_value = guide[index];
        let values = [guide_value, guide_value * guide_value];
        for channel in 0..2 {
            input[index * 2 + channel] = values[channel];
            mins[channel] = mins[channel].min(values[channel]);
            maxs[channel] = maxs[channel].max(values[channel]);
        }
    }
    gaussian_blur(
        &input, output, width, height, 2, sigma, &mins, &maxs, cancelled,
    )?;
    for index in 0..elements {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        let average = output[index * 2];
        output[index * 2 + 1] -= average * average;
    }
    Ok(())
}

fn eigf_blending(
    image: &mut [f32],
    mask: &[f32],
    averages: &[f32],
    blend: FilterBlend,
    feathering: f32,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    for index in 0..image.len() {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        let average_guide = averages[index * 4];
        let average_mask = averages[index * 4 + 2];
        let variance_guide = averages[index * 4 + 1];
        let covariance = averages[index * 4 + 3];
        let norm_guide = (average_guide * image[index]).max(1.0e-6);
        let norm_mask = (average_mask * mask[index]).max(1.0e-6);
        let normalized_variance = variance_guide / norm_guide;
        let normalized_covariance = covariance / (norm_guide * norm_mask).sqrt();
        let a = normalized_covariance / (normalized_variance + feathering);
        let b = average_mask - a * average_guide;
        let filtered = (image[index] * a + b).max(MIN_FLOAT);
        image[index] = match blend {
            FilterBlend::Linear => filtered,
            FilterBlend::Geomean => (image[index] * filtered).sqrt(),
        };
    }
    Ok(())
}

fn eigf_blending_no_mask(
    image: &mut [f32],
    averages: &[f32],
    blend: FilterBlend,
    feathering: f32,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    for index in 0..image.len() {
        if index % 1024 == 0 && cancelled() {
            return Err(FilterError::Cancelled);
        }
        let average_guide = averages[index * 2];
        let variance_guide = averages[index * 2 + 1];
        let norm_guide = (average_guide * image[index]).max(1.0e-6);
        let normalized_variance = variance_guide / norm_guide;
        let a = normalized_variance / (normalized_variance + feathering);
        let b = average_guide - a * average_guide;
        let filtered = (image[index] * a + b).max(MIN_FLOAT);
        image[index] = match blend {
            FilterBlend::Linear => filtered,
            FilterBlend::Geomean => (image[index] * filtered).sqrt(),
        };
    }
    Ok(())
}

/// Source `fast_eigf_surface_blur`, including its full-resolution mask and
/// exposure-independent Gaussian statistics.
pub(crate) fn eigf_surface_blur(
    image: &mut [f32],
    width: usize,
    height: usize,
    sigma: f32,
    feathering: f32,
    iterations: usize,
    blend: FilterBlend,
    quantization: f32,
    quantize_min: f32,
    quantize_max: f32,
    cancelled: &mut impl FnMut() -> bool,
) -> Result<(), FilterError> {
    let scaling = sigma.clamp(1.0, 4.0);
    let ds_sigma = (sigma / scaling).max(1.0);
    let ds_width = (width as f32 / scaling) as usize;
    let ds_height = (height as f32 / scaling) as usize;
    if ds_width == 0 || ds_height == 0 || image.len() != width * height {
        return Err(FilterError::InvalidDimensions);
    }
    let elements = width * height;
    let ds_elements = ds_width * ds_height;
    let mut mask = alloc_f32(elements)?;
    let mut ds_image = alloc_f32(ds_elements)?;
    let mut ds_mask = alloc_f32(ds_elements)?;
    let mut ds_averages = alloc_f32(ds_elements * 4)?;
    let mut averages = alloc_f32(elements * 4)?;
    for iteration in 0..iterations {
        interpolate_bilinear(
            image,
            width,
            height,
            &mut ds_image,
            ds_width,
            ds_height,
            1,
            cancelled,
        )?;
        let iteration_blend = if iteration + 1 == iterations {
            blend
        } else {
            FilterBlend::Linear
        };
        if quantization != 0.0 {
            quantize(
                image,
                &mut mask,
                quantization,
                quantize_min,
                quantize_max,
                cancelled,
            )?;
            interpolate_bilinear(
                &mask,
                width,
                height,
                &mut ds_mask,
                ds_width,
                ds_height,
                1,
                cancelled,
            )?;
            eigf_variance_analysis(
                &ds_mask,
                &ds_image,
                &mut ds_averages,
                ds_width,
                ds_height,
                ds_sigma,
                cancelled,
            )?;
            interpolate_bilinear(
                &ds_averages,
                ds_width,
                ds_height,
                &mut averages,
                width,
                height,
                4,
                cancelled,
            )?;
            eigf_blending(
                &mut *image,
                &mask,
                &averages,
                iteration_blend,
                feathering,
                cancelled,
            )?;
        } else {
            eigf_variance_analysis_no_mask(
                &ds_image,
                &mut ds_averages[..ds_elements * 2],
                ds_width,
                ds_height,
                ds_sigma,
                cancelled,
            )?;
            interpolate_bilinear(
                &ds_averages[..ds_elements * 2],
                ds_width,
                ds_height,
                &mut averages[..elements * 2],
                width,
                height,
                2,
                cancelled,
            )?;
            eigf_blending_no_mask(
                image,
                &averages[..elements * 2],
                iteration_blend,
                feathering,
                cancelled,
            )?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FilterError {
    AllocationFailed { elements: usize },
    InvalidDimensions,
    Cancelled,
}
