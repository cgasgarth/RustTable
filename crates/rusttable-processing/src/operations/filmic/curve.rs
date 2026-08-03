//! Legacy filmic S-curve and concavity LUTs.
//!
//! This is the bounded processing portion of `compute_curve_lut()` and
//! `commit_params()` in `src/iop/filmic.c`.  Curve sampling delegates to the
//! already source-shaped safe port of `src/common/curve_tools.c`; no generic
//! tone curve is substituted for the legacy operation.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::excessive_precision,
    clippy::float_cmp,
    clippy::if_not_else,
    clippy::manual_midpoint,
    clippy::many_single_char_names,
    clippy::similar_names,
    reason = "native filmic compares sanitized f32 nodes exactly and uses source-shaped arithmetic"
)]
#![expect(
    clippy::suboptimal_flops,
    reason = "Native Filmic curve equations preserve source evaluation order and IEEE-754 parity."
)]

#[cfg(not(test))]
use crate::common::curve_tools::{
    Curve, CurveAnchor, CurveBounds, CurveError, CurveType, sample_curve_v1,
};
#[cfg(test)]
use rusttable_processing::common::curve_tools::{
    Curve, CurveAnchor, CurveBounds, CurveError, CurveType, sample_curve_v1,
};

use super::codec::ParametersV3;

pub const LUT_SIZE: usize = 0x10000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeLoss {
    None,
    Toe,
    Shoulder,
    Both,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nodes {
    count: usize,
    x: [f32; 5],
    y: [f32; 5],
    loss: NodeLoss,
    latitude_min: f32,
    latitude_max: f32,
}

impl Nodes {
    #[must_use]
    pub const fn count(self) -> usize {
        self.count
    }

    #[must_use]
    pub const fn x(self) -> [f32; 5] {
        self.x
    }

    #[must_use]
    pub const fn y(self) -> [f32; 5] {
        self.y
    }

    #[must_use]
    pub const fn loss(self) -> NodeLoss {
        self.loss
    }

    #[must_use]
    pub const fn latitude_min(self) -> f32 {
        self.latitude_min
    }

    #[must_use]
    pub const fn latitude_max(self) -> f32 {
        self.latitude_max
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CurveLuts {
    pub table: Vec<f32>,
    pub grad_2: Vec<f32>,
    pub nodes: Nodes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CurveBuildError {
    InvalidDerivedState(&'static str),
    NonFiniteLut { table: &'static str, index: usize },
    Curve(CurveError),
}

impl std::fmt::Display for CurveBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDerivedState(stage) => {
                write!(formatter, "filmic derived state is invalid: {stage}")
            }
            Self::NonFiniteLut { table, index } => {
                write!(
                    formatter,
                    "filmic {table} LUT is non-finite at index {index}"
                )
            }
            Self::Curve(error) => write!(formatter, "filmic curve sampling failed: {error}"),
        }
    }
}

impl std::error::Error for CurveBuildError {}

impl From<CurveError> for CurveBuildError {
    fn from(error: CurveError) -> Self {
        Self::Curve(error)
    }
}

/// Builds the exact 65,536-entry curve and concavity tables.
pub fn build_luts(parameters: ParametersV3) -> Result<CurveLuts, CurveBuildError> {
    let derived = derive_filmic_nodes(parameters)?;
    validate_nodes(&derived)?;
    let mut table = build_table(&derived, parameters.interpolator)?;

    // `CurveDataSample` stores the output in a 16-bit domain.  Keep the
    // resulting f32 values, including the 1 - 1/65536 endpoint behavior.
    if table.len() != LUT_SIZE {
        return Err(CurveBuildError::InvalidDerivedState("curve LUT length"));
    }
    for (index, value) in table.iter().copied().enumerate() {
        if !value.is_finite() {
            return Err(CurveBuildError::NonFiniteLut {
                table: "table",
                index,
            });
        }
    }

    let mut grad_2 = vec![0.0_f32; LUT_SIZE];
    let saturation = parameters.saturation / 100.0_f32;
    let sigma = saturation
        * saturation
        * (derived.latitude_max - derived.latitude_min)
        * (derived.latitude_max - derived.latitude_min);
    let center = (derived.latitude_max + derived.latitude_min) / 2.0_f32;
    for (index, value) in grad_2.iter_mut().enumerate() {
        let x = (index as f32) / 65536.0_f32;
        *value = if sigma != 0.0_f32 {
            (-0.5_f32 * (center - x) * (center - x) / sigma).exp()
        } else {
            0.0_f32
        };
        if !value.is_finite() {
            return Err(CurveBuildError::NonFiniteLut {
                table: "grad_2",
                index,
            });
        }
    }

    // Keep the mutable local to make the source's table-building ordering
    // obvious to reviewers, even though the sampled vector is already owned.
    table.shrink_to_fit();
    Ok(CurveLuts {
        table,
        grad_2,
        nodes: derived,
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "Native Filmic curve derivation keeps the source's toe, shoulder, and LUT state machine together."
)]
pub fn derive_filmic_nodes(parameters: ParametersV3) -> Result<Nodes, CurveBuildError> {
    let source_values = [
        parameters.grey_point_source,
        parameters.black_point_source,
        parameters.white_point_source,
        parameters.security_factor,
        parameters.grey_point_target,
        parameters.black_point_target,
        parameters.white_point_target,
        parameters.output_power,
        parameters.latitude_stops,
        parameters.contrast,
        parameters.saturation,
        parameters.global_saturation,
        parameters.balance,
    ];
    if source_values.iter().any(|value| !value.is_finite()) {
        return Err(CurveBuildError::InvalidDerivedState("nonfinite parameter"));
    }

    let white_source = parameters.white_point_source;
    let black_source = parameters.black_point_source;
    let dynamic_range = white_source - black_source;
    if !dynamic_range.is_finite() || dynamic_range <= 0.0_f32 {
        return Err(CurveBuildError::InvalidDerivedState(
            "nonpositive dynamic range",
        ));
    }
    if !parameters.grey_point_source.is_finite() || parameters.grey_point_source <= 0.0_f32 {
        return Err(CurveBuildError::InvalidDerivedState(
            "nonpositive source grey",
        ));
    }
    if !parameters.output_power.is_finite() || parameters.output_power <= 0.0_f32 {
        return Err(CurveBuildError::InvalidDerivedState(
            "nonpositive output power",
        ));
    }
    if !parameters.contrast.is_finite() {
        return Err(CurveBuildError::InvalidDerivedState("nonfinite contrast"));
    }

    let black_log = 0.0_f32;
    let grey_log = black_source.abs() / dynamic_range;
    let white_log = 1.0_f32;
    if !grey_log.is_finite() {
        return Err(CurveBuildError::InvalidDerivedState("nonfinite source log"));
    }

    let black_display = clamp(
        parameters.black_point_target,
        0.0_f32,
        parameters.grey_point_target,
    ) / 100.0_f32;
    let grey_display = (clamp(
        parameters.grey_point_target,
        parameters.black_point_target,
        parameters.white_point_target,
    ) / 100.0_f32)
        .powf(1.0_f32 / parameters.output_power);
    let white_display = clamp(
        parameters.white_point_target,
        parameters.grey_point_target,
        100.0_f32,
    ) / 100.0_f32;
    if [black_display, grey_display, white_display]
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(CurveBuildError::InvalidDerivedState(
            "nonfinite target luminance",
        ));
    }
    let latitude = clamp(
        parameters.latitude_stops,
        0.01_f32,
        dynamic_range * 0.99_f32,
    );
    if !latitude.is_finite() || latitude < 0.01_f32 {
        return Err(CurveBuildError::InvalidDerivedState("invalid latitude"));
    }
    let balance = clamp(parameters.balance, -50.0_f32, 50.0_f32) / 100.0_f32;
    let contrast = parameters.contrast;

    let mut toe_log = grey_log - latitude / dynamic_range * (black_source / dynamic_range).abs();
    let mut shoulder_log = grey_log + latitude / dynamic_range * (white_source / dynamic_range);
    let linear_intercept = grey_display - contrast * grey_log;
    let mut toe_display = toe_log * contrast + linear_intercept;
    let mut shoulder_display = shoulder_log * contrast + linear_intercept;

    let norm = (contrast.powf(2.0_f32) + 1.0_f32).powf(0.5_f32);
    if !norm.is_finite() || norm == 0.0_f32 {
        return Err(CurveBuildError::InvalidDerivedState(
            "invalid contrast norm",
        ));
    }
    let coeff = -(dynamic_range - latitude) / dynamic_range * balance;
    toe_display += coeff * contrast / norm;
    shoulder_display += coeff * contrast / norm;
    toe_log += coeff / norm;
    shoulder_log += coeff / norm;
    if [toe_log, shoulder_log, toe_display, shoulder_display]
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(CurveBuildError::InvalidDerivedState("nonfinite curve node"));
    }

    toe_log = clamp(toe_log, black_log, grey_log);
    shoulder_log = clamp(shoulder_log, grey_log, white_log);
    toe_display = clamp(toe_display, black_display, grey_display);
    shoulder_display = clamp(shoulder_display, grey_display, white_display);

    let toe_lost = (toe_log == grey_log && toe_display == grey_display)
        || (toe_log == 0.0_f32 && toe_display == black_display);
    let shoulder_lost = (shoulder_log == grey_log && shoulder_display == grey_display)
        || (shoulder_log == 1.0_f32 && shoulder_display == white_display);

    let mut x = [0.0_f32; 5];
    let mut y = [0.0_f32; 5];
    let (count, loss, latitude_min, latitude_max) = if shoulder_lost && !toe_lost {
        x[..4].copy_from_slice(&[black_log, toe_log, grey_log, white_log]);
        y[..4].copy_from_slice(&[black_display, toe_display, grey_display, white_display]);
        (4, NodeLoss::Shoulder, toe_log, white_log)
    } else if toe_lost && !shoulder_lost {
        x[..4].copy_from_slice(&[black_log, grey_log, shoulder_log, white_log]);
        y[..4].copy_from_slice(&[black_display, grey_display, shoulder_display, white_display]);
        (4, NodeLoss::Toe, black_log, shoulder_log)
    } else if toe_lost && shoulder_lost {
        x[..3].copy_from_slice(&[black_log, grey_log, white_log]);
        y[..3].copy_from_slice(&[black_display, grey_display, white_display]);
        (3, NodeLoss::Both, black_log, white_log)
    } else {
        // This is intentionally the native four-node branch.  The source
        // comments mention five nodes, but the actual assignments omit grey.
        x[..4].copy_from_slice(&[black_log, toe_log, shoulder_log, white_log]);
        y[..4].copy_from_slice(&[black_display, toe_display, shoulder_display, white_display]);
        (4, NodeLoss::None, toe_log, shoulder_log)
    };

    Ok(Nodes {
        count,
        x,
        y,
        loss,
        latitude_min,
        latitude_max,
    })
}

fn validate_nodes(nodes: &Nodes) -> Result<(), CurveBuildError> {
    if nodes.x[..nodes.count]
        .windows(2)
        .any(|pair| pair[1] <= pair[0])
    {
        return Err(CurveBuildError::InvalidDerivedState(
            "duplicate or unordered curve knots",
        ));
    }
    let mut values = nodes.x[..nodes.count]
        .iter()
        .copied()
        .chain(nodes.y[..nodes.count].iter().copied())
        .chain([nodes.latitude_min, nodes.latitude_max]);
    if values.any(|value| !value.is_finite()) {
        return Err(CurveBuildError::InvalidDerivedState("nonfinite curve node"));
    }
    Ok(())
}

fn build_table(nodes: &Nodes, interpolator: i32) -> Result<Vec<f32>, CurveBuildError> {
    let anchors: Vec<_> = (0..nodes.count)
        .map(|index| CurveAnchor::new(nodes.x[index], nodes.y[index]))
        .collect();
    let cubic = sample(&anchors, CurveType::CubicSpline)?;
    if interpolator == 3 {
        let monotone = sample(&anchors, CurveType::MonotoneHermite)?;
        Ok(cubic
            .into_iter()
            .zip(monotone)
            .map(|(left, right)| (left + right) / 2.0_f32)
            .collect())
    } else {
        let curve_type = match interpolator {
            1 => CurveType::CatmullRom,
            2 => CurveType::MonotoneHermite,
            _ => CurveType::CubicSpline,
        };
        if curve_type == CurveType::CubicSpline {
            Ok(cubic)
        } else {
            sample(&anchors, curve_type)
        }
    }
}

fn sample(anchors: &[CurveAnchor], curve_type: CurveType) -> Result<Vec<f32>, CurveBuildError> {
    let curve = Curve::new(curve_type, CurveBounds::unit(), anchors)?;
    let samples = sample_curve_v1(&curve, LUT_SIZE as u32, LUT_SIZE as u32)?;
    Ok(samples
        .into_iter()
        .map(|value| f32::from(value) * (1.0_f32 / 65536.0_f32))
        .collect())
}

#[inline]
fn clamp(value: f32, lower: f32, upper: f32) -> f32 {
    if value >= lower {
        if value <= upper { value } else { upper }
    } else {
        lower
    }
}
