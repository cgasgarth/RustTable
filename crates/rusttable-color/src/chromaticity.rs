use crate::{FiniteF32, FiniteF32Error, Matrix3, Primaries, WhitePoint};
use std::fmt;

const RAY_EPSILON: f32 = 1.0e-5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaticityMatrixError {
    NonFinite,
    InvalidWhitePoint,
    Singular,
    InvalidPrimaryIndex,
}

impl fmt::Display for ChromaticityMatrixError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "chromaticity contains a non-finite value",
            Self::InvalidWhitePoint => "white-point chromaticity has zero Y",
            Self::Singular => "chromaticity matrix is singular",
            Self::InvalidPrimaryIndex => "primary index is outside RGB",
        })
    }
}

impl std::error::Error for ChromaticityMatrixError {}

/// Builds the row-major RGB-to-XYZ matrix from xy primaries and an xy white point.
///
/// The construction is the standard primary matrix followed by the white-point
/// scale solve. It is deliberately shared by `primaries` and profile evidence
/// so CPU and reflected GPU plans use the same coefficients.
pub fn rgb_to_xyz_matrix(
    primaries: [(f32, f32); 3],
    white: WhitePoint,
) -> Result<Matrix3, ChromaticityMatrixError> {
    let primary_matrix = chromaticity_matrix(primaries)?;
    let (white_x, white_y) = white.xy();
    if !white_x.is_finite() || !white_y.is_finite() || white_y.abs() <= f32::EPSILON {
        return Err(ChromaticityMatrixError::InvalidWhitePoint);
    }
    let white_xyz = [white_x / white_y, 1.0, (1.0 - white_x - white_y) / white_y];
    let inverse = primary_matrix
        .inverse()
        .map_err(|_| ChromaticityMatrixError::Singular)?;
    let scale = inverse.apply(white_xyz);
    let values = primary_matrix.rows();
    Matrix3::new([
        values[0] * scale[0],
        values[1] * scale[1],
        values[2] * scale[2],
        values[3] * scale[0],
        values[4] * scale[1],
        values[5] * scale[2],
        values[6] * scale[0],
        values[7] * scale[1],
        values[8] * scale[2],
    ])
    .map_err(|_| ChromaticityMatrixError::Singular)
}

/// Rotates a primary around the profile white point and scales its distance to
/// the profile gamut boundary, matching Darktable's custom-primary contract.
pub fn rotate_and_scale_primary(
    profile: Primaries,
    scaling: f32,
    rotation: f32,
    primary_index: usize,
) -> Result<(f32, f32), ChromaticityMatrixError> {
    let scaling = finite(scaling)?;
    let rotation = finite(rotation)?;
    if primary_index >= 3 {
        return Err(ChromaticityMatrixError::InvalidPrimaryIndex);
    }
    let white = profile.white().xy();
    let points = [profile.red(), profile.green(), profile.blue()];
    let primary = points[primary_index];
    let angle = (primary.1.get() - white.1).atan2(primary.0.get() - white.0) + rotation;
    let (cosine, sine) = (angle.cos(), angle.sin());
    let mut distance = f32::INFINITY;
    for index in 0..3 {
        let next = (index + 1) % 3;
        let candidate = ray_segment_intersection(
            white,
            (white.0 + cosine, white.1 + sine),
            (points[index].0.get(), points[index].1.get()),
            (points[next].0.get(), points[next].1.get()),
        );
        distance = distance.min(candidate);
    }
    if !distance.is_finite() {
        return Err(ChromaticityMatrixError::Singular);
    }
    let output = (
        white.0 + scaling * distance * cosine,
        white.1 + scaling * distance * sine,
    );
    if output.0.is_finite() && output.1.is_finite() {
        Ok(output)
    } else {
        Err(ChromaticityMatrixError::NonFinite)
    }
}

fn chromaticity_matrix(primaries: [(f32, f32); 3]) -> Result<Matrix3, ChromaticityMatrixError> {
    let mut values = [0.0; 9];
    for (index, (x, y)) in primaries.into_iter().enumerate() {
        if !x.is_finite() || !y.is_finite() || y.abs() <= f32::EPSILON {
            return Err(ChromaticityMatrixError::NonFinite);
        }
        values[index] = x / y;
        values[3 + index] = 1.0;
        values[6 + index] = (1.0 - x - y) / y;
    }
    Matrix3::new(values).map_err(|_| ChromaticityMatrixError::Singular)
}

fn ray_segment_intersection(
    ray_start: (f32, f32),
    ray_end: (f32, f32),
    segment_start: (f32, f32),
    segment_end: (f32, f32),
) -> f32 {
    let denominator = determinant(
        ray_start.0 - ray_end.0,
        segment_start.0 - segment_end.0,
        ray_start.1 - ray_end.1,
        segment_start.1 - segment_end.1,
    );
    if denominator.abs() <= f32::EPSILON {
        return f32::INFINITY;
    }
    let t = determinant(
        ray_start.0 - segment_start.0,
        segment_start.0 - segment_end.0,
        ray_start.1 - segment_start.1,
        segment_start.1 - segment_end.1,
    ) / denominator;
    let u = determinant(
        ray_start.0 - segment_start.0,
        ray_start.0 - ray_end.0,
        ray_start.1 - segment_start.1,
        ray_start.1 - ray_end.1,
    ) / denominator;
    // A profile's published white/primary values are rounded, so a ray that
    // should meet a vertex can land a few ulps outside the closed segment.
    if t >= -RAY_EPSILON && (-RAY_EPSILON..=1.0 + RAY_EPSILON).contains(&u) {
        t
    } else {
        f32::INFINITY
    }
}

const fn determinant(a: f32, b: f32, c: f32, d: f32) -> f32 {
    a * d - b * c
}

fn finite(value: f32) -> Result<f32, ChromaticityMatrixError> {
    FiniteF32::new(value)
        .map(FiniteF32::get)
        .map_err(|_: FiniteF32Error| ChromaticityMatrixError::NonFinite)
}
