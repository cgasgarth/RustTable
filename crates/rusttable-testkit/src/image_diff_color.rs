#![allow(clippy::excessive_precision, clippy::unreadable_literal)] // Published color-space matrices are retained at their reference precision.

use super::{CanonicalProfile, DiffError};

pub fn profile_convert(
    source: CanonicalProfile,
    target: CanonicalProfile,
    rgb: [f32; 3],
) -> [f32; 3] {
    if source == target {
        return rgb;
    }
    let xyz = multiply(profile_matrix(source), rgb);
    let adapted = adapt_white_point(xyz, white_point(source), white_point(target));
    multiply(inverse_profile_matrix(target), adapted)
}

#[must_use]
pub fn ciede2000(first: [f32; 3], second: [f32; 3]) -> f32 {
    let c1 = (first[1] * first[1] + first[2] * first[2]).sqrt();
    let c2 = (second[1] * second[1] + second[2] * second[2]).sqrt();
    let c_bar = f32::midpoint(c1, c2);
    let twenty_five_7 = 25.0_f32.powi(7);
    let c_bar_7 = c_bar.powi(7);
    let g = 0.5 * (1.0 - (c_bar_7 / (c_bar_7 + twenty_five_7)).sqrt());
    let a1 = (1.0 + g) * first[1];
    let a2 = (1.0 + g) * second[1];
    let c1 = (a1 * a1 + first[2] * first[2]).sqrt();
    let c2 = (a2 * a2 + second[2] * second[2]).sqrt();
    let h1 = hue_angle(a1, first[2]);
    let h2 = hue_angle(a2, second[2]);
    let delta_l = second[0] - first[0];
    let delta_c = c2 - c1;
    let delta_h = if c1 * c2 == 0.0 {
        0.0
    } else if (h2 - h1).abs() <= 180.0 {
        h2 - h1
    } else if h2 <= h1 {
        h2 - h1 + 360.0
    } else {
        h2 - h1 - 360.0
    };
    let delta_big_h = 2.0 * (c1 * c2).sqrt() * (delta_h.to_radians() / 2.0).sin();
    let l_bar = f32::midpoint(first[0], second[0]);
    let c_bar = f32::midpoint(c1, c2);
    let h_bar = if c1 * c2 == 0.0 {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        f32::midpoint(h1, h2)
    } else if h1 + h2 < 360.0 {
        (h1 + h2 + 360.0) / 2.0
    } else {
        (h1 + h2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * ((h_bar - 30.0).to_radians()).cos()
        + 0.24 * ((2.0 * h_bar).to_radians()).cos()
        + 0.32 * ((3.0 * h_bar + 6.0).to_radians()).cos()
        - 0.20 * ((4.0 * h_bar - 63.0).to_radians()).cos();
    let delta_theta = 30.0 * (-((h_bar - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (c_bar.powi(7) / (c_bar.powi(7) + twenty_five_7)).sqrt();
    let sl = 1.0 + 0.015 * (l_bar - 50.0).powi(2) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * c_bar;
    let sh = 1.0 + 0.015 * c_bar * t;
    let rt = -(2.0 * delta_theta.to_radians()).sin() * rc;
    ((delta_l / sl).powi(2)
        + (delta_c / sc).powi(2)
        + (delta_big_h / sh).powi(2)
        + rt * (delta_c / sc) * (delta_big_h / sh))
        .max(0.0)
        .sqrt()
}

pub fn delta_e_2000(left: [f32; 4], right: [f32; 4], profile: CanonicalProfile) -> f32 {
    if left[..3].iter().any(|value| !value.is_finite())
        || right[..3].iter().any(|value| !value.is_finite())
    {
        return f32::MAX;
    }
    ciede2000(
        rgb_to_lab([left[0], left[1], left[2]], profile),
        rgb_to_lab([right[0], right[1], right[2]], profile),
    )
}

fn hue_angle(a: f32, b: f32) -> f32 {
    let angle = b.atan2(a).to_degrees();
    if angle < 0.0 { angle + 360.0 } else { angle }
}

fn rgb_to_lab(rgb: [f32; 3], profile: CanonicalProfile) -> [f32; 3] {
    let xyz = adapt_white_point(
        multiply(profile_matrix(profile), rgb),
        white_point(profile),
        [0.96422, 1.0, 0.82521],
    );
    let f = |value: f32| {
        const EPSILON: f32 = 216.0 / 24_389.0;
        const KAPPA: f32 = 24_389.0 / 27.0;
        if value > EPSILON {
            value.powf(1.0 / 3.0)
        } else {
            (KAPPA * value + 16.0) / 116.0
        }
    };
    let (fx, fy, fz) = (f(xyz[0] / 0.96422), f(xyz[1]), f(xyz[2] / 0.82521));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn profile_matrix(profile: CanonicalProfile) -> [[f32; 3]; 3] {
    match profile {
        CanonicalProfile::Srgb => [
            [0.4124564, 0.3575761, 0.1804375],
            [0.2126729, 0.7151522, 0.0721750],
            [0.0193339, 0.1191920, 0.9503041],
        ],
        CanonicalProfile::DisplayP3 => [
            [0.48657095, 0.26566769, 0.19821729],
            [0.22897456, 0.69173852, 0.07928691],
            [0.00000000, 0.04511338, 1.04394437],
        ],
        CanonicalProfile::Rec2020 => [
            [0.63695805, 0.14461690, 0.16888097],
            [0.26270021, 0.67799807, 0.05930172],
            [0.00000000, 0.02807269, 1.06098506],
        ],
    }
}

fn inverse_profile_matrix(profile: CanonicalProfile) -> [[f32; 3]; 3] {
    match profile {
        CanonicalProfile::Srgb => [
            [3.2404542, -1.5371385, -0.4985314],
            [-0.9692660, 1.8760108, 0.0415560],
            [0.0556434, -0.2040259, 1.0572252],
        ],
        CanonicalProfile::DisplayP3 => [
            [2.4934969, -0.9313836, -0.4027108],
            [-0.8294890, 1.7626640, 0.0236247],
            [0.0358458, -0.0761724, 0.9568845],
        ],
        CanonicalProfile::Rec2020 => [
            [1.7166512, -0.3556708, -0.2533663],
            [-0.6666844, 1.6164812, 0.0157685],
            [0.0176399, -0.0427706, 0.9421031],
        ],
    }
}

fn white_point(profile: CanonicalProfile) -> [f32; 3] {
    match profile {
        CanonicalProfile::Srgb | CanonicalProfile::DisplayP3 | CanonicalProfile::Rec2020 => {
            [0.95047, 1.0, 1.08883]
        }
    }
}

fn multiply(matrix: [[f32; 3]; 3], vector: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * vector[0] + matrix[0][1] * vector[1] + matrix[0][2] * vector[2],
        matrix[1][0] * vector[0] + matrix[1][1] * vector[1] + matrix[1][2] * vector[2],
        matrix[2][0] * vector[0] + matrix[2][1] * vector[1] + matrix[2][2] * vector[2],
    ]
}

#[allow(clippy::float_cmp)]
fn adapt_white_point(xyz: [f32; 3], source: [f32; 3], target: [f32; 3]) -> [f32; 3] {
    if source == target {
        return xyz;
    }
    let bradford = [
        [0.8951, 0.2664, -0.1614],
        [-0.7502, 1.7135, 0.0367],
        [0.0389, -0.0685, 1.0296],
    ];
    let inverse = [
        [0.9869929, -0.1470543, 0.1599627],
        [0.4323053, 0.5183603, 0.0492912],
        [-0.0085287, 0.0400428, 0.9684867],
    ];
    let source_cone = multiply(bradford, source);
    let target_cone = multiply(bradford, target);
    let scale = [
        [target_cone[0] / source_cone[0], 0.0, 0.0],
        [0.0, target_cone[1] / source_cone[1], 0.0],
        [0.0, 0.0, target_cone[2] / source_cone[2]],
    ];
    multiply(inverse, multiply(scale, multiply(bradford, xyz)))
}

pub fn converter_error() -> DiffError {
    DiffError::InvalidImage("profile converter rejected the input".to_owned())
}
