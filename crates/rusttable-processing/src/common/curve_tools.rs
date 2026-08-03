//! Safe Rust mechanical port of Darktable's retained
//! `src/common/curve_tools.c` and `src/common/curve_tools.h`.
//!
//! The source fixes the 20-anchor capacity, interpolation arithmetic,
//! endpoint handling, and integer quantization used by legacy curve LUTs.
//! This port keeps those numerical boundaries while replacing unchecked
//! arrays, raw allocation, and invalid float-to-integer conversions with
//! bounded storage and typed errors.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    reason = "the retained C algorithm requires source-width casts, exact zero checks, and index-shaped arithmetic"
)]

use std::fmt;
use std::mem::size_of;

/// Maximum LUT resolution supported by the retained curve representation.
pub const MAX_RESOLUTION: u32 = 65_536;

/// Fixed anchor capacity of Darktable's `CurveData`.
pub const MAX_ANCHORS: usize = 20;

const PARAMETER_CAPACITY: usize = MAX_ANCHORS + 1;
const MATRIX_CAPACITY: usize = 3 * MAX_ANCHORS;
const MONOTONE_EPSILON: f32 = 2.0 * f32::MIN_POSITIVE;
const I32_MIN_F32: f32 = -2_147_483_648.0;
const I32_MAX_EXCLUSIVE_F32: f32 = 2_147_483_648.0;
const I32_MIN_F64: f64 = -2_147_483_648.0;
const I32_MAX_EXCLUSIVE_F64: f64 = 2_147_483_648.0;

/// Retained curve interpolation tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum CurveType {
    /// Natural cubic spline with zero second derivatives at both endpoints.
    CubicSpline = 0,
    /// Piecewise cubic Hermite interpolation with Catmull-Rom tangents.
    CatmullRom = 1,
    /// Piecewise cubic Hermite interpolation with monotonicity-limited tangents.
    MonotoneHermite = 2,
}

/// One source-shaped curve anchor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveAnchor {
    x: f32,
    y: f32,
}

impl CurveAnchor {
    const ZERO: Self = Self::new(0.0, 0.0);

    /// Creates an anchor without sorting, clamping, or normalizing it.
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Returns the source x coordinate.
    #[must_use]
    pub const fn x(self) -> f32 {
        self.x
    }

    /// Returns the source y coordinate.
    #[must_use]
    pub const fn y(self) -> f32 {
        self.y
    }
}

/// Curve box used to transform relative anchors before interpolation.
///
/// Bounds are deliberately not reordered or normalized. V1 validates the
/// transformed abscissas, while the V2 spline implementation owns its native
/// sorting, clipping, and periodic rules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveBounds {
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

impl CurveBounds {
    /// Creates finite source bounds without changing their order.
    ///
    /// # Errors
    ///
    /// Returns [`CurveError::NonFiniteBound`] when any bound is NaN or
    /// infinite.
    pub fn new(min_x: f32, max_x: f32, min_y: f32, max_y: f32) -> Result<Self, CurveError> {
        for (bound, value) in [
            ("min_x", min_x),
            ("max_x", max_x),
            ("min_y", min_y),
            ("max_y", max_y),
        ] {
            if !value.is_finite() {
                return Err(CurveError::NonFiniteBound { bound });
            }
        }
        Ok(Self {
            min_x,
            max_x,
            min_y,
            max_y,
        })
    }

    /// Returns the unit curve box used by `dt_draw_curve_new`.
    #[must_use]
    pub const fn unit() -> Self {
        Self {
            min_x: 0.0,
            max_x: 1.0,
            min_y: 0.0,
            max_y: 1.0,
        }
    }

    /// Returns the minimum x box coordinate.
    #[must_use]
    pub const fn min_x(self) -> f32 {
        self.min_x
    }

    /// Returns the maximum x box coordinate.
    #[must_use]
    pub const fn max_x(self) -> f32 {
        self.max_x
    }

    /// Returns the minimum y box coordinate.
    #[must_use]
    pub const fn min_y(self) -> f32 {
        self.min_y
    }

    /// Returns the maximum y box coordinate.
    #[must_use]
    pub const fn max_y(self) -> f32 {
        self.max_y
    }
}

/// Bounded, finite source-shaped curve data shared by V1 and V2 samplers.
///
/// Construction preserves anchor order and permits anchors outside the curve
/// box. Those properties are significant to legacy synthetic anchors and the
/// V2 periodic implementation.
#[derive(Debug, Clone, PartialEq)]
pub struct Curve {
    kind: CurveType,
    bounds: CurveBounds,
    anchors: [CurveAnchor; MAX_ANCHORS],
    anchor_count: usize,
}

impl Curve {
    /// Copies a finite curve into fixed 20-anchor storage.
    ///
    /// # Errors
    ///
    /// Returns an error when more than [`MAX_ANCHORS`] are supplied or an
    /// active coordinate is non-finite. Anchor order and range are retained.
    pub fn new(
        curve_type: CurveType,
        bounds: CurveBounds,
        anchors: &[CurveAnchor],
    ) -> Result<Self, CurveError> {
        validate_anchor_capacity(anchors.len())?;
        validate_finite_anchors(anchors)?;

        let mut stored = [CurveAnchor::ZERO; MAX_ANCHORS];
        stored[..anchors.len()].copy_from_slice(anchors);
        Ok(Self {
            kind: curve_type,
            bounds,
            anchors: stored,
            anchor_count: anchors.len(),
        })
    }

    /// Returns the interpolation tag.
    #[must_use]
    pub const fn curve_type(&self) -> CurveType {
        self.kind
    }

    /// Returns the curve box.
    #[must_use]
    pub const fn bounds(&self) -> CurveBounds {
        self.bounds
    }

    /// Returns the active source-order anchor prefix.
    #[must_use]
    pub fn anchors(&self) -> &[CurveAnchor] {
        &self.anchors[..self.anchor_count]
    }
}

/// Failure from validating, compiling, evaluating, or sampling a V1 curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveError {
    /// An input cannot fit Darktable's fixed `CurveData` storage.
    TooManyAnchors {
        /// Supplied active anchor count.
        count: usize,
        /// Retained fixed capacity.
        maximum: usize,
    },
    /// A nonempty V1 interpolation has fewer than two anchors.
    TooFewAnchors {
        /// Supplied active anchor count.
        count: usize,
        /// Minimum required by the retained interpolators.
        minimum: usize,
    },
    /// A curve box coordinate is NaN or infinite.
    NonFiniteBound {
        /// Source field name.
        bound: &'static str,
    },
    /// An active raw anchor coordinate is NaN or infinite.
    NonFiniteAnchor {
        /// Active anchor index.
        index: usize,
        /// Either `"x"` or `"y"`.
        coordinate: &'static str,
    },
    /// A direct interpolation query is NaN or infinite.
    NonFiniteEvaluationInput,
    /// V1 abscissas are not strictly increasing after box transformation.
    NonIncreasingAnchors {
        /// Left anchor index.
        left: usize,
        /// Right anchor index.
        right: usize,
    },
    /// Sampling resolution is outside `2..=MAX_RESOLUTION`.
    InvalidSamplingResolution {
        /// Rejected resolution.
        resolution: u32,
    },
    /// Output resolution is outside `2..=MAX_RESOLUTION`.
    InvalidOutputResolution {
        /// Rejected resolution.
        resolution: u32,
    },
    /// The retained tridiagonal system has a zero diagonal.
    SingularSystem,
    /// Finite input overflowed or otherwise produced a non-finite value.
    NonFiniteResult {
        /// Numerical stage that failed.
        stage: &'static str,
        /// LUT sample index when the failure belongs to one sample.
        sample: Option<usize>,
    },
    /// A source float-to-`int` conversion would be undefined in C.
    QuantizationOutOfRange {
        /// Source expression being converted.
        context: &'static str,
    },
    /// The bounded LUT allocation failed.
    AllocationFailed {
        /// Requested byte count.
        required_bytes: usize,
    },
    /// Platform size arithmetic cannot represent a bounded source value.
    SizeOverflow,
}

impl fmt::Display for CurveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::TooManyAnchors { count, maximum } => {
                write!(
                    formatter,
                    "curve has {count} active anchors; maximum is {maximum}"
                )
            }
            Self::TooFewAnchors { count, minimum } => {
                write!(
                    formatter,
                    "curve has {count} active anchors; minimum is {minimum}"
                )
            }
            Self::NonFiniteBound { bound } => {
                write!(formatter, "curve bound {bound} is non-finite")
            }
            Self::NonFiniteAnchor { index, coordinate } => {
                write!(
                    formatter,
                    "curve anchor {index} {coordinate} coordinate is non-finite"
                )
            }
            Self::NonFiniteEvaluationInput => {
                formatter.write_str("curve evaluation coordinate is non-finite")
            }
            Self::NonIncreasingAnchors { left, right } => {
                write!(
                    formatter,
                    "curve anchors {left} and {right} are not strictly increasing"
                )
            }
            Self::InvalidSamplingResolution { resolution } => {
                write!(
                    formatter,
                    "curve sampling resolution {resolution} is outside 2..={MAX_RESOLUTION}"
                )
            }
            Self::InvalidOutputResolution { resolution } => {
                write!(
                    formatter,
                    "curve output resolution {resolution} is outside 2..={MAX_RESOLUTION}"
                )
            }
            Self::SingularSystem => formatter.write_str("curve interpolation system is singular"),
            Self::NonFiniteResult {
                stage,
                sample: Some(sample),
            } => {
                write!(
                    formatter,
                    "curve {stage} produced a non-finite value at sample {sample}"
                )
            }
            Self::NonFiniteResult {
                stage,
                sample: None,
            } => {
                write!(formatter, "curve {stage} produced a non-finite value")
            }
            Self::QuantizationOutOfRange { context } => {
                write!(
                    formatter,
                    "curve {context} is outside the defined C int conversion range"
                )
            }
            Self::AllocationFailed { required_bytes } => {
                write!(
                    formatter,
                    "unable to allocate curve LUT requiring {required_bytes} bytes"
                )
            }
            Self::SizeOverflow => formatter.write_str("curve LUT size overflowed"),
        }
    }
}

impl std::error::Error for CurveError {}

/// Interpolates absolute anchors with the retained V1 implementation.
///
/// Values outside the first and last x coordinates are extrapolated using the
/// first or last interval, matching `interpolate_val`.
///
/// # Errors
///
/// Returns an error for an invalid anchor count, non-finite input, non-
/// increasing x coordinates, a singular cubic system, or non-finite arithmetic.
pub fn interpolate_value_v1(
    anchors: &[CurveAnchor],
    x: f32,
    curve_type: CurveType,
) -> Result<f32, CurveError> {
    if !x.is_finite() {
        return Err(CurveError::NonFiniteEvaluationInput);
    }
    validate_anchor_capacity(anchors.len())?;
    validate_finite_anchors(anchors)?;
    let interpolator = InterpolatorV1::from_anchors(anchors, curve_type)?;
    let value = interpolator.evaluate(x);
    if !value.is_finite() {
        return Err(CurveError::NonFiniteResult {
            stage: "evaluation",
            sample: None,
        });
    }
    Ok(value)
}

/// Samples a source-shaped curve with retained V1 endpoint and quantization
/// semantics.
///
/// An empty curve expands to the box diagonal. Endpoint extension truncates
/// before assignment, while interpolated samples multiply in `f32`, add `0.5`
/// in `f64`, and then truncate to `int`.
///
/// # Errors
///
/// Returns an error for invalid resolutions, invalid V1 anchor order or count,
/// non-finite arithmetic, undefined source float-to-int conversion, a singular
/// cubic system, or allocation failure.
pub fn sample_curve_v1(
    curve: &Curve,
    sampling_resolution: u32,
    output_resolution: u32,
) -> Result<Vec<u16>, CurveError> {
    validate_sampling_resolution(sampling_resolution)?;
    validate_output_resolution(output_resolution)?;

    let bounds = curve.bounds();
    let box_width = bounds.max_x() - bounds.min_x();
    let box_height = bounds.max_y() - bounds.min_y();
    let mut x = [0.0_f32; MAX_ANCHORS];
    let mut y = [0.0_f32; MAX_ANCHORS];
    let len;

    if curve.anchors().is_empty() {
        x[0] = bounds.min_x();
        y[0] = bounds.min_y();
        x[1] = bounds.max_x();
        y[1] = bounds.max_y();
        len = 2;
    } else {
        len = curve.anchors().len();
        for (index, anchor) in curve.anchors().iter().copied().enumerate() {
            let scaled_x = anchor.x() * box_width;
            let mapped_x = scaled_x + bounds.min_x();
            let scaled_y = anchor.y() * box_height;
            let mapped_y = scaled_y + bounds.min_y();
            if !mapped_x.is_finite() || !mapped_y.is_finite() {
                return Err(CurveError::NonFiniteResult {
                    stage: "anchor transformation",
                    sample: Some(index),
                });
            }
            x[index] = mapped_x;
            y[index] = mapped_y;
        }
    }

    let interpolator = InterpolatorV1::from_arrays(x, y, len, curve.curve_type())?;
    let sampling_scale = (sampling_resolution - 1) as f32;
    let output_scale = (output_resolution - 1) as f32;
    let resolution_reciprocal = narrow_f64(1.0 / f64::from(sampling_scale));

    let first_point_x = truncate_f32_to_i32(
        interpolator.x[0] * sampling_scale,
        "first endpoint x quantization",
    )?;
    let first_point_y = truncate_f32_to_i32(
        interpolator.y[0] * output_scale,
        "first endpoint y quantization",
    )?;
    let last = interpolator.len - 1;
    let last_point_x = truncate_f32_to_i32(
        interpolator.x[last] * sampling_scale,
        "last endpoint x quantization",
    )?;
    let last_point_y = truncate_f32_to_i32(
        interpolator.y[last] * output_scale,
        "last endpoint y quantization",
    )?;
    let max_y = truncate_f32_to_i32(bounds.max_y() * output_scale, "maximum y quantization")?;
    let min_y = truncate_f32_to_i32(bounds.min_y() * output_scale, "minimum y quantization")?;

    let sample_count =
        usize::try_from(sampling_resolution).map_err(|_| CurveError::SizeOverflow)?;
    let required_bytes = sample_count
        .checked_mul(size_of::<u16>())
        .ok_or(CurveError::SizeOverflow)?;
    let mut samples = Vec::new();
    samples
        .try_reserve_exact(sample_count)
        .map_err(|_| CurveError::AllocationFailed { required_bytes })?;

    for index in 0..sample_count {
        let source_index = i32::try_from(index).map_err(|_| CurveError::SizeOverflow)?;
        let quantized = if source_index < first_point_x {
            first_point_y
        } else if source_index > last_point_x {
            last_point_y
        } else {
            let sample_x = source_index as f32 * resolution_reciprocal;
            let value = interpolator.evaluate(sample_x);
            if !value.is_finite() {
                return Err(CurveError::NonFiniteResult {
                    stage: "evaluation",
                    sample: Some(index),
                });
            }
            let scaled = value * output_scale;
            let mut value =
                truncate_f64_to_i32(f64::from(scaled) + 0.5, "interior sample quantization")?;
            if value > max_y {
                value = max_y;
            }
            if value < min_y {
                value = min_y;
            }
            value
        };
        samples.push(quantized as u16);
    }
    Ok(samples)
}

#[derive(Debug, Clone)]
struct InterpolatorV1 {
    x: [f32; MAX_ANCHORS],
    y: [f32; MAX_ANCHORS],
    parameters: [f32; PARAMETER_CAPACITY],
    len: usize,
    kind: CurveType,
}

impl InterpolatorV1 {
    fn from_anchors(anchors: &[CurveAnchor], curve_type: CurveType) -> Result<Self, CurveError> {
        let mut x = [0.0_f32; MAX_ANCHORS];
        let mut y = [0.0_f32; MAX_ANCHORS];
        for (index, anchor) in anchors.iter().copied().enumerate() {
            x[index] = anchor.x();
            y[index] = anchor.y();
        }
        Self::from_arrays(x, y, anchors.len(), curve_type)
    }

    fn from_arrays(
        x: [f32; MAX_ANCHORS],
        y: [f32; MAX_ANCHORS],
        len: usize,
        curve_type: CurveType,
    ) -> Result<Self, CurveError> {
        if len < 2 {
            return Err(CurveError::TooFewAnchors {
                count: len,
                minimum: 2,
            });
        }
        for index in 0..len - 1 {
            if x[index + 1] <= x[index] {
                return Err(CurveError::NonIncreasingAnchors {
                    left: index,
                    right: index + 1,
                });
            }
        }

        let parameters = match curve_type {
            CurveType::CubicSpline => cubic_parameters(len, &x, &y)?,
            CurveType::CatmullRom => catmull_rom_parameters(len, &x, &y),
            CurveType::MonotoneHermite => monotone_hermite_parameters(len, &x, &y),
        };
        let parameter_len = match curve_type {
            CurveType::MonotoneHermite => len + 1,
            CurveType::CubicSpline | CurveType::CatmullRom => len,
        };
        if parameters[..parameter_len]
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(CurveError::NonFiniteResult {
                stage: "parameter calculation",
                sample: None,
            });
        }
        Ok(Self {
            x,
            y,
            parameters,
            len,
            kind: curve_type,
        })
    }

    fn evaluate(&self, x: f32) -> f32 {
        match self.kind {
            CurveType::CubicSpline => cubic_value(self.len, &self.x, &self.y, &self.parameters, x),
            CurveType::CatmullRom | CurveType::MonotoneHermite => {
                hermite_value(self.len, &self.x, &self.y, &self.parameters, x)
            }
        }
    }
}

const fn validate_anchor_capacity(count: usize) -> Result<(), CurveError> {
    if count > MAX_ANCHORS {
        return Err(CurveError::TooManyAnchors {
            count,
            maximum: MAX_ANCHORS,
        });
    }
    Ok(())
}

fn validate_finite_anchors(anchors: &[CurveAnchor]) -> Result<(), CurveError> {
    for (index, anchor) in anchors.iter().copied().enumerate() {
        if !anchor.x().is_finite() {
            return Err(CurveError::NonFiniteAnchor {
                index,
                coordinate: "x",
            });
        }
        if !anchor.y().is_finite() {
            return Err(CurveError::NonFiniteAnchor {
                index,
                coordinate: "y",
            });
        }
    }
    Ok(())
}

fn validate_sampling_resolution(resolution: u32) -> Result<(), CurveError> {
    if !(2..=MAX_RESOLUTION).contains(&resolution) {
        return Err(CurveError::InvalidSamplingResolution { resolution });
    }
    Ok(())
}

fn validate_output_resolution(resolution: u32) -> Result<(), CurveError> {
    if !(2..=MAX_RESOLUTION).contains(&resolution) {
        return Err(CurveError::InvalidOutputResolution { resolution });
    }
    Ok(())
}

fn cubic_parameters(
    len: usize,
    x: &[f32; MAX_ANCHORS],
    y: &[f32; MAX_ANCHORS],
) -> Result<[f32; PARAMETER_CAPACITY], CurveError> {
    let mut matrix = [0.0_f32; MATRIX_CAPACITY];
    let mut right_hand_side = [0.0_f32; MAX_ANCHORS];

    right_hand_side[0] = 0.0;
    matrix[1] = 1.0;
    matrix[3] = 0.0;

    for index in 1..len - 1 {
        let next_y_delta = y[index + 1] - y[index];
        let next_x_delta = x[index + 1] - x[index];
        let next_slope = next_y_delta / next_x_delta;
        let previous_y_delta = y[index] - y[index - 1];
        let previous_x_delta = x[index] - x[index - 1];
        let previous_slope = previous_y_delta / previous_x_delta;
        right_hand_side[index] = next_slope - previous_slope;

        let left_width = x[index] - x[index - 1];
        matrix[2 + (index - 1) * 3] = narrow_f64(f64::from(left_width) / 6.0);
        let full_width = x[index + 1] - x[index - 1];
        matrix[1 + index * 3] = narrow_f64(f64::from(full_width) / 3.0);
        let right_width = x[index + 1] - x[index];
        matrix[(index + 1) * 3] = narrow_f64(f64::from(right_width) / 6.0);
    }

    right_hand_side[len - 1] = 0.0;
    matrix[2 + (len - 2) * 3] = 0.0;
    matrix[1 + (len - 1) * 3] = 1.0;

    let solved = solve_tridiagonal(len, matrix, &right_hand_side)?;
    let mut parameters = [0.0_f32; PARAMETER_CAPACITY];
    parameters[..len].copy_from_slice(&solved[..len]);
    Ok(parameters)
}

fn solve_tridiagonal(
    len: usize,
    mut matrix: [f32; MATRIX_CAPACITY],
    right_hand_side: &[f32; MAX_ANCHORS],
) -> Result<[f32; MAX_ANCHORS], CurveError> {
    for index in 0..len {
        if matrix[1 + index * 3] == 0.0 {
            return Err(CurveError::SingularSystem);
        }
    }

    let mut solution = [0.0_f32; MAX_ANCHORS];
    solution[..len].copy_from_slice(&right_hand_side[..len]);

    for index in 1..len {
        let multiplier = matrix[2 + (index - 1) * 3] / matrix[1 + (index - 1) * 3];
        let diagonal_delta = multiplier * matrix[index * 3];
        matrix[1 + index * 3] -= diagonal_delta;
        let right_delta = multiplier * solution[index - 1];
        solution[index] -= right_delta;
    }

    solution[len - 1] /= matrix[1 + (len - 1) * 3];
    for index in (0..len - 1).rev() {
        let upper_product = matrix[(index + 1) * 3] * solution[index + 1];
        let numerator = solution[index] - upper_product;
        solution[index] = numerator / matrix[1 + index * 3];
    }
    Ok(solution)
}

fn catmull_rom_parameters(
    len: usize,
    x: &[f32; MAX_ANCHORS],
    y: &[f32; MAX_ANCHORS],
) -> [f32; PARAMETER_CAPACITY] {
    let mut tangents = [0.0_f32; PARAMETER_CAPACITY];
    let first_y_delta = y[1] - y[0];
    let first_x_delta = x[1] - x[0];
    tangents[0] = first_y_delta / first_x_delta;
    for index in 1..len - 1 {
        let y_delta = y[index + 1] - y[index - 1];
        let x_delta = x[index + 1] - x[index - 1];
        tangents[index] = y_delta / x_delta;
    }
    let last_y_delta = y[len - 1] - y[len - 2];
    let last_x_delta = x[len - 1] - x[len - 2];
    tangents[len - 1] = last_y_delta / last_x_delta;
    tangents
}

fn monotone_hermite_parameters(
    len: usize,
    x: &[f32; MAX_ANCHORS],
    y: &[f32; MAX_ANCHORS],
) -> [f32; PARAMETER_CAPACITY] {
    let mut delta = [0.0_f32; MAX_ANCHORS];
    let mut tangents = [0.0_f32; PARAMETER_CAPACITY];

    for index in 0..len - 1 {
        let y_delta = y[index + 1] - y[index];
        let x_delta = x[index + 1] - x[index];
        delta[index] = y_delta / x_delta;
    }
    delta[len - 1] = delta[len - 2];
    tangents[0] = delta[0];
    tangents[len - 1] = delta[len - 1];

    for index in 1..len - 1 {
        let slope_sum = delta[index - 1] + delta[index];
        tangents[index] = slope_sum * 0.5;
    }

    for index in 0..len {
        if delta[index].abs() < MONOTONE_EPSILON {
            tangents[index] = 0.0;
            tangents[index + 1] = 0.0;
        } else {
            let alpha = tangents[index] / delta[index];
            let beta = tangents[index + 1] / delta[index];
            let alpha_squared = alpha * alpha;
            let beta_squared = beta * beta;
            let tau = alpha_squared + beta_squared;
            if tau > 9.0 {
                let root = tau.sqrt();
                let alpha_scaled = 3.0 * alpha;
                let alpha_scaled = alpha_scaled * delta[index];
                tangents[index] = alpha_scaled / root;
                let beta_scaled = 3.0 * beta;
                let beta_scaled = beta_scaled * delta[index];
                tangents[index + 1] = beta_scaled / root;
            }
        }
    }
    tangents
}

fn hermite_value(
    len: usize,
    x: &[f32; MAX_ANCHORS],
    y: &[f32; MAX_ANCHORS],
    tangents: &[f32; PARAMETER_CAPACITY],
    x_value: f32,
) -> f32 {
    let mut interval = len - 2;
    for index in 0..len - 2 {
        if x_value < x[index + 1] {
            interval = index;
            break;
        }
    }

    let m0 = tangents[interval];
    let m1 = tangents[interval + 1];
    let h = x[interval + 1] - x[interval];
    let x_delta = x_value - x[interval];
    let normalized = x_delta / h;
    let normalized_squared = normalized * normalized;
    let normalized_cubed = normalized * normalized_squared;

    // These literals are unsuffixed in C. The basis is therefore evaluated in
    // double precision and narrowed once at each `const float` assignment.
    let h00_left = 2.0 * f64::from(normalized_cubed);
    let h00_right = 3.0 * f64::from(normalized_squared);
    let h00 = narrow_f64((h00_left - h00_right) + 1.0);
    let h10_left = f64::from(normalized_cubed);
    let h10_right = 2.0 * f64::from(normalized_squared);
    let h10 = narrow_f64((h10_left - h10_right) + f64::from(normalized));
    let h01_left = -2.0 * f64::from(normalized_cubed);
    let h01_right = 3.0 * f64::from(normalized_squared);
    let h01 = narrow_f64(h01_left + h01_right);
    let h11_left = f64::from(normalized_cubed);
    let h11_right = f64::from(normalized_squared);
    let h11 = narrow_f64(h11_left - h11_right);

    let first = h00 * y[interval];
    let second = h10 * h;
    let second = second * m0;
    let mut value = first + second;
    let third = h01 * y[interval + 1];
    value += third;
    let fourth = h11 * h;
    let fourth = fourth * m1;
    value + fourth
}

fn cubic_value(
    len: usize,
    x: &[f32; MAX_ANCHORS],
    y: &[f32; MAX_ANCHORS],
    second_derivatives: &[f32; PARAMETER_CAPACITY],
    x_value: f32,
) -> f32 {
    let mut interval = len - 2;
    for index in 0..len - 1 {
        if x_value < x[index + 1] {
            interval = index;
            break;
        }
    }

    let x_delta = x_value - x[interval];
    let h = x[interval + 1] - x[interval];
    let y_delta = y[interval + 1] - y[interval];
    let slope = y_delta / h;
    let derivative_delta = second_derivatives[interval + 1] - second_derivatives[interval];

    // The unsuffixed constants promote the nested polynomial to double after
    // the explicitly float-valued differences and slope above.
    let curvature_next = f64::from(second_derivatives[interval + 1]) / 6.0;
    let curvature_current = f64::from(second_derivatives[interval]) / 3.0;
    let curvature_sum = curvature_next + curvature_current;
    let curvature = curvature_sum * f64::from(h);
    let leading = f64::from(slope) - curvature;

    let denominator = 6.0 * f64::from(h);
    let derivative_term = f64::from(derivative_delta) / denominator;
    let inner_current = 0.5 * f64::from(second_derivatives[interval]);
    let inner_delta = f64::from(x_delta) * derivative_term;
    let inner = inner_current + inner_delta;
    let nested = f64::from(x_delta) * inner;
    let polynomial = leading + nested;
    let offset = f64::from(x_delta) * polynomial;
    narrow_f64(f64::from(y[interval]) + offset)
}

fn truncate_f32_to_i32(value: f32, context: &'static str) -> Result<i32, CurveError> {
    if !value.is_finite() || !(I32_MIN_F32..I32_MAX_EXCLUSIVE_F32).contains(&value) {
        return Err(CurveError::QuantizationOutOfRange { context });
    }
    Ok(value as i32)
}

fn truncate_f64_to_i32(value: f64, context: &'static str) -> Result<i32, CurveError> {
    if !value.is_finite() || !(I32_MIN_F64..I32_MAX_EXCLUSIVE_F64).contains(&value) {
        return Err(CurveError::QuantizationOutOfRange { context });
    }
    Ok(value as i32)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "this helper marks the exact C double-to-float assignment boundaries"
)]
const fn narrow_f64(value: f64) -> f32 {
    value as f32
}
