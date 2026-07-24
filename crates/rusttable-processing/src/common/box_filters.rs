//! Safe Rust mechanical port of the retained Darktable
//! `src/common/box_filters.cc` and `src/common/box_filters.h`.
//!
//! The source lineage fixes the public channel modes, compensated summation
//! order, shrinking edge windows, separable in-place passes, and the default
//! iteration count. This port replaces unchecked pointer arithmetic and
//! aborting unsupported-mode paths with validated slices and explicit errors.

#![allow(
    clippy::cast_precision_loss,
    reason = "the retained implementation normalizes usize hit counts with an f32 divisor"
)]

use std::fmt;
use std::mem::size_of;

/// Default number of passes used by Darktable's box-mean approximation.
pub const BOX_ITERATIONS: u32 = 8;

/// Channel-mode bit requesting Kahan compensated summation.
pub const BOXFILTER_KAHAN_SUM: u32 = 0x0100_0000;

const MAX_CHANNELS: usize = 16;
const MAX_CHANNELS_U32: u32 = 16;
const MAX_VERTICAL_LANES: usize = 16;
const MIN_PARALLEL_SAMPLES_PER_WORKER: usize = 1 << 18;
const KAHAN_2: u32 = 2 | BOXFILTER_KAHAN_SUM;
const KAHAN_4: u32 = 4 | BOXFILTER_KAHAN_SUM;
const KAHAN_9: u32 = 9 | BOXFILTER_KAHAN_SUM;

/// Failure from validating or allocating an in-place box-filter pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxFilterError {
    /// Width and height must both be nonzero.
    InvalidDimensions { width: usize, height: usize },
    /// The entry point does not implement the encoded channel mode.
    UnsupportedChannels {
        operation: &'static str,
        channels: u32,
    },
    /// The image slice does not exactly match its declared shape.
    BufferShape { expected: usize, actual: usize },
    /// Caller-provided horizontal scratch is too short.
    ScratchShape { minimum: usize, actual: usize },
    /// The retained finite-math algorithm does not accept NaN or infinity.
    NonFiniteInput { sample: usize },
    /// Shape or byte-count arithmetic overflowed.
    SizeOverflow,
    /// A bounded temporary buffer could not be allocated.
    AllocationFailed { required_bytes: usize },
}

impl fmt::Display for BoxFilterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidDimensions { width, height } => {
                write!(
                    formatter,
                    "box filter dimensions must be nonzero, got {width}x{height}"
                )
            }
            Self::UnsupportedChannels {
                operation,
                channels,
            } => {
                write!(
                    formatter,
                    "{operation} does not support channel mode {channels:#010x}"
                )
            }
            Self::BufferShape { expected, actual } => {
                write!(
                    formatter,
                    "box filter expected {expected} samples, got {actual}"
                )
            }
            Self::ScratchShape { minimum, actual } => {
                write!(
                    formatter,
                    "box filter needs at least {minimum} scratch samples, got {actual}"
                )
            }
            Self::NonFiniteInput { sample } => {
                write!(
                    formatter,
                    "box filter input has a non-finite value at sample {sample}"
                )
            }
            Self::SizeOverflow => formatter.write_str("box filter size overflowed"),
            Self::AllocationFailed { required_bytes } => {
                write!(
                    formatter,
                    "unable to allocate box-filter temporary requiring {required_bytes} bytes"
                )
            }
        }
    }
}

impl std::error::Error for BoxFilterError {}

/// Applies an in-place separable box mean for one, two, or four channels.
///
/// Ordinary summation is selected by channel modes `1`, `2`, and `4`.
/// Kahan summation is selected by `2 | BOXFILTER_KAHAN_SUM` or
/// `4 | BOXFILTER_KAHAN_SUM`. Every edge is divided by the number of samples
/// that actually intersects the image.
///
/// # Errors
///
/// Returns an error for zero dimensions, an unsupported channel mode, a
/// mismatched buffer shape, non-finite input, size overflow, or scratch
/// allocation failure.
pub fn box_mean(
    buffer: &mut [f32],
    height: usize,
    width: usize,
    channels: u32,
    radius: usize,
    iterations: u32,
) -> Result<(), BoxFilterError> {
    let (channel_count, compensated) = decode_mean_channels(channels)?;
    let shape = validate_image(buffer, height, width, channel_count)?;
    if iterations == 0 || radius == 0 {
        return Ok(());
    }

    let vertical_scratch_rows = vertical_scratch_rows(height, radius);
    let vertical_scratch_len = vertical_scratch_rows
        .checked_mul(MAX_VERTICAL_LANES)
        .ok_or(BoxFilterError::SizeOverflow)?;
    let horizontal_workers = parallel_worker_count(buffer.len(), height);
    let horizontal_scratch_len = shape
        .row_len
        .checked_mul(horizontal_workers)
        .ok_or(BoxFilterError::SizeOverflow)?;
    let scratch_len = horizontal_scratch_len.max(vertical_scratch_len);
    let mut scratch = allocate_f32(scratch_len)?;
    let vertical_groups = shape.row_len.div_ceil(MAX_VERTICAL_LANES);
    let vertical_workers = parallel_worker_count(buffer.len(), vertical_groups);
    let mut vertical_output = if vertical_workers > 1 {
        Some(allocate_f32(buffer.len())?)
    } else {
        None
    };
    for _ in 0..iterations {
        blur_horizontal_rows(
            buffer,
            width,
            channel_count,
            radius,
            compensated,
            shape.row_len,
            horizontal_workers,
            &mut scratch[..horizontal_scratch_len],
        );
        if let Some(output) = &mut vertical_output {
            blur_vertical_parallel(
                buffer,
                output,
                height,
                shape.row_len,
                radius,
                compensated,
                vertical_workers,
            );
        } else {
            blur_vertical(
                buffer,
                height,
                shape.row_len,
                radius,
                compensated,
                vertical_scratch_rows,
                &mut scratch[..vertical_scratch_len],
            );
        }
    }
    Ok(())
}

/// Applies one in-place compensated horizontal pass to a single row.
///
/// Retained channel modes are `4 | BOXFILTER_KAHAN_SUM` and
/// `9 | BOXFILTER_KAHAN_SUM`. If supplied, `scratch` may be longer than the
/// row but must contain at least `width * channels` samples.
///
/// # Errors
///
/// Returns an error for zero width, an unsupported channel mode, a mismatched
/// row or scratch shape, non-finite input, size overflow, or scratch allocation
/// failure.
pub fn box_mean_horizontal(
    buffer: &mut [f32],
    width: usize,
    channels: u32,
    radius: usize,
    scratch: Option<&mut [f32]>,
) -> Result<(), BoxFilterError> {
    let channel_count = decode_horizontal_channels(channels)?;
    validate_dimensions(1, width)?;
    let row_len = width
        .checked_mul(channel_count)
        .ok_or(BoxFilterError::SizeOverflow)?;
    validate_buffer_len(buffer, row_len)?;
    validate_finite(buffer)?;
    if let Some(provided) = scratch.as_deref()
        && provided.len() < row_len
    {
        return Err(BoxFilterError::ScratchShape {
            minimum: row_len,
            actual: provided.len(),
        });
    }
    if radius == 0 {
        return Ok(());
    }

    if let Some(scratch) = scratch {
        blur_horizontal(
            buffer,
            width,
            channel_count,
            radius,
            true,
            &mut scratch[..row_len],
        );
    } else {
        let mut owned = allocate_f32(row_len)?;
        blur_horizontal(buffer, width, channel_count, radius, true, &mut owned);
    }
    Ok(())
}

/// Applies one in-place compensated vertical pass to an entire image.
///
/// The retained entry point accepts every encoded Kahan channel count from
/// one through sixteen.
///
/// # Errors
///
/// Returns an error for zero dimensions, an unsupported channel mode, a
/// mismatched image shape, non-finite input, size overflow, or scratch
/// allocation failure.
pub fn box_mean_vertical(
    buffer: &mut [f32],
    height: usize,
    width: usize,
    channels: u32,
    radius: usize,
) -> Result<(), BoxFilterError> {
    let channel_count = decode_vertical_channels(channels)?;
    let shape = validate_image(buffer, height, width, channel_count)?;
    if radius == 0 {
        return Ok(());
    }

    let vertical_groups = shape.row_len.div_ceil(MAX_VERTICAL_LANES);
    let workers = parallel_worker_count(buffer.len(), vertical_groups);
    if workers > 1 {
        let mut output = allocate_f32(buffer.len())?;
        blur_vertical_parallel(
            buffer,
            &mut output,
            height,
            shape.row_len,
            radius,
            true,
            workers,
        );
    } else {
        let scratch_rows = vertical_scratch_rows(height, radius);
        let scratch_len = scratch_rows
            .checked_mul(MAX_VERTICAL_LANES)
            .ok_or(BoxFilterError::SizeOverflow)?;
        let mut scratch = allocate_f32(scratch_len)?;
        blur_vertical(
            buffer,
            height,
            shape.row_len,
            radius,
            true,
            scratch_rows,
            &mut scratch,
        );
    }
    Ok(())
}

/// Applies an in-place one-channel two-dimensional moving minimum.
///
/// # Errors
///
/// Returns an error for zero dimensions, a channel mode other than `1`, a
/// mismatched image shape, non-finite input, size overflow, or scratch
/// allocation failure.
pub fn box_min(
    buffer: &mut [f32],
    height: usize,
    width: usize,
    channels: u32,
    radius: usize,
) -> Result<(), BoxFilterError> {
    box_extreme(
        buffer,
        height,
        width,
        channels,
        radius,
        Extreme::Minimum,
        "box_min",
    )
}

/// Applies an in-place one-channel two-dimensional moving maximum.
///
/// # Errors
///
/// Returns an error for zero dimensions, a channel mode other than `1`, a
/// mismatched image shape, non-finite input, size overflow, or scratch
/// allocation failure.
pub fn box_max(
    buffer: &mut [f32],
    height: usize,
    width: usize,
    channels: u32,
    radius: usize,
) -> Result<(), BoxFilterError> {
    box_extreme(
        buffer,
        height,
        width,
        channels,
        radius,
        Extreme::Maximum,
        "box_max",
    )
}

#[derive(Debug, Clone, Copy)]
struct ImageShape {
    row_len: usize,
}

fn validate_image(
    buffer: &[f32],
    height: usize,
    width: usize,
    channels: usize,
) -> Result<ImageShape, BoxFilterError> {
    validate_dimensions(height, width)?;
    let row_len = width
        .checked_mul(channels)
        .ok_or(BoxFilterError::SizeOverflow)?;
    let expected = row_len
        .checked_mul(height)
        .ok_or(BoxFilterError::SizeOverflow)?;
    validate_buffer_len(buffer, expected)?;
    validate_finite(buffer)?;
    Ok(ImageShape { row_len })
}

fn validate_dimensions(height: usize, width: usize) -> Result<(), BoxFilterError> {
    if width == 0 || height == 0 {
        return Err(BoxFilterError::InvalidDimensions { width, height });
    }
    Ok(())
}

fn validate_buffer_len(buffer: &[f32], expected: usize) -> Result<(), BoxFilterError> {
    if buffer.len() != expected {
        return Err(BoxFilterError::BufferShape {
            expected,
            actual: buffer.len(),
        });
    }
    Ok(())
}

fn validate_finite(buffer: &[f32]) -> Result<(), BoxFilterError> {
    if let Some(sample) = buffer.iter().position(|value| !value.is_finite()) {
        return Err(BoxFilterError::NonFiniteInput { sample });
    }
    Ok(())
}

fn decode_mean_channels(channels: u32) -> Result<(usize, bool), BoxFilterError> {
    match channels {
        1 => Ok((1, false)),
        2 => Ok((2, false)),
        4 => Ok((4, false)),
        KAHAN_2 => Ok((2, true)),
        KAHAN_4 => Ok((4, true)),
        _ => Err(BoxFilterError::UnsupportedChannels {
            operation: "box_mean",
            channels,
        }),
    }
}

fn decode_horizontal_channels(channels: u32) -> Result<usize, BoxFilterError> {
    match channels {
        KAHAN_4 => Ok(4),
        KAHAN_9 => Ok(9),
        _ => Err(BoxFilterError::UnsupportedChannels {
            operation: "box_mean_horizontal",
            channels,
        }),
    }
}

fn decode_vertical_channels(channels: u32) -> Result<usize, BoxFilterError> {
    let count = channels & !BOXFILTER_KAHAN_SUM;
    if channels & BOXFILTER_KAHAN_SUM != 0 && (1..=MAX_CHANNELS_U32).contains(&count) {
        return usize::try_from(count).map_err(|_| BoxFilterError::UnsupportedChannels {
            operation: "box_mean_vertical",
            channels,
        });
    }
    Err(BoxFilterError::UnsupportedChannels {
        operation: "box_mean_vertical",
        channels,
    })
}

fn allocate_f32(length: usize) -> Result<Vec<f32>, BoxFilterError> {
    let required_bytes = length
        .checked_mul(size_of::<f32>())
        .ok_or(BoxFilterError::SizeOverflow)?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| BoxFilterError::AllocationFailed { required_bytes })?;
    values.resize(length, 0.0);
    Ok(values)
}

fn allocate_extreme_scratch(length: usize) -> Result<(Vec<f32>, Vec<usize>), BoxFilterError> {
    let required_bytes = length
        .checked_mul(size_of::<f32>().saturating_add(size_of::<usize>()))
        .ok_or(BoxFilterError::SizeOverflow)?;

    let mut samples = Vec::new();
    samples
        .try_reserve_exact(length)
        .map_err(|_| BoxFilterError::AllocationFailed { required_bytes })?;
    samples.resize(length, 0.0);

    let mut queue = Vec::new();
    queue
        .try_reserve_exact(length)
        .map_err(|_| BoxFilterError::AllocationFailed { required_bytes })?;
    queue.resize(length, 0);
    Ok((samples, queue))
}

fn blur_horizontal(
    buffer: &mut [f32],
    width: usize,
    channels: usize,
    radius: usize,
    compensated: bool,
    scratch: &mut [f32],
) {
    let mut sum = [0.0_f32; MAX_CHANNELS];
    let mut correction = [0.0_f32; MAX_CHANNELS];
    let initial_last = radius.min(width - 1);
    let mut hits = initial_last + 1;

    for x in 0..=initial_last {
        let offset = x * channels;
        for channel in 0..channels {
            let value = buffer[offset + channel];
            scratch[offset + channel] = value;
            add_value(
                &mut sum[channel],
                &mut correction[channel],
                value,
                compensated,
            );
        }
    }

    for x in 0..width {
        let output = x * channels;
        let scale = hits as f32;
        for channel in 0..channels {
            buffer[output + channel] = sum[channel] / scale;
        }
        if x + 1 == width {
            break;
        }

        if x >= radius {
            let old = (x - radius) * channels;
            hits -= 1;
            for channel in 0..channels {
                subtract_value(
                    &mut sum[channel],
                    &mut correction[channel],
                    scratch[old + channel],
                    compensated,
                );
            }
        }
        if let Some(next) = next_window_sample(x, radius, width) {
            let offset = next * channels;
            hits += 1;
            for channel in 0..channels {
                let value = buffer[offset + channel];
                scratch[offset + channel] = value;
                add_value(
                    &mut sum[channel],
                    &mut correction[channel],
                    value,
                    compensated,
                );
            }
        }
    }
}

fn parallel_worker_count(sample_count: usize, independent_units: usize) -> usize {
    let available = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let workers_by_size = sample_count
        .checked_div(MIN_PARALLEL_SAMPLES_PER_WORKER)
        .unwrap_or(0)
        .max(1);
    available.min(independent_units).min(workers_by_size)
}

#[allow(
    clippy::too_many_arguments,
    reason = "row workers need the retained filter geometry and isolated scratch slices"
)]
fn blur_horizontal_rows(
    buffer: &mut [f32],
    width: usize,
    channels: usize,
    radius: usize,
    compensated: bool,
    row_len: usize,
    workers: usize,
    scratch: &mut [f32],
) {
    if workers == 1 {
        for row in buffer.chunks_exact_mut(row_len) {
            blur_horizontal(
                row,
                width,
                channels,
                radius,
                compensated,
                &mut scratch[..row_len],
            );
        }
        return;
    }

    let height = buffer.len() / row_len;
    let rows_per_worker = height.div_ceil(workers);
    let samples_per_worker = rows_per_worker * row_len;
    std::thread::scope(|scope| {
        for (rows, scratch) in buffer
            .chunks_mut(samples_per_worker)
            .zip(scratch.chunks_exact_mut(row_len))
        {
            let _worker = scope.spawn(move || {
                for row in rows.chunks_exact_mut(row_len) {
                    blur_horizontal(row, width, channels, radius, compensated, scratch);
                }
            });
        }
    });
}

fn vertical_scratch_rows(height: usize, radius: usize) -> usize {
    let window = radius.saturating_mul(2).saturating_add(1);
    if window >= height {
        return height;
    }
    window
        .checked_next_power_of_two()
        .unwrap_or(height)
        .min(height)
}

#[allow(
    clippy::too_many_arguments,
    reason = "the retained vertical pass needs explicit raster and cache geometry"
)]
fn blur_vertical(
    buffer: &mut [f32],
    height: usize,
    stride: usize,
    radius: usize,
    compensated: bool,
    scratch_rows: usize,
    scratch: &mut [f32],
) {
    let mut column = 0;
    while column + MAX_VERTICAL_LANES <= stride {
        blur_vertical_lanes::<MAX_VERTICAL_LANES>(
            buffer,
            height,
            stride,
            column,
            radius,
            compensated,
            scratch_rows,
            scratch,
        );
        column += MAX_VERTICAL_LANES;
    }
    while column + 4 <= stride {
        blur_vertical_lanes::<4>(
            buffer,
            height,
            stride,
            column,
            radius,
            compensated,
            scratch_rows,
            scratch,
        );
        column += 4;
    }
    while column < stride {
        blur_vertical_lanes::<1>(
            buffer,
            height,
            stride,
            column,
            radius,
            compensated,
            scratch_rows,
            scratch,
        );
        column += 1;
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "parallel vertical blocks retain explicit source and raster geometry"
)]
fn blur_vertical_parallel(
    buffer: &mut [f32],
    output: &mut [f32],
    height: usize,
    stride: usize,
    radius: usize,
    compensated: bool,
    workers: usize,
) {
    let block_width = vertical_block_width(stride, workers);
    let block_len = block_width * height;
    {
        let source: &[f32] = buffer;
        std::thread::scope(|scope| {
            for (block, destination) in output.chunks_mut(block_len).enumerate() {
                let first_column = block * block_width;
                let width = destination.len() / height;
                let _worker = scope.spawn(move || {
                    blur_vertical_block(
                        source,
                        destination,
                        height,
                        stride,
                        first_column,
                        width,
                        radius,
                        compensated,
                    );
                });
            }
        });
    }
    scatter_vertical_blocks(buffer, output, height, stride, block_width, workers);
}

fn vertical_block_width(stride: usize, workers: usize) -> usize {
    stride
        .div_ceil(workers)
        .div_ceil(MAX_VERTICAL_LANES)
        .saturating_mul(MAX_VERTICAL_LANES)
        .min(stride)
}

#[allow(
    clippy::too_many_arguments,
    reason = "a compact worker block retains its source and destination strides"
)]
fn blur_vertical_block(
    source: &[f32],
    output: &mut [f32],
    height: usize,
    source_stride: usize,
    first_column: usize,
    width: usize,
    radius: usize,
    compensated: bool,
) {
    let mut local_column = 0;
    while local_column + MAX_VERTICAL_LANES <= width {
        blur_vertical_lanes_to::<MAX_VERTICAL_LANES>(
            source,
            output,
            height,
            source_stride,
            width,
            first_column + local_column,
            local_column,
            radius,
            compensated,
        );
        local_column += MAX_VERTICAL_LANES;
    }
    while local_column + 4 <= width {
        blur_vertical_lanes_to::<4>(
            source,
            output,
            height,
            source_stride,
            width,
            first_column + local_column,
            local_column,
            radius,
            compensated,
        );
        local_column += 4;
    }
    while local_column < width {
        blur_vertical_lanes_to::<1>(
            source,
            output,
            height,
            source_stride,
            width,
            first_column + local_column,
            local_column,
            radius,
            compensated,
        );
        local_column += 1;
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "const-width output lanes mirror the retained vertical arithmetic"
)]
fn blur_vertical_lanes_to<const LANES: usize>(
    source: &[f32],
    output: &mut [f32],
    height: usize,
    source_stride: usize,
    output_stride: usize,
    source_column: usize,
    output_column: usize,
    radius: usize,
    compensated: bool,
) {
    let initial_last = radius.min(height - 1);
    let mut hits = initial_last + 1;
    let mut sum = [0.0_f32; LANES];
    let mut correction = [0.0_f32; LANES];

    for y in 0..=initial_last {
        let input = y * source_stride + source_column;
        for lane in 0..LANES {
            add_value(
                &mut sum[lane],
                &mut correction[lane],
                source[input + lane],
                compensated,
            );
        }
    }

    for y in 0..height {
        let destination = y * output_stride + output_column;
        let scale = hits as f32;
        for lane in 0..LANES {
            output[destination + lane] = sum[lane] / scale;
        }
        if y + 1 == height {
            break;
        }

        if y >= radius {
            hits -= 1;
            let outgoing = (y - radius) * source_stride + source_column;
            for lane in 0..LANES {
                subtract_value(
                    &mut sum[lane],
                    &mut correction[lane],
                    source[outgoing + lane],
                    compensated,
                );
            }
        }
        if let Some(next) = next_window_sample(y, radius, height) {
            hits += 1;
            let incoming = next * source_stride + source_column;
            for lane in 0..LANES {
                add_value(
                    &mut sum[lane],
                    &mut correction[lane],
                    source[incoming + lane],
                    compensated,
                );
            }
        }
    }
}

fn scatter_vertical_blocks(
    buffer: &mut [f32],
    blocks: &[f32],
    height: usize,
    stride: usize,
    block_width: usize,
    workers: usize,
) {
    let rows_per_worker = height.div_ceil(workers.min(height));
    let samples_per_worker = rows_per_worker * stride;
    let block_len = block_width * height;
    std::thread::scope(|scope| {
        for (row_block, rows) in buffer.chunks_mut(samples_per_worker).enumerate() {
            let first_row = row_block * rows_per_worker;
            let _worker = scope.spawn(move || {
                for (local_row, destination) in rows.chunks_exact_mut(stride).enumerate() {
                    let source_row = first_row + local_row;
                    for (block, source) in blocks.chunks(block_len).enumerate() {
                        let first_column = block * block_width;
                        let width = source.len() / height;
                        let source_start = source_row * width;
                        destination[first_column..first_column + width]
                            .copy_from_slice(&source[source_start..source_start + width]);
                    }
                }
            });
        }
    });
}

#[allow(
    clippy::too_many_arguments,
    reason = "const-width vertical lanes mirror Darktable's cache-oriented template"
)]
fn blur_vertical_lanes<const LANES: usize>(
    buffer: &mut [f32],
    height: usize,
    stride: usize,
    column: usize,
    radius: usize,
    compensated: bool,
    scratch_rows: usize,
    scratch: &mut [f32],
) {
    let initial_last = radius.min(height - 1);
    let mut hits = initial_last + 1;
    let mut sum = [0.0_f32; LANES];
    let mut correction = [0.0_f32; LANES];

    for y in 0..=initial_last {
        let input = y * stride + column;
        let scratch_row = vertical_scratch_row(y, height, scratch_rows);
        let cached = scratch_row * LANES;
        for lane in 0..LANES {
            let value = buffer[input + lane];
            scratch[cached + lane] = value;
            add_value(&mut sum[lane], &mut correction[lane], value, compensated);
        }
    }

    for y in 0..height {
        let output = y * stride + column;
        let scale = hits as f32;
        for lane in 0..LANES {
            buffer[output + lane] = sum[lane] / scale;
        }
        if y + 1 == height {
            break;
        }

        if y >= radius {
            hits -= 1;
            let outgoing = y - radius;
            let scratch_row = vertical_scratch_row(outgoing, height, scratch_rows);
            let cached = scratch_row * LANES;
            for lane in 0..LANES {
                subtract_value(
                    &mut sum[lane],
                    &mut correction[lane],
                    scratch[cached + lane],
                    compensated,
                );
            }
        }
        if let Some(next) = next_window_sample(y, radius, height) {
            let input = next * stride + column;
            let scratch_row = vertical_scratch_row(next, height, scratch_rows);
            let cached = scratch_row * LANES;
            hits += 1;
            for lane in 0..LANES {
                let value = buffer[input + lane];
                scratch[cached + lane] = value;
                add_value(&mut sum[lane], &mut correction[lane], value, compensated);
            }
        }
    }
}

fn vertical_scratch_row(row: usize, height: usize, scratch_rows: usize) -> usize {
    if scratch_rows == height {
        row
    } else {
        row & (scratch_rows - 1)
    }
}

fn next_window_sample(center: usize, radius: usize, length: usize) -> Option<usize> {
    center
        .checked_add(radius)
        .and_then(|index| index.checked_add(1))
        .filter(|&index| index < length)
}

fn add_value(sum: &mut f32, correction: &mut f32, value: f32, compensated: bool) {
    if compensated {
        kahan_update(sum, correction, value);
    } else {
        *sum += value;
    }
}

fn subtract_value(sum: &mut f32, correction: &mut f32, value: f32, compensated: bool) {
    if compensated {
        kahan_update(sum, correction, -value);
    } else {
        *sum -= value;
    }
}

fn kahan_update(sum: &mut f32, correction: &mut f32, value: f32) {
    let adjusted = value - *correction;
    let updated = *sum + adjusted;
    *correction = (updated - *sum) - adjusted;
    *sum = updated;
}

#[derive(Debug, Clone, Copy)]
enum Extreme {
    Minimum,
    Maximum,
}

fn box_extreme(
    buffer: &mut [f32],
    height: usize,
    width: usize,
    channels: u32,
    radius: usize,
    extreme: Extreme,
    operation: &'static str,
) -> Result<(), BoxFilterError> {
    if channels != 1 {
        return Err(BoxFilterError::UnsupportedChannels {
            operation,
            channels,
        });
    }
    validate_image(buffer, height, width, 1)?;
    if radius == 0 {
        return Ok(());
    }

    let row_workers = parallel_worker_count(buffer.len(), height);
    let row_scratch_len = width
        .checked_mul(row_workers)
        .ok_or(BoxFilterError::SizeOverflow)?;
    let column_groups = width.div_ceil(MAX_VERTICAL_LANES);
    let column_workers = parallel_worker_count(buffer.len(), column_groups);
    let column_scratch_len = height
        .checked_mul(column_workers)
        .ok_or(BoxFilterError::SizeOverflow)?;
    let scratch_len = row_scratch_len.max(column_scratch_len);
    let (mut samples, mut queue) = allocate_extreme_scratch(scratch_len)?;
    let mut vertical_output = if column_workers > 1 {
        Some(allocate_f32(buffer.len())?)
    } else {
        None
    };
    extreme_horizontal_rows(
        buffer,
        width,
        radius,
        extreme,
        row_workers,
        &mut samples[..row_scratch_len],
        &mut queue[..row_scratch_len],
    );

    if let Some(output) = &mut vertical_output {
        extreme_vertical_parallel(
            buffer,
            output,
            height,
            width,
            radius,
            extreme,
            column_workers,
            &mut samples[..column_scratch_len],
            &mut queue[..column_scratch_len],
        );
    } else {
        for x in 0..width {
            for y in 0..height {
                samples[y] = buffer[y * width + x];
            }
            sliding_extreme_column(
                &samples[..height],
                buffer,
                height,
                width,
                x,
                radius,
                extreme,
                &mut queue[..height],
            );
        }
    }
    Ok(())
}

fn extreme_horizontal_rows(
    buffer: &mut [f32],
    width: usize,
    radius: usize,
    extreme: Extreme,
    workers: usize,
    samples: &mut [f32],
    queue: &mut [usize],
) {
    if workers == 1 {
        for row in buffer.chunks_exact_mut(width) {
            samples[..width].copy_from_slice(row);
            sliding_extreme(&samples[..width], row, radius, extreme, &mut queue[..width]);
        }
        return;
    }

    let height = buffer.len() / width;
    let rows_per_worker = height.div_ceil(workers);
    let samples_per_worker = rows_per_worker * width;
    std::thread::scope(|scope| {
        for ((rows, samples), queue) in buffer
            .chunks_mut(samples_per_worker)
            .zip(samples.chunks_exact_mut(width))
            .zip(queue.chunks_exact_mut(width))
        {
            let _worker = scope.spawn(move || {
                for row in rows.chunks_exact_mut(width) {
                    samples.copy_from_slice(row);
                    sliding_extreme(samples, row, radius, extreme, queue);
                }
            });
        }
    });
}

#[allow(
    clippy::too_many_arguments,
    reason = "parallel extrema columns need compact output and isolated worker scratch"
)]
fn extreme_vertical_parallel(
    buffer: &mut [f32],
    output: &mut [f32],
    height: usize,
    width: usize,
    radius: usize,
    extreme: Extreme,
    workers: usize,
    samples: &mut [f32],
    queue: &mut [usize],
) {
    let block_width = vertical_block_width(width, workers);
    let block_len = block_width * height;
    {
        let source: &[f32] = buffer;
        std::thread::scope(|scope| {
            for (block, ((destination, samples), queue)) in output
                .chunks_mut(block_len)
                .zip(samples.chunks_exact_mut(height))
                .zip(queue.chunks_exact_mut(height))
                .enumerate()
            {
                let first_column = block * block_width;
                let columns = destination.len() / height;
                let _worker = scope.spawn(move || {
                    for local_column in 0..columns {
                        let source_column = first_column + local_column;
                        for y in 0..height {
                            samples[y] = source[y * width + source_column];
                        }
                        sliding_extreme_column(
                            samples,
                            destination,
                            height,
                            columns,
                            local_column,
                            radius,
                            extreme,
                            queue,
                        );
                    }
                });
            }
        });
    }
    scatter_vertical_blocks(buffer, output, height, width, block_width, workers);
}

fn sliding_extreme(
    source: &[f32],
    output: &mut [f32],
    radius: usize,
    extreme: Extreme,
    queue: &mut [usize],
) {
    let mut head = 0;
    let mut tail = 0;
    let mut next = 0;
    for (center, destination) in output.iter_mut().enumerate() {
        let right = center.saturating_add(radius).min(source.len() - 1);
        while next <= right {
            push_extreme(source, queue, &mut head, &mut tail, next, extreme);
            next += 1;
        }
        let left = center.saturating_sub(radius);
        while head < tail && queue[head] < left {
            head += 1;
        }
        *destination = bounded_extreme_value(source[queue[head]], extreme);
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "column output retains explicit source height and destination geometry"
)]
fn sliding_extreme_column(
    source: &[f32],
    output: &mut [f32],
    height: usize,
    stride: usize,
    column: usize,
    radius: usize,
    extreme: Extreme,
    queue: &mut [usize],
) {
    let mut head = 0;
    let mut tail = 0;
    let mut next = 0;
    for center in 0..height {
        let right = center.saturating_add(radius).min(height - 1);
        while next <= right {
            push_extreme(source, queue, &mut head, &mut tail, next, extreme);
            next += 1;
        }
        let left = center.saturating_sub(radius);
        while head < tail && queue[head] < left {
            head += 1;
        }
        output[center * stride + column] = bounded_extreme_value(source[queue[head]], extreme);
    }
}

fn push_extreme(
    source: &[f32],
    queue: &mut [usize],
    head: &mut usize,
    tail: &mut usize,
    candidate: usize,
    extreme: Extreme,
) {
    let candidate_value = bounded_extreme_value(source[candidate], extreme);
    while *head < *tail {
        let prior = bounded_extreme_value(source[queue[*tail - 1]], extreme);
        if !dominates(candidate_value, prior, extreme) {
            break;
        }
        *tail -= 1;
    }
    queue[*tail] = candidate;
    *tail += 1;
}

fn bounded_extreme_value(value: f32, extreme: Extreme) -> f32 {
    match extreme {
        Extreme::Minimum => value.min(f32::MAX),
        Extreme::Maximum => value.max(-f32::MAX),
    }
}

fn dominates(candidate: f32, prior: f32, extreme: Extreme) -> bool {
    match extreme {
        Extreme::Minimum => candidate <= prior,
        Extreme::Maximum => candidate >= prior,
    }
}
