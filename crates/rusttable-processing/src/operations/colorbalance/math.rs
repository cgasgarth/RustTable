//! Scalar f32 math for the Color Balance processing leaf.
//!
//! Direct source lineage: `src/common/colorspaces_inline_conversions.h`,
//! `src/common/math.h`, and `src/common/sse.h`.  The D50 constants and the
//! ProPhoto matrices intentionally remain local to this operation until the
//! shared color-boundary integration is qualified.

#![allow(
    clippy::excessive_precision,
    clippy::unreadable_literal,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    reason = "native matrices, approximation coefficients, bit operations, and source terminology are ported exactly"
)]
#![expect(
    clippy::suboptimal_flops,
    reason = "Native Color Balance approximation and color-boundary equations preserve source evaluation order and IEEE-754 parity."
)]

const D50: [f32; 3] = [0.9642, 1.0, 0.8249];
const D50_INV: [f32; 3] = [1.0 / 0.9642, 1.0, 1.0 / 0.8249];
const LAB_EPSILON: f32 = 216.0 / 24389.0;
const LAB_KAPPA: f32 = 24389.0 / 27.0;
const LAB_EPSILON_CUBEROOT: f32 = 0.20689655172413796;

/// Native D50 ProPhoto RGB → XYZ matrix, in the transposed multiplication
/// order used by `dt_apply_transposed_color_matrix`.
pub const PROPHOTO_RGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.7976749, 0.2880402, 0.0],
    [0.1351917, 0.7118741, 0.0],
    [0.0313534, 0.0000857, 0.8252100],
];

/// Native D50 XYZ → ProPhoto RGB matrix, in transposed multiplication order.
pub const XYZ_TO_PROPHOTO_RGB: [[f32; 3]; 3] = [
    [1.3459433, -0.5445989, 0.0],
    [-0.2556075, 1.5081673, 0.0],
    [-0.0511118, 0.0205351, 1.2118128],
];

const SRGB_TO_XYZ_D50: [[f32; 3]; 3] = [
    [0.4360747, 0.2225045, 0.0139322],
    [0.3850649, 0.7168786, 0.0971045],
    [0.1430804, 0.0606169, 0.7141733],
];

const XYZ_TO_SRGB_D50: [[f32; 3]; 3] = [
    [3.1338561, -0.9787684, 0.0719453],
    [-1.6168667, 1.9161415, -0.2289914],
    [-0.4906146, 0.0334540, 1.4052427],
];

/// Port of `dt_vector_log2` for one f32 lane.
#[must_use]
pub fn approximate_log2(value: f32) -> f32 {
    let bits = value.to_bits();
    let mantissa = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    let exponent = ((bits & 0x7f80_0000) >> 23) as f32 - 127.0;
    let log_mantissa = (((0.0596515482674574969533_f32 * mantissa - 0.465725644288844778798_f32)
        * mantissa
        + 1.48116647521213171641_f32)
        * mantissa
        - 2.52074962577807006663_f32)
        * mantissa
        + 2.8882704548164776201_f32;
    log_mantissa * (mantissa - 1.0) + exponent
}

/// The scalar fallback in `dt_vector_exp2` uses `roundf`, which rounds ties
/// away from zero. SSE2's `_mm_cvtps_epi32` instead uses the default
/// nearest-even MXCSR mode. Keep both conversion rules explicit so the
/// approximation follows the native build selected for the target.
#[cfg(any(
    test,
    all(
        any(target_arch = "x86", target_arch = "x86_64"),
        target_feature = "sse2"
    )
))]
fn round_to_nearest_even(value: f32) -> i32 {
    let lower = value.floor();
    let fraction = value - lower;
    let rounded = if fraction < 0.5 {
        lower
    } else if fraction > 0.5 || lower % 2.0 != 0.0 {
        lower + 1.0
    } else {
        lower
    };
    rounded as i32
}

#[cfg(any(
    test,
    not(all(
        any(target_arch = "x86", target_arch = "x86_64"),
        target_feature = "sse2"
    ))
))]
const fn round_away_from_zero(value: f32) -> i32 {
    value.round() as i32
}

#[cfg(all(
    any(target_arch = "x86", target_arch = "x86_64"),
    target_feature = "sse2"
))]
fn exp2_integer_part(value: f32) -> i32 {
    round_to_nearest_even(value)
}

#[cfg(not(all(
    any(target_arch = "x86", target_arch = "x86_64"),
    target_feature = "sse2"
)))]
const fn exp2_integer_part(value: f32) -> i32 {
    round_away_from_zero(value)
}

/// Port of `dt_vector_exp2` for one f32 lane.
#[must_use]
pub fn approximate_exp2(value: f32) -> f32 {
    let value = value.clamp(-126.99999, 129.0);
    let integer = exp2_integer_part(value - 0.5);
    let fraction = value - integer as f32;
    let exponent_bits = (127_i32 + integer).wrapping_shl(23) as u32;
    let exponent = f32::from_bits(exponent_bits);
    let fractional = (((1.3534167e-2_f32 * fraction + 5.2011464e-2_f32) * fraction
        + 2.4144275e-1_f32)
        * fraction
        + 6.9300383e-1_f32)
        * fraction
        + 1.0000026_f32;
    exponent * fractional
}

/// Port of `dt_vector_powf`: `2^(log2(x) * p)` with Darktable's native
/// degree-five log and degree-four exp approximations.
#[must_use]
pub fn approximate_powf(value: f32, power: f32) -> f32 {
    approximate_exp2(approximate_log2(value) * power)
}

/// Native scalar Lab D50 → XYZ D50 conversion.
#[must_use]
pub fn lab_to_xyz(lab: [f32; 4]) -> [f32; 4] {
    let f = [lab[1], lab[0], lab[2], lab[3]];
    let scaled = [
        (f[0] + 0.0) * (1.0 / 500.0),
        (f[1] + 16.0) * (1.0 / 116.0),
        (f[2] + 0.0) * (-1.0 / 200.0),
        f[3] * 0.0,
    ];
    let inverse = [
        lab_f_inverse(scaled[0] + scaled[1]),
        lab_f_inverse(scaled[1]),
        lab_f_inverse(scaled[2] + scaled[1]),
        lab_f_inverse(scaled[3]),
    ];
    [
        D50[0] * inverse[0],
        D50[1] * inverse[1],
        D50[2] * inverse[2],
        0.0,
    ]
}

/// Native scalar XYZ D50 → Lab conversion. The spare lane is zeroed by the
/// conversion, matching `dt_XYZ_to_Lab` and the native Color Balance CPU path.
#[must_use]
pub fn xyz_to_lab(xyz: [f32; 4]) -> [f32; 4] {
    let mut f = [0.0_f32; 4];
    for channel in 0..3 {
        let value = xyz[channel] * D50_INV[channel];
        f[channel] = if value > LAB_EPSILON {
            value.cbrt()
        } else {
            (LAB_KAPPA * value + 16.0) / 116.0
        };
    }
    [
        116.0 * f[1] - 16.0,
        500.0 * (f[0] - f[1]),
        -200.0 * (f[2] - f[1]),
        0.0,
    ]
}

/// Native transposed 3×3 multiplication order.
#[must_use]
pub fn apply_transposed(matrix: [[f32; 3]; 3], input: [f32; 4]) -> [f32; 4] {
    [
        matrix[0][0] * input[0] + matrix[1][0] * input[1] + matrix[2][0] * input[2],
        matrix[0][1] * input[0] + matrix[1][1] * input[1] + matrix[2][1] * input[2],
        matrix[0][2] * input[0] + matrix[1][2] * input[1] + matrix[2][2] * input[2],
        0.0,
    ]
}

#[must_use]
pub fn xyz_to_prophoto(xyz: [f32; 4]) -> [f32; 4] {
    apply_transposed(XYZ_TO_PROPHOTO_RGB, xyz)
}

#[must_use]
pub fn prophoto_to_xyz(rgb: [f32; 4]) -> [f32; 4] {
    apply_transposed(PROPHOTO_RGB_TO_XYZ, rgb)
}

#[must_use]
pub fn xyz_to_srgb(xyz: [f32; 4]) -> [f32; 4] {
    let linear = apply_transposed(XYZ_TO_SRGB_D50, xyz);
    let mut curved = [0.0; 4];
    for channel in 0..3 {
        let toe = 12.92 * linear[channel];
        let powered = approximate_powf(linear[channel], 1.0 / 2.4);
        curved[channel] = if linear[channel] <= 0.0031308 {
            toe
        } else {
            1.055 * powered - 0.055
        };
    }
    curved
}

#[must_use]
pub fn srgb_to_xyz(srgb: [f32; 4]) -> [f32; 4] {
    let mut linear = [0.0; 4];
    for channel in 0..3 {
        let toe = srgb[channel] / 12.92;
        let scaled = (srgb[channel] + 0.055) / 1.055;
        let powered = approximate_powf(scaled, 2.4);
        linear[channel] = if srgb[channel] <= 0.04045 {
            toe
        } else {
            powered
        };
    }
    apply_transposed(SRGB_TO_XYZ_D50, linear)
}

#[must_use]
pub fn lab_to_prophoto(lab: [f32; 4]) -> [f32; 4] {
    xyz_to_prophoto(lab_to_xyz(lab))
}

#[must_use]
pub fn prophoto_to_lab(rgb: [f32; 4]) -> [f32; 4] {
    xyz_to_lab(prophoto_to_xyz(rgb))
}

fn lab_f_inverse(value: f32) -> f32 {
    if value > LAB_EPSILON_CUBEROOT {
        value * value * value
    } else {
        (116.0 * value - 16.0) / LAB_KAPPA
    }
}

#[must_use]
pub fn max_zero(value: f32) -> f32 {
    if value > 0.0 { value } else { 0.0 }
}

#[must_use]
pub fn prophoto_luma(rgb: [f32; 4]) -> f32 {
    PROPHOTO_RGB_TO_XYZ[0][1] * rgb[0]
        + PROPHOTO_RGB_TO_XYZ[1][1] * rgb[1]
        + PROPHOTO_RGB_TO_XYZ[2][1] * rgb[2]
}

#[cfg(test)]
mod tests {
    use super::{round_away_from_zero, round_to_nearest_even};

    #[test]
    fn rounding_helpers_preserve_their_native_tie_rules() {
        for (value, nearest_even, away_from_zero) in [
            (-2.5, -2, -3),
            (-1.5, -2, -2),
            (-0.5, 0, -1),
            (0.5, 0, 1),
            (1.5, 2, 2),
            (2.5, 2, 3),
        ] {
            assert_eq!(round_to_nearest_even(value), nearest_even);
            assert_eq!(round_away_from_zero(value), away_from_zero);
        }
    }
}
