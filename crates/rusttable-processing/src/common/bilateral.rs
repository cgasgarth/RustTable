//! Safe CPU port of Darktable's `src/common/bilateral.c` and
//! `src/common/bilateral.h`.
//!
//! This is Darktable's bilateral grid, not a conventional neighborhood
//! bilateral filter. Pixels splat scalar density into an x/y/L grid, the
//! grid is blurred in x and y and differentiated in L, and slicing adjusts
//! only the input lightness channel.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the source algorithm maps checked raster coordinates through an f32 grid"
)]

use std::fmt;
use std::mem::size_of;

const LIGHTNESS_RANGE: f32 = 100.0;
const MIN_SPATIAL_SIGMA: f32 = 0.5;
const MIN_GRID_RESOLUTION: f32 = 4.0;
const MAX_SPATIAL_RESOLUTION: f32 = 3_000.0;
const MAX_RANGE_RESOLUTION: f32 = 50.0;

/// Failure from constructing or evaluating a bilateral grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BilateralError {
    InvalidDimensions,
    InvalidParameter(&'static str),
    NonFiniteLightness { pixel: usize },
    NonFiniteOutput { pixel: usize },
    BufferShape { expected: usize, actual: usize },
    SizeOverflow,
    AllocationFailed { required_bytes: usize },
    Cancelled,
}

impl fmt::Display for BilateralError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions => {
                formatter.write_str("bilateral grid dimensions must be nonzero")
            }
            Self::InvalidParameter(name) => {
                write!(formatter, "bilateral grid parameter {name} is invalid")
            }
            Self::NonFiniteLightness { pixel } => {
                write!(
                    formatter,
                    "bilateral grid input has non-finite lightness at pixel {pixel}"
                )
            }
            Self::NonFiniteOutput { pixel } => {
                write!(
                    formatter,
                    "bilateral grid produced non-finite lightness at pixel {pixel}"
                )
            }
            Self::BufferShape { expected, actual } => {
                write!(
                    formatter,
                    "bilateral grid expected {expected} pixels, got {actual}"
                )
            }
            Self::SizeOverflow => formatter.write_str("bilateral grid size overflowed"),
            Self::AllocationFailed { required_bytes } => {
                write!(
                    formatter,
                    "unable to allocate bilateral grid requiring {required_bytes} bytes"
                )
            }
            Self::Cancelled => formatter.write_str("bilateral grid evaluation was cancelled"),
        }
    }
}

impl std::error::Error for BilateralError {}

#[derive(Debug, Clone, Copy)]
struct GridGeometry {
    width: usize,
    height: usize,
    pixels: usize,
    size_x: usize,
    size_y: usize,
    size_z: usize,
    sigma_s: f32,
    sigma_r: f32,
    spatial_reciprocal: f32,
    range_reciprocal: f32,
    grid_values: usize,
    memory_bytes: usize,
}

impl GridGeometry {
    fn new(
        width: usize,
        height: usize,
        sigma_s: f32,
        sigma_r: f32,
    ) -> Result<Self, BilateralError> {
        if width == 0 || height == 0 {
            return Err(BilateralError::InvalidDimensions);
        }
        if !sigma_s.is_finite() {
            return Err(BilateralError::InvalidParameter("sigma_s"));
        }
        if !sigma_r.is_finite() || sigma_r <= 0.0 {
            return Err(BilateralError::InvalidParameter("sigma_r"));
        }

        let requested_sigma_s = sigma_s.max(MIN_SPATIAL_SIGMA);
        let width_f = width as f32;
        let height_f = height as f32;
        let resolution_x = (width_f / requested_sigma_s)
            .round()
            .clamp(MIN_GRID_RESOLUTION, MAX_SPATIAL_RESOLUTION);
        let resolution_y = (height_f / requested_sigma_s)
            .round()
            .clamp(MIN_GRID_RESOLUTION, MAX_SPATIAL_RESOLUTION);
        let resolution_z = (LIGHTNESS_RANGE / sigma_r)
            .round()
            .clamp(MIN_GRID_RESOLUTION, MAX_RANGE_RESOLUTION);

        let effective_sigma_s = (height_f / resolution_y).max(width_f / resolution_x);
        let effective_sigma_r = LIGHTNESS_RANGE / resolution_z;
        let spatial_reciprocal = effective_sigma_s.recip();
        let range_reciprocal = effective_sigma_r.recip();
        let size_x = (width_f * spatial_reciprocal).ceil() as usize + 1;
        let size_y = (height_f * spatial_reciprocal).ceil() as usize + 1;
        let size_z = (LIGHTNESS_RANGE * range_reciprocal).ceil() as usize + 1;

        let pixels = width
            .checked_mul(height)
            .ok_or(BilateralError::SizeOverflow)?;
        let xy = size_x
            .checked_mul(size_y)
            .ok_or(BilateralError::SizeOverflow)?;
        let grid_values = xy.checked_mul(size_z).ok_or(BilateralError::SizeOverflow)?;
        let scratch_values = size_x
            .checked_mul(size_z)
            .and_then(|plane| plane.checked_mul(3))
            .ok_or(BilateralError::SizeOverflow)?;
        let memory_bytes = grid_values
            .checked_add(scratch_values)
            .and_then(|values| values.checked_mul(size_of::<f32>()))
            .ok_or(BilateralError::SizeOverflow)?;

        Ok(Self {
            width,
            height,
            pixels,
            size_x,
            size_y,
            size_z,
            sigma_s: effective_sigma_s,
            sigma_r: effective_sigma_r,
            spatial_reciprocal,
            range_reciprocal,
            grid_values,
            memory_bytes,
        })
    }
}

/// Darktable's scalar x/y/L bilateral grid.
#[derive(Debug)]
pub struct BilateralGrid {
    geometry: GridGeometry,
    buffer: Vec<f32>,
}

impl BilateralGrid {
    /// Creates the grid used by Darktable's CPU implementation.
    ///
    /// Spatial sigmas below `0.5`, including negative values, are clamped in
    /// the same way as the retained source. Range sigma must be positive.
    ///
    /// # Errors
    ///
    /// Returns an error for zero dimensions, non-finite or non-positive range
    /// parameters, size overflow, or allocation failure.
    pub fn new(
        width: usize,
        height: usize,
        sigma_s: f32,
        sigma_r: f32,
    ) -> Result<Self, BilateralError> {
        let geometry = GridGeometry::new(width, height, sigma_s, sigma_r)?;
        let allocation_bytes = geometry
            .grid_values
            .checked_mul(size_of::<f32>())
            .ok_or(BilateralError::SizeOverflow)?;
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(geometry.grid_values)
            .map_err(|_| BilateralError::AllocationFailed {
                required_bytes: allocation_bytes,
            })?;
        buffer.resize(geometry.grid_values, 0.0);
        Ok(Self { geometry, buffer })
    }

    /// Returns Darktable's one-worker CPU memory estimate without allocating.
    ///
    /// # Errors
    ///
    /// Returns the same dimension, parameter, and size errors as [`Self::new`].
    pub fn required_memory_bytes(
        width: usize,
        height: usize,
        sigma_s: f32,
        sigma_r: f32,
    ) -> Result<usize, BilateralError> {
        GridGeometry::new(width, height, sigma_s, sigma_r).map(|geometry| geometry.memory_bytes)
    }

    #[must_use]
    pub const fn grid_dimensions(&self) -> [usize; 3] {
        [
            self.geometry.size_x,
            self.geometry.size_y,
            self.geometry.size_z,
        ]
    }

    #[must_use]
    pub const fn effective_sigma_s(&self) -> f32 {
        self.geometry.sigma_s
    }

    #[must_use]
    pub const fn effective_sigma_r(&self) -> f32 {
        self.geometry.sigma_r
    }

    #[must_use]
    pub const fn memory_bytes(&self) -> usize {
        self.geometry.memory_bytes
    }

    /// Splat input density into the bilateral grid.
    ///
    /// # Errors
    ///
    /// Returns an error when the input shape does not match the grid or input
    /// lightness is non-finite.
    pub fn splat(&mut self, input: &[[f32; 4]]) -> Result<(), BilateralError> {
        self.splat_with_cancel(input, &mut || false)
    }

    /// Splat input density while allowing row-granularity cancellation.
    ///
    /// # Errors
    ///
    /// Returns an error for a mismatched input shape, non-finite lightness, or
    /// cancellation. Cancellation after splatting begins leaves a partial
    /// grid; callers must discard it.
    pub fn splat_with_cancel<F>(
        &mut self,
        input: &[[f32; 4]],
        cancelled: &mut F,
    ) -> Result<(), BilateralError>
    where
        F: FnMut() -> bool,
    {
        if cancelled() {
            return Err(BilateralError::Cancelled);
        }
        self.validate_input(input)?;
        let size_x = self.geometry.size_x;
        let size_z = self.geometry.size_z;
        let offset_x = size_z;
        let offset_y = size_x * size_z;
        let offsets = [
            0,
            offset_x,
            offset_y,
            offset_x + offset_y,
            1,
            1 + offset_x,
            1 + offset_y,
            1 + offset_y + offset_x,
        ];
        let sigma_s_squared = self.geometry.sigma_s * self.geometry.sigma_s;

        for y in 0..self.geometry.height {
            if cancelled() {
                return Err(BilateralError::Cancelled);
            }
            for x in 0..self.geometry.width {
                let pixel = input[y * self.geometry.width + x];
                let (grid_index, xf, yf, zf) = self.image_to_grid(x, y, pixel[0]);
                let contributions = [
                    (1.0 - xf) * (1.0 - yf) * 100.0 / sigma_s_squared,
                    xf * (1.0 - yf) * 100.0 / sigma_s_squared,
                    (1.0 - xf) * yf * 100.0 / sigma_s_squared,
                    xf * yf * 100.0 / sigma_s_squared,
                ];
                for index in 0..4 {
                    self.buffer[grid_index + offsets[index]] += contributions[index] * (1.0 - zf);
                    self.buffer[grid_index + offsets[index + 4]] += contributions[index] * zf;
                }
            }
        }
        Ok(())
    }

    /// Blur x/y and apply Darktable's derivative kernel in L.
    ///
    /// # Errors
    ///
    /// This non-cancellable entry point currently cannot fail; it returns a
    /// result to match [`Self::blur_with_cancel`].
    pub fn blur(&mut self) -> Result<(), BilateralError> {
        self.blur_with_cancel(&mut || false)
    }

    /// Blur the grid while allowing cancellation between grid lines.
    ///
    /// # Errors
    ///
    /// Returns an error when cancellation is requested. Cancellation after a
    /// blur pass begins leaves a partial grid; callers must discard it.
    pub fn blur_with_cancel<F>(&mut self, cancelled: &mut F) -> Result<(), BilateralError>
    where
        F: FnMut() -> bool,
    {
        let size_x = self.geometry.size_x;
        let size_y = self.geometry.size_y;
        let size_z = self.geometry.size_z;
        let offset_x = size_z;
        let offset_y = size_x * size_z;
        blur_line(
            &mut self.buffer,
            1,
            offset_y,
            offset_x,
            size_z,
            size_y,
            size_x,
            cancelled,
        )?;
        blur_line(
            &mut self.buffer,
            1,
            offset_x,
            offset_y,
            size_z,
            size_x,
            size_y,
            cancelled,
        )?;
        blur_line_z(
            &mut self.buffer,
            offset_x,
            offset_y,
            1,
            size_x,
            size_y,
            size_z,
            cancelled,
        )
    }

    /// Slice the grid and return a copy whose lightness is adjusted.
    ///
    /// # Errors
    ///
    /// Returns an error for a mismatched input shape, non-finite input or
    /// result lightness, non-finite detail, size overflow, or allocation
    /// failure.
    pub fn slice(&self, input: &[[f32; 4]], detail: f32) -> Result<Vec<[f32; 4]>, BilateralError> {
        self.slice_with_cancel(input, detail, &mut || false)
    }

    /// Slice the grid with row-granularity cancellation.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::slice`], plus cancellation.
    pub fn slice_with_cancel<F>(
        &self,
        input: &[[f32; 4]],
        detail: f32,
        cancelled: &mut F,
    ) -> Result<Vec<[f32; 4]>, BilateralError>
    where
        F: FnMut() -> bool,
    {
        if cancelled() {
            return Err(BilateralError::Cancelled);
        }
        self.validate_input(input)?;
        let norm = self.normalization(detail)?;
        let allocation_bytes = self
            .geometry
            .pixels
            .checked_mul(size_of::<[f32; 4]>())
            .ok_or(BilateralError::SizeOverflow)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(self.geometry.pixels)
            .map_err(|_| BilateralError::AllocationFailed {
                required_bytes: allocation_bytes,
            })?;

        for y in 0..self.geometry.height {
            if cancelled() {
                return Err(BilateralError::Cancelled);
            }
            for x in 0..self.geometry.width {
                let index = y * self.geometry.width + x;
                let source = input[index];
                let mut result = source;
                result[0] = self.corrected_lightness(x, y, source[0], source[0], norm, index)?;
                output.push(result);
            }
        }
        Ok(output)
    }

    /// Slice a buffer in place, matching retained callers that alias input and
    /// output in `dt_bilateral_slice`.
    ///
    /// # Errors
    ///
    /// Returns an error for a mismatched shape, non-finite input or result
    /// lightness, or non-finite detail.
    pub fn slice_in_place(
        &self,
        pixels: &mut [[f32; 4]],
        detail: f32,
    ) -> Result<(), BilateralError> {
        self.slice_in_place_with_cancel(pixels, detail, &mut || false)
    }

    /// Slice a buffer in place with row-granularity cancellation.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::slice_in_place`], plus cancellation.
    /// Cancellation during the mutation pass can leave a completed row prefix;
    /// callers must discard that partial buffer.
    pub fn slice_in_place_with_cancel<F>(
        &self,
        pixels: &mut [[f32; 4]],
        detail: f32,
        cancelled: &mut F,
    ) -> Result<(), BilateralError>
    where
        F: FnMut() -> bool,
    {
        if cancelled() {
            return Err(BilateralError::Cancelled);
        }
        self.validate_input(pixels)?;
        let norm = self.normalization(detail)?;
        self.validate_in_place_results(pixels, norm, cancelled)?;
        for y in 0..self.geometry.height {
            if cancelled() {
                return Err(BilateralError::Cancelled);
            }
            for x in 0..self.geometry.width {
                let index = y * self.geometry.width + x;
                let lightness = pixels[index][0];
                pixels[index][0] =
                    self.corrected_lightness(x, y, lightness, lightness, norm, index)?;
            }
        }
        Ok(())
    }

    /// Add the sliced lightness correction to an existing output buffer.
    ///
    /// # Errors
    ///
    /// Returns an error for mismatched input/output shapes, non-finite guide,
    /// target, or result lightness, or non-finite detail.
    pub fn slice_to_output(
        &self,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
        detail: f32,
    ) -> Result<(), BilateralError> {
        self.slice_to_output_with_cancel(input, output, detail, &mut || false)
    }

    /// Add the sliced correction while allowing row-granularity cancellation.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::slice_to_output`], plus cancellation.
    /// Cancellation during the mutation pass can leave a completed row prefix;
    /// callers must discard that partial output.
    pub fn slice_to_output_with_cancel<F>(
        &self,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
        detail: f32,
        cancelled: &mut F,
    ) -> Result<(), BilateralError>
    where
        F: FnMut() -> bool,
    {
        if cancelled() {
            return Err(BilateralError::Cancelled);
        }
        self.validate_input(input)?;
        self.validate_output(output)?;
        let norm = self.normalization(detail)?;
        self.validate_output_results(input, output, norm, cancelled)?;
        for y in 0..self.geometry.height {
            if cancelled() {
                return Err(BilateralError::Cancelled);
            }
            for x in 0..self.geometry.width {
                let index = y * self.geometry.width + x;
                output[index][0] =
                    self.corrected_lightness(x, y, input[index][0], output[index][0], norm, index)?;
            }
        }
        Ok(())
    }

    /// Apply `slice_to_output` when the guide and output are the same buffer.
    ///
    /// # Errors
    ///
    /// Returns an error for a mismatched shape, non-finite input or result
    /// lightness, or non-finite detail.
    pub fn slice_to_output_in_place(
        &self,
        pixels: &mut [[f32; 4]],
        detail: f32,
    ) -> Result<(), BilateralError> {
        self.slice_to_output_in_place_with_cancel(pixels, detail, &mut || false)
    }

    /// Apply the aliased output slice with row-granularity cancellation.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::slice_to_output_in_place`], plus
    /// cancellation. Cancellation during mutation can leave a completed row
    /// prefix; callers must discard that partial buffer.
    pub fn slice_to_output_in_place_with_cancel<F>(
        &self,
        pixels: &mut [[f32; 4]],
        detail: f32,
        cancelled: &mut F,
    ) -> Result<(), BilateralError>
    where
        F: FnMut() -> bool,
    {
        self.slice_in_place_with_cancel(pixels, detail, cancelled)
    }

    fn validate_input(&self, input: &[[f32; 4]]) -> Result<(), BilateralError> {
        if input.len() != self.geometry.pixels {
            return Err(BilateralError::BufferShape {
                expected: self.geometry.pixels,
                actual: input.len(),
            });
        }
        if let Some(pixel) = input.iter().position(|value| !value[0].is_finite()) {
            Err(BilateralError::NonFiniteLightness { pixel })
        } else {
            Ok(())
        }
    }

    fn validate_output(&self, output: &[[f32; 4]]) -> Result<(), BilateralError> {
        if output.len() != self.geometry.pixels {
            return Err(BilateralError::BufferShape {
                expected: self.geometry.pixels,
                actual: output.len(),
            });
        }
        if let Some(pixel) = output.iter().position(|value| !value[0].is_finite()) {
            Err(BilateralError::NonFiniteOutput { pixel })
        } else {
            Ok(())
        }
    }

    fn image_to_grid(&self, x: usize, y: usize, lightness: f32) -> (usize, f32, f32, f32) {
        let grid_x = ((x as f32) * self.geometry.spatial_reciprocal)
            .clamp(0.0, (self.geometry.size_x - 1) as f32);
        let grid_y = ((y as f32) * self.geometry.spatial_reciprocal)
            .clamp(0.0, (self.geometry.size_y - 1) as f32);
        let grid_z = (lightness * self.geometry.range_reciprocal)
            .clamp(0.0, (self.geometry.size_z - 1) as f32);
        let index_x = (grid_x as usize).min(self.geometry.size_x - 2);
        let index_y = (grid_y as usize).min(self.geometry.size_y - 2);
        let index_z = (grid_z as usize).min(self.geometry.size_z - 2);
        let fraction_x = grid_x - index_x as f32;
        let fraction_y = grid_y - index_y as f32;
        let fraction_z = grid_z - index_z as f32;
        let index = ((index_x + index_y * self.geometry.size_x) * self.geometry.size_z) + index_z;
        (index, fraction_x, fraction_y, fraction_z)
    }

    fn interpolated_grid(&self, x: usize, y: usize, lightness: f32) -> f32 {
        let (index, xf, yf, zf) = self.image_to_grid(x, y, lightness);
        let offset_x = self.geometry.size_z;
        let offset_y = self.geometry.size_x * self.geometry.size_z;
        let near = 1.0 - zf;
        let far = zf;
        self.buffer[index] * (1.0 - xf) * (1.0 - yf) * near
            + self.buffer[index + offset_x] * xf * (1.0 - yf) * near
            + self.buffer[index + offset_y] * (1.0 - xf) * yf * near
            + self.buffer[index + offset_x + offset_y] * xf * yf * near
            + self.buffer[index + 1] * (1.0 - xf) * (1.0 - yf) * far
            + self.buffer[index + offset_x + 1] * xf * (1.0 - yf) * far
            + self.buffer[index + offset_y + 1] * (1.0 - xf) * yf * far
            + self.buffer[index + offset_x + offset_y + 1] * xf * yf * far
    }

    fn normalization(&self, detail: f32) -> Result<f32, BilateralError> {
        validate_detail(detail)?;
        let norm = -detail * self.geometry.sigma_r * 0.04;
        if norm.is_finite() {
            Ok(norm)
        } else {
            Err(BilateralError::InvalidParameter("detail"))
        }
    }

    fn corrected_lightness(
        &self,
        x: usize,
        y: usize,
        guide: f32,
        base: f32,
        norm: f32,
        pixel: usize,
    ) -> Result<f32, BilateralError> {
        let correction = norm * self.interpolated_grid(x, y, guide);
        let lightness = (base + correction).max(0.0);
        if lightness.is_finite() {
            Ok(lightness)
        } else {
            Err(BilateralError::NonFiniteOutput { pixel })
        }
    }

    fn validate_in_place_results<F>(
        &self,
        pixels: &[[f32; 4]],
        norm: f32,
        cancelled: &mut F,
    ) -> Result<(), BilateralError>
    where
        F: FnMut() -> bool,
    {
        for y in 0..self.geometry.height {
            if cancelled() {
                return Err(BilateralError::Cancelled);
            }
            for x in 0..self.geometry.width {
                let index = y * self.geometry.width + x;
                let lightness = pixels[index][0];
                self.corrected_lightness(x, y, lightness, lightness, norm, index)?;
            }
        }
        Ok(())
    }

    fn validate_output_results<F>(
        &self,
        input: &[[f32; 4]],
        output: &[[f32; 4]],
        norm: f32,
        cancelled: &mut F,
    ) -> Result<(), BilateralError>
    where
        F: FnMut() -> bool,
    {
        for y in 0..self.geometry.height {
            if cancelled() {
                return Err(BilateralError::Cancelled);
            }
            for x in 0..self.geometry.width {
                let index = y * self.geometry.width + x;
                self.corrected_lightness(x, y, input[index][0], output[index][0], norm, index)?;
            }
        }
        Ok(())
    }
}

fn validate_detail(detail: f32) -> Result<(), BilateralError> {
    if detail.is_finite() {
        Ok(())
    } else {
        Err(BilateralError::InvalidParameter("detail"))
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "arguments preserve Darktable's stride-oriented blur helper"
)]
fn blur_line<F>(
    buffer: &mut [f32],
    offset1: usize,
    offset2: usize,
    offset3: usize,
    size1: usize,
    size2: usize,
    size3: usize,
    cancelled: &mut F,
) -> Result<(), BilateralError>
where
    F: FnMut() -> bool,
{
    const W0: f32 = 6.0 / 16.0;
    const W1: f32 = 4.0 / 16.0;
    const W2: f32 = 1.0 / 16.0;
    if size3 < 4 {
        return blur_short_lines(
            buffer,
            offset1,
            offset2,
            offset3,
            size1,
            size2,
            size3,
            [W2, W1, W0, W1, W2],
            cancelled,
        );
    }
    for outer in 0..size1 {
        for line in 0..size2 {
            if cancelled() {
                return Err(BilateralError::Cancelled);
            }
            let mut index = outer * offset1 + line * offset2;
            let mut previous_two = buffer[index];
            buffer[index] = buffer[index] * W0
                + W1 * buffer[index + offset3]
                + W2 * buffer[index + 2 * offset3];
            index += offset3;
            let mut previous = buffer[index];
            buffer[index] = buffer[index] * W0
                + W1 * (buffer[index + offset3] + previous_two)
                + W2 * buffer[index + 2 * offset3];
            index += offset3;
            for _ in 2..size3 - 2 {
                let current = buffer[index];
                buffer[index] = buffer[index] * W0
                    + W1 * (buffer[index + offset3] + previous)
                    + W2 * (buffer[index + 2 * offset3] + previous_two);
                index += offset3;
                previous_two = previous;
                previous = current;
            }
            let current = buffer[index];
            buffer[index] =
                buffer[index] * W0 + W1 * (buffer[index + offset3] + previous) + W2 * previous_two;
            index += offset3;
            buffer[index] = buffer[index] * W0 + W1 * current + W2 * previous;
        }
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "arguments preserve Darktable's stride-oriented blur helper"
)]
fn blur_line_z<F>(
    buffer: &mut [f32],
    offset1: usize,
    offset2: usize,
    offset3: usize,
    size1: usize,
    size2: usize,
    size3: usize,
    cancelled: &mut F,
) -> Result<(), BilateralError>
where
    F: FnMut() -> bool,
{
    const W1: f32 = 4.0 / 16.0;
    const W2: f32 = 2.0 / 16.0;
    if size3 < 4 {
        return blur_short_lines(
            buffer,
            offset1,
            offset2,
            offset3,
            size1,
            size2,
            size3,
            [-W2, -W1, 0.0, W1, W2],
            cancelled,
        );
    }
    for outer in 0..size1 {
        for line in 0..size2 {
            if cancelled() {
                return Err(BilateralError::Cancelled);
            }
            let mut index = outer * offset1 + line * offset2;
            let mut previous_two = buffer[index];
            buffer[index] = W1 * buffer[index + offset3] + W2 * buffer[index + 2 * offset3];
            index += offset3;
            let mut previous = buffer[index];
            buffer[index] =
                W1 * (buffer[index + offset3] - previous_two) + W2 * buffer[index + 2 * offset3];
            index += offset3;
            for _ in 2..size3 - 2 {
                let current = buffer[index];
                buffer[index] = W1 * (buffer[index + offset3] - previous)
                    + W2 * (buffer[index + 2 * offset3] - previous_two);
                index += offset3;
                previous_two = previous;
                previous = current;
            }
            let current = buffer[index];
            buffer[index] = W1 * (buffer[index + offset3] - previous) - W2 * previous_two;
            index += offset3;
            buffer[index] = -W1 * current - W2 * previous;
        }
    }
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "arguments preserve Darktable's stride-oriented blur helper"
)]
fn blur_short_lines<F>(
    buffer: &mut [f32],
    offset1: usize,
    offset2: usize,
    offset3: usize,
    size1: usize,
    size2: usize,
    size3: usize,
    kernel: [f32; 5],
    cancelled: &mut F,
) -> Result<(), BilateralError>
where
    F: FnMut() -> bool,
{
    debug_assert!(size3 < 4);
    for outer in 0..size1 {
        for line in 0..size2 {
            if cancelled() {
                return Err(BilateralError::Cancelled);
            }
            let start = outer * offset1 + line * offset2;
            let mut source = [0.0; 3];
            for (position, value) in source.iter_mut().take(size3).enumerate() {
                *value = buffer[start + position * offset3];
            }
            for position in 0..size3 {
                let mut value = 0.0;
                for (kernel_index, weight) in kernel.into_iter().enumerate() {
                    if let Some(source_index) = position
                        .checked_add(kernel_index)
                        .and_then(|value| value.checked_sub(2))
                        && source_index < size3
                    {
                        value += source[source_index] * weight;
                    }
                }
                buffer[start + position * offset3] = value;
            }
        }
    }
    Ok(())
}
