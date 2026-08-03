#![expect(
    clippy::suboptimal_flops,
    reason = "Native Color Balance RGB execution order is preserved for IEEE-754 parity."
)]

//! Native CPU `process`, `commit_params`, and gamut-LUT preparation.
//!
//! Direct source lineage: `src/iop/colorbalancergb.c` (`commit_params`,
//! `opacity_masks`, `process`) and the coupled conversion helpers named by
//! `math.rs`.  This module intentionally stops before shared pixelpipe
//! routing, external blend-if evaluation, GUI mask preview, and GPU/OpenCL.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::large_types_passed_by_value,
    clippy::manual_midpoint,
    clippy::match_same_arms,
    clippy::needless_range_loop,
    clippy::struct_excessive_bools,
    clippy::trivially_copy_pass_by_ref,
    clippy::unreadable_literal,
    clippy::float_cmp,
    clippy::missing_errors_doc,
    clippy::similar_names,
    clippy::too_many_arguments,
    reason = "native source-shaped f32 raster code and exact ABI boundary"
)]

use std::fmt;
use std::mem::size_of;

use rusttable_processing::{FiniteF32, RasterDimensions};

use super::codec::{ColorBalanceRgbParametersV5, ColorBalanceRgbSaturationFormula, RGB_CHANNELS};
use super::math::{
    self, LUT_ELEM, Matrix3, Pixel, apply, build_jz_gamut_lut, build_ucs_gamut_lut,
    gamut_check_yrg, grading_rgb_to_lms, jzazbz_to_xyz, lms_to_grading_rgb, lms_to_xyz, lms_to_yrg,
    lookup_gamut, make_ych, soft_clip, ucs_hcb_to_jch, ucs_hsb_to_jch, ucs_jch_to_hcb,
    ucs_jch_to_hsb, xyz_to_jzazbz, xyz_to_xyy, ych_to_grading_rgb, yrg_to_lms, yrg_to_ych,
};

pub const COLORBALANCERGB_COMPATIBILITY_ID: &str = "colorbalancergb";
pub const COLORBALANCERGB_RUST_ID: &str = "rusttable.colorbalancergb";
pub const MASK_LUMA_EXPONENT: f32 = 0.4101205819200422;
pub const ANGLE_SHIFT_DEGREES: f32 = -30.0;

/// The two operation-local native input/output matrix sides.  Matrices are
/// conventional output-row matrices: RGB→XYZ D50 and XYZ D50→RGB.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceRgbProfile {
    input_rgb_to_xyz_d50: Matrix3,
    output_xyz_d50_to_rgb: Matrix3,
}

impl ColorBalanceRgbProfile {
    pub fn new(
        input_rgb_to_xyz_d50: Matrix3,
        output_xyz_d50_to_rgb: Matrix3,
    ) -> Result<Self, ColorBalanceRgbProfileError> {
        if input_rgb_to_xyz_d50
            .into_iter()
            .flatten()
            .chain(output_xyz_d50_to_rgb.into_iter().flatten())
            .any(|value| !value.is_finite())
        {
            return Err(ColorBalanceRgbProfileError::NonFiniteMatrix);
        }
        Ok(Self {
            input_rgb_to_xyz_d50,
            output_xyz_d50_to_rgb,
        })
    }

    #[must_use]
    pub const fn identity() -> Self {
        Self {
            input_rgb_to_xyz_d50: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            output_xyz_d50_to_rgb: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }

    #[must_use]
    pub const fn input_rgb_to_xyz_d50(self) -> Matrix3 {
        self.input_rgb_to_xyz_d50
    }

    #[must_use]
    pub const fn output_xyz_d50_to_rgb(self) -> Matrix3 {
        self.output_xyz_d50_to_rgb
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBalanceRgbProfileError {
    NonFiniteMatrix,
}

impl fmt::Display for ColorBalanceRgbProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteMatrix => {
                formatter.write_str("colorbalancergb profile matrix is non-finite")
            }
        }
    }
}

impl std::error::Error for ColorBalanceRgbProfileError {}

/// Finite committed parameters. Native UI ranges are metadata, not execution
/// clamps; finite outliers are retained exactly for the source equations.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceRgbConfig {
    parameters: ColorBalanceRgbParametersV5,
    finite_values: [FiniteF32; 32],
}

impl ColorBalanceRgbConfig {
    pub fn new(
        parameters: ColorBalanceRgbParametersV5,
    ) -> Result<Self, ColorBalanceRgbParameterError> {
        let values = parameters.v4.values();
        let mut finite_values = [FiniteF32::new(0.0).expect("zero is finite"); 32];
        for (index, value) in values.into_iter().enumerate() {
            finite_values[index] =
                FiniteF32::new(value).map_err(|_| ColorBalanceRgbParameterError::NonFinite {
                    field: parameter_name(index),
                    index: array_index(index),
                })?;
        }
        Ok(Self {
            parameters,
            finite_values,
        })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(ColorBalanceRgbParametersV5::defaults())
            .expect("native Color Balance RGB defaults are finite")
    }

    #[must_use]
    pub const fn parameters(self) -> ColorBalanceRgbParametersV5 {
        self.parameters
    }

    #[must_use]
    pub const fn value(self, index: usize) -> f32 {
        self.finite_values[index].get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBalanceRgbParameterError {
    NonFinite {
        field: &'static str,
        index: Option<usize>,
    },
}

impl fmt::Display for ColorBalanceRgbParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite {
                field,
                index: Some(index),
            } => {
                write!(formatter, "colorbalancergb {field}[{index}] is non-finite")
            }
            Self::NonFinite { field, index: None } => {
                write!(formatter, "colorbalancergb {field} is non-finite")
            }
        }
    }
}

impl std::error::Error for ColorBalanceRgbParameterError {}

const fn parameter_name(index: usize) -> &'static str {
    match index {
        0..=2 => "shadows",
        3..=5 => "midtones",
        6..=8 => "highlights",
        9..=11 => "global",
        12 => "shadows_weight",
        13 => "white_fulcrum",
        14 => "highlights_weight",
        15..=18 => "chroma",
        19..=22 => "saturation",
        23 => "hue_angle",
        24..=27 => "brilliance",
        28 => "mask_grey_fulcrum",
        29 => "vibrance",
        30 => "grey_fulcrum",
        31 => "contrast",
        _ => "unknown",
    }
}

const fn array_index(index: usize) -> Option<usize> {
    match index {
        0..=2 => Some(index % 3),
        3..=5 => Some(index % 3),
        6..=8 => Some(index % 3),
        9..=11 => Some(index % 3),
        15..=22 => Some(index % 4),
        24..=27 => Some(index % 4),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceRgbCoefficients {
    pub global: Pixel,
    pub shadows: Pixel,
    pub midtones: Pixel,
    pub highlights: Pixel,
    pub chroma: Pixel,
    pub saturation: Pixel,
    pub brilliance: Pixel,
    pub vibrance: f32,
    pub contrast: f32,
    pub midtones_y: f32,
    pub shadows_weight: f32,
    pub highlights_weight: f32,
    pub midtones_weight: f32,
    pub mask_grey_fulcrum: f32,
    pub white_fulcrum: f32,
    pub grey_fulcrum: f32,
    pub hue_angle: f32,
    pub saturation_formula: ColorBalanceRgbSaturationFormula,
}

impl ColorBalanceRgbCoefficients {
    pub fn commit(config: ColorBalanceRgbConfig) -> Self {
        let p = config.parameters();
        let v1 = p.v4.v3.v2.v1;
        let v2 = p.v4.v3.v2;
        let v3 = p.v4.v3;
        let v4 = p.v4;
        let rgb_norm = ych_to_grading_rgb([1.0, 0.0, 1.0, 0.0]);
        let make = |y: f32, c: f32, h: f32| {
            ych_to_grading_rgb(make_ych(y, c, (h + ANGLE_SHIFT_DEGREES).to_radians()))
        };
        let global_raw = make(1.0, v1.global_c, v1.global_h);
        let global =
            std::array::from_fn(|c| (global_raw[c] - rgb_norm[c]) + rgb_norm[c] * v1.global_y);
        let shadows_raw = make(1.0, v1.shadows_c, v1.shadows_h);
        let shadows = std::array::from_fn(|c| 1.0 + (shadows_raw[c] - rgb_norm[c]) + v1.shadows_y);
        let highlights_raw = make(1.0, v1.highlights_c, v1.highlights_h);
        let highlights =
            std::array::from_fn(|c| 1.0 + (highlights_raw[c] - rgb_norm[c]) + v1.highlights_y);
        let midtones_raw = make(1.0, v1.midtones_c, v1.midtones_h);
        let midtones = std::array::from_fn(|c| 1.0 / (1.0 + (midtones_raw[c] - rgb_norm[c])));
        let shadows_weight = 2.0 + v1.shadows_weight * 2.0;
        let highlights_weight = 2.0 + v1.highlights_weight * 2.0;
        let shadows_weight_squared = shadows_weight * shadows_weight;
        let highlights_weight_squared = highlights_weight * highlights_weight;
        Self {
            global,
            shadows,
            midtones,
            highlights,
            // Lane 3 carries the native global control through this compact
            // grading plan; lanes 0..=2 remain shadow/midtone/highlight.
            chroma: [
                v1.chroma_shadows,
                v1.chroma_midtones,
                v1.chroma_highlights,
                v1.chroma_global,
            ],
            saturation: [
                v1.saturation_shadows,
                v1.saturation_midtones,
                v1.saturation_highlights,
                v1.saturation_global,
            ],
            brilliance: [
                v2.brilliance_shadows,
                v2.brilliance_midtones,
                v2.brilliance_highlights,
                v2.brilliance_global,
            ],
            vibrance: v4.vibrance,
            contrast: 1.0 + v4.contrast,
            midtones_y: 1.0 / (1.0 + v1.midtones_y),
            shadows_weight,
            highlights_weight,
            midtones_weight: shadows_weight_squared * highlights_weight_squared
                / (shadows_weight_squared + highlights_weight_squared),
            mask_grey_fulcrum: v3.mask_grey_fulcrum.powf(MASK_LUMA_EXPONENT),
            white_fulcrum: 2.0_f32.powf(v1.white_fulcrum),
            grey_fulcrum: v4.grey_fulcrum,
            hue_angle: v1.hue_angle.to_radians(),
            saturation_formula: p.saturation_formula,
        }
    }

    #[must_use]
    pub const fn formula(self) -> ColorBalanceRgbSaturationFormula {
        self.saturation_formula
    }

    #[must_use]
    pub const fn rgb_norm() -> Pixel {
        [1.0, 1.0, 1.0, 0.0]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceRgbMaskWeights {
    pub shadows: f32,
    pub midtones: f32,
    pub highlights: f32,
    pub shadows_complement: f32,
    pub midtones_complement: f32,
    pub highlights_complement: f32,
}

#[must_use]
pub fn opacity_masks(
    x: f32,
    coefficients: ColorBalanceRgbCoefficients,
) -> ColorBalanceRgbMaskWeights {
    let offset = x - coefficients.mask_grey_fulcrum;
    let offset_norm = offset / coefficients.mask_grey_fulcrum;
    let shadows = 1.0 / (1.0 + (offset_norm * coefficients.shadows_weight).exp());
    let highlights = 1.0 / (1.0 + (-offset_norm * coefficients.highlights_weight).exp());
    let shadows_complement = 1.0 - shadows;
    let highlights_complement = 1.0 - highlights;
    let midtones = (-(offset * offset) * coefficients.midtones_weight / 4.0).exp()
        * shadows_complement
        * shadows_complement
        * highlights_complement
        * highlights_complement
        * 8.0;
    ColorBalanceRgbMaskWeights {
        shadows,
        midtones,
        highlights,
        shadows_complement,
        midtones_complement: 1.0 - midtones,
        highlights_complement,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBalanceRgbExecutionError {
    DimensionsMismatch { expected: usize, actual: usize },
    AllocationFailed { required_bytes: usize },
    SizeOverflow,
    NonFiniteOutput { pixel: usize },
    Cancelled,
}

impl fmt::Display for ColorBalanceRgbExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsMismatch { expected, actual } => {
                write!(
                    formatter,
                    "colorbalancergb expected {expected} pixels, got {actual}"
                )
            }
            Self::AllocationFailed { required_bytes } => {
                write!(
                    formatter,
                    "colorbalancergb allocation failed for {required_bytes} bytes"
                )
            }
            Self::SizeOverflow => formatter.write_str("colorbalancergb raster size overflowed"),
            Self::NonFiniteOutput { pixel } => {
                write!(
                    formatter,
                    "colorbalancergb produced non-finite output at pixel {pixel}"
                )
            }
            Self::Cancelled => formatter.write_str("colorbalancergb execution was cancelled"),
        }
    }
}

impl std::error::Error for ColorBalanceRgbExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBalanceRgbCapabilityError {
    GpuUnavailable,
    GtkUnavailable,
    ExternalBlendDeferred,
    ProductionRoutingDeferred,
}

impl fmt::Display for ColorBalanceRgbCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GpuUnavailable => {
                formatter.write_str("colorbalancergb GPU execution is unavailable")
            }
            Self::GtkUnavailable => {
                formatter.write_str("colorbalancergb GTK controls are unavailable")
            }
            Self::ExternalBlendDeferred => {
                formatter.write_str("colorbalancergb outer blending is deferred")
            }
            Self::ProductionRoutingDeferred => {
                formatter.write_str("colorbalancergb production routing is deferred")
            }
        }
    }
}

impl std::error::Error for ColorBalanceRgbCapabilityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorBalanceRgbCapabilities {
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub gtk_supported: bool,
    pub internal_luma_masks: bool,
    pub external_mask_blending_deferred: bool,
    pub production_routing_deferred: bool,
    pub cpu_alpha_behavior: ColorBalanceRgbAlphaBehavior,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorBalanceRgbAlphaBehavior {
    NativeCpuFourthLaneZero,
}

impl ColorBalanceRgbCapabilities {
    #[must_use]
    pub const fn bounded_cpu_leaf() -> Self {
        Self {
            cpu_supported: true,
            gpu_supported: false,
            gtk_supported: false,
            internal_luma_masks: true,
            external_mask_blending_deferred: true,
            production_routing_deferred: true,
            cpu_alpha_behavior: ColorBalanceRgbAlphaBehavior::NativeCpuFourthLaneZero,
        }
    }

    pub const fn require_gpu(self) -> Result<(), ColorBalanceRgbCapabilityError> {
        if self.gpu_supported {
            Ok(())
        } else {
            Err(ColorBalanceRgbCapabilityError::GpuUnavailable)
        }
    }

    pub const fn require_gtk(self) -> Result<(), ColorBalanceRgbCapabilityError> {
        if self.gtk_supported {
            Ok(())
        } else {
            Err(ColorBalanceRgbCapabilityError::GtkUnavailable)
        }
    }

    pub const fn require_external_blending(self) -> Result<(), ColorBalanceRgbCapabilityError> {
        if self.external_mask_blending_deferred {
            Err(ColorBalanceRgbCapabilityError::ExternalBlendDeferred)
        } else {
            Ok(())
        }
    }
}

#[must_use]
pub const fn capabilities() -> ColorBalanceRgbCapabilities {
    ColorBalanceRgbCapabilities::bounded_cpu_leaf()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorBalanceRgbTiling {
    pub overlap_pixels: u32,
    pub alignment_pixels: u32,
    pub input_multiplier_milli: u32,
    pub output_multiplier_milli: u32,
    pub temporary_multiplier_milli: u32,
}

#[must_use]
pub const fn tiling() -> ColorBalanceRgbTiling {
    ColorBalanceRgbTiling {
        overlap_pixels: 0,
        alignment_pixels: 1,
        input_multiplier_milli: 1000,
        output_multiplier_milli: 1000,
        temporary_multiplier_milli: 1000,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ColorBalanceRgbPlan {
    config: ColorBalanceRgbConfig,
    coefficients: ColorBalanceRgbCoefficients,
    profile: ColorBalanceRgbProfile,
    input_matrix: Matrix3,
    output_matrix: Matrix3,
    gamut_lut: [f32; LUT_ELEM],
}

impl ColorBalanceRgbPlan {
    pub fn new(
        config: ColorBalanceRgbConfig,
        profile: ColorBalanceRgbProfile,
    ) -> Result<Self, ColorBalanceRgbExecutionError> {
        Self::new_with_cancel(config, profile, || false)
    }

    pub fn new_with_cancel<F: FnMut() -> bool>(
        config: ColorBalanceRgbConfig,
        profile: ColorBalanceRgbProfile,
        mut cancelled: F,
    ) -> Result<Self, ColorBalanceRgbExecutionError> {
        if cancelled() {
            return Err(ColorBalanceRgbExecutionError::Cancelled);
        }
        let coefficients = ColorBalanceRgbCoefficients::commit(config);
        // The native process matrix includes XYZ-D65 → LMS-D65, while the
        // gamut LUTs sample XYZ-D65 directly before their perceptual transform.
        let xyz_d65_input_matrix = math::xyz_d65_input_matrix(profile.input_rgb_to_xyz_d50());
        let input_matrix = math::input_matrix(profile.input_rgb_to_xyz_d50());
        let output_matrix = math::output_matrix(profile.output_xyz_d50_to_rgb());
        let gamut_lut = match coefficients.saturation_formula {
            ColorBalanceRgbSaturationFormula::JzAzBz => {
                build_jz_gamut_lut(xyz_d65_input_matrix, &mut cancelled)
                    .map_err(|()| ColorBalanceRgbExecutionError::Cancelled)?
            }
            ColorBalanceRgbSaturationFormula::DarktableUcs2022 => {
                build_ucs_gamut_lut(xyz_d65_input_matrix, &mut cancelled)
                    .map_err(|()| ColorBalanceRgbExecutionError::Cancelled)?
            }
        };
        Ok(Self {
            config,
            coefficients,
            profile,
            input_matrix,
            output_matrix,
            gamut_lut,
        })
    }

    #[must_use]
    pub const fn config(self) -> ColorBalanceRgbConfig {
        self.config
    }

    #[must_use]
    pub const fn coefficients(self) -> ColorBalanceRgbCoefficients {
        self.coefficients
    }

    #[must_use]
    pub const fn profile(self) -> ColorBalanceRgbProfile {
        self.profile
    }

    #[must_use]
    pub const fn gamut_lut(self) -> [f32; LUT_ELEM] {
        self.gamut_lut
    }

    pub fn execute(
        &self,
        dimensions: RasterDimensions,
        input: &[[f32; RGB_CHANNELS]],
    ) -> Result<Vec<[f32; RGB_CHANNELS]>, ColorBalanceRgbExecutionError> {
        self.execute_with_cancel(dimensions, input, || false)
    }

    /// Executes one identity-ROI tile.  Cancellation is checked before the
    /// raster and at every row; output is private until the whole tile passes.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        dimensions: RasterDimensions,
        input: &[[f32; RGB_CHANNELS]],
        mut cancelled: F,
    ) -> Result<Vec<[f32; RGB_CHANNELS]>, ColorBalanceRgbExecutionError> {
        let expected = usize::try_from(dimensions.pixel_count())
            .map_err(|_| ColorBalanceRgbExecutionError::SizeOverflow)?;
        if input.len() != expected {
            return Err(ColorBalanceRgbExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if cancelled() {
            return Err(ColorBalanceRgbExecutionError::Cancelled);
        }
        let width = usize::try_from(dimensions.width())
            .map_err(|_| ColorBalanceRgbExecutionError::SizeOverflow)?;
        let mut output = Vec::new();
        let required_bytes = expected
            .checked_mul(size_of::<[f32; RGB_CHANNELS]>())
            .ok_or(ColorBalanceRgbExecutionError::SizeOverflow)?;
        output
            .try_reserve_exact(expected)
            .map_err(|_| ColorBalanceRgbExecutionError::AllocationFailed { required_bytes })?;
        for (index, pixel) in input.iter().copied().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(ColorBalanceRgbExecutionError::Cancelled);
            }
            let processed = self.process_pixel(pixel);
            if processed[..3].iter().any(|value| !value.is_finite()) {
                return Err(ColorBalanceRgbExecutionError::NonFiniteOutput { pixel: index });
            }
            output.push(processed);
        }
        Ok(output)
    }

    fn process_pixel(&self, input: Pixel) -> Pixel {
        let rgb = [input[0].max(0.0), input[1].max(0.0), input[2].max(0.0), 0.0];
        let converted_lms = apply(self.input_matrix, [rgb[0], rgb[1], rgb[2]]);
        let mut lms = [converted_lms[0], converted_lms[1], converted_lms[2], 0.0];
        let mut yrg = lms_to_yrg(lms);
        let mut ych = yrg_to_ych(yrg);
        ych[0] = ych[0].max(0.0);
        let opacities = opacity_masks(ych[0].powf(MASK_LUMA_EXPONENT), self.coefficients);
        let cos_h = ych[2];
        let sin_h = ych[3];
        let cos_rotation = self.coefficients.hue_angle.cos();
        let sin_rotation = self.coefficients.hue_angle.sin();
        ych[2] = cos_rotation * cos_h - sin_rotation * sin_h;
        ych[3] = sin_rotation * cos_h + cos_rotation * sin_h;
        let chroma_boost = self.coefficients.chroma[3]
            + opacities.shadows * self.coefficients.chroma[0]
            + opacities.midtones * self.coefficients.chroma[1]
            + opacities.highlights * self.coefficients.chroma[2];
        let vibrance =
            self.coefficients.vibrance * (1.0 - ych[1].powf(self.coefficients.vibrance.abs()));
        let chroma_factor = (1.0 + chroma_boost + vibrance).max(0.0);
        ych[1] *= chroma_factor;
        gamut_check_yrg(&mut ych);
        yrg = math::ych_to_yrg(ych);
        lms = yrg_to_lms(yrg);
        let mut grading_rgb = lms_to_grading_rgb(lms);
        for c in 0..4 {
            grading_rgb[c] += self.coefficients.global[c];
        }
        for c in 0..4 {
            grading_rgb[c] *= opacities.highlights_complement
                * (opacities.shadows_complement + opacities.shadows * self.coefficients.shadows[c])
                + opacities.highlights * self.coefficients.highlights[c];
        }
        for c in 0..4 {
            let sign = if grading_rgb[c] < 0.0 { -1.0 } else { 1.0 };
            let absolute = grading_rgb[c].abs();
            let scaled = absolute / self.coefficients.white_fulcrum;
            grading_rgb[c] =
                scaled.powf(self.coefficients.midtones[c]) * sign * self.coefficients.white_fulcrum;
        }
        lms = grading_rgb_to_lms(grading_rgb);
        yrg = lms_to_yrg(lms);
        yrg[0] = (yrg[0] / self.coefficients.white_fulcrum)
            .max(0.0)
            .powf(self.coefficients.midtones_y)
            * self.coefficients.white_fulcrum;
        yrg[0] = self.coefficients.grey_fulcrum
            * (yrg[0] / self.coefficients.grey_fulcrum).powf(self.coefficients.contrast);
        lms = yrg_to_lms(yrg);
        let xyz_d65 = lms_to_xyz(lms);
        let xyz_d65 = match self.coefficients.saturation_formula {
            ColorBalanceRgbSaturationFormula::JzAzBz => self.jz_adjust(xyz_d65, opacities),
            ColorBalanceRgbSaturationFormula::DarktableUcs2022 => {
                self.ucs_adjust(xyz_d65, opacities)
            }
        };
        let output = apply(self.output_matrix, [xyz_d65[0], xyz_d65[1], xyz_d65[2]]);
        [
            output[0].max(0.0),
            output[1].max(0.0),
            output[2].max(0.0),
            0.0,
        ]
    }

    fn jz_adjust(&self, xyz_d65: Pixel, opacities: ColorBalanceRgbMaskWeights) -> Pixel {
        let jab = xyz_to_jzazbz(xyz_d65);
        let mut jc = [jab[0], jab[1].hypot(jab[2])];
        let hue = jab[2].atan2(jab[1]);
        let t = jc[1].atan2(jc[0]);
        let sin_t = t.sin();
        let cos_t = t.cos();
        let so0 = jc[0] * cos_t + jc[1] * sin_t;
        let saturation = self.coefficients.saturation[3]
            + opacities.shadows * self.coefficients.saturation[0]
            + opacities.midtones * self.coefficients.saturation[1]
            + opacities.highlights * self.coefficients.saturation[2];
        let brilliance = 1.0
            + self.coefficients.brilliance[3]
            + opacities.shadows * self.coefficients.brilliance[0]
            + opacities.midtones * self.coefficients.brilliance[1]
            + opacities.highlights * self.coefficients.brilliance[2];
        let so1 = so0 * (t * saturation).clamp(-t, std::f32::consts::FRAC_PI_2 - t);
        let so0 = (so0 * brilliance).max(0.0);
        jc[0] = (so0 * cos_t - so1 * sin_t).max(0.0);
        jc[1] = (so0 * sin_t + so1 * cos_t).max(0.0);
        let max_sat = lookup_gamut(&self.gamut_lut, hue);
        let saturation = if jc[0] > 0.0 {
            soft_clip(jc[1] / jc[0], 0.8 * max_sat, max_sat)
        } else {
            max_sat
        };
        let max_c_at_sat = jc[0] * saturation;
        let max_j_at_sat = if saturation > 0.0 {
            jc[1] / saturation
        } else {
            jc[0]
        };
        jc[0] = (jc[0] + max_j_at_sat) / 2.0;
        jc[1] = (jc[1] + max_c_at_sat) / 2.0;
        let cos_h = hue.cos();
        let sin_h = hue.sin();
        let d0 = 1.6295499532821566e-11;
        let d = -0.56;
        let mut iz = jc[0] + d0;
        iz /= 1.0 + d - d * iz;
        iz = iz.max(0.0);
        let ai = [
            [1.0, 0.1386050432715393, 0.0580473161561189],
            [1.0, -0.1386050432715393, -0.0580473161561189],
            [1.0, -0.0960192420263190, -0.8118918960560390],
        ];
        let lms = apply(ai, [iz, jc[1] * cos_h, jc[1] * sin_h]);
        let mut max_c = jc[1];
        for lane in 0..3 {
            if lms[lane] < 0.0 {
                let denominator = ai[lane][1] * cos_h + ai[lane][2] * sin_h;
                max_c = (-iz / denominator).min(max_c);
            }
        }
        jzazbz_to_xyz([jc[0], max_c * cos_h, max_c * sin_h, 0.0])
    }

    fn ucs_adjust(&self, xyz_d65: Pixel, opacities: ColorBalanceRgbMaskWeights) -> Pixel {
        let l_white = math::y_to_ucs_lstar(self.coefficients.white_fulcrum);
        let xyy = xyz_to_xyy(xyz_d65);
        let mut jch = math::xyy_to_ucs_jch(xyy, l_white);
        let mut hcb = ucs_jch_to_hcb(jch);
        let radius = hcb[1].hypot(hcb[2]);
        let sin_t = if radius > 0.0 { hcb[1] / radius } else { 0.0 };
        let cos_t = if radius > 0.0 { hcb[2] / radius } else { 0.0 };
        let p = f32::MIN_POSITIVE.max(hcb[1]);
        let w = sin_t * hcb[1] + cos_t * hcb[2];
        let saturation = self.coefficients.saturation[3]
            + opacities.shadows * self.coefficients.saturation[0]
            + opacities.midtones * self.coefficients.saturation[1]
            + opacities.highlights * self.coefficients.saturation[2];
        let brilliance = self.coefficients.brilliance[3]
            + opacities.shadows * self.coefficients.brilliance[0]
            + opacities.midtones * self.coefficients.brilliance[1]
            + opacities.highlights * self.coefficients.brilliance[2];
        let a = soft_clip(
            (1.0 + saturation).max(0.0),
            0.5 * (p.hypot(w) / p),
            p.hypot(w) / p,
        );
        let b = (1.0 + brilliance).max(0.0);
        let p_prime = (a - 1.0) * p;
        let w_prime = (p * p * (1.0 - a * a) + w * w).sqrt() * b;
        hcb[1] = (cos_t * p_prime + sin_t * w_prime).max(0.0);
        hcb[2] = (-sin_t * p_prime + cos_t * w_prime).max(0.0);
        jch = ucs_hcb_to_jch(hcb);
        let max_colorfulness = lookup_gamut(&self.gamut_lut, jch[2]);
        let max_chroma = 15.932993652962535
            * (jch[0] * l_white).powf(0.6523997524738018)
            * max_colorfulness.powf(0.6007557017508491)
            / l_white;
        let hsb_boundary = ucs_jch_to_hsb([jch[0], max_chroma, jch[2], 0.0]);
        let mut hsb = [
            hcb[0],
            if hcb[2] > 0.0 { hcb[1] / hcb[2] } else { 0.0 },
            hcb[2],
            0.0,
        ];
        hsb[1] = soft_clip(hsb[1], 0.8 * hsb_boundary[1], hsb_boundary[1]);
        jch = ucs_hsb_to_jch(hsb);
        math::xyy_to_xyz(math::ucs_jch_to_xyy(jch, l_white))
    }
}
