//! Immutable CPU execution and commit state ported from `process()`,
//! `commit_params()`, `init_pipe()`, and `_generate_curve_lut()` in
//! `src/iop/rgbcurve.c`.
//!
//! GPU, GTK, and outer mask/blend ownership remain deliberately unavailable in
//! this operation-local leaf. The candidate pass preserves alpha exactly.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "native tile and pixel loops use source-width arithmetic"
)]
#![expect(
    clippy::suboptimal_flops,
    reason = "Native RGB Curve luminance and execution equations preserve source evaluation order and IEEE-754 parity."
)]

use std::fmt;

use super::curve::{
    CompiledCurveSet, CurveCompileError, RgbCurveProfileCacheKey, RgbCurveProfileEvidence,
    compile_parameters,
};
use super::parameters::{
    CHANNELS, LUT_RESOLUTION, PreserveColors, RgbCurveAutoscale, RgbCurveChannel,
    RgbCurveParametersV1,
};

/// Native operation metadata from `rgbcurve.c`.
pub const OPERATION_NAME: &str = "rgb curve";
pub const DEFAULT_GROUPS: [&str; 2] = ["tone", "grading"];
pub const DEFAULT_COLORSPACE: &str = "RGB";
pub const DESCRIPTION: &str = "alter an image’s tones using curves in RGB color space";
/// Native flags are retained as named contracts; shared registration owns their
/// eventual bit projection.
pub const SUPPORTS_BLENDING: bool = true;
pub const ALLOW_TILING: bool = true;

/// Native global OpenCL registration metadata. The leaf remains CPU-only until
/// a GPU owner binds the kernel ABI and qualification/fallback path.
pub const GPU_PROGRAM_INDEX: u32 = 25;
pub const GPU_KERNEL_NAME: &str = "rgbcurve";
pub const GPU_SUPPORTED: bool = false;
/// GTK controls remain unavailable rather than projecting generic controls.
pub const GTK_SUPPORTED: bool = false;

/// Explicitly independent capability surfaces for this bounded leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbCurveCapabilities {
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub gpu_fallback_only: bool,
    pub gtk_supported: bool,
    pub masks_consumed: bool,
    pub outer_blend_deferred: bool,
    /// Native `_generate_curve_lut()` uses raw nodes when no profile exists.
    pub middle_grey_requires_profile_evidence: bool,
}

#[must_use]
pub const fn capabilities() -> RgbCurveCapabilities {
    RgbCurveCapabilities {
        cpu_supported: true,
        gpu_supported: GPU_SUPPORTED,
        gpu_fallback_only: true,
        gtk_supported: GTK_SUPPORTED,
        masks_consumed: false,
        outer_blend_deferred: true,
        middle_grey_requires_profile_evidence: false,
    }
}

/// Four-float native RGBA pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbCurvePixel {
    channels: [f32; 4],
}

impl RgbCurvePixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            channels: [red, green, blue, alpha],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; 4]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
        self.channels
    }
}

/// A rectangular tile in an identity ROI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbCurveTile {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl RgbCurveTile {
    #[must_use]
    pub const fn new(x: usize, y: usize, width: usize, height: usize) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// Result of a native-compatible CPU pass.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbCurveExecution {
    pub pixels: Vec<RgbCurvePixel>,
    pub input_format_problem: bool,
}

/// Immutable compiled operation plan.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbCurvePlan {
    parameters: RgbCurveParametersV1,
    curves: CompiledCurveSet,
    profile: Option<RgbCurveProfileEvidence>,
}

impl RgbCurvePlan {
    pub fn new(
        parameters: RgbCurveParametersV1,
        profile: Option<RgbCurveProfileEvidence>,
    ) -> Result<Self, RgbCurveExecutionError> {
        let curves = compile_parameters(&parameters, profile.as_ref())?;
        Ok(Self {
            parameters,
            curves,
            profile,
        })
    }

    #[must_use]
    pub const fn parameters(&self) -> &RgbCurveParametersV1 {
        &self.parameters
    }

    #[must_use]
    pub const fn curves(&self) -> &CompiledCurveSet {
        &self.curves
    }

    /// Executes the four-channel CPU pass and publishes no partial result on
    /// cancellation.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[RgbCurvePixel],
        mut cancelled: F,
    ) -> Result<RgbCurveExecution, RgbCurveExecutionError> {
        if cancelled() {
            return Err(RgbCurveExecutionError::Cancelled);
        }
        let mut output = Vec::with_capacity(input.len());
        for pixel in input.iter().copied() {
            if cancelled() {
                return Err(RgbCurveExecutionError::Cancelled);
            }
            output.push(self.evaluate_pixel(pixel));
        }
        Ok(RgbCurveExecution {
            pixels: output,
            input_format_problem: false,
        })
    }

    /// Models native `dt_iop_have_required_input_format`: copy-through with a
    /// trouble flag when full-color four-channel input is unavailable.
    pub fn execute_required_format_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[RgbCurvePixel],
        required_format_available: bool,
        mut cancelled: F,
    ) -> Result<RgbCurveExecution, RgbCurveExecutionError> {
        if cancelled() {
            return Err(RgbCurveExecutionError::Cancelled);
        }
        if !required_format_available {
            return Ok(RgbCurveExecution {
                pixels: input.to_vec(),
                input_format_problem: true,
            });
        }
        self.execute_with_cancel(input, cancelled)
    }

    /// Executes arbitrary non-overlapping tiles and proves identity-ROI
    /// equivalence independently of the shared pixelpipe scheduler.
    pub fn execute_tiles_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[RgbCurvePixel],
        image_width: usize,
        image_height: usize,
        tiles: &[RgbCurveTile],
        required_format_available: bool,
        mut cancelled: F,
    ) -> Result<RgbCurveExecution, RgbCurveExecutionError> {
        let expected = image_width
            .checked_mul(image_height)
            .ok_or(RgbCurveExecutionError::InvalidDimensions)?;
        if input.len() != expected || image_width == 0 || image_height == 0 {
            return Err(RgbCurveExecutionError::InvalidDimensions);
        }
        if cancelled() {
            return Err(RgbCurveExecutionError::Cancelled);
        }

        // Validate geometry and coverage before either copy-through or
        // publication. Invalid schedules must never silently bypass the tile
        // contract merely because the input format is unavailable.
        let mut covered = vec![false; input.len()];
        for tile in tiles {
            let tile_end_x = tile
                .x
                .checked_add(tile.width)
                .ok_or(RgbCurveExecutionError::InvalidTile)?;
            let tile_end_y = tile
                .y
                .checked_add(tile.height)
                .ok_or(RgbCurveExecutionError::InvalidTile)?;
            if tile.width == 0
                || tile.height == 0
                || tile_end_x > image_width
                || tile_end_y > image_height
            {
                return Err(RgbCurveExecutionError::InvalidTile);
            }
            for y in tile.y..tile_end_y {
                for x in tile.x..tile_end_x {
                    if cancelled() {
                        return Err(RgbCurveExecutionError::Cancelled);
                    }
                    let index = y * image_width + x;
                    if covered[index] {
                        return Err(RgbCurveExecutionError::OverlappingTiles);
                    }
                    covered[index] = true;
                }
            }
        }
        if covered.iter().any(|covered| !covered) {
            return Err(RgbCurveExecutionError::IncompleteTiles);
        }
        if !required_format_available {
            return Ok(RgbCurveExecution {
                pixels: input.to_vec(),
                input_format_problem: true,
            });
        }

        let mut output = vec![RgbCurvePixel::from_channels([0.0; 4]); input.len()];
        for tile in tiles {
            let tile_end_x = tile.x + tile.width;
            let tile_end_y = tile.y + tile.height;
            for y in tile.y..tile_end_y {
                for x in tile.x..tile_end_x {
                    if cancelled() {
                        return Err(RgbCurveExecutionError::Cancelled);
                    }
                    let index = y * image_width + x;
                    output[index] = self.evaluate_pixel(input[index]);
                }
            }
        }
        Ok(RgbCurveExecution {
            pixels: output,
            input_format_problem: false,
        })
    }

    fn evaluate_pixel(&self, pixel: RgbCurvePixel) -> RgbCurvePixel {
        let input = pixel.channels;
        let rgb = match self.parameters.curve_autoscale {
            RgbCurveAutoscale::ManualRgb => [
                self.curves.channel(0).evaluate(input[0]),
                self.curves.channel(1).evaluate(input[1]),
                self.curves.channel(2).evaluate(input[2]),
            ],
            RgbCurveAutoscale::AutomaticRgb => match self.parameters.preserve_colors {
                PreserveColors::None => [
                    self.curves.channel(0).evaluate(input[0]),
                    self.curves.channel(0).evaluate(input[1]),
                    self.curves.channel(0).evaluate(input[2]),
                ],
                mode => {
                    let lum = rgb_norm(input, mode, self.profile.as_ref());
                    let ratio = if lum > 0.0 {
                        self.curves.channel(0).evaluate(lum) / lum
                    } else {
                        1.0
                    };
                    [ratio * input[0], ratio * input[1], ratio * input[2]]
                }
            },
        };
        RgbCurvePixel::new(rgb[0], rgb[1], rgb[2], input[3])
    }
}

/// Source `dt_rgb_norm`, with unknown enum values impossible in checked params.
fn rgb_norm(
    pixel: [f32; 4],
    mode: PreserveColors,
    profile: Option<&RgbCurveProfileEvidence>,
) -> f32 {
    match mode {
        PreserveColors::None => unreachable!("None is handled before norm selection"),
        PreserveColors::Luminance => profile.map_or_else(
            || pixel[0] * 0.2225045 + pixel[1] * 0.7168786 + pixel[2] * 0.0606169,
            |profile| profile.luminance([pixel[0], pixel[1], pixel[2]]),
        ),
        PreserveColors::Max => pixel[0].max(pixel[1]).max(pixel[2]),
        PreserveColors::Average => (pixel[0] + pixel[1] + pixel[2]) / 3.0,
        PreserveColors::Sum => pixel[0] + pixel[1] + pixel[2],
        PreserveColors::Norm => {
            (pixel[0] * pixel[0] + pixel[1] * pixel[1] + pixel[2] * pixel[2]).sqrt()
        }
        PreserveColors::Power => {
            let red = pixel[0] * pixel[0];
            let green = pixel[1] * pixel[1];
            let blue = pixel[2] * pixel[2];
            (pixel[0] * red + pixel[1] * green + pixel[2] * blue) / (red + green + blue)
        }
    }
}

/// Operation-local analogue of the native piece data and commit transition.
#[derive(Debug, Clone)]
pub struct RgbCurveRuntime {
    parameters: RgbCurveParametersV1,
    compiled: Option<CompiledCurveSet>,
    compiled_profile: Option<RgbCurveProfileCacheKey>,
    curve_changed: [bool; CHANNELS],
    request_histogram: bool,
    histogram_middle_grey: bool,
}

impl RgbCurveRuntime {
    #[must_use]
    pub const fn new(parameters: RgbCurveParametersV1) -> Self {
        let histogram_middle_grey = parameters.compensate_middle_grey;
        Self {
            parameters,
            compiled: None,
            compiled_profile: None,
            curve_changed: [false; CHANNELS],
            request_histogram: true,
            histogram_middle_grey,
        }
    }

    /// Native preview/non-preview histogram request and derived-state order.
    pub fn commit_params(&mut self, parameters: RgbCurveParametersV1, preview: bool) {
        self.request_histogram = preview;
        if preview {
            self.histogram_middle_grey = parameters.compensate_middle_grey;
        }
        for channel in 0..CHANNELS {
            self.curve_changed[channel] =
                self.parameters.curve_type[channel] != parameters.curve_type[channel];
        }
        self.parameters = parameters;
        self.compiled = None;
        self.compiled_profile = None;
    }

    pub fn plan(
        &mut self,
        profile: Option<RgbCurveProfileEvidence>,
    ) -> Result<RgbCurvePlan, RgbCurveExecutionError> {
        let profile_key = profile.as_ref().map(RgbCurveProfileEvidence::cache_key);
        if self.compiled.is_none() || self.compiled_profile != profile_key {
            self.compiled = Some(compile_parameters(&self.parameters, profile.as_ref())?);
            self.compiled_profile = profile_key;
            self.curve_changed = [false; CHANNELS];
        }
        Ok(RgbCurvePlan {
            parameters: self.parameters.clone(),
            curves: self.compiled.clone().expect("compiled immediately above"),
            profile,
        })
    }

    #[must_use]
    pub const fn parameters(&self) -> &RgbCurveParametersV1 {
        &self.parameters
    }

    #[must_use]
    pub const fn curve_changed(&self) -> [bool; CHANNELS] {
        self.curve_changed
    }

    #[must_use]
    pub const fn request_histogram(&self) -> bool {
        self.request_histogram
    }

    #[must_use]
    pub const fn histogram_middle_grey(&self) -> bool {
        self.histogram_middle_grey
    }

    #[must_use]
    pub const fn lut_is_built(&self) -> bool {
        self.compiled.is_some()
    }

    /// Native `init_pipe()` starts with integer-division zeros, but valid
    /// processing always calls lazy LUT generation first.
    #[must_use]
    pub const fn initial_table_value(&self, _channel: usize, _index: usize) -> f32 {
        0.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbCurveExecutionError {
    Curve(CurveCompileError),
    Cancelled,
    InvalidDimensions,
    InvalidTile,
    OverlappingTiles,
    IncompleteTiles,
}

impl From<CurveCompileError> for RgbCurveExecutionError {
    fn from(error: CurveCompileError) -> Self {
        Self::Curve(error)
    }
}

impl fmt::Display for RgbCurveExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Curve(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("RGB Curve execution was cancelled"),
            Self::InvalidDimensions => {
                formatter.write_str("RGB Curve raster dimensions are invalid")
            }
            Self::InvalidTile => formatter.write_str("RGB Curve tile is outside the raster"),
            Self::OverlappingTiles => formatter.write_str("RGB Curve tiles overlap"),
            Self::IncompleteTiles => formatter.write_str("RGB Curve tiles do not cover the raster"),
        }
    }
}

impl std::error::Error for RgbCurveExecutionError {}

/// Retained source constant for tests that assert the 65536 table shape.
#[must_use]
pub const fn lut_resolution() -> u32 {
    LUT_RESOLUTION
}

/// Retained source channel IDs are contiguous R/G/B.
#[must_use]
pub const fn channel_index(channel: RgbCurveChannel) -> usize {
    channel.index()
}
