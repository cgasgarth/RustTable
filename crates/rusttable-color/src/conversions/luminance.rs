use super::{ColorMathError, finite3};
use crate::BuiltinSpace;

/// Returns relative CIE Y from linear RGB in a matrix-based built-in space.
pub fn relative_luminance(
    linear_rgb: [f32; 3],
    space: BuiltinSpace,
) -> Result<f32, ColorMathError> {
    let rgb = finite3(linear_rgb)?;
    let matrix = space
        .to_xyz_matrix()
        .ok_or(ColorMathError::UnsupportedColorSpace)?;
    let rows = matrix.rows();
    let luminance = rows[3] * rgb[0] + rows[4] * rgb[1] + rows[5] * rgb[2];
    luminance
        .is_finite()
        .then_some(luminance)
        .ok_or(ColorMathError::NonFinite)
}

/// Returns the Y component of finite CIE XYZ without clipping HDR or negative Y.
pub fn xyz_luminance(xyz: [f32; 3]) -> Result<f32, ColorMathError> {
    finite3(xyz).map(|value| value[1])
}
