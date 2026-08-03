//! Tone Curve LUT compilation ported from `commit_params()` and `process()` in
//! `src/iop/tonecurve.c`, with conversion helpers from
//! `src/common/colorspaces_inline_conversions.h`, `src/common/rgb_norms.h`,
//! `src/common/iop_profile.h`, `src/develop/imageop_math.h`,
//! `src/gui/draw.h`, and `src/common/curve_tools.c`.

#![forbid(unsafe_code)]
#![expect(
    clippy::suboptimal_flops,
    reason = "Darktable's color conversions and LUT interpolation preserve separate f32 multiply/add rounding"
)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    reason = "native curve and color conversion arithmetic uses source-width f32 operations"
)]

use std::fmt;

#[cfg(not(test))]
use crate::common::curve_tools::{
    Curve, CurveAnchor, CurveBounds, CurveError, CurveType, sample_curve_v1,
};
#[cfg(test)]
use rusttable_processing::common::curve_tools::{
    Curve, CurveAnchor, CurveBounds, CurveError, CurveType, sample_curve_v1,
};

use super::parameters::{
    CHANNELS, LUT_RESOLUTION, PreserveColors, ToneCurveAutoscale, ToneCurveParametersV5,
    ToneCurveType,
};

pub const PROFILE_MATRIX_ORIENTATION: &str =
    "matrix_in is row-major (non-transposed) and its row 1 supplies ProPhoto Y";

const PROPHOTO_MATRIX_IN: [[f32; 3]; 3] = [
    [0.7976749, 0.1351917, 0.0313534],
    [0.2880402, 0.7118741, 0.0000857],
    [0.0, 0.0, 0.8252100],
];

/// Explicit numeric evidence for the native ProPhoto working-profile lookup.
/// The leaf never substitutes camera luminance when this evidence is absent.
#[derive(Debug, Clone, PartialEq)]
pub struct ToneCurveProfileEvidence {
    /// Native `matrix_in`, represented as ordinary row-major rows.
    pub matrix_in: [[f32; 3]; 3],
    /// Native `lut_in[3]` channel tables.
    pub lut_in: [Vec<f32>; 3],
    /// Native `unbounded_coeffs_in[3][3]` channel fits.
    pub unbounded_coeffs_in: [[f32; 3]; 3],
    /// Native `lutsize`, kept explicit rather than inferred at execution time.
    pub lut_size: usize,
    /// Native `nonlinearlut` flag, independent of the LUT payload.
    pub nonlinearlut: bool,
}

impl ToneCurveProfileEvidence {
    /// Returns the exact linear profile selected by native Tone Curve RGB mode.
    #[must_use]
    pub fn prophoto() -> Self {
        Self::new_linear(PROPHOTO_MATRIX_IN)
    }

    #[must_use]
    pub fn new_linear(matrix_in: [[f32; 3]; 3]) -> Self {
        Self {
            matrix_in,
            lut_in: std::array::from_fn(|_| vec![0.0, 1.0]),
            unbounded_coeffs_in: [[-1.0, 0.0, 1.0]; 3],
            lut_size: 2,
            nonlinearlut: false,
        }
    }

    pub fn new_with_trc(
        matrix_in: [[f32; 3]; 3],
        lut_in: [Vec<f32>; 3],
        unbounded_coeffs_in: [[f32; 3]; 3],
        lut_size: usize,
        nonlinearlut: bool,
    ) -> Result<Self, ToneCurveProfileError> {
        let evidence = Self {
            matrix_in,
            lut_in,
            unbounded_coeffs_in,
            lut_size,
            nonlinearlut,
        };
        evidence.validate()?;
        Ok(evidence)
    }

    fn validate(&self) -> Result<(), ToneCurveProfileError> {
        if self
            .matrix_in
            .iter()
            .flatten()
            .any(|value| !value.is_finite())
        {
            return Err(ToneCurveProfileError::NonFiniteMatrix);
        }
        if self.nonlinearlut {
            return Err(ToneCurveProfileError::UnsupportedNonlinearEvidence);
        }
        Ok(())
    }

    /// Native `dt_ioppr_get_rgb_matrix_luminance` with explicit profile data.
    pub fn luminance(&self, rgb: [f32; 3]) -> f32 {
        let rgb = if self.nonlinearlut {
            [
                apply_profile_trc(
                    rgb[0],
                    &self.lut_in[0],
                    self.unbounded_coeffs_in[0],
                    self.lut_size,
                ),
                apply_profile_trc(
                    rgb[1],
                    &self.lut_in[1],
                    self.unbounded_coeffs_in[1],
                    self.lut_size,
                ),
                apply_profile_trc(
                    rgb[2],
                    &self.lut_in[2],
                    self.unbounded_coeffs_in[2],
                    self.lut_size,
                ),
            ]
        } else {
            rgb
        };
        self.matrix_in[1][0] * rgb[0]
            + self.matrix_in[1][1] * rgb[1]
            + self.matrix_in[1][2] * rgb[2]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToneCurveProfileError {
    NonFiniteMatrix,
    InvalidLut,
    NonFiniteCoefficients,
    UnsupportedNonlinearEvidence,
}

impl fmt::Display for ToneCurveProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteMatrix => "Tone Curve profile matrix is non-finite",
            Self::InvalidLut => "Tone Curve profile LUT size or samples are invalid",
            Self::NonFiniteCoefficients => "Tone Curve profile coefficients are non-finite",
            Self::UnsupportedNonlinearEvidence => {
                "Tone Curve nonlinear profile evidence is unsupported"
            }
        })
    }
}

impl std::error::Error for ToneCurveProfileError {}

/// One native 65536-entry table plus its right-side exponential fit.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledToneCurve {
    table: Vec<f32>,
    coefficients: [f32; 3],
    extrapolation_threshold: f32,
    left_coefficients: [f32; 3],
}

impl CompiledToneCurve {
    fn new(table: Vec<f32>, coefficients: [f32; 3]) -> Result<Self, CurveCompileError> {
        let extrapolation_threshold = 1.0 / coefficients[0];
        if !extrapolation_threshold.is_finite() {
            return Err(CurveCompileError::NonFiniteExtrapolation);
        }
        Ok(Self {
            table,
            coefficients,
            extrapolation_threshold,
            left_coefficients: [1.0, 0.0, 1.0],
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
    pub const fn extrapolation_threshold(&self) -> f32 {
        self.extrapolation_threshold
    }

    /// Native CPU lookup: reciprocal-fit threshold, then truncating LUT index.
    #[must_use]
    pub fn evaluate(&self, input: f32) -> f32 {
        if input < self.extrapolation_threshold {
            self.table[native_index(input)]
        } else {
            eval_exp(self.coefficients, input)
        }
    }
}

/// The three compiled native tables. Channel order is L, a, b.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledToneCurveSet {
    channels: [CompiledToneCurve; CHANNELS],
    low_approximation: f32,
}

impl CompiledToneCurveSet {
    #[must_use]
    pub const fn channel(&self, channel: usize) -> &CompiledToneCurve {
        &self.channels[channel]
    }

    #[must_use]
    pub const fn channels(&self) -> &[CompiledToneCurve; CHANNELS] {
        &self.channels
    }

    #[must_use]
    pub const fn low_approximation(&self) -> f32 {
        self.low_approximation
    }

    /// Native two-sided a/b lookup and extrapolation.
    #[must_use]
    pub fn evaluate_ab(&self, channel: usize, input: f32, unbound: bool) -> f32 {
        if !unbound {
            return self.channels[channel].table[native_index(input)];
        }
        let coefficients = self.channels[channel].coefficients;
        let right_threshold = 1.0 / coefficients[0];
        let left_threshold = 1.0 - 1.0 / self.left_coefficients(channel)[0];
        if input > right_threshold {
            eval_exp(coefficients, input)
        } else if input < left_threshold {
            eval_exp(self.left_coefficients(channel), 1.0 - input)
        } else {
            self.channels[channel].table[native_index(input)]
        }
    }

    const fn left_coefficients(&self, channel: usize) -> [f32; 3] {
        self.channels[channel].left_coefficients
    }
}

impl CompiledToneCurve {
    const fn with_left_coefficients(mut self, left_coefficients: [f32; 3]) -> Self {
        self.left_coefficients = left_coefficients;
        self
    }
}

#[must_use]
pub const fn requires_profile_evidence(parameters: &ToneCurveParametersV5) -> bool {
    matches!(
        (
            parameters.tonecurve_autoscale_ab,
            parameters.preserve_colors,
        ),
        (ToneCurveAutoscale::AutomaticRgb, PreserveColors::Luminance)
    )
}

/// Compiles all native tables in the exact `commit_params()` order:
/// curves, sampling, Lab scaling, XYZ/RGB derivation, then exponential fits.
pub fn compile_parameters(
    parameters: &ToneCurveParametersV5,
    profile: Option<&ToneCurveProfileEvidence>,
) -> Result<CompiledToneCurveSet, CurveCompileError> {
    parameters
        .validate()
        .map_err(CurveCompileError::InvalidParameters)?;
    if let Some(profile) = profile {
        profile
            .validate()
            .map_err(CurveCompileError::InvalidProfile)?;
    }
    // 1-2. Build all three curves and sample all three 65536-entry tables.
    let mut tables = [
        sample_parameters(parameters, 0)?,
        sample_parameters(parameters, 1)?,
        sample_parameters(parameters, 2)?,
    ];

    // 3. Native Lab-domain scaling.
    for index in 0..LUT_RESOLUTION as usize {
        tables[0][index] *= 100.0;
        tables[1][index] = tables[1][index] * 256.0 - 128.0;
        tables[2][index] = tables[2][index] * 256.0 - 128.0;
    }

    // 4-5. For linked XYZ/RGB modes, replace the L table with the derived
    // normalized luminance/channel table before fitting exponentials.
    match parameters.tonecurve_autoscale_ab {
        ToneCurveAutoscale::AutomaticXyz => derive_xyz_table(&mut tables[0]),
        ToneCurveAutoscale::AutomaticRgb => derive_rgb_table(&mut tables[0]),
        ToneCurveAutoscale::ManualLab | ToneCurveAutoscale::AutomaticLab => {}
    }

    // 6. Fit after every table has reached its final runtime domain.
    let l_coefficients = estimate_exp_for_right(&tables[0], parameters, 0)?;
    let a_right = estimate_exp_for_right(&tables[1], parameters, 1)?;
    let a_left = estimate_exp_for_left(&tables[1], parameters, 1)?;
    let b_right = estimate_exp_for_right(&tables[2], parameters, 2)?;
    let b_left = estimate_exp_for_left(&tables[2], parameters, 2)?;

    let l = CompiledToneCurve::new(tables[0].clone(), l_coefficients)?;
    let a = CompiledToneCurve::new(tables[1].clone(), a_right)?.with_left_coefficients(a_left);
    let b = CompiledToneCurve::new(tables[2].clone(), b_right)?.with_left_coefficients(b_left);
    Ok(CompiledToneCurveSet {
        low_approximation: tables[0][native_index(0.01)],
        channels: [l, a, b],
    })
}

fn sample_parameters(
    parameters: &ToneCurveParametersV5,
    channel: usize,
) -> Result<Vec<f32>, CurveCompileError> {
    let count = usize::try_from(parameters.tonecurve_nodes[channel]).expect("validated count");
    let anchors = parameters.tonecurve[channel][..count]
        .iter()
        .map(|node| CurveAnchor::new(node.x, node.y))
        .collect::<Vec<_>>();
    let curve = Curve::new(
        parameters.tonecurve_type[channel].into(),
        CurveBounds::unit(),
        &anchors,
    )?;
    let samples = sample_curve_v1(&curve, LUT_RESOLUTION, LUT_RESOLUTION)?;
    Ok(samples
        .into_iter()
        .map(|sample| f32::from(sample) / 65_536.0)
        .collect())
}

fn derive_xyz_table(table: &mut [f32]) {
    for index in 0..LUT_RESOLUTION as usize {
        let value = index as f32 / 65_536.0;
        let mut xyz = [value, value, value];
        let mut lab = [0.0_f32; 3];
        xyz_to_lab(xyz, &mut lab);
        lab[0] = table[native_index(lab[0] / 100.0)];
        lab_to_xyz(lab, &mut xyz);
        table[index] = xyz[1];
    }
}

fn derive_rgb_table(table: &mut [f32]) {
    for index in 0..LUT_RESOLUTION as usize {
        let value = index as f32 / 65_536.0;
        let mut rgb = [value, value, value];
        let mut lab = [0.0_f32; 3];
        prophoto_to_lab(rgb, &mut lab);
        lab[0] = table[native_index(lab[0] / 100.0)];
        lab_to_prophoto(lab, &mut rgb);
        table[index] = rgb[1];
    }
}

fn estimate_exp_for_right(
    table: &[f32],
    parameters: &ToneCurveParametersV5,
    channel: usize,
) -> Result<[f32; 3], CurveCompileError> {
    let count = usize::try_from(parameters.tonecurve_nodes[channel]).expect("validated count");
    let final_x = parameters.tonecurve[channel][count - 1].x;
    let x = [0.7 * final_x, 0.8 * final_x, 0.9 * final_x, final_x];
    let y = x.map(|value| table[native_index(value)]);
    estimate_exp(x, y)
}

fn estimate_exp_for_left(
    table: &[f32],
    parameters: &ToneCurveParametersV5,
    channel: usize,
) -> Result<[f32; 3], CurveCompileError> {
    let first_x = parameters.tonecurve[channel][0].x;
    let mirrored = 1.0 - first_x;
    let x = [0.7 * mirrored, 0.8 * mirrored, 0.9 * mirrored, mirrored];
    let y = x.map(|value| table[native_index(1.0 - value)]);
    estimate_exp(x, y)
}

fn estimate_exp(x: [f32; 4], y: [f32; 4]) -> Result<[f32; 3], CurveCompileError> {
    let x0 = x[3];
    if !(x0 > 0.0) || !x0.is_finite() {
        return Err(CurveCompileError::InvalidExtrapolationDomain);
    }
    let y0 = y[3];
    let mut exponent = 0.0_f32;
    let mut count = 0_i32;
    for index in 0..3 {
        let yy = y[index] / y0;
        let xx = x[index] / x0;
        if yy > 0.0 && xx > 0.0 {
            let y_log = (y[index] / y0).ln();
            let x_log = (x[index] / x0).ln();
            exponent += y_log / x_log;
            count += 1;
        }
    }
    if count != 0 {
        exponent *= 1.0 / count as f32;
    } else {
        exponent = 1.0;
    }
    let coefficients = [1.0 / x0, y0, exponent];
    if coefficients.iter().any(|value| !value.is_finite()) {
        return Err(CurveCompileError::NonFiniteExtrapolation);
    }
    Ok(coefficients)
}

fn native_index(value: f32) -> usize {
    let index = (value * LUT_RESOLUTION as f32) as i32;
    index.clamp(0, i32::from(u16::MAX)) as usize
}

fn eval_exp(coefficients: [f32; 3], value: f32) -> f32 {
    coefficients[1] * (value * coefficients[0]).powf(coefficients[2])
}

fn apply_profile_trc(value: f32, lut: &[f32], coefficients: [f32; 3], lut_size: usize) -> f32 {
    if lut[0] >= 0.0 {
        if value < 1.0 {
            let ft = (value * (lut_size - 1) as f32).clamp(0.0, (lut_size - 1) as f32);
            let index = if ft < (lut_size - 2) as f32 {
                ft as usize
            } else {
                lut_size - 2
            };
            let fraction = ft - index as f32;
            lut[index] * (1.0 - fraction) + lut[index + 1] * fraction
        } else {
            eval_exp(coefficients, value)
        }
    } else {
        value
    }
}

pub(super) fn xyz_to_lab(xyz: [f32; 3], lab: &mut [f32; 3]) {
    let d50_inv = [1.0_f32 / 0.9642_f32, 1.0_f32, 1.0_f32 / 0.8249_f32];
    let epsilon = 216.0_f32 / 24389.0_f32;
    let kappa = 24389.0_f32 / 27.0_f32;
    let mut f = [0.0_f32; 3];
    for index in 0..3 {
        let value = xyz[index] * d50_inv[index];
        f[index] = if value > epsilon {
            value.cbrt()
        } else {
            (kappa * value + 16.0_f32) / 116.0_f32
        };
    }
    let coeff = [116.0_f32, 500.0_f32, -200.0_f32];
    let offset = [16.0_f32, 0.0_f32, 0.0_f32];
    let tmp1 = [f[1], f[0], f[2]];
    let tmp2 = [0.0_f32, f[1], f[1]];
    for index in 0..3 {
        lab[index] = (coeff[index] * (tmp1[index] - tmp2[index])) - offset[index];
    }
}

pub(super) fn lab_to_xyz(lab: [f32; 3], xyz: &mut [f32; 3]) {
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
        let value = scaled[index] + scaled[1] * add_coeff[index];
        inv[index] = if value > epsilon {
            value * value * value
        } else {
            (116.0_f32 * value - 16.0_f32) / kappa
        };
    }
    let d50 = [0.9642_f32, 1.0_f32, 0.8249_f32];
    for index in 0..3 {
        xyz[index] = d50[index] * inv[index];
    }
}

fn prophoto_to_xyz(rgb: [f32; 3], xyz: &mut [f32; 3]) {
    // `dt_apply_transposed_color_matrix` with the D50 ProPhoto matrix.
    xyz[0] = 0.7976749_f32 * rgb[0] + 0.1351917_f32 * rgb[1] + 0.0313534_f32 * rgb[2];
    xyz[1] = 0.2880402_f32 * rgb[0] + 0.7118741_f32 * rgb[1] + 0.0000857_f32 * rgb[2];
    xyz[2] = 0.8252100_f32 * rgb[2];
}

fn xyz_to_prophoto(xyz: [f32; 3], rgb: &mut [f32; 3]) {
    rgb[0] = 1.3459433_f32 * xyz[0] - 0.2556075_f32 * xyz[1] - 0.0511118_f32 * xyz[2];
    rgb[1] = -0.5445989_f32 * xyz[0] + 1.5081673_f32 * xyz[1] + 0.0205351_f32 * xyz[2];
    rgb[2] = 1.2118128_f32 * xyz[2];
}

pub(super) fn prophoto_to_lab(rgb: [f32; 3], lab: &mut [f32; 3]) {
    let mut xyz = [0.0_f32; 3];
    prophoto_to_xyz(rgb, &mut xyz);
    xyz_to_lab(xyz, lab);
}

pub(super) fn lab_to_prophoto(lab: [f32; 3], rgb: &mut [f32; 3]) {
    let mut xyz = [0.0_f32; 3];
    lab_to_xyz(lab, &mut xyz);
    xyz_to_prophoto(xyz, rgb);
}

impl From<ToneCurveType> for CurveType {
    fn from(value: ToneCurveType) -> Self {
        match value {
            ToneCurveType::CubicSpline => Self::CubicSpline,
            ToneCurveType::CatmullRom => Self::CatmullRom,
            ToneCurveType::MonotoneHermite => Self::MonotoneHermite,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurveCompileError {
    InvalidParameters(super::parameters::ParameterError),
    Curve(CurveError),
    InvalidExtrapolationDomain,
    NonFiniteExtrapolation,
    InvalidProfile(ToneCurveProfileError),
    MissingProfileEvidence,
}

impl fmt::Display for CurveCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidParameters(error) => error.fmt(formatter),
            Self::Curve(error) => error.fmt(formatter),
            Self::InvalidExtrapolationDomain => {
                formatter.write_str("Tone Curve extrapolation domain must be finite and positive")
            }
            Self::NonFiniteExtrapolation => {
                formatter.write_str("Tone Curve exponential fit is non-finite")
            }
            Self::InvalidProfile(error) => error.fmt(formatter),
            Self::MissingProfileEvidence => {
                formatter.write_str("Tone Curve RGB luminance requires ProPhoto profile evidence")
            }
        }
    }
}

impl std::error::Error for CurveCompileError {}

impl From<CurveError> for CurveCompileError {
    fn from(error: CurveError) -> Self {
        Self::Curve(error)
    }
}

#[cfg(test)]
mod tests {
    use super::lab_to_xyz;

    #[test]
    fn lab_to_xyz_preserves_native_scaled_component_order() {
        let mut xyz = [0.0_f32; 3];
        lab_to_xyz([-2.7088928, -71.725235, 63.4198], &mut xyz);
        assert_eq!(xyz[0].to_bits(), 0xbca9_320a);
    }
}
