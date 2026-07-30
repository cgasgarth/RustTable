//! Safdar et al. (2017) `JzCzhz` conversion for absolute D65 XYZ.
//!
//! Constants and expression order follow “Perceptually uniform color space for
//! image signals including high dynamic range and wide gamut”, Optics Express
//! 25(13), 15131–15151, <https://doi.org/10.1364/OE.25.015131>. Inputs are
//! nonnegative absolute XYZ in cd/m²; transformed LMS must be in `[0, 10000]`.
//! The 2017 `JzAzBz` constants are used.

use super::{ColorMathError, finite_output, finite3, map_slice};

const B: f32 = 1.15;
const G: f32 = 0.66;
const D: f32 = -0.56;
const D0: f32 = 1.629_55e-11;
const M1: f32 = 2_610.0 / 16_384.0;
const M2: f32 = 1.7 * 2_523.0 / 32.0;
const C1: f32 = 3_424.0 / 4_096.0;
const C2: f32 = 2_413.0 / 128.0;
const C3: f32 = 2_392.0 / 128.0;
const PEAK_LUMINANCE: f32 = 10_000.0;
const NEUTRAL_CHROMA_EPSILON: f32 = 1.0e-9;

const XYZ_TO_LMS: [f32; 9] = [
    0.414_789_7,
    0.579_999,
    0.014_648,
    -0.201_51,
    1.120_649,
    0.053_100_8,
    -0.016_600_8,
    0.264_8,
    0.668_479_9,
];
const LMS_TO_XYZ: [f32; 9] = [
    1.924_226_4,
    -1.004_792_3,
    0.037_651_405,
    0.350_316_76,
    0.726_481_2,
    -0.065_384_425,
    -0.090_982_81,
    -0.312_728_3,
    1.522_766_6,
];
const LMS_P_TO_IZAZBZ: [f32; 9] = [
    0.5, 0.5, 0.0, 3.524, -4.066_708, 0.542_708, 0.199_076, 1.096_799, -1.295_875,
];
const IZAZBZ_TO_LMS_P: [f32; 9] = [
    1.0,
    0.138_605_04,
    0.058_047_317,
    1.0,
    -0.138_605_04,
    -0.058_047_317,
    1.0,
    -0.096_019_24,
    -0.811_891_9,
];

pub fn xyz_d65_to_jzczhz(xyz: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    let [x, y, z] = finite3(xyz)?;
    if [x, y, z].into_iter().any(|value| value < 0.0) {
        return Err(ColorMathError::OutOfRange);
    }
    let adjusted = [B * x - (B - 1.0) * z, G * y - (G - 1.0) * x, z];
    let lms = multiply(XYZ_TO_LMS, adjusted);
    if lms
        .into_iter()
        .any(|value| !(0.0..=PEAK_LUMINANCE).contains(&value))
    {
        return Err(ColorMathError::OutOfRange);
    }
    let encoded = lms.map(pq_encode_absolute);
    let [iz, az, bz] = multiply(LMS_P_TO_IZAZBZ, encoded);
    let jz = ((1.0 + D) * iz) / (1.0 + D * iz) - D0;
    let raw_chroma = az.hypot(bz);
    let (chroma, hue) = if raw_chroma <= NEUTRAL_CHROMA_EPSILON {
        (0.0, 0.0)
    } else {
        (raw_chroma, bz.atan2(az).to_degrees().rem_euclid(360.0))
    };
    finite_output([jz, chroma, hue])
}

pub fn jzczhz_to_xyz_d65(jzczhz: [f32; 3]) -> Result<[f32; 3], ColorMathError> {
    let [jz, chroma, hue] = finite3(jzczhz)?;
    if !(0.0..=1.0).contains(&jz) || chroma < 0.0 {
        return Err(ColorMathError::OutOfRange);
    }
    let radians = hue.rem_euclid(360.0).to_radians();
    let az = chroma * radians.cos();
    let bz = chroma * radians.sin();
    let adjusted_jz = jz + D0;
    let denominator = 1.0 + D - D * adjusted_jz;
    if denominator == 0.0 {
        return Err(ColorMathError::Singular);
    }
    let iz = adjusted_jz / denominator;
    let encoded = multiply(IZAZBZ_TO_LMS_P, [iz, az, bz]);
    if encoded
        .into_iter()
        .any(|value| !(0.0..=1.0).contains(&value))
    {
        return Err(ColorMathError::OutOfRange);
    }
    let lms = encoded.map(pq_decode_absolute);
    let [x_adjusted, y_adjusted, z] = multiply(LMS_TO_XYZ, lms);
    let x = (x_adjusted + (B - 1.0) * z) / B;
    let y = (y_adjusted + (G - 1.0) * x) / G;
    let xyz = finite_output([x, y, z])?;
    if xyz.into_iter().any(|value| value < 0.0) {
        return Err(ColorMathError::OutOfRange);
    }
    Ok(xyz)
}

pub fn xyz_d65_to_jzczhz_slice(
    input: &[[f32; 3]],
    output: &mut [[f32; 3]],
) -> Result<(), ColorMathError> {
    map_slice(input, output, xyz_d65_to_jzczhz)
}

pub fn jzczhz_to_xyz_d65_slice(
    input: &[[f32; 3]],
    output: &mut [[f32; 3]],
) -> Result<(), ColorMathError> {
    map_slice(input, output, jzczhz_to_xyz_d65)
}

fn pq_encode_absolute(value: f32) -> f32 {
    let powered = (value / PEAK_LUMINANCE).powf(M1);
    ((C1 + C2 * powered) / (1.0 + C3 * powered)).powf(M2)
}

fn pq_decode_absolute(value: f32) -> f32 {
    let powered = value.powf(1.0 / M2);
    let numerator = (powered - C1).max(0.0);
    PEAK_LUMINANCE * (numerator / (C2 - C3 * powered)).powf(1.0 / M1)
}

fn multiply(matrix: [f32; 9], vector: [f32; 3]) -> [f32; 3] {
    [
        matrix[0] * vector[0] + matrix[1] * vector[1] + matrix[2] * vector[2],
        matrix[3] * vector[0] + matrix[4] * vector[1] + matrix[5] * vector[2],
        matrix[6] * vector[0] + matrix[7] * vector[1] + matrix[8] * vector[2],
    ]
}
