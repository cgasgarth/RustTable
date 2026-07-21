#![allow(
    clippy::cast_lossless,
    clippy::missing_errors_doc,
    clippy::manual_midpoint,
    clippy::needless_range_loop
)]

use std::fmt;

use crate::RasterDimensions;

use super::codec::{PerspectiveConfig, Quad};

const EPSILON: f64 = 1.0e-12;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    x: f64,
    y: f64,
}

impl Point {
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    #[must_use]
    pub const fn x(self) -> f64 {
        self.x
    }
    #[must_use]
    pub const fn y(self) -> f64 {
        self.y
    }
    pub fn validate(self) -> Result<(), PointError> {
        (self.x.is_finite() && self.y.is_finite())
            .then_some(())
            .ok_or(PointError::NonFinite)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointError {
    NonFinite,
}

impl fmt::Display for PointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("point is non-finite")
    }
}
impl std::error::Error for PointError {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl Rect {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Result<Self, RectError> {
        if [min_x, min_y, max_x, max_y]
            .iter()
            .any(|value| !value.is_finite())
        {
            return Err(RectError::NonFinite);
        }
        if max_x < min_x || max_y < min_y {
            return Err(RectError::Reversed);
        }
        Ok(Self {
            min_x,
            min_y,
            max_x,
            max_y,
        })
    }
    #[must_use]
    pub const fn min_x(self) -> f64 {
        self.min_x
    }
    #[must_use]
    pub const fn min_y(self) -> f64 {
        self.min_y
    }
    #[must_use]
    pub const fn max_x(self) -> f64 {
        self.max_x
    }
    #[must_use]
    pub const fn max_y(self) -> f64 {
        self.max_y
    }
    #[must_use]
    pub const fn width(self) -> f64 {
        self.max_x - self.min_x
    }
    #[must_use]
    pub const fn height(self) -> f64 {
        self.max_y - self.min_y
    }
    #[must_use]
    pub const fn center(self) -> Point {
        Point::new(
            (self.min_x + self.max_x) * 0.5,
            (self.min_y + self.max_y) * 0.5,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RectError {
    NonFinite,
    Reversed,
    Overflow,
}

impl fmt::Display for RectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "rectangle is non-finite",
            Self::Reversed => "rectangle bounds are reversed",
            Self::Overflow => "rectangle dimensions overflowed",
        })
    }
}
impl std::error::Error for RectError {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Homography {
    coefficients: [f64; 9],
}

impl Homography {
    pub fn new(coefficients: [f64; 9]) -> Result<Self, HomographyError> {
        if coefficients.iter().any(|value| !value.is_finite()) {
            return Err(HomographyError::NonFinite);
        }
        let determinant = determinant(coefficients);
        if !determinant.is_finite() || determinant.abs() <= EPSILON {
            return Err(HomographyError::Singular);
        }
        let scale = coefficients
            .iter()
            .map(|value| value.abs())
            .fold(0.0, f64::max);
        if scale > 1.0e12 {
            return Err(HomographyError::IllConditioned);
        }
        Ok(Self { coefficients })
    }

    #[must_use]
    pub const fn identity() -> Self {
        Self {
            coefficients: [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        }
    }
    #[must_use]
    pub const fn coefficients(self) -> [f64; 9] {
        self.coefficients
    }

    pub fn inverse(self) -> Result<Self, HomographyError> {
        Self::new(invert(self.coefficients)?)
    }

    pub fn apply(self, point: Point) -> Result<Point, TransformError> {
        point
            .validate()
            .map_err(|_| TransformError::NonFinitePoint)?;
        let c = self.coefficients;
        let denominator = c[6] * point.x + c[7] * point.y + c[8];
        if !denominator.is_finite() || denominator.abs() <= EPSILON {
            return Err(TransformError::AtInfinity);
        }
        let x = (c[0] * point.x + c[1] * point.y + c[2]) / denominator;
        let y = (c[3] * point.x + c[4] * point.y + c[5]) / denominator;
        Point::new(x, y)
            .validate()
            .map_err(|_| TransformError::NonFiniteResult)?;
        Ok(Point::new(x, y))
    }

    #[must_use]
    pub fn then(self, next: Self) -> Self {
        // Both operands have already been checked; multiplication can only
        // fail for pathological coefficients, which callers cannot create.
        Self {
            coefficients: multiply(next.coefficients, self.coefficients),
        }
    }

    pub fn bounds(self, dimensions: RasterDimensions) -> Result<Rect, TransformError> {
        let last_x = f64::from(dimensions.width() - 1);
        let last_y = f64::from(dimensions.height() - 1);
        let points = [
            Point::new(0.0, 0.0),
            Point::new(last_x, 0.0),
            Point::new(last_x, last_y),
            Point::new(0.0, last_y),
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for point in points {
            let point = self.apply(point)?;
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
            max_x = max_x.max(point.x);
            max_y = max_y.max(point.y);
        }
        Rect::new(min_x, min_y, max_x, max_y).map_err(|_| TransformError::InvalidBounds)
    }

    pub fn from_quad(
        quad: Quad,
        output_width: u32,
        output_height: u32,
    ) -> Result<Self, HomographyError> {
        if output_width < 2 || output_height < 2 {
            return Err(HomographyError::DegenerateGeometry);
        }
        let source = quad.points();
        let target = [
            Point::new(0.0, 0.0),
            Point::new(f64::from(output_width - 1), 0.0),
            Point::new(f64::from(output_width - 1), f64::from(output_height - 1)),
            Point::new(0.0, f64::from(output_height - 1)),
        ];
        for point in source {
            point.validate().map_err(|_| HomographyError::NonFinite)?;
        }
        solve_quad(source, target)
    }

    pub fn camera(
        config: &PerspectiveConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, HomographyError> {
        if config.rotation().get().abs() <= f32::EPSILON
            && config.lensshift_v().get().abs() <= f32::EPSILON
            && config.lensshift_h().get().abs() <= f32::EPSILON
            && config.shear().get().abs() <= f32::EPSILON
            && (config.aspect().get() - 1.0).abs() <= f32::EPSILON
        {
            return Ok(Self::identity());
        }
        let width = f64::from(dimensions.width());
        let height = f64::from(dimensions.height());
        let angle = f64::from(config.rotation().get()).to_radians();
        let (sine, cosine) = angle.sin_cos();
        let aspect = f64::from(config.aspect().get()).sqrt();
        let vertical = shift_matrix(f64::from(config.lensshift_v().get()), width, height, true)?;
        let horizontal = shift_matrix(f64::from(config.lensshift_h().get()), width, height, false)?;
        let vertical_compression =
            compression_matrix(vertical_factor(config, width, height, true), width, true);
        let horizontal_compression =
            compression_matrix(vertical_factor(config, width, height, false), height, false);
        let rotate = [
            cosine,
            -sine,
            (-0.5 * height * cosine) + (0.5 * width * sine) + (0.5 * height),
            sine,
            cosine,
            (-0.5 * height * sine) - (0.5 * width * cosine) + (0.5 * width),
            0.0,
            0.0,
            1.0,
        ];
        let shear = [
            1.0,
            f64::from(config.shear().get()),
            0.0,
            f64::from(config.shear().get()),
            1.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ];
        let scale = [aspect, 0.0, 0.0, 0.0, 1.0 / aspect, 0.0, 0.0, 0.0, 1.0];
        let raw = multiply(
            scale,
            multiply(
                horizontal_compression,
                multiply(
                    horizontal,
                    multiply(
                        swap_xy(),
                        multiply(
                            vertical_compression,
                            multiply(vertical, multiply(shear, multiply(rotate, swap_xy()))),
                        ),
                    ),
                ),
            ),
        );
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        for point in [
            Point::new(0.0, 0.0),
            Point::new(width - 1.0, 0.0),
            Point::new(width - 1.0, height - 1.0),
            Point::new(0.0, height - 1.0),
        ] {
            let point = apply_raw(raw, point)?;
            min_x = min_x.min(point.x);
            min_y = min_y.min(point.y);
        }
        let shifted = multiply([1.0, 0.0, -min_x, 0.0, 1.0, -min_y, 0.0, 0.0, 1.0], raw);
        Self::new(shifted)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomographyError {
    NonFinite,
    Singular,
    IllConditioned,
    DegenerateGeometry,
    ArithmeticOverflow,
}

impl fmt::Display for HomographyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "homography is non-finite",
            Self::Singular => "homography is singular",
            Self::IllConditioned => "homography is ill-conditioned",
            Self::DegenerateGeometry => "homography geometry is degenerate",
            Self::ArithmeticOverflow => "homography arithmetic overflowed",
        })
    }
}
impl std::error::Error for HomographyError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformError {
    NonFinitePoint,
    NonFiniteResult,
    AtInfinity,
    InvalidBounds,
    OutsideSource,
    Overflow,
}

impl fmt::Display for TransformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinitePoint => "transform point is non-finite",
            Self::NonFiniteResult => "transform result is non-finite",
            Self::AtInfinity => "transform point is at projective infinity",
            Self::InvalidBounds => "transform bounds are invalid",
            Self::OutsideSource => "transform point is outside the source",
            Self::Overflow => "transform arithmetic overflowed",
        })
    }
}
impl std::error::Error for TransformError {}

fn determinant(c: [f64; 9]) -> f64 {
    c[0] * (c[4] * c[8] - c[5] * c[7]) - c[1] * (c[3] * c[8] - c[5] * c[6])
        + c[2] * (c[3] * c[7] - c[4] * c[6])
}

fn invert(c: [f64; 9]) -> Result<[f64; 9], HomographyError> {
    let determinant = determinant(c);
    if determinant.abs() <= EPSILON || !determinant.is_finite() {
        return Err(HomographyError::Singular);
    }
    let inverse = [
        c[4] * c[8] - c[5] * c[7],
        c[2] * c[7] - c[1] * c[8],
        c[1] * c[5] - c[2] * c[4],
        c[5] * c[6] - c[3] * c[8],
        c[0] * c[8] - c[2] * c[6],
        c[2] * c[3] - c[0] * c[5],
        c[3] * c[7] - c[4] * c[6],
        c[1] * c[6] - c[0] * c[7],
        c[0] * c[4] - c[1] * c[3],
    ];
    Ok(inverse.map(|value| value / determinant))
}

fn multiply(left: [f64; 9], right: [f64; 9]) -> [f64; 9] {
    let mut output = [0.0; 9];
    for row in 0..3 {
        for column in 0..3 {
            output[row * 3 + column] = (0..3)
                .map(|index| left[row * 3 + index] * right[index * 3 + column])
                .sum();
        }
    }
    output
}

fn apply_raw(c: [f64; 9], point: Point) -> Result<Point, HomographyError> {
    let denominator = c[6] * point.x() + c[7] * point.y() + c[8];
    if denominator.abs() <= EPSILON || !denominator.is_finite() {
        return Err(HomographyError::Singular);
    }
    let result = Point::new(
        (c[0] * point.x() + c[1] * point.y() + c[2]) / denominator,
        (c[3] * point.x() + c[4] * point.y() + c[5]) / denominator,
    );
    result
        .validate()
        .map_err(|_| HomographyError::NonFinite)
        .map(|()| result)
}

fn swap_xy() -> [f64; 9] {
    [0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]
}

fn shift_matrix(
    shift: f64,
    width: f64,
    height: f64,
    vertical: bool,
) -> Result<[f64; 9], HomographyError> {
    let exponential = shift.exp();
    let denominator = exponential + 1.0;
    if !exponential.is_finite() || denominator.abs() <= EPSILON {
        return Err(HomographyError::NonFinite);
    }
    let (major, minor) = if vertical {
        (width, height)
    } else {
        (height, width)
    };
    let matrix = [
        exponential,
        0.0,
        0.0,
        0.5 * (exponential - 1.0) * major / minor,
        2.0 * exponential / denominator,
        -0.5 * (exponential - 1.0) * major / denominator,
        (exponential - 1.0) / minor,
        0.0,
        1.0,
    ];
    if vertical {
        Ok(matrix)
    } else {
        Ok(multiply(swap_xy(), multiply(matrix, swap_xy())))
    }
}

fn vertical_factor(config: &PerspectiveConfig, width: f64, height: f64, vertical: bool) -> f64 {
    let focal = f64::from(config.focal_length().get());
    let ratio = if vertical {
        height / width
    } else {
        width / height
    };
    let fdb = focal / (14.4 + (ratio - 1.0) * 7.2);
    let shift = if vertical {
        config.lensshift_v()
    } else {
        config.lensshift_h()
    };
    let rad = fdb * (f64::from(shift.get()).exp() - 1.0) / (f64::from(shift.get()).exp() + 1.0);
    let alpha = rad.atan().clamp(-1.5, 1.5);
    let sine = (0.5 * alpha).sin();
    (1.0 + 2.0 * ((100.0 - f64::from(config.orthocorr().get())) / -100.0) * sine * sine).max(0.1)
}

fn compression_matrix(factor: f64, center: f64, horizontal_axis: bool) -> [f64; 9] {
    if horizontal_axis {
        [
            1.0,
            0.0,
            0.0,
            0.0,
            factor,
            0.5 * center * (1.0 - factor),
            0.0,
            0.0,
            1.0,
        ]
    } else {
        [
            factor,
            0.0,
            0.0,
            0.0,
            1.0,
            0.5 * center * (1.0 - factor),
            0.0,
            0.0,
            1.0,
        ]
    }
}

fn solve_quad(source: [Point; 4], target: [Point; 4]) -> Result<Homography, HomographyError> {
    let mut matrix = [[0.0; 9]; 8];
    for (index, (from, to)) in source.into_iter().zip(target).enumerate() {
        let row = index * 2;
        matrix[row] = [
            from.x(),
            from.y(),
            1.0,
            0.0,
            0.0,
            0.0,
            -to.x() * from.x(),
            -to.x() * from.y(),
            to.x(),
        ];
        matrix[row + 1] = [
            0.0,
            0.0,
            0.0,
            from.x(),
            from.y(),
            1.0,
            -to.y() * from.x(),
            -to.y() * from.y(),
            to.y(),
        ];
    }
    for pivot in 0..8 {
        let mut best = pivot;
        for row in (pivot + 1)..8 {
            if matrix[row][pivot].abs() > matrix[best][pivot].abs() {
                best = row;
            }
        }
        if matrix[best][pivot].abs() <= EPSILON {
            return Err(HomographyError::DegenerateGeometry);
        }
        matrix.swap(pivot, best);
        let divisor = matrix[pivot][pivot];
        for column in pivot..9 {
            matrix[pivot][column] /= divisor;
        }
        for row in 0..8 {
            if row == pivot {
                continue;
            }
            let factor = matrix[row][pivot];
            for column in pivot..9 {
                matrix[row][column] -= factor * matrix[pivot][column];
            }
        }
    }
    Homography::new([
        matrix[0][8],
        matrix[1][8],
        matrix[2][8],
        matrix[3][8],
        matrix[4][8],
        matrix[5][8],
        matrix[6][8],
        matrix[7][8],
        1.0,
    ])
}
