//! ISO/CIE 11664-4:2019 CIELAB and its cylindrical `LCh` representation.
//!
//! The rational constants and conversion order match the CIE equations recorded
//! in [CSS Color 4 sample conversions](https://www.w3.org/TR/css-color-4/#color-conversion-code).

use super::{ColorMathError, finite_output, finite3, map_slice};
use crate::WhitePoint;

const LAB_EPSILON: f32 = 216.0 / 24_389.0;
const LAB_KAPPA: f32 = 24_389.0 / 27.0;

pub fn xyz_to_lab(xyz: [f32; 3], white_point: WhitePoint) -> Result<[f32; 3], ColorMathError> {
    let xyz = finite3(xyz)?;
    let white = white_point.xyz();
    let lab_f = |value: f32, reference: f32| {
        let ratio = value / reference;
        if ratio > LAB_EPSILON {
            ratio.cbrt()
        } else {
            (LAB_KAPPA * ratio + 16.0) / 116.0
        }
    };
    let f = [
        lab_f(xyz[0], white[0]),
        lab_f(xyz[1], white[1]),
        lab_f(xyz[2], white[2]),
    ];
    finite_output([
        116.0 * f[1] - 16.0,
        500.0 * (f[0] - f[1]),
        200.0 * (f[1] - f[2]),
    ])
}

pub fn lab_to_xyz(lab: [f32; 3], white_point: WhitePoint) -> Result<[f32; 3], ColorMathError> {
    let lab = finite3(lab)?;
    let fy = (lab[0] + 16.0) / 116.0;
    let fx = fy + lab[1] / 500.0;
    let fz = fy - lab[2] / 200.0;
    let inverse = |value: f32| {
        let cube = value * value * value;
        if cube > LAB_EPSILON {
            cube
        } else {
            (116.0 * value - 16.0) / LAB_KAPPA
        }
    };
    let white = white_point.xyz();
    finite_output([
        inverse(fx) * white[0],
        inverse(fy) * white[1],
        inverse(fz) * white[2],
    ])
}

/// Converts Lab to `[L, C, h°]`; neutral colors canonically use hue zero.
pub fn lab_to_lch(lab: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    let [lightness, a, b] = finite3(lab)?;
    let chroma = a.hypot(b);
    let hue = if chroma == 0.0 {
        0.0
    } else {
        b.atan2(a).to_degrees().rem_euclid(360.0)
    };
    finite_output([lightness, chroma, hue])
}

/// Converts `[L, C, h°]` to Lab; hue is normalized modulo 360 degrees.
pub fn lch_to_lab(lch: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    let [lightness, chroma, hue] = finite3(lch)?;
    if chroma < 0.0 {
        return Err(ColorMathError::OutOfRange);
    }
    let radians = hue.rem_euclid(360.0).to_radians();
    finite_output([lightness, chroma * radians.cos(), chroma * radians.sin()])
}

pub fn xyz_to_lab_slice(
    input: &[[f32; 3]],
    output: &mut [[f32; 3]],
    white_point: WhitePoint,
) -> Result<(), ColorMathError> {
    map_slice(input, output, |value| xyz_to_lab(value, white_point))
}

pub fn lab_to_xyz_slice(
    input: &[[f32; 3]],
    output: &mut [[f32; 3]],
    white_point: WhitePoint,
) -> Result<(), ColorMathError> {
    map_slice(input, output, |value| lab_to_xyz(value, white_point))
}
