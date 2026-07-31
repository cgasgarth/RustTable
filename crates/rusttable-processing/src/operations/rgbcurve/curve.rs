//! RGB Curve LUT and extrapolation ported from `src/iop/rgbcurve.c`,
//! `src/common/curve_tools.c`, `src/gui/draw.h`, and `src/develop/imageop_math.h`.
//!
//! The shared Rust V1 sampler is used deliberately. The OpenCL lookup seam is
//! not advertised here because native CPU and OpenCL extrapolation disagree for
//! a final anchor below one.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::many_single_char_names,
    reason = "the source algorithm uses f32 index and interpolation arithmetic"
)]

use std::fmt;

#[cfg(not(test))]
use crate::common::curve_tools::{
    Curve, CurveAnchor, CurveBounds, CurveError, CurveType, interpolate_value_v1, sample_curve_v1,
};
#[cfg(test)]
use rusttable_processing::common::curve_tools::{
    Curve, CurveAnchor, CurveBounds, CurveError, CurveType, interpolate_value_v1, sample_curve_v1,
};

use super::parameters::{
    CHANNELS, LUT_RESOLUTION, MAX_NODES, PreserveColors, RgbCurveParametersV1, RgbCurveType,
};

/// Profile matrices passed to this leaf are ordinary non-transposed,
/// row-major `matrix_in`/`matrix_out` values. This is the orientation of the
/// fields in native `dt_iop_order_iccprofile_info_t`; the native transposed
/// SIMD fields are represented by row multiplication here.
pub const PROFILE_MATRIX_ORIENTATION: &str =
    "matrix_in and matrix_out are row-major (non-transposed) 3x3 matrices";

/// Native profile evidence needed for profile-backed luminance and middle-grey
/// compensation. The shared `WorkingFrameDescriptor` intentionally does not
/// carry these ICC LUTs, so callers must supply this evidence explicitly.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbCurveProfileEvidence {
    pub profile_type: i32,
    pub filename: Vec<u8>,
    /// Native `matrix_in` in row-major, non-transposed orientation.
    pub matrix_in: [[f32; 3]; 3],
    /// Native `matrix_out` in row-major, non-transposed orientation.
    pub matrix_out: [[f32; 3]; 3],
    pub lut_in: [Vec<f32>; 3],
    pub lut_out: [Vec<f32>; 3],
    pub unbounded_coeffs_in: [[f32; 3]; 3],
    pub unbounded_coeffs_out: [[f32; 3]; 3],
    /// Native `nonlinearlut`, independent of the matrix and LUT payloads.
    pub nonlinearlut: bool,
}

impl RgbCurveProfileEvidence {
    /// Creates linear profile evidence with native matrix semantics.
    #[must_use]
    pub fn new_linear(
        profile_type: i32,
        filename: impl Into<Vec<u8>>,
        matrix_in: [[f32; 3]; 3],
        matrix_out: [[f32; 3]; 3],
    ) -> Self {
        Self {
            profile_type,
            filename: filename.into(),
            matrix_in,
            matrix_out,
            lut_in: std::array::from_fn(|_| vec![0.0, 1.0]),
            lut_out: std::array::from_fn(|_| vec![0.0, 1.0]),
            unbounded_coeffs_in: [[-1.0, 0.0, 1.0]; 3],
            unbounded_coeffs_out: [[-1.0, 0.0, 1.0]; 3],
            nonlinearlut: false,
        }
    }

    /// Creates complete ICC-like evidence for source-derived TRC tests.
    ///
    /// The final argument is the native `nonlinearlut` field; it is not
    /// inferred from the presence of LUT samples or extrapolation coefficients.
    pub fn new_with_trc(
        profile_type: i32,
        filename: impl Into<Vec<u8>>,
        matrix_in: [[f32; 3]; 3],
        matrix_out: [[f32; 3]; 3],
        lut_in: [Vec<f32>; 3],
        lut_out: [Vec<f32>; 3],
        unbounded_coeffs_in: [[f32; 3]; 3],
        unbounded_coeffs_out: [[f32; 3]; 3],
        nonlinearlut: bool,
    ) -> Result<Self, RgbCurveProfileError> {
        let evidence = Self {
            profile_type,
            filename: filename.into(),
            matrix_in,
            matrix_out,
            lut_in,
            lut_out,
            unbounded_coeffs_in,
            unbounded_coeffs_out,
            nonlinearlut,
        };
        evidence.validate()?;
        Ok(evidence)
    }

    fn validate(&self) -> Result<(), RgbCurveProfileError> {
        for matrix in [self.matrix_in, self.matrix_out] {
            if matrix.iter().flatten().any(|value| !value.is_finite()) {
                return Err(RgbCurveProfileError::NonFiniteMatrix);
            }
        }
        if self.nonlinearlut {
            let lutsize = self.lut_in[0].len();
            for lut in self.lut_in.iter().chain(self.lut_out.iter()) {
                if lut.len() != lutsize
                    || lut.len() < 2
                    || lut.iter().any(|value| !value.is_finite())
                {
                    return Err(RgbCurveProfileError::InvalidLut);
                }
            }
            for coefficients in self
                .unbounded_coeffs_in
                .iter()
                .chain(self.unbounded_coeffs_out.iter())
            {
                if coefficients.iter().any(|value| !value.is_finite()) {
                    return Err(RgbCurveProfileError::NonFiniteCoefficients);
                }
            }
        }
        Ok(())
    }

    /// Returns the bounded native profile cache key (type plus 512 bytes).
    #[must_use]
    pub fn cache_key(&self) -> RgbCurveProfileCacheKey {
        let mut filename = [0_u8; 512];
        let source_len = self
            .filename
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(self.filename.len());
        let copy_len = source_len.min(filename.len().saturating_sub(1));
        filename[..copy_len].copy_from_slice(&self.filename[..copy_len]);
        RgbCurveProfileCacheKey {
            profile_type: self.profile_type,
            filename,
        }
    }

    /// Native `dt_rgb_norm(..., DT_RGB_NORM_LUMINANCE, profile)`.
    pub fn luminance(&self, rgb: [f32; 3]) -> f32 {
        let rgb = if self.nonlinearlut {
            [
                apply_trc(
                    rgb[0],
                    &self.lut_in[0],
                    self.unbounded_coeffs_in[0],
                    self.lut_in[0].len(),
                ),
                apply_trc(
                    rgb[1],
                    &self.lut_in[1],
                    self.unbounded_coeffs_in[1],
                    self.lut_in[1].len(),
                ),
                apply_trc(
                    rgb[2],
                    &self.lut_in[2],
                    self.unbounded_coeffs_in[2],
                    self.lut_in[2].len(),
                ),
            ]
        } else {
            rgb
        };
        self.matrix_in[1][0] * rgb[0]
            + self.matrix_in[1][1] * rgb[1]
            + self.matrix_in[1][2] * rgb[2]
    }

    /// Native `dt_ioppr_compensate_middle_grey` for one neutral value.
    pub fn compensate_middle_grey(&self, value: f32) -> f32 {
        let rgb = if self.nonlinearlut {
            [
                apply_trc(
                    value,
                    &self.lut_in[0],
                    self.unbounded_coeffs_in[0],
                    self.lut_in[0].len(),
                ),
                apply_trc(
                    value,
                    &self.lut_in[1],
                    self.unbounded_coeffs_in[1],
                    self.lut_in[1].len(),
                ),
                apply_trc(
                    value,
                    &self.lut_in[2],
                    self.unbounded_coeffs_in[2],
                    self.lut_in[2].len(),
                ),
            ]
        } else {
            [value; 3]
        };
        let xyz = multiply_row_major(rgb, self.matrix_in);
        xyz_to_lab(xyz)[0] * 0.01
    }

    /// Native `dt_ioppr_uncompensate_middle_grey` for one normalized Lab L.
    pub fn uncompensate_middle_grey(&self, value: f32) -> f32 {
        let xyz = lab_to_xyz([value * 100.0, 0.0, 0.0]);
        let rgb = multiply_row_major(xyz, self.matrix_out);
        if self.nonlinearlut {
            apply_trc(
                rgb[0],
                &self.lut_out[0],
                self.unbounded_coeffs_out[0],
                self.lut_out[0].len(),
            )
        } else {
            rgb[0]
        }
    }
}

/// Bounded cache identity matching native profile type plus filename.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbCurveProfileCacheKey {
    pub profile_type: i32,
    pub filename: [u8; 512],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbCurveProfileError {
    NonFiniteMatrix,
    InvalidLut,
    NonFiniteCoefficients,
}

impl fmt::Display for RgbCurveProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteMatrix => "RGB Curve profile matrix is non-finite",
            Self::InvalidLut => "RGB Curve profile LUT is too short or non-finite",
            Self::NonFiniteCoefficients => {
                "RGB Curve profile extrapolation coefficients are non-finite"
            }
        })
    }
}

impl std::error::Error for RgbCurveProfileError {}

/// A compiled native 65536-entry curve and right-side exponential fit.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledCurve {
    table: Vec<f32>,
    coefficients: [f32; 3],
    final_x: f32,
    extrapolation_threshold: f32,
}

impl CompiledCurve {
    pub fn from_nodes(
        nodes: &[CurveAnchor],
        curve_type: RgbCurveType,
    ) -> Result<Self, CurveCompileError> {
        let curve = Curve::new(curve_type.into(), CurveBounds::unit(), nodes)?;
        let samples = sample_curve_v1(&curve, LUT_RESOLUTION, LUT_RESOLUTION)?;
        let table: Vec<f32> = samples
            .into_iter()
            .map(|value| f32::from(value) / 65_536.0)
            .collect();
        let final_x = nodes.last().ok_or(CurveCompileError::TooFewNodes)?.x();
        if !(final_x > 0.0) || !final_x.is_finite() {
            return Err(CurveCompileError::InvalidFinalX);
        }
        let mut x = [0.0_f32; 4];
        let mut y = [0.0_f32; 4];
        for index in 0..4 {
            x[index] = [0.7, 0.8, 0.9, 1.0][index] * final_x;
            y[index] = table[native_index(x[index])];
        }
        let coefficients = estimate_exp(x, y);
        if coefficients.iter().any(|value| !value.is_finite()) {
            return Err(CurveCompileError::NonFiniteExtrapolation);
        }
        // Native process() derives its branch threshold from the stored fit,
        // rather than reusing the final anchor: `1.0f / coeffs[0]`.
        let extrapolation_threshold = 1.0 / coefficients[0];
        if !extrapolation_threshold.is_finite() {
            return Err(CurveCompileError::NonFiniteExtrapolation);
        }
        Ok(Self {
            table,
            coefficients,
            final_x,
            extrapolation_threshold,
        })
    }

    #[must_use]
    pub fn table(&self) -> &[f32] {
        &self.table
    }

    #[must_use]
    pub const fn coefficients(&self) -> [f32; 3] {
        self.coefficients
    }

    #[must_use]
    pub const fn final_x(&self) -> f32 {
        self.final_x
    }

    /// Native CPU branch threshold, exactly `1.0f / coeffs[0]`.
    #[must_use]
    pub const fn extrapolation_threshold(&self) -> f32 {
        self.extrapolation_threshold
    }

    /// Native CPU `process()` lookup. Equality at the coefficient-derived
    /// threshold takes the exponential branch, unlike the retained OpenCL
    /// helper's `x < 1` gate.
    #[must_use]
    pub fn evaluate(&self, input: f32) -> f32 {
        if input < self.extrapolation_threshold {
            self.table[native_index(input)]
        } else {
            self.coefficients[1] * (input * self.coefficients[0]).powf(self.coefficients[2])
        }
    }

    /// Native GUI direct evaluation, recomputed and clamped to the unit box.
    pub fn evaluate_gui(
        &self,
        nodes: &[CurveAnchor],
        curve_type: RgbCurveType,
        input: f32,
    ) -> Result<f32, CurveCompileError> {
        let value = interpolate_value_v1(nodes, input, curve_type.into())?;
        Ok(value.clamp(0.0, 1.0))
    }
}

/// Three channel LUTs compiled from one checked parameter payload.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledCurveSet {
    channels: [CompiledCurve; CHANNELS],
}

impl CompiledCurveSet {
    #[must_use]
    pub const fn channel(&self, channel: usize) -> &CompiledCurve {
        &self.channels[channel]
    }

    #[must_use]
    pub const fn channels(&self) -> &[CompiledCurve; CHANNELS] {
        &self.channels
    }
}

/// Compiles all three channels, applying native profile uncompensation when
/// the editor's middle-grey flag is enabled. If no work profile exists, native
/// `_generate_curve_lut()` compiles the raw parameter nodes; this leaf retains
/// that behavior rather than inventing a profile requirement.
pub fn compile_parameters(
    parameters: &RgbCurveParametersV1,
    profile: Option<&RgbCurveProfileEvidence>,
) -> Result<CompiledCurveSet, CurveCompileError> {
    parameters
        .validate()
        .map_err(CurveCompileError::InvalidParameters)?;
    if let Some(profile) = profile {
        profile
            .validate()
            .map_err(CurveCompileError::InvalidProfile)?;
    }
    let nodes = |channel: usize| {
        let count = usize::try_from(parameters.curve_num_nodes[channel]).expect("validated count");
        let mut result = [CurveAnchor::new(0.0, 0.0); MAX_NODES];
        for (index, node) in parameters.curve_nodes[channel][..count].iter().enumerate() {
            let (x, y) = if parameters.compensate_middle_grey {
                if let Some(profile) = profile {
                    (
                        profile.uncompensate_middle_grey(node.x),
                        profile.uncompensate_middle_grey(node.y),
                    )
                } else {
                    (node.x, node.y)
                }
            } else {
                (node.x, node.y)
            };
            result[index] = CurveAnchor::new(x, y);
        }
        result[..count].to_vec()
    };
    Ok(CompiledCurveSet {
        channels: [
            CompiledCurve::from_nodes(&nodes(0), parameters.curve_type[0])?,
            CompiledCurve::from_nodes(&nodes(1), parameters.curve_type[1])?,
            CompiledCurve::from_nodes(&nodes(2), parameters.curve_type[2])?,
        ],
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurveCompileError {
    InvalidParameters(super::parameters::ParameterError),
    Curve(CurveError),
    TooFewNodes,
    InvalidFinalX,
    NonFiniteExtrapolation,
    InvalidProfile(RgbCurveProfileError),
}

impl fmt::Display for CurveCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParameters(error) => error.fmt(formatter),
            Self::Curve(error) => error.fmt(formatter),
            Self::TooFewNodes => formatter.write_str("RGB Curve needs at least two nodes"),
            Self::InvalidFinalX => {
                formatter.write_str("RGB Curve final x must be finite and positive")
            }
            Self::NonFiniteExtrapolation => {
                formatter.write_str("RGB Curve exponential fit is non-finite")
            }
            Self::InvalidProfile(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for CurveCompileError {}

impl From<CurveError> for CurveCompileError {
    fn from(error: CurveError) -> Self {
        Self::Curve(error)
    }
}

impl From<RgbCurveType> for CurveType {
    fn from(value: RgbCurveType) -> Self {
        match value {
            RgbCurveType::CubicSpline => Self::CubicSpline,
            RgbCurveType::CatmullRom => Self::CatmullRom,
            RgbCurveType::MonotoneHermite => Self::MonotoneHermite,
        }
    }
}

fn native_index(value: f32) -> usize {
    let index = (value * LUT_RESOLUTION as f32) as i32;
    index.clamp(0, i32::from(u16::MAX)) as usize
}

fn estimate_exp(x: [f32; 4], y: [f32; 4]) -> [f32; 3] {
    let x0 = x[3];
    let y0 = y[3];
    let mut exponent = 0.0_f32;
    let mut count = 0_i32;
    for index in 0..3 {
        let yy = y[index] / y0;
        let xx = x[index] / x0;
        if yy > 0.0 && xx > 0.0 {
            exponent += (y[index] / y0).ln() / (x[index] / x0).ln();
            count += 1;
        }
    }
    if count != 0 {
        exponent *= 1.0 / count as f32;
    } else {
        exponent = 1.0;
    }
    [1.0 / x0, y0, exponent]
}

fn apply_trc(value: f32, lut: &[f32], coefficients: [f32; 3], lutsize: usize) -> f32 {
    // Native `dt_ioppr_apply_trc` uses the first LUT sample as the marker for
    // whether this channel has a tone curve. The extrapolation coefficients
    // are independent evidence and must not select this branch.
    if lut[0] >= 0.0 {
        if value < 1.0 {
            let ft = (value * (lutsize - 1) as f32).clamp(0.0, (lutsize - 1) as f32);
            let t = if ft < (lutsize - 2) as f32 {
                ft as usize
            } else {
                lutsize - 2
            };
            let fraction = ft - t as f32;
            lut[t] * (1.0 - fraction) + lut[t + 1] * fraction
        } else {
            coefficients[1] * (value * coefficients[0]).powf(coefficients[2])
        }
    } else {
        value
    }
}

/// Multiplies an ordinary row-major matrix. This is equivalent to applying
/// native `matrix_*_transposed` through `dt_apply_transposed_color_matrix`.
fn multiply_row_major(input: [f32; 3], matrix: [[f32; 3]; 3]) -> [f32; 3] {
    [
        matrix[0][0] * input[0] + matrix[0][1] * input[1] + matrix[0][2] * input[2],
        matrix[1][0] * input[0] + matrix[1][1] * input[1] + matrix[1][2] * input[2],
        matrix[2][0] * input[0] + matrix[2][1] * input[1] + matrix[2][2] * input[2],
    ]
}

fn xyz_to_lab(xyz: [f32; 3]) -> [f32; 3] {
    // Keep the native `dt_XYZ_to_Lab` temporaries and operation order. In
    // particular, the final expression subtracts the zero/f[1] temporary
    // before multiplying rather than folding 16/116 into the subtraction.
    let d50_inv = [1.0_f32 / 0.9642_f32, 1.0_f32, 1.0_f32 / 0.8249_f32];
    let epsilon = 216.0_f32 / 24389.0_f32;
    let kappa = 24389.0_f32 / 27.0_f32;
    let mut f = [0.0_f32; 3];
    for index in 0..3 {
        let x = xyz[index] * d50_inv[index];
        f[index] = if x > epsilon {
            x.cbrt()
        } else {
            (kappa * x + 16.0_f32) / 116.0_f32
        };
    }
    let coeff = [116.0_f32, 500.0_f32, -200.0_f32];
    let offset = [16.0_f32, 0.0_f32, 0.0_f32];
    let tmp1 = [f[1], f[0], f[2]];
    let tmp2 = [0.0_f32, f[1], f[1]];
    [
        (coeff[0] * (tmp1[0] - tmp2[0])) - offset[0],
        (coeff[1] * (tmp1[1] - tmp2[1])) - offset[1],
        (coeff[2] * (tmp1[2] - tmp2[2])) - offset[2],
    ]
}

fn lab_to_xyz(lab: [f32; 3]) -> [f32; 3] {
    // Keep the native `dt_Lab_to_XYZ` f/scaled/inv sequence and its D50
    // multiplication order instead of algebraically folding the equations.
    let f = [lab[1], lab[0], lab[2]];
    let offset = [0.0_f32, 16.0_f32, 0.0_f32];
    let coeff = [
        1.0_f32 / 500.0_f32,
        1.0_f32 / 116.0_f32,
        -1.0_f32 / 200.0_f32,
    ];
    let add_coeff = [1.0_f32, 0.0_f32, 1.0_f32];
    let mut scaled = [0.0_f32; 3];
    for index in 0..3 {
        scaled[index] = (f[index] + offset[index]) * coeff[index];
    }

    let epsilon = 0.20689655172413796_f32;
    let kappa = 24389.0_f32 / 27.0_f32;
    let mut inv = [0.0_f32; 3];
    for index in 0..3 {
        let x = scaled[index] + scaled[1] * add_coeff[index];
        inv[index] = if x > epsilon {
            x * x * x
        } else {
            (116.0_f32 * x - 16.0_f32) / kappa
        };
    }

    let d50 = [0.9642_f32, 1.0_f32, 0.8249_f32];
    [d50[0] * inv[0], d50[1] * inv[1], d50[2] * inv[2]]
}

/// Documented native CPU/OpenCL mismatch for a movable final anchor.
#[must_use]
pub const fn native_gpu_extrapolation_mismatch() -> &'static str {
    "CPU branches at input < 1.0 / coeffs[0]; OpenCL lookup_unbounded branches at input < 1.0"
}

#[allow(dead_code)]
fn _preserve_mode_name(mode: PreserveColors) -> &'static str {
    match mode {
        PreserveColors::None => "none",
        PreserveColors::Luminance => "luminance",
        PreserveColors::Max => "max",
        PreserveColors::Average => "average",
        PreserveColors::Sum => "sum",
        PreserveColors::Norm => "norm",
        PreserveColors::Power => "power",
    }
}
