//! Legacy `filmic` CPU execution at the Lab D50 boundary.
//!
//! The equations and constants are mapped from `_process_pixel()` and
//! `dt_Lab_to_XYZ()`/`dt_prophotorgb_to_Lab()` in `src/iop/filmic.c`,
//! `src/common/colorspaces_inline_conversions.h`, `src/common/dttypes.h`,
//! and `src/common/math.h`; the output lane follows `copy_pixel_nontemporal()`
//! in `src/develop/imageop.h`.  This leaf intentionally has no GPU pass,
//! profile dependency, mask generator, or registry route.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::excessive_precision,
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "the source uses bit-shaped f32 approximations and source naming"
)]

use super::codec::ParametersV3;
use super::curve::{CurveBuildError, LUT_SIZE, build_luts};

pub const EPS: f32 = 1.52587890625e-5_f32;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilmicPixel {
    channels: [f32; 4],
}

impl FilmicPixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
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

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FilmicPlan {
    parameters: ParametersV3,
    table: Vec<f32>,
    grad_2: Vec<f32>,
    grey_source: f32,
    black_source: f32,
    dynamic_range: f32,
    inverse_dynamic_range: f32,
    output_power: f32,
    global_saturation: f32,
    preserve_color: bool,
    effective_contrast: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FilmicPlanError {
    NonFiniteParameter,
    InvalidDerivedState(&'static str),
    Curve(CurveBuildError),
}

impl std::fmt::Display for FilmicPlanError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NonFiniteParameter => formatter.write_str("filmic parameter is non-finite"),
            Self::InvalidDerivedState(stage) => {
                write!(formatter, "filmic derived state is invalid: {stage}")
            }
            Self::Curve(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for FilmicPlanError {}

impl From<CurveBuildError> for FilmicPlanError {
    fn from(error: CurveBuildError) -> Self {
        Self::Curve(error)
    }
}

impl FilmicPlan {
    pub fn from_parameters(parameters: ParametersV3) -> Result<Self, FilmicPlanError> {
        parameters
            .validate_finite()
            .map_err(|_| FilmicPlanError::NonFiniteParameter)?;
        let dynamic_range = parameters.white_point_source - parameters.black_point_source;
        let grey_source = parameters.grey_point_source / 100.0_f32;
        let grey_log = parameters.black_point_source.abs() / dynamic_range;
        let inverse_dynamic_range = 1.0_f32 / dynamic_range;
        if !dynamic_range.is_finite()
            || dynamic_range <= 0.0_f32
            || !inverse_dynamic_range.is_finite()
            || !grey_source.is_finite()
            || grey_source <= 0.0_f32
            || !grey_log.is_finite()
            || grey_log <= 0.0_f32
        {
            return Err(FilmicPlanError::InvalidDerivedState("source luminance"));
        }
        // Native `commit_params()` uses the original grey target here, not
        // the clamped target used later by `compute_curve_lut()`.
        let grey_display =
            (parameters.grey_point_target / 100.0_f32).powf(1.0_f32 / parameters.output_power);
        if !grey_display.is_finite() {
            return Err(FilmicPlanError::InvalidDerivedState("target grey"));
        }
        let mut effective_contrast = parameters.contrast;
        if effective_contrast < grey_display / grey_log {
            // This is stored in native `d->contrast`, but the LUT builder is
            // passed the original parameter struct and therefore still uses
            // `parameters.contrast`.  Keep that source quirk explicit.
            effective_contrast = 1.0001_f32 * grey_display / grey_log;
        }
        if !effective_contrast.is_finite() {
            return Err(FilmicPlanError::InvalidDerivedState("effective contrast"));
        }
        let luts = build_luts(parameters)?;
        Ok(Self {
            parameters,
            table: luts.table,
            grad_2: luts.grad_2,
            grey_source,
            black_source: parameters.black_point_source,
            dynamic_range,
            inverse_dynamic_range,
            output_power: parameters.output_power,
            global_saturation: parameters.global_saturation,
            preserve_color: parameters.preserve_color != 0,
            effective_contrast,
        })
    }

    #[must_use]
    pub const fn parameters(&self) -> ParametersV3 {
        self.parameters
    }

    #[must_use]
    pub fn table(&self) -> &[f32] {
        &self.table
    }

    #[must_use]
    pub fn grad_2(&self) -> &[f32] {
        &self.grad_2
    }

    #[must_use]
    pub const fn dynamic_range(&self) -> f32 {
        self.dynamic_range
    }

    #[must_use]
    pub const fn effective_contrast(&self) -> f32 {
        self.effective_contrast
    }

    /// Executes an independent, zero-overlap tile.
    ///
    /// The native CPU leaf writes all four lanes.  Its padded fourth lane is a
    /// zero spare value; preserving an input alpha here would diverge from the
    /// CPU path (the native `OpenCL` path is intentionally unavailable).
    ///
    /// Cancellation polling and budget-aware fallible allocation are deliberately
    /// deferred to the pixelpipe seam; this leaf is not a direct production route.
    #[must_use]
    pub fn execute_tile(&self, input: &[FilmicPixel]) -> Vec<FilmicPixel> {
        input
            .iter()
            .copied()
            .map(|pixel| self.process(pixel))
            .collect()
    }

    /// Executes a complete raster in row-major carrier order.
    #[must_use]
    pub fn execute(&self, input: &[FilmicPixel]) -> Vec<FilmicPixel> {
        self.execute_tile(input)
    }

    fn process(&self, pixel: FilmicPixel) -> FilmicPixel {
        let input = pixel.channels;
        let xyz = lab_d50_to_xyz(input);
        let input_rgb_with_spare = xyz_d50_to_prophoto_rgb(xyz);
        let mut input_rgb = [
            input_rgb_with_spare[0],
            input_rgb_with_spare[1],
            input_rgb_with_spare[2],
        ];

        let desaturate = self.global_saturation != 100.0_f32;
        let saturation = self.global_saturation / 100.0_f32;
        let concavity;
        let mut luma;
        let mut rgb;

        if desaturate {
            luma = xyz[1];
            for channel in &mut input_rgb {
                *channel = luma + saturation * (*channel - luma);
            }
        }

        if self.preserve_color {
            let maximum = max3(input_rgb);
            let ratios = [
                input_rgb[0] / maximum,
                input_rgb[1] / maximum,
                input_rgb[2] / maximum,
            ];
            let mut mapped = maximum / self.grey_source;
            mapped = if mapped > EPS {
                (fastlog2(mapped) - self.black_source) * self.inverse_dynamic_range
            } else {
                EPS
            };
            mapped = clamp(mapped, 0.0_f32, 1.0_f32);
            let index = lut_index(mapped);
            mapped = self.table[index];
            concavity = self.grad_2[index];
            rgb = [ratios[0] * mapped, ratios[1] * mapped, ratios[2] * mapped];
            luma = mapped;
        } else {
            for channel in &mut input_rgb {
                *channel /= self.grey_source;
            }
            let log_rgb = [
                vector_log2(input_rgb[0]),
                vector_log2(input_rgb[1]),
                vector_log2(input_rgb[2]),
            ];
            let mut mapped = [0.0_f32; 3];
            for channel in 0..3 {
                mapped[channel] = if input_rgb[channel] > EPS {
                    (log_rgb[channel] - self.black_source) * self.inverse_dynamic_range
                } else {
                    EPS
                };
                mapped[channel] = clamp(mapped[channel], 0.0_f32, 1.0_f32);
            }
            let xyz_luma = prophoto_to_xyz_luma(mapped);
            concavity = self.grad_2[lut_index(xyz_luma)];
            rgb = [
                self.table[lut_index(mapped[0])],
                self.table[lut_index(mapped[1])],
                self.table[lut_index(mapped[2])],
            ];
            luma = prophoto_to_xyz_luma(rgb);
        }

        for channel in &mut rgb {
            *channel = clamp(luma + concavity * (*channel - luma), 0.0_f32, 1.0_f32);
        }
        let powered = vector_pow(rgb, self.output_power);
        let result = prophoto_rgb_to_lab([powered[0], powered[1], powered[2], 0.0_f32]);
        FilmicPixel::from_channels(result)
    }
}

pub fn lab_d50_to_xyz(lab: [f32; 4]) -> [f32; 4] {
    let fy = (lab[0] + 16.0_f32) * (1.0_f32 / 116.0_f32);
    let fx = (lab[1] + 0.0_f32) * (1.0_f32 / 500.0_f32) + fy;
    let fz = (lab[2] + 0.0_f32) * (-1.0_f32 / 200.0_f32) + fy;
    [
        0.9642_f32 * lab_f_inv(fx),
        lab_f_inv(fy),
        0.8249_f32 * lab_f_inv(fz),
        0.0_f32,
    ]
}

pub fn prophoto_rgb_to_lab(rgb: [f32; 4]) -> [f32; 4] {
    let xyz = [
        0.7976749_f32 * rgb[0] + 0.1351917_f32 * rgb[1] + 0.0313534_f32 * rgb[2],
        0.2880402_f32 * rgb[0] + 0.7118741_f32 * rgb[1] + 0.0000857_f32 * rgb[2],
        0.8252100_f32 * rgb[2],
        0.0_f32,
    ];
    xyz_to_lab(xyz)
}

fn xyz_to_lab(xyz: [f32; 4]) -> [f32; 4] {
    let x = xyz[0] * (1.0_f32 / 0.9642_f32);
    let y = xyz[1] * 1.0_f32;
    let z = xyz[2] * (1.0_f32 / 0.8249_f32);
    let fx = lab_f(x);
    let fy = lab_f(y);
    let fz = lab_f(z);
    [
        116.0_f32 * fy - 16.0_f32,
        500.0_f32 * (fx - fy),
        -200.0_f32 * (fz - fy),
        0.0_f32,
    ]
}

pub fn xyz_d50_to_prophoto_rgb(xyz: [f32; 4]) -> [f32; 4] {
    [
        1.3459433_f32 * xyz[0] - 0.2556075_f32 * xyz[1] - 0.0511118_f32 * xyz[2],
        -0.5445989_f32 * xyz[0] + 1.5081673_f32 * xyz[1] + 0.0205351_f32 * xyz[2],
        1.2118128_f32 * xyz[2],
        0.0_f32,
    ]
}

fn prophoto_to_xyz_luma(rgb: [f32; 3]) -> f32 {
    0.1351917_f32 * rgb[0] + 0.7118741_f32 * rgb[1] + 0.0000857_f32 * rgb[2]
}

fn lab_f_inv(value: f32) -> f32 {
    let epsilon = 0.20689655172413796_f32;
    let kappa = 24389.0_f32 / 27.0_f32;
    if value > epsilon {
        value * value * value
    } else {
        (116.0_f32 * value - 16.0_f32) / kappa
    }
}

fn lab_f(value: f32) -> f32 {
    let epsilon = 216.0_f32 / 24389.0_f32;
    let kappa = 24389.0_f32 / 27.0_f32;
    if value > epsilon {
        value.cbrt()
    } else {
        (kappa * value + 16.0_f32) / 116.0_f32
    }
}

#[inline]
fn max3(values: [f32; 3]) -> f32 {
    let first = if values[0] > values[1] {
        values[0]
    } else {
        values[1]
    };
    if first > values[2] { first } else { values[2] }
}

#[inline]
fn clamp(value: f32, lower: f32, upper: f32) -> f32 {
    if value >= lower {
        if value <= upper { value } else { upper }
    } else {
        lower
    }
}

#[inline]
pub fn lut_index(value: f32) -> usize {
    let value = clamp(value, 0.0_f32, 1.0_f32);
    let index = (value * LUT_SIZE as f32) as usize;
    index.min(LUT_SIZE - 1)
}

/// The scalar preserve-color path uses the exact bit-level approximation from
/// `common/math.h`; this is intentionally not `f32::log2()`.
#[inline]
pub fn fastlog2(value: f32) -> f32 {
    let bits = value.to_bits();
    let mantissa = f32::from_bits((bits & 0x007f_ffff) | 0x3f00_0000);
    let exponent = (bits as f32) * 1.1920928955078125e-7_f32;
    exponent
        - 124.22551499_f32
        - 1.498030302_f32 * mantissa
        - 1.72587999_f32 / (0.3520887068_f32 + mantissa)
}

/// Degree-five `dt_vector_log2()` approximation, including its bit split.
#[inline]
pub fn vector_log2(value: f32) -> f32 {
    let bits = value.to_bits();
    let mantissa = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    let exponent = ((bits & 0x7f80_0000) >> 23) as f32 - 127.0_f32;
    let mut logmantissa = 0.0596515482674574969533_f32 * mantissa - 0.465725644288844778798_f32;
    logmantissa = logmantissa * mantissa + 1.48116647521213171641_f32;
    logmantissa = logmantissa * mantissa - 2.52074962577807006663_f32;
    logmantissa = logmantissa * mantissa + 2.8882704548164776201_f32;
    logmantissa * (mantissa - 1.0_f32) + exponent
}

#[inline]
fn vector_pow(value: [f32; 3], power: f32) -> [f32; 3] {
    [
        vector_exp2(vector_log2(value[0]) * power),
        vector_exp2(vector_log2(value[1]) * power),
        vector_exp2(vector_log2(value[2]) * power),
    ]
}

/// Degree-four `dt_vector_exp2()` approximation with native ARM scalar rounding.
#[inline]
pub fn vector_exp2(value: f32) -> f32 {
    let x = clamp(value, -126.99999_f32, 129.0_f32);
    let adjusted = x - 0.5_f32;
    // `dt_vector_round()` uses `roundf()` in the ARM scalar path, whose exact
    // halfway rule is ties-away-from-zero rather than ties-to-even.
    let integer = round_away_from_zero(adjusted);
    let fraction = x - integer;
    let mut polynomial = 1.3534167e-2_f32 * fraction + 5.2011464e-2_f32;
    polynomial = polynomial * fraction + 2.4144275e-1_f32;
    polynomial = polynomial * fraction + 6.9300383e-1_f32;
    polynomial = polynomial * fraction + 1.0000026_f32;
    let exponent = ((127 + integer as i32) << 23) as u32;
    f32::from_bits(exponent) * polynomial
}

#[inline]
fn round_away_from_zero(value: f32) -> f32 {
    value.round()
}

// Keep this assertion next to the source-sized LUT contract.
const _: () = assert!(LUT_SIZE == 0x10000);
