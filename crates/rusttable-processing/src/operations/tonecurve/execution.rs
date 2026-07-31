//! Bounded CPU Tone Curve execution ported from `process()`, `commit_params()`,
//! and the native pipe data transitions in `src/iop/tonecurve.c`.
//!
//! Production history routing, registry/descriptor projection, pixelpipe
//! dispatch, GPU, GTK, blending, and presets remain unavailable seams.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::needless_range_loop,
    clippy::similar_names,
    reason = "native pixel and tile loops preserve source-shaped f32 arithmetic"
)]

use std::fmt;

use super::curve::{
    CompiledToneCurveSet, CurveCompileError, ToneCurveProfileEvidence, compile_parameters,
    lab_to_prophoto, lab_to_xyz, prophoto_to_lab, xyz_to_lab,
};
use super::parameters::{
    CHANNELS, LUT_RESOLUTION, PreserveColors, ToneCurveAutoscale, ToneCurveParametersV5,
};

pub const OPERATION_NAME: &str = "tone curve";
pub const DEFAULT_GROUPS: [&str; 2] = ["tone", "grading"];
pub const DEFAULT_COLORSPACE: &str = "Lab";
pub const DESCRIPTION: &str = "alter an image’s tones using curves";
pub const SUPPORTS_BLENDING: bool = true;
pub const ALLOW_TILING: bool = true;
pub const GPU_SUPPORTED: bool = false;
pub const GTK_SUPPORTED: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToneCurveCapabilities {
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub gtk_supported: bool,
    pub masks_consumed: bool,
    pub outer_blend_deferred: bool,
    pub rgb_luminance_requires_profile_evidence: bool,
}

#[must_use]
pub const fn capabilities() -> ToneCurveCapabilities {
    ToneCurveCapabilities {
        cpu_supported: true,
        gpu_supported: GPU_SUPPORTED,
        gtk_supported: GTK_SUPPORTED,
        masks_consumed: false,
        outer_blend_deferred: true,
        rgb_luminance_requires_profile_evidence: true,
    }
}

/// Four-float native full-color pixel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneCurvePixel {
    channels: [f32; 4],
}

impl ToneCurvePixel {
    #[must_use]
    pub const fn new(l: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [l, a, b, alpha],
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

/// A rectangular identity-ROI tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToneCurveTile {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl ToneCurveTile {
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

#[derive(Debug, Clone, PartialEq)]
pub struct ToneCurveExecution {
    pub pixels: Vec<ToneCurvePixel>,
    pub input_format_problem: bool,
}

/// Immutable compiled Tone Curve plan.
#[derive(Debug, Clone, PartialEq)]
pub struct ToneCurvePlan {
    parameters: ToneCurveParametersV5,
    curves: CompiledToneCurveSet,
    profile: Option<ToneCurveProfileEvidence>,
}

impl ToneCurvePlan {
    pub fn new(
        parameters: ToneCurveParametersV5,
        profile: Option<ToneCurveProfileEvidence>,
    ) -> Result<Self, ToneCurveExecutionError> {
        let curves = compile_parameters(&parameters, profile.as_ref())?;
        Ok(Self {
            parameters,
            curves,
            profile,
        })
    }

    #[must_use]
    pub const fn parameters(&self) -> &ToneCurveParametersV5 {
        &self.parameters
    }

    #[must_use]
    pub const fn curves(&self) -> &CompiledToneCurveSet {
        &self.curves
    }

    /// Executes without ever returning a partially filled output on cancellation.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[ToneCurvePixel],
        mut cancelled: F,
    ) -> Result<ToneCurveExecution, ToneCurveExecutionError> {
        if cancelled() {
            return Err(ToneCurveExecutionError::Cancelled);
        }
        let mut output = Vec::with_capacity(input.len());
        for pixel in input.iter().copied() {
            if cancelled() {
                return Err(ToneCurveExecutionError::Cancelled);
            }
            output.push(self.evaluate_pixel(pixel));
        }
        Ok(ToneCurveExecution {
            pixels: output,
            input_format_problem: false,
        })
    }

    /// Models native required-format copy-through and trouble reporting.
    pub fn execute_required_format_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[ToneCurvePixel],
        required_format_available: bool,
        mut cancelled: F,
    ) -> Result<ToneCurveExecution, ToneCurveExecutionError> {
        // Native process() checks the required input format before doing any
        // processing work.  A failed format check copies through and reports
        // the trouble flag, even when the caller would otherwise be cancelled.
        if !required_format_available {
            return Ok(ToneCurveExecution {
                pixels: input.to_vec(),
                input_format_problem: true,
            });
        }
        if cancelled() {
            return Err(ToneCurveExecutionError::Cancelled);
        }
        self.execute_with_cancel(input, cancelled)
    }

    /// Executes a complete, non-overlapping identity-ROI tile schedule.
    pub fn execute_tiles_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[ToneCurvePixel],
        image_width: usize,
        image_height: usize,
        tiles: &[ToneCurveTile],
        required_format_available: bool,
        mut cancelled: F,
    ) -> Result<ToneCurveExecution, ToneCurveExecutionError> {
        // Match process(): an unavailable required format is copied through
        // before cancellation, dimensions, or tile coverage are considered.
        if !required_format_available {
            return Ok(ToneCurveExecution {
                pixels: input.to_vec(),
                input_format_problem: true,
            });
        }

        let expected = image_width
            .checked_mul(image_height)
            .ok_or(ToneCurveExecutionError::InvalidDimensions)?;
        if input.len() != expected || image_width == 0 || image_height == 0 {
            return Err(ToneCurveExecutionError::InvalidDimensions);
        }
        if cancelled() {
            return Err(ToneCurveExecutionError::Cancelled);
        }

        let mut covered = vec![false; input.len()];
        for tile in tiles {
            let end_x = tile
                .x
                .checked_add(tile.width)
                .ok_or(ToneCurveExecutionError::InvalidTile)?;
            let end_y = tile
                .y
                .checked_add(tile.height)
                .ok_or(ToneCurveExecutionError::InvalidTile)?;
            if tile.width == 0 || tile.height == 0 || end_x > image_width || end_y > image_height {
                return Err(ToneCurveExecutionError::InvalidTile);
            }
            for y in tile.y..end_y {
                for x in tile.x..end_x {
                    if cancelled() {
                        return Err(ToneCurveExecutionError::Cancelled);
                    }
                    let index = y * image_width + x;
                    if covered[index] {
                        return Err(ToneCurveExecutionError::OverlappingTiles);
                    }
                    covered[index] = true;
                }
            }
        }
        if covered.iter().any(|covered| !covered) {
            return Err(ToneCurveExecutionError::IncompleteTiles);
        }

        let mut output = vec![ToneCurvePixel::from_channels([0.0; 4]); input.len()];
        for tile in tiles {
            let end_x = tile.x + tile.width;
            let end_y = tile.y + tile.height;
            for y in tile.y..end_y {
                for x in tile.x..end_x {
                    if cancelled() {
                        return Err(ToneCurveExecutionError::Cancelled);
                    }
                    let index = y * image_width + x;
                    output[index] = self.evaluate_pixel(input[index]);
                }
            }
        }
        Ok(ToneCurveExecution {
            pixels: output,
            input_format_problem: false,
        })
    }

    fn evaluate_pixel(&self, pixel: ToneCurvePixel) -> ToneCurvePixel {
        let input = pixel.channels;
        let l_input = input[0] / 100.0;
        let l_output = self.curves.channel(0).evaluate(l_input);
        let channels = match self.parameters.tonecurve_autoscale_ab {
            ToneCurveAutoscale::ManualLab => {
                let a_input = (input[1] + 128.0) / 256.0;
                let b_input = (input[2] + 128.0) / 256.0;
                [
                    l_output,
                    self.curves
                        .evaluate_ab(1, a_input, self.parameters.tonecurve_unbound_ab),
                    self.curves
                        .evaluate_ab(2, b_input, self.parameters.tonecurve_unbound_ab),
                ]
            }
            ToneCurveAutoscale::AutomaticLab => {
                if l_input > 0.01 {
                    [
                        l_output,
                        input[1] * l_output / input[0],
                        input[2] * l_output / input[0],
                    ]
                } else {
                    [
                        l_output,
                        input[1] * self.curves.low_approximation(),
                        input[2] * self.curves.low_approximation(),
                    ]
                }
            }
            ToneCurveAutoscale::AutomaticXyz => {
                let mut xyz = [0.0_f32; 3];
                lab_to_xyz([input[0], input[1], input[2]], &mut xyz);
                for value in &mut xyz {
                    *value = self.curves.channel(0).evaluate(*value);
                }
                let mut lab = [0.0_f32; 3];
                xyz_to_lab(xyz, &mut lab);
                lab
            }
            ToneCurveAutoscale::AutomaticRgb => {
                let mut rgb = [0.0_f32; 3];
                lab_to_prophoto([input[0], input[1], input[2]], &mut rgb);
                if self.parameters.preserve_colors == PreserveColors::None {
                    for value in &mut rgb {
                        *value = self.curves.channel(0).evaluate(*value);
                    }
                } else {
                    let luminance =
                        rgb_norm(rgb, self.parameters.preserve_colors, self.profile.as_ref());
                    let ratio = if luminance > 0.0 {
                        self.curves.channel(0).evaluate(luminance) / luminance
                    } else {
                        1.0
                    };
                    for value in &mut rgb {
                        *value *= ratio;
                    }
                }
                let mut lab = [0.0_f32; 3];
                prophoto_to_lab(rgb, &mut lab);
                lab
            }
        };
        ToneCurvePixel::new(channels[0], channels[1], channels[2], input[3])
    }
}

fn rgb_norm(
    rgb: [f32; 3],
    mode: PreserveColors,
    profile: Option<&ToneCurveProfileEvidence>,
) -> f32 {
    match mode {
        PreserveColors::None => unreachable!("None is handled before norm selection"),
        PreserveColors::Luminance => profile
            .expect("RGB luminance profile was checked at compile time")
            .luminance(rgb),
        PreserveColors::Max => rgb[0].max(rgb[1]).max(rgb[2]),
        PreserveColors::Average => (rgb[0] + rgb[1] + rgb[2]) / 3.0,
        PreserveColors::Sum => rgb[0] + rgb[1] + rgb[2],
        PreserveColors::Norm => (rgb[0] * rgb[0] + rgb[1] * rgb[1] + rgb[2] * rgb[2]).sqrt(),
        PreserveColors::Power => {
            let red = rgb[0] * rgb[0];
            let green = rgb[1] * rgb[1];
            let blue = rgb[2] * rgb[2];
            (rgb[0] * red + rgb[1] * green + rgb[2] * blue) / (red + green + blue)
        }
    }
}

/// Operation-local analogue of native commit state.
#[derive(Debug, Clone)]
pub struct ToneCurveRuntime {
    parameters: ToneCurveParametersV5,
    compiled: Option<CompiledToneCurveSet>,
    request_histogram: bool,
}

impl ToneCurveRuntime {
    #[must_use]
    pub fn new(parameters: ToneCurveParametersV5) -> Self {
        Self {
            parameters,
            compiled: None,
            request_histogram: true,
        }
    }

    /// Native preview request transition; compilation remains lazy and ordered
    /// by [`compile_parameters`].
    pub fn commit_params(&mut self, parameters: ToneCurveParametersV5, preview: bool) {
        self.parameters = parameters;
        self.compiled = None;
        self.request_histogram = preview;
    }

    pub fn plan(
        &mut self,
        profile: Option<ToneCurveProfileEvidence>,
    ) -> Result<ToneCurvePlan, ToneCurveExecutionError> {
        let compiled = compile_parameters(&self.parameters, profile.as_ref())?;
        self.compiled = Some(compiled.clone());
        Ok(ToneCurvePlan {
            parameters: self.parameters.clone(),
            curves: compiled,
            profile,
        })
    }

    #[must_use]
    pub const fn parameters(&self) -> &ToneCurveParametersV5 {
        &self.parameters
    }

    #[must_use]
    pub const fn request_histogram(&self) -> bool {
        self.request_histogram
    }

    #[must_use]
    pub const fn lut_is_built(&self) -> bool {
        self.compiled.is_some()
    }

    /// Native identity pipe table before the first commit.
    #[must_use]
    pub fn initial_table_value(&self, channel: usize, index: usize) -> f32 {
        let index = index as f32;
        match channel {
            0 => 100.0_f32 * index / LUT_RESOLUTION as f32,
            1 | 2 => 256.0_f32 * index / LUT_RESOLUTION as f32 - 128.0_f32,
            _ => 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToneCurveExecutionError {
    Curve(CurveCompileError),
    Cancelled,
    InvalidDimensions,
    InvalidTile,
    OverlappingTiles,
    IncompleteTiles,
}

impl From<CurveCompileError> for ToneCurveExecutionError {
    fn from(error: CurveCompileError) -> Self {
        Self::Curve(error)
    }
}

impl fmt::Display for ToneCurveExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Curve(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("Tone Curve execution was cancelled"),
            Self::InvalidDimensions => {
                formatter.write_str("Tone Curve raster dimensions are invalid")
            }
            Self::InvalidTile => formatter.write_str("Tone Curve tile is outside the raster"),
            Self::OverlappingTiles => formatter.write_str("Tone Curve tiles overlap"),
            Self::IncompleteTiles => {
                formatter.write_str("Tone Curve tiles do not cover the raster")
            }
        }
    }
}

impl std::error::Error for ToneCurveExecutionError {}

#[must_use]
pub const fn lut_resolution() -> u32 {
    LUT_RESOLUTION
}

#[must_use]
pub const fn channel_count() -> usize {
    CHANNELS
}
