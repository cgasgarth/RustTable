//! Allocation-free colorimetric conversions and bounded gamut queries.

mod gamut;
mod jz;
mod lab;
mod luminance;

pub use gamut::{clip_to_unit_gamut, is_in_unit_gamut};
pub use jz::{
    jzczhz_to_xyz_d65, jzczhz_to_xyz_d65_slice, xyz_d65_to_jzczhz, xyz_d65_to_jzczhz_slice,
};
pub use lab::{lab_to_lch, lab_to_xyz, lab_to_xyz_slice, lch_to_lab, xyz_to_lab, xyz_to_lab_slice};
pub use luminance::{relative_luminance, xyz_luminance};

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMathError {
    NonFinite,
    OutOfRange,
    UnsupportedColorSpace,
    LengthMismatch,
    Singular,
}

pub(super) fn finite3(value: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    value
        .into_iter()
        .all(f32::is_finite)
        .then_some(value)
        .ok_or(ColorMathError::NonFinite)
}

pub(super) fn finite_output(value: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    finite3(value)
}

pub(super) fn map_slice(
    input: &[[f32; 3]],
    output: &mut [[f32; 3]],
    mut convert: impl FnMut([f32; 3]) -> Result<[f32; 3], ColorMathError>,
) -> Result<(), ColorMathError> {
    if input.len() != output.len() {
        return Err(ColorMathError::LengthMismatch);
    }
    for (&source, target) in input.iter().zip(output) {
        *target = convert(source)?;
    }
    Ok(())
}

impl fmt::Display for ColorMathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "color value is non-finite",
            Self::OutOfRange => "color value is outside the conversion domain",
            Self::UnsupportedColorSpace => "color space does not define this primitive",
            Self::LengthMismatch => "color input and output slice lengths differ",
            Self::Singular => "color conversion is singular",
        })
    }
}

impl std::error::Error for ColorMathError {}
