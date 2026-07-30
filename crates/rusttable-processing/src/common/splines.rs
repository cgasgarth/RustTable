//! Safe Rust port of Darktable's `src/common/splines.cpp` V2 curve path.
//!
//! The retained behavior includes source-order curve-box expansion, clipping
//! or periodic folding before sorting, the four native tangent strategies,
//! Hermite evaluation, and the `u16` table quantization used by
//! `dt_draw_curve_calc_values_V2`. Native cases whose floating-point
//! conversions have no defined result are rejected with typed errors.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::manual_midpoint,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::suboptimal_flops,
    reason = "the source port retains C++ f32 arithmetic, exact zero tests, and index-oriented solvers"
)]

use std::cmp::Ordering;
use std::fmt;
use std::mem::size_of;

use super::curve_tools::{Curve, CurveAnchor, CurveType, MAX_RESOLUTION};

/// Failure from constructing, evaluating, or quantizing a V2 spline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplineError {
    /// Direct interpolation requires at least one anchor.
    EmptyAnchors,
    /// A direct interpolation anchor, coordinate, or period is not finite.
    NonFiniteEvaluationInput,
    /// A periodic domain has zero width.
    InvalidPeriod,
    /// Sorting, transformation, or folding produced two equal x coordinates.
    DuplicateAbscissa { first: usize, second: usize },
    /// Sampling resolution is outside `2..=MAX_RESOLUTION`.
    InvalidSamplingResolution(u32),
    /// Output resolution is outside `2..=MAX_RESOLUTION`.
    InvalidOutputResolution(u32),
    /// The smooth-cubic derivative system is singular.
    SingularSystem,
    /// Finite inputs produced a non-finite transformed point, derivative, or value.
    NonFiniteResult,
    /// A native float-to-integer conversion is not representable.
    QuantizationOutOfRange { sample: usize },
    /// A bounded temporary or output allocation failed.
    AllocationFailed { required_bytes: usize },
}

impl fmt::Display for SplineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EmptyAnchors => formatter.write_str("V2 spline requires at least one anchor"),
            Self::NonFiniteEvaluationInput => {
                formatter.write_str("V2 spline interpolation inputs must be finite")
            }
            Self::InvalidPeriod => formatter.write_str("V2 spline period must be nonzero"),
            Self::DuplicateAbscissa { first, second } => write!(
                formatter,
                "V2 spline anchors {first} and {second} have the same transformed x coordinate"
            ),
            Self::InvalidSamplingResolution(value) => write!(
                formatter,
                "V2 spline sampling resolution {value} is outside 2..={MAX_RESOLUTION}"
            ),
            Self::InvalidOutputResolution(value) => write!(
                formatter,
                "V2 spline output resolution {value} is outside 2..={MAX_RESOLUTION}"
            ),
            Self::SingularSystem => {
                formatter.write_str("V2 smooth-cubic derivative system is singular")
            }
            Self::NonFiniteResult => {
                formatter.write_str("V2 spline produced a non-finite intermediate or result")
            }
            Self::QuantizationOutOfRange { sample } => write!(
                formatter,
                "V2 spline sample {sample} cannot be represented by native integer quantization"
            ),
            Self::AllocationFailed { required_bytes } => write!(
                formatter,
                "unable to allocate V2 spline temporary requiring {required_bytes} bytes"
            ),
        }
    }
}

impl std::error::Error for SplineError {}

#[derive(Debug, Clone, Copy)]
struct SourcePoint {
    x: f32,
    y: f32,
    source: usize,
}

#[derive(Debug, Clone, Copy)]
struct BasePoint {
    x: f32,
    y: f32,
    dy: f32,
    source: usize,
}

#[derive(Debug, Clone, Copy)]
struct Limits {
    min: f32,
    max: f32,
}

impl Limits {
    fn new(first: f32, second: f32) -> Self {
        if second < first {
            Self {
                min: second,
                max: first,
            }
        } else {
            Self {
                min: first,
                max: second,
            }
        }
    }

    const fn infinity() -> Self {
        Self {
            min: f32::NEG_INFINITY,
            max: f32::INFINITY,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum TangentKind {
    SmoothCubic,
    CatmullRom,
    MonotoneHermite,
    MonotoneHermiteVariant,
}

impl TangentKind {
    const fn for_curve(curve_type: CurveType, periodic_sample: bool) -> Self {
        match curve_type {
            CurveType::CubicSpline => Self::SmoothCubic,
            CurveType::CatmullRom => Self::CatmullRom,
            CurveType::MonotoneHermite if periodic_sample => Self::MonotoneHermiteVariant,
            CurveType::MonotoneHermite => Self::MonotoneHermite,
        }
    }
}

#[derive(Debug)]
struct Spline {
    points: Vec<BasePoint>,
    x_limits: Limits,
    y_limits: Limits,
    periodic: bool,
}

impl Spline {
    fn unbounded(source: &[SourcePoint], tangent_kind: TangentKind) -> Result<Self, SplineError> {
        Self::build(source, None, Limits::infinity(), false, tangent_kind)
    }

    fn bounded(
        source: &[SourcePoint],
        x_limits: Limits,
        y_limits: Limits,
        periodic: bool,
        tangent_kind: TangentKind,
    ) -> Result<Self, SplineError> {
        Self::build(source, Some(x_limits), y_limits, periodic, tangent_kind)
    }

    fn build(
        source: &[SourcePoint],
        supplied_x_limits: Option<Limits>,
        y_limits: Limits,
        periodic: bool,
        tangent_kind: TangentKind,
    ) -> Result<Self, SplineError> {
        if source.is_empty() {
            return Err(SplineError::EmptyAnchors);
        }

        let mut points = allocate_vec(source.len(), size_of::<BasePoint>())?;
        if let Some(x_limits) = supplied_x_limits {
            if periodic {
                let period = x_limits.max - x_limits.min;
                if !period.is_finite() {
                    return Err(SplineError::NonFiniteResult);
                }
                if period == 0.0 {
                    return Err(SplineError::InvalidPeriod);
                }
                for point in source {
                    let mut x = point.x % period;
                    if x < 0.0 {
                        x += period;
                    }
                    if !x.is_finite() {
                        return Err(SplineError::NonFiniteResult);
                    }
                    points.push(BasePoint {
                        x,
                        y: point.y,
                        dy: 0.0,
                        source: point.source,
                    });
                }
            } else {
                for point in source {
                    if x_limits.min <= point.x && point.x <= x_limits.max {
                        points.push(BasePoint {
                            x: point.x,
                            y: point.y,
                            dy: 0.0,
                            source: point.source,
                        });
                    }
                }
            }
        } else {
            points.extend(source.iter().map(|point| BasePoint {
                x: point.x,
                y: point.y,
                dy: 0.0,
                source: point.source,
            }));
        }

        if points.is_empty() {
            return Err(SplineError::EmptyAnchors);
        }
        points.sort_unstable_by(|left, right| {
            left.x.partial_cmp(&right.x).unwrap_or(Ordering::Equal)
        });
        for adjacent in points.windows(2) {
            if adjacent[0].x == adjacent[1].x {
                return Err(SplineError::DuplicateAbscissa {
                    first: adjacent[0].source,
                    second: adjacent[1].source,
                });
            }
        }

        let x_limits = supplied_x_limits.unwrap_or_else(|| {
            Limits::new(
                points.first().expect("nonempty points").x,
                points.last().expect("nonempty points").x,
            )
        });
        let mut spline = Self {
            points,
            x_limits,
            y_limits,
            periodic,
        };
        spline.initialize_tangents(tangent_kind)?;
        if spline.points.iter().any(|point| !point.dy.is_finite()) {
            return Err(SplineError::NonFiniteResult);
        }
        Ok(spline)
    }

    fn evaluate(&self, mut x: f32) -> Result<f32, SplineError> {
        if self.points.len() == 1 {
            return Ok(self.points[0].y);
        }

        let (n0, n1, h);
        if self.periodic {
            let period = self.x_limits.max - self.x_limits.min;
            x %= period;
            if x < self.points[0].x {
                x += period;
            }
            let upper = self.points.partition_point(|point| point.x <= x);
            n0 = if upper > 0 {
                upper - 1
            } else {
                self.points.len() - 1
            };
            n1 = if n0 + 1 < self.points.len() {
                n0 + 1
            } else {
                0
            };
            h = if n1 > n0 {
                self.points[n1].x - self.points[n0].x
            } else {
                self.points[n1].x - (self.points[n0].x - period)
            };
        } else {
            x = cpp_max(x, self.x_limits.min);
            x = cpp_min(x, self.x_limits.max);
            let mut lower = 0;
            if x >= self.points[0].x {
                let upper = self.points.partition_point(|point| point.x <= x);
                if upper > 0 {
                    lower = (upper - 1).min(self.points.len() - 2);
                }
            }
            n0 = lower;
            n1 = n0 + 1;
            h = self.points[n1].x - self.points[n0].x;
        }

        let raw = if !self.periodic
            && (x <= self.points[0].x || x >= self.points[self.points.len() - 1].x)
        {
            let point = if x <= self.points[0].x {
                self.points[0]
            } else {
                self.points[self.points.len() - 1]
            };
            point.y + (x - point.x) * point.dy
        } else {
            let dx = (x - self.points[n0].x) / h;
            let dx2 = dx * dx;
            let dx3 = dx2 * dx;
            let h00 = 2.0 * dx3 - 3.0 * dx2 + 1.0;
            let h10 = dx3 - 2.0 * dx2 + dx;
            let h01 = -2.0 * dx3 + 3.0 * dx2;
            let h11 = dx3 - dx2;
            h00 * self.points[n0].y
                + h10 * h * self.points[n0].dy
                + h01 * self.points[n1].y
                + h11 * h * self.points[n1].dy
        };
        let clipped = cpp_min(cpp_max(raw, self.y_limits.min), self.y_limits.max);
        if clipped.is_finite() {
            Ok(clipped)
        } else {
            Err(SplineError::NonFiniteResult)
        }
    }

    fn initialize_tangents(&mut self, tangent_kind: TangentKind) -> Result<(), SplineError> {
        match tangent_kind {
            TangentKind::SmoothCubic => self.initialize_smooth_cubic(),
            TangentKind::CatmullRom => {
                self.initialize_catmull_rom();
                Ok(())
            }
            TangentKind::MonotoneHermite => self.initialize_monotone_hermite(),
            TangentKind::MonotoneHermiteVariant => self.initialize_monotone_hermite_variant(),
        }
    }

    fn initialize_catmull_rom(&mut self) {
        if self.points.len() == 1 {
            self.points[0].dy = 0.0;
            return;
        }
        let n = self.points.len();
        if self.periodic {
            let period = self.x_limits.max - self.x_limits.min;
            self.points[0].dy = (self.points[1].y - self.points[n - 1].y)
                / (self.points[1].x - self.points[n - 1].x + period);
            for i in 1..n - 1 {
                self.points[i].dy = (self.points[i + 1].y - self.points[i - 1].y)
                    / (self.points[i + 1].x - self.points[i - 1].x);
            }
            self.points[n - 1].dy = (self.points[0].y - self.points[n - 2].y)
                / (self.points[0].x - self.points[n - 2].x + period);
        } else {
            self.points[0].dy =
                (self.points[1].y - self.points[0].y) / (self.points[1].x - self.points[0].x);
            for i in 1..n - 1 {
                self.points[i].dy = (self.points[i + 1].y - self.points[i - 1].y)
                    / (self.points[i + 1].x - self.points[i - 1].x);
            }
            self.points[n - 1].dy = (self.points[n - 1].y - self.points[n - 2].y)
                / (self.points[n - 1].x - self.points[n - 2].x);
        }
    }

    fn initialize_monotone_hermite(&mut self) -> Result<(), SplineError> {
        if self.points.len() == 1 {
            self.points[0].dy = 0.0;
            return Ok(());
        }
        let n = self.points.len();
        if self.periodic {
            let period = self.x_limits.max - self.x_limits.min;
            let mut delta = allocate_vec(n, size_of::<f32>())?;
            for i in 0..n - 1 {
                delta.push(
                    (self.points[i + 1].y - self.points[i].y)
                        / (self.points[i + 1].x - self.points[i].x),
                );
            }
            delta.push(
                (self.points[0].y - self.points[n - 1].y)
                    / (self.points[0].x - self.points[n - 1].x + period),
            );
            self.points[0].dy = if delta[n - 1] * delta[0] <= 0.0 {
                0.0
            } else {
                (delta[n - 1] + delta[0]) / 2.0
            };
            for i in 1..n {
                self.points[i].dy = if delta[i - 1] * delta[i] <= 0.0 {
                    0.0
                } else {
                    (delta[i - 1] + delta[i]) / 2.0
                };
            }
            for i in 0..n {
                let next = if i + 1 < n { i + 1 } else { 0 };
                if delta[i].abs() < f32::EPSILON {
                    self.points[i].dy = 0.0;
                    self.points[next].dy = 0.0;
                } else {
                    let alpha = self.points[i].dy / delta[i];
                    let beta = self.points[next].dy / delta[i];
                    let tau = alpha * alpha + beta * beta;
                    if tau > 9.0 {
                        self.points[i].dy = 3.0 * alpha * delta[i] / tau.sqrt();
                        self.points[next].dy = 3.0 * beta * delta[i] / tau.sqrt();
                    }
                }
            }
        } else {
            let mut delta = allocate_vec(n - 1, size_of::<f32>())?;
            for i in 0..n - 1 {
                delta.push(
                    (self.points[i + 1].y - self.points[i].y)
                        / (self.points[i + 1].x - self.points[i].x),
                );
            }
            self.points[0].dy = delta[0];
            for i in 1..n - 1 {
                self.points[i].dy = if delta[i - 1] * delta[i] <= 0.0 {
                    0.0
                } else {
                    (delta[i - 1] + delta[i]) / 2.0
                };
            }
            if n >= 2 {
                self.points[n - 1].dy = delta[n - 2];
            }
            for i in 0..n - 1 {
                if delta[i].abs() < f32::EPSILON {
                    self.points[i].dy = 0.0;
                    self.points[i + 1].dy = 0.0;
                } else {
                    let alpha = self.points[i].dy / delta[i];
                    let beta = self.points[i + 1].dy / delta[i];
                    let tau = alpha * alpha + beta * beta;
                    if tau > 9.0 {
                        self.points[i].dy = 3.0 * alpha * delta[i] / tau.sqrt();
                        self.points[i + 1].dy = 3.0 * beta * delta[i] / tau.sqrt();
                    }
                }
            }
        }
        Ok(())
    }

    fn initialize_monotone_hermite_variant(&mut self) -> Result<(), SplineError> {
        if self.points.len() == 1 {
            self.points[0].dy = 0.0;
            return Ok(());
        }
        let n = self.points.len();
        if self.periodic {
            let period = self.x_limits.max - self.x_limits.min;
            let mut h = allocate_vec(n, size_of::<f32>())?;
            let mut delta = allocate_vec(n, size_of::<f32>())?;
            for i in 0..n - 1 {
                let interval = self.points[i + 1].x - self.points[i].x;
                h.push(interval);
                delta.push((self.points[i + 1].y - self.points[i].y) / interval);
            }
            let interval = self.points[0].x - self.points[n - 1].x + period;
            h.push(interval);
            delta.push((self.points[0].y - self.points[n - 1].y) / interval);
            self.points[0].dy = monotone_variant_slope(delta[n - 1], delta[0], h[n - 1], h[0]);
            for i in 1..n {
                self.points[i].dy = monotone_variant_slope(delta[i - 1], delta[i], h[i - 1], h[i]);
            }
        } else {
            let mut h = allocate_vec(n - 1, size_of::<f32>())?;
            let mut delta = allocate_vec(n - 1, size_of::<f32>())?;
            for i in 0..n - 1 {
                let interval = self.points[i + 1].x - self.points[i].x;
                h.push(interval);
                delta.push((self.points[i + 1].y - self.points[i].y) / interval);
            }
            self.points[0].dy = delta[0];
            for i in 1..n - 1 {
                self.points[i].dy = monotone_variant_slope(delta[i - 1], delta[i], h[i - 1], h[i]);
            }
            if n >= 2 {
                self.points[n - 1].dy = delta[n - 2];
            }
        }
        Ok(())
    }

    fn initialize_smooth_cubic(&mut self) -> Result<(), SplineError> {
        if self.points.len() == 1 {
            self.points[0].dy = 0.0;
            return Ok(());
        }

        let n = self.points.len();
        let interval_count = if self.periodic { n } else { n - 1 };
        let mut delta_x = allocate_vec(interval_count, size_of::<f32>())?;
        let mut delta_y = allocate_vec(interval_count, size_of::<f32>())?;
        for i in 0..n - 1 {
            delta_x.push(self.points[i + 1].x - self.points[i].x);
            delta_y.push(self.points[i + 1].y - self.points[i].y);
        }
        if self.periodic {
            let period = self.x_limits.max - self.x_limits.min;
            delta_x.push(self.points[0].x - self.points[n - 1].x + period);
            delta_y.push(self.points[0].y - self.points[n - 1].y);
        }

        let mut matrix = Matrix::new(n, !self.periodic)?;
        let mut right_hand_side = zeroed_vec(n, size_of::<f32>())?;
        for i in 1..n - 1 {
            matrix.set(i, i - 1, delta_x[i - 1] / 6.0);
            matrix.set(i, i, (delta_x[i - 1] + delta_x[i]) / 3.0);
            matrix.set(i, i + 1, delta_x[i] / 6.0);
            right_hand_side[i] = delta_y[i] / delta_x[i] - delta_y[i - 1] / delta_x[i - 1];
        }
        if self.periodic {
            matrix.set(0, 0, (delta_x[n - 1] + delta_x[0]) / 3.0);
            matrix.set(n - 1, n - 1, (delta_x[n - 2] + delta_x[n - 1]) / 3.0);
            right_hand_side[0] = delta_y[0] / delta_x[0] - delta_y[n - 1] / delta_x[n - 1];
            right_hand_side[n - 1] =
                delta_y[n - 1] / delta_x[n - 1] - delta_y[n - 2] / delta_x[n - 2];
            if n > 2 {
                matrix.set(0, 1, delta_x[0] / 6.0);
                matrix.set(n - 1, n - 2, delta_x[n - 2] / 6.0);
                matrix.set(0, n - 1, delta_x[n - 1] / 6.0);
                matrix.set(n - 1, 0, delta_x[n - 1] / 6.0);
            } else {
                let off_diagonal = (delta_x[0] + delta_x[1]) / 6.0;
                matrix.set(0, 1, off_diagonal);
                matrix.set(1, 0, off_diagonal);
            }
        } else {
            matrix.set(0, 0, 1.0);
            matrix.set(n - 1, n - 1, 1.0);
            right_hand_side[0] = 0.0;
            right_hand_side[n - 1] = 0.0;
        }

        if !matrix.lu_factor() {
            return Err(SplineError::SingularSystem);
        }
        matrix.lu_solve(&mut right_hand_side);

        let mut c_i = 0.0;
        for i in 0..n - 1 {
            c_i = delta_y[i] / delta_x[i]
                - delta_x[i] / 6.0 * (right_hand_side[i + 1] - right_hand_side[i]);
            self.points[i].dy = -delta_x[i] * right_hand_side[i] / 2.0 + c_i;
        }
        self.points[n - 1].dy = if self.periodic {
            delta_x[n - 2] * right_hand_side[n - 1] / 2.0 + c_i
        } else {
            c_i
        };
        Ok(())
    }
}

#[derive(Debug)]
struct Matrix {
    size: usize,
    banded: bool,
    values: Vec<f32>,
}

impl Matrix {
    fn new(size: usize, banded: bool) -> Result<Self, SplineError> {
        let elements = if banded {
            size.checked_mul(3)
        } else {
            size.checked_mul(size)
        }
        .ok_or(SplineError::AllocationFailed {
            required_bytes: usize::MAX,
        })?;
        Ok(Self {
            size,
            banded,
            values: zeroed_vec(elements, size_of::<f32>())?,
        })
    }

    fn index(&self, row: usize, column: usize) -> usize {
        if self.banded {
            if row == column {
                return row + self.size;
            }
            if row + 1 == column {
                return row;
            }
            if row == column + 1 {
                return row + 2 * self.size;
            }
        }
        row + self.size * column
    }

    fn get(&self, row: usize, column: usize) -> f32 {
        self.values[self.index(row, column)]
    }

    fn set(&mut self, row: usize, column: usize, value: f32) {
        let index = self.index(row, column);
        self.values[index] = value;
    }

    fn lu_factor(&mut self) -> bool {
        if self.size < 1 {
            return false;
        }
        if self.banded {
            for i in 0..self.size - 1 {
                let pivot = self.get(i, i);
                if pivot == 0.0 {
                    return false;
                }
                let factor = self.get(i + 1, i) / pivot;
                self.set(i + 1, i, factor);
                let diagonal = self.get(i + 1, i + 1) - factor * self.get(i, i + 1);
                self.set(i + 1, i + 1, diagonal);
            }
        } else {
            for i in 0..self.size - 1 {
                let pivot = self.get(i, i);
                if pivot == 0.0 {
                    return false;
                }
                for row in i + 1..self.size {
                    let factor = self.get(row, i) / pivot;
                    self.set(row, i, factor);
                    for column in i + 1..self.size {
                        let value = self.get(row, column) - factor * self.get(i, column);
                        self.set(row, column, value);
                    }
                }
            }
        }
        true
    }

    fn lu_solve(&self, right_hand_side: &mut [f32]) {
        if self.size < 1 || self.size != right_hand_side.len() {
            return;
        }
        if self.banded {
            for i in 0..self.size {
                if i > 0 {
                    right_hand_side[i] -= self.get(i, i - 1) * right_hand_side[i - 1];
                }
            }
            for i in (0..self.size).rev() {
                if i + 1 < self.size {
                    right_hand_side[i] -= self.get(i, i + 1) * right_hand_side[i + 1];
                }
                right_hand_side[i] /= self.get(i, i);
            }
        } else {
            for i in 0..self.size {
                for prior in 0..i {
                    right_hand_side[i] -= self.get(i, prior) * right_hand_side[prior];
                }
            }
            for i in (0..self.size).rev() {
                for following in i + 1..self.size {
                    right_hand_side[i] -= self.get(i, following) * right_hand_side[following];
                }
                right_hand_side[i] /= self.get(i, i);
            }
        }
    }
}

/// Evaluates Darktable's V2 non-periodic spline at one coordinate.
///
/// Anchors are sorted by x before tangent construction. A one-anchor spline is
/// the anchor's constant y value.
///
/// # Errors
///
/// Returns an error for an empty set, a non-finite query, transformed duplicate
/// x coordinates, a singular cubic system, allocation failure, or a non-finite
/// result.
pub fn interpolate_value_v2(
    anchors: &[CurveAnchor],
    x: f32,
    curve_type: CurveType,
) -> Result<f32, SplineError> {
    if !x.is_finite() {
        return Err(SplineError::NonFiniteEvaluationInput);
    }
    let points = copy_anchors(anchors)?;
    Spline::unbounded(&points, TangentKind::for_curve(curve_type, false))?.evaluate(x)
}

/// Evaluates Darktable's V2 periodic spline at one coordinate.
///
/// This mirrors `interpolate_val_V2_periodic`: type 2 uses the ordinary
/// monotone Hermite derivative construction. The monotone variant is selected
/// only by [`sample_curve_v2`] for a periodic table.
///
/// # Errors
///
/// Returns an error for an empty set, non-finite inputs, a zero period,
/// duplicate x coordinates after folding, a singular cubic system, allocation
/// failure, or a non-finite result.
pub fn interpolate_value_v2_periodic(
    anchors: &[CurveAnchor],
    x: f32,
    curve_type: CurveType,
    period: f32,
) -> Result<f32, SplineError> {
    if !x.is_finite() || !period.is_finite() {
        return Err(SplineError::NonFiniteEvaluationInput);
    }
    let x_limits = Limits::new(0.0, period);
    if x_limits.max - x_limits.min == 0.0 {
        return Err(SplineError::InvalidPeriod);
    }
    let points = copy_anchors(anchors)?;
    Spline::bounded(
        &points,
        x_limits,
        Limits::infinity(),
        true,
        TangentKind::for_curve(curve_type, false),
    )?
    .evaluate(x)
}

/// Samples a curve through Darktable's V2 non-periodic or periodic table path.
///
/// Zero anchors expand to the curve box diagonal. Non-periodic construction
/// transforms anchors through the box, clips them to the source-order first and
/// last x coordinates, and sorts them. Periodic construction folds transformed
/// x coordinates by the (ordered) box width and sorts them. Periodic monotone
/// sampling uses Darktable's Fritsch-Butland variant; every other dispatch uses
/// the ordinary V2 tangent strategy.
///
/// Values are returned as the native `u16` table. Callers that mirror
/// `dt_draw_curve_smaple_values` must apply its separate `1.0 / 65536.0`
/// normalization.
///
/// # Errors
///
/// Returns an error for invalid resolutions, a zero periodic domain,
/// transformed or folded duplicate x coordinates, a singular cubic system,
/// allocation failure, a non-finite intermediate, or a value that cannot be
/// represented by the native quantization.
pub fn sample_curve_v2(
    curve: &Curve,
    sampling_resolution: u32,
    output_resolution: u32,
    periodic: bool,
) -> Result<Vec<u16>, SplineError> {
    validate_resolutions(sampling_resolution, output_resolution)?;
    let points = transform_curve_points(curve)?;
    if periodic {
        sample_periodic(curve, &points, sampling_resolution, output_resolution)
    } else {
        sample_nonperiodic(curve, &points, sampling_resolution, output_resolution)
    }
}

fn sample_nonperiodic(
    curve: &Curve,
    points: &[SourcePoint],
    sampling_resolution: u32,
    output_resolution: u32,
) -> Result<Vec<u16>, SplineError> {
    let sampling_scale = (sampling_resolution - 1) as f32;
    let output_scale = (output_resolution - 1) as f32;
    let step = sampling_scale.recip();
    let first = points.first().expect("curve expansion is nonempty");
    let last = points.last().expect("curve expansion is nonempty");
    let first_point_x = truncate_i32(first.x * sampling_scale, 0)?;
    let first_point_y = truncate_i32(first.y * output_scale, 0)?;
    let last_point_x = truncate_i32(last.x * sampling_scale, 0)?;
    let last_point_y = truncate_i32(last.y * output_scale, 0)?;
    let bounds = curve.bounds();
    let maximum_y = truncate_i32(bounds.max_y() * output_scale, 0)?;
    let minimum_y = truncate_i32(bounds.min_y() * output_scale, 0)?;

    let spline = Spline::bounded(
        points,
        Limits::new(first.x, last.x),
        Limits::new(bounds.min_y(), bounds.max_y()),
        false,
        TangentKind::for_curve(curve.curve_type(), false),
    )?;
    let sample_count = usize::try_from(sampling_resolution).expect("MAX_RESOLUTION fits usize");
    let mut output = zeroed_vec(sample_count, size_of::<u16>())?;
    for (sample, destination) in output.iter_mut().enumerate() {
        let sample_i32 = i32::try_from(sample).expect("MAX_RESOLUTION fits i32");
        let value = if sample_i32 < first_point_x {
            first_point_y as u16
        } else if sample_i32 > last_point_x {
            last_point_y as u16
        } else {
            let x = (sample as f32) * step;
            let evaluated = spline.evaluate(x)?;
            let mut quantized = round_i32(evaluated * output_scale, sample)?;
            if quantized > maximum_y {
                quantized = maximum_y;
            }
            if quantized < minimum_y {
                quantized = minimum_y;
            }
            quantized as u16
        };
        *destination = value;
    }
    Ok(output)
}

fn sample_periodic(
    curve: &Curve,
    points: &[SourcePoint],
    sampling_resolution: u32,
    output_resolution: u32,
) -> Result<Vec<u16>, SplineError> {
    let bounds = curve.bounds();
    let x_limits = Limits::new(bounds.min_x(), bounds.max_x());
    if x_limits.max - x_limits.min == 0.0 {
        return Err(SplineError::InvalidPeriod);
    }
    let spline = Spline::bounded(
        points,
        x_limits,
        Limits::new(bounds.min_y(), bounds.max_y()),
        true,
        TangentKind::for_curve(curve.curve_type(), true),
    )?;
    let sampling_scale = (sampling_resolution - 1) as f32;
    let output_scale = (output_resolution - 1) as f32;
    let step = sampling_scale.recip();
    let sample_count = usize::try_from(sampling_resolution).expect("MAX_RESOLUTION fits usize");
    let mut output = zeroed_vec(sample_count, size_of::<u16>())?;
    for (sample, destination) in output.iter_mut().enumerate() {
        let x = (sample as f32) * step;
        *destination = round_u16(spline.evaluate(x)? * output_scale, sample)?;
    }
    Ok(output)
}

fn copy_anchors(anchors: &[CurveAnchor]) -> Result<Vec<SourcePoint>, SplineError> {
    if anchors.is_empty() {
        return Err(SplineError::EmptyAnchors);
    }
    let mut points = allocate_vec(anchors.len(), size_of::<SourcePoint>())?;
    for (source, anchor) in anchors.iter().enumerate() {
        if !anchor.x().is_finite() || !anchor.y().is_finite() {
            return Err(SplineError::NonFiniteEvaluationInput);
        }
        points.push(SourcePoint {
            x: anchor.x(),
            y: anchor.y(),
            source,
        });
    }
    Ok(points)
}

fn transform_curve_points(curve: &Curve) -> Result<Vec<SourcePoint>, SplineError> {
    let bounds = curve.bounds();
    let box_width = bounds.max_x() - bounds.min_x();
    let box_height = bounds.max_y() - bounds.min_y();
    if !box_width.is_finite() || !box_height.is_finite() {
        return Err(SplineError::NonFiniteResult);
    }

    let anchors = curve.anchors();
    let capacity = anchors.len().max(2);
    let mut points = allocate_vec(capacity, size_of::<SourcePoint>())?;
    if anchors.is_empty() {
        points.push(SourcePoint {
            x: bounds.min_x(),
            y: bounds.min_y(),
            source: 0,
        });
        points.push(SourcePoint {
            x: bounds.max_x(),
            y: bounds.max_y(),
            source: 1,
        });
    } else {
        for (source, anchor) in anchors.iter().enumerate() {
            let x = anchor.x() * box_width + bounds.min_x();
            let y = anchor.y() * box_height + bounds.min_y();
            if !x.is_finite() || !y.is_finite() {
                return Err(SplineError::NonFiniteResult);
            }
            points.push(SourcePoint { x, y, source });
        }
    }
    Ok(points)
}

fn monotone_variant_slope(s1: f32, s2: f32, h1: f32, h2: f32) -> f32 {
    if s1 * s2 > 0.0 {
        let alpha = (h1 + 2.0 * h2) / (3.0 * (h1 + h2));
        s1 * s2 / (alpha * s2 + (1.0 - alpha) * s1)
    } else {
        0.0
    }
}

fn validate_resolutions(
    sampling_resolution: u32,
    output_resolution: u32,
) -> Result<(), SplineError> {
    if !(2..=MAX_RESOLUTION).contains(&sampling_resolution) {
        return Err(SplineError::InvalidSamplingResolution(sampling_resolution));
    }
    if !(2..=MAX_RESOLUTION).contains(&output_resolution) {
        return Err(SplineError::InvalidOutputResolution(output_resolution));
    }
    Ok(())
}

fn truncate_i32(value: f32, sample: usize) -> Result<i32, SplineError> {
    let widened = f64::from(value);
    if !value.is_finite() || widened < f64::from(i32::MIN) || widened > f64::from(i32::MAX) {
        return Err(SplineError::QuantizationOutOfRange { sample });
    }
    Ok(value as i32)
}

fn round_i32(value: f32, sample: usize) -> Result<i32, SplineError> {
    truncate_i32(value.round(), sample)
}

fn round_u16(value: f32, sample: usize) -> Result<u16, SplineError> {
    let rounded = value.round();
    if !rounded.is_finite() || rounded < 0.0 || rounded > f32::from(u16::MAX) {
        return Err(SplineError::QuantizationOutOfRange { sample });
    }
    Ok(rounded as u16)
}

fn cpp_max(left: f32, right: f32) -> f32 {
    if left < right { right } else { left }
}

fn cpp_min(left: f32, right: f32) -> f32 {
    if right < left { right } else { left }
}

fn allocate_vec<T>(capacity: usize, element_size: usize) -> Result<Vec<T>, SplineError> {
    let required_bytes = capacity.saturating_mul(element_size);
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| SplineError::AllocationFailed { required_bytes })?;
    Ok(values)
}

fn zeroed_vec<T: Clone + Default>(
    length: usize,
    element_size: usize,
) -> Result<Vec<T>, SplineError> {
    let mut values = allocate_vec(length, element_size)?;
    values.resize(length, T::default());
    Ok(values)
}
