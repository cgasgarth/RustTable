//! Darktable UCS/JCH, CAT-adjacent hue, and RYB lookup math.
//!
//! This is a local CPU port of the formulas used by
//! `src/common/colorspaces_inline_conversions.h`, `src/common/color_ryb.h`,
//! and the UCS helpers in `src/iop/colorharmonizer.c`.  It intentionally does
//! not call the existing Rust color-space abstractions: those do not implement
//! Darktable UCS/JCH and substituting Lab, JzAzBz, OKLab, HSV, or Bradford
//! would change the operation.

#![allow(
    clippy::approx_constant,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::excessive_precision,
    clippy::float_cmp,
    clippy::if_not_else,
    clippy::manual_midpoint,
    clippy::unreadable_literal
)]

use std::sync::OnceLock;

use super::codec::{
    COLORHARMONIZER_MAX_NODES, COLORHARMONIZER_RYB_INVERSE_STEPS, ColorHarmonizerRule,
};

const PI_F: f32 = 3.14159265358979323846_f32;
const TWO_PI_F: f32 = 6.28318530717958647693_f32;
const FLT_MIN_F: f32 = f32::MIN_POSITIVE;

const DT_UCS_L_STAR_RANGE: f32 = 2.098883786377_f32;
const DT_UCS_L_STAR_UPPER_LIMIT: f32 = 2.09885_f32;

/// The native 720-entry forward and inverse tables.
#[derive(Debug, Clone, PartialEq)]
pub struct HarmonyTables {
    ucs_to_ryb: [f32; COLORHARMONIZER_RYB_INVERSE_STEPS],
    ryb_to_ucs: [f32; COLORHARMONIZER_RYB_INVERSE_STEPS],
}

impl HarmonyTables {
    /// Builds the source table pair, including strict-`<` inverse tie choice.
    #[must_use]
    pub fn build() -> Self {
        let mut ucs_to_ryb = [0.0_f32; COLORHARMONIZER_RYB_INVERSE_STEPS];
        for (index, value) in ucs_to_ryb.iter_mut().enumerate() {
            *value = ucs_hue_to_ryb_hue(index as f32 / COLORHARMONIZER_RYB_INVERSE_STEPS as f32);
        }

        let mut ryb_to_ucs = [0.0_f32; COLORHARMONIZER_RYB_INVERSE_STEPS];
        for (target_index, value) in ryb_to_ucs.iter_mut().enumerate() {
            let target = target_index as f32 / COLORHARMONIZER_RYB_INVERSE_STEPS as f32;
            let mut best_dist = 1.0_f32;
            let mut best_ucs = 0.0_f32;
            for (ucs_index, source) in ucs_to_ryb.iter().enumerate() {
                let mut distance = (*source - target).abs();
                if distance > 0.5_f32 {
                    distance = 1.0_f32 - distance;
                }
                if distance < best_dist {
                    best_dist = distance;
                    best_ucs = ucs_index as f32 / COLORHARMONIZER_RYB_INVERSE_STEPS as f32;
                }
            }
            *value = best_ucs;
        }
        Self {
            ucs_to_ryb,
            ryb_to_ucs,
        }
    }

    #[must_use]
    pub fn ucs_to_ryb(&self, hue: f32) -> f32 {
        lookup_lut(&self.ucs_to_ryb, hue)
    }

    #[must_use]
    pub fn ryb_to_ucs(&self, hue: f32) -> f32 {
        lookup_lut(&self.ryb_to_ucs, hue)
    }

    #[must_use]
    pub fn forward(&self) -> &[f32; COLORHARMONIZER_RYB_INVERSE_STEPS] {
        &self.ucs_to_ryb
    }

    #[must_use]
    pub fn inverse(&self) -> &[f32; COLORHARMONIZER_RYB_INVERSE_STEPS] {
        &self.ryb_to_ucs
    }
}

static HARMONY_TABLES: OnceLock<HarmonyTables> = OnceLock::new();

#[must_use]
pub fn harmony_tables() -> &'static HarmonyTables {
    HARMONY_TABLES.get_or_init(HarmonyTables::build)
}

fn lookup_lut(table: &[f32; COLORHARMONIZER_RYB_INVERSE_STEPS], hue: f32) -> f32 {
    // The parameter boundary rejects negative hue values before this native
    // modulo operation.  Inclusive 1.0 intentionally maps to table element 0,
    // as `(int)720 % 720` does in the C source.
    let position = hue * COLORHARMONIZER_RYB_INVERSE_STEPS as f32;
    let index0 = (position as usize) % COLORHARMONIZER_RYB_INVERSE_STEPS;
    let index1 = (index0 + 1) % COLORHARMONIZER_RYB_INVERSE_STEPS;
    hue_lerp(
        table[index0],
        table[index1],
        position - position as usize as f32,
    )
}

/// Circular interpolation used by both native fast LUT lookups.
#[must_use]
pub fn hue_lerp(mut a: f32, mut b: f32, t: f32) -> f32 {
    if b - a > 0.5_f32 {
        b -= 1.0_f32;
    } else if a - b > 0.5_f32 {
        a -= 1.0_f32;
    }
    let mut result = a + t * (b - a);
    if result < 0.0_f32 {
        result += 1.0_f32;
    }
    result
}

/// Returns source harmony nodes in UCS hue space and their exact source count.
#[must_use]
pub fn harmony_nodes(
    rule: ColorHarmonizerRule,
    anchor_hue: f32,
    custom_hue: &[f32; COLORHARMONIZER_MAX_NODES],
    custom_count: i32,
    tables: &HarmonyTables,
) -> ([f32; COLORHARMONIZER_MAX_NODES], usize) {
    if rule == ColorHarmonizerRule::Custom {
        let count = custom_count.clamp(1, COLORHARMONIZER_MAX_NODES as i32) as usize;
        let mut nodes = [0.0_f32; COLORHARMONIZER_MAX_NODES];
        nodes[..count].copy_from_slice(&custom_hue[..count]);
        return (nodes, count);
    }

    let rotation = (tables.ucs_to_ryb(anchor_hue) * 360.0_f32).round() as i32 % 360;
    let offsets = rule.geometry();
    let mut nodes = [0.0_f32; COLORHARMONIZER_MAX_NODES];
    for (index, offset) in offsets.iter().enumerate() {
        let mut angle = *offset + rotation as f32 / 360.0_f32;
        angle -= angle.floor();
        nodes[index] = tables.ryb_to_ucs(angle);
    }
    (nodes, offsets.len())
}

/// Native xyY-to-UCS-JCH conversion. Input xyY is D65-adapted.
#[must_use]
pub fn xyy_to_jch(xyy: [f32; 3], lightness_white: f32) -> [f32; 3] {
    let uv_star_prime = xyy_to_ucs_uv(xyy);
    let lightness = y_to_l_star(xyy[2]);
    let m2 = uv_star_prime[0] * uv_star_prime[0] + uv_star_prime[1] * uv_star_prime[1];
    [
        lightness / lightness_white,
        15.932993652962535_f32
            * lightness.powf(0.6523997524738018_f32)
            * m2.powf(0.6007557017508491_f32)
            / lightness_white,
        uv_star_prime[1].atan2(uv_star_prime[0]),
    ]
}

/// Native UCS-JCH-to-xyY conversion, including signed `FLT_MIN` protection.
#[must_use]
pub fn jch_to_xyy(jch: [f32; 3], lightness_white: f32) -> [f32; 3] {
    let lightness = clampf(jch[0] * lightness_white, 0.0_f32, DT_UCS_L_STAR_UPPER_LIMIT);
    let m = if lightness != 0.0_f32 {
        (jch[1] * lightness_white
            / (15.932993652962535_f32 * lightness.powf(0.6523997524738018_f32)))
        .powf(0.8322850678616855_f32)
    } else {
        0.0_f32
    };
    let u_star_prime = m * jch[2].cos();
    let v_star_prime = m * jch[2].sin();

    let uv_star = [
        -5.037522385190711_f32 * u_star_prime - 2.504856328185843_f32 * v_star_prime,
        4.760029407436461_f32 * u_star_prime + 2.874012963239247_f32 * v_star_prime,
    ];
    let factors = [1.39656225667_f32, 1.4513954287_f32];
    let half_values = [1.49217352929_f32, 1.52488637914_f32];
    let mut uv = [0.0_f32; 2];
    for channel in 0..2 {
        uv[channel] =
            -half_values[channel] * uv_star[channel] / (uv_star[channel].abs() - factors[channel]);
    }

    let u_factors = [
        0.167171472114775_f32,
        -0.150959086409163_f32,
        0.940254742367256_f32,
    ];
    let v_factors = [0.141299802443708_f32, -0.155185060382272_f32, 1.0_f32];
    let offsets = [
        -0.00801531300850582_f32,
        -0.00843312433578007_f32,
        -0.0256325967652889_f32,
    ];
    let xyd = [
        u_factors[0] * uv[0] + v_factors[0] * uv[1] + offsets[0],
        u_factors[1] * uv[0] + v_factors[1] * uv[1] + offsets[1],
        u_factors[2] * uv[0] + v_factors[2] * uv[1] + offsets[2],
    ];
    let divisor = signed_min_protected(xyd[2]);
    [xyd[0] / divisor, xyd[1] / divisor, l_star_to_y(lightness)]
}

#[must_use]
pub fn xyy_to_xyz(xyy: [f32; 3]) -> [f32; 3] {
    if xyy[1] == 0.0_f32 {
        [0.0_f32; 3]
    } else {
        [
            xyy[2] * xyy[0] / xyy[1],
            xyy[2],
            xyy[2] * (1.0_f32 - xyy[0] - xyy[1]) / xyy[1],
        ]
    }
}

#[must_use]
pub fn xyz_d65_to_xyy(xyz: [f32; 3]) -> [f32; 3] {
    let xyz = [
        if xyz[0] > 0.0_f32 { xyz[0] } else { 0.0_f32 },
        if xyz[1] > 0.0_f32 { xyz[1] } else { 0.0_f32 },
        if xyz[2] > 0.0_f32 { xyz[2] } else { 0.0_f32 },
    ];
    let sum = xyz[0] + xyz[1] + xyz[2];
    if sum > 0.0_f32 {
        [xyz[0] / sum, xyz[1] / sum, xyz[1]]
    } else {
        [0.31271_f32, 0.32902_f32, xyz[1]]
    }
}

#[must_use]
pub fn y_to_l_star(y: f32) -> f32 {
    let y_hat = y.powf(0.631651345306265_f32);
    DT_UCS_L_STAR_RANGE * y_hat / (y_hat + 1.12426773749357_f32)
}

#[must_use]
pub fn l_star_to_y(lightness: f32) -> f32 {
    (1.12426773749357_f32 * lightness / (DT_UCS_L_STAR_RANGE - lightness))
        .powf(1.5831518565279648_f32)
}

fn xyy_to_ucs_uv(xyy: [f32; 3]) -> [f32; 2] {
    let x_factors = [
        -0.783941002840055_f32,
        0.745273540913283_f32,
        0.318707282433486_f32,
    ];
    let y_factors = [
        0.277512987809202_f32,
        -0.205375866083878_f32,
        2.16743692732158_f32,
    ];
    let offsets = [
        0.153836578598858_f32,
        -0.165478376301988_f32,
        0.291320554395942_f32,
    ];
    let uvd = [
        x_factors[0] * xyy[0] + y_factors[0] * xyy[1] + offsets[0],
        x_factors[1] * xyy[0] + y_factors[1] * xyy[1] + offsets[1],
        x_factors[2] * xyy[0] + y_factors[2] * xyy[1] + offsets[2],
    ];
    let divisor = signed_min_protected(uvd[2]);
    let uvd = [uvd[0] / divisor, uvd[1] / divisor];
    let factors = [1.39656225667_f32, 1.4513954287_f32];
    let half_values = [1.49217352929_f32, 1.52488637914_f32];
    let uv_star = [
        factors[0] * uvd[0] / (uvd[0].abs() + half_values[0]),
        factors[1] * uvd[1] / (uvd[1].abs() + half_values[1]),
    ];
    [
        -1.124983854323892_f32 * uv_star[0] - 0.980483721769325_f32 * uv_star[1],
        1.86323315098672_f32 * uv_star[0] + 1.971853092390862_f32 * uv_star[1],
    ]
}

fn signed_min_protected(value: f32) -> f32 {
    if value >= 0.0_f32 {
        if value > FLT_MIN_F { value } else { FLT_MIN_F }
    } else if value < -FLT_MIN_F {
        value
    } else {
        -FLT_MIN_F
    }
}

#[must_use]
pub fn jch_to_srgb(jch: [f32; 3], lightness_white: f32) -> [f32; 3] {
    let xyz = xyy_to_xyz(jch_to_xyy(jch, lightness_white));
    let linear = [
        3.2404542_f32 * xyz[0] - 0.9692660_f32 * xyz[1] + 0.0556434_f32 * xyz[2],
        -1.5371385_f32 * xyz[0] + 1.8760108_f32 * xyz[1] - 0.2040259_f32 * xyz[2],
        -0.4985314_f32 * xyz[0] + 0.0415560_f32 * xyz[1] + 1.0572252_f32 * xyz[2],
    ];
    [
        if linear[0] <= 0.0031308_f32 {
            12.92_f32 * linear[0]
        } else {
            1.055_f32 * linear[0].powf(1.0_f32 / 2.4_f32) - 0.055_f32
        },
        if linear[1] <= 0.0031308_f32 {
            12.92_f32 * linear[1]
        } else {
            1.055_f32 * linear[1].powf(1.0_f32 / 2.4_f32) - 0.055_f32
        },
        if linear[2] <= 0.0031308_f32 {
            12.92_f32 * linear[2]
        } else {
            1.055_f32 * linear[2].powf(1.0_f32 / 2.4_f32) - 0.055_f32
        },
    ]
}

fn find_max_chroma(hue: f32, lightness_white: f32) -> f32 {
    let heading = hue * TWO_PI_F - PI_F;
    let mut chroma_low = 0.0_f32;
    let mut chroma_high = 2.0_f32;
    for _ in 0..16 {
        let chroma_mid = (chroma_low + chroma_high) * 0.5_f32;
        let srgb = jch_to_srgb([0.65_f32, chroma_mid, heading], lightness_white);
        if srgb
            .iter()
            .all(|channel| *channel >= 0.0_f32 && *channel <= 1.0_f32)
        {
            chroma_low = chroma_mid;
        } else {
            chroma_high = chroma_mid;
        }
    }
    chroma_low
}

fn hue_to_srgb(hue: f32, lightness_white: f32) -> [f32; 3] {
    let heading = hue * TWO_PI_F - PI_F;
    let chroma = find_max_chroma(hue, lightness_white) * 0.85_f32;
    let srgb = jch_to_srgb([0.65_f32, chroma, heading], lightness_white);
    [
        clampf(srgb[0], 0.0_f32, 1.0_f32),
        clampf(srgb[1], 0.0_f32, 1.0_f32),
        clampf(srgb[2], 0.0_f32, 1.0_f32),
    ]
}

fn rgb_to_hcv(rgb: [f32; 3]) -> [f32; 3] {
    let min = min3(rgb);
    let max = max3(rgb);
    let delta = max - min;
    let (hue, chroma) = if max.abs() > 1.0e-6_f32 && delta.abs() > 1.0e-6_f32 {
        let mut hue = if rgb[0] == max {
            (rgb[1] - rgb[2]) / delta
        } else if rgb[1] == max {
            2.0_f32 + (rgb[2] - rgb[0]) / delta
        } else {
            4.0_f32 + (rgb[0] - rgb[1]) / delta
        };
        hue /= 6.0_f32;
        (hue - hue.floor(), delta)
    } else {
        (0.0_f32, 0.0_f32)
    };
    [hue, chroma, max]
}

fn min3(values: [f32; 3]) -> f32 {
    values[0].min(values[1]).min(values[2])
}

fn max3(values: [f32; 3]) -> f32 {
    values[0].max(values[1]).max(values[2])
}

#[must_use]
pub fn rgb_hue_to_ryb_hue(hue: f32) -> f32 {
    let x_vtx = [
        0.0_f32,
        1.0_f32 / 6.0_f32,
        2.0_f32 / 6.0_f32,
        3.0_f32 / 6.0_f32,
        4.0_f32 / 6.0_f32,
        5.0_f32 / 6.0_f32,
        1.0_f32,
    ];
    let y_vtx = [
        0.0_f32,
        1.0_f32 / 3.0_f32,
        0.472217_f32,
        0.611105_f32,
        0.715271_f32,
        5.0_f32 / 6.0_f32,
        1.0_f32,
    ];
    let hue = hue - hue.floor();
    let mut index = 0;
    while index < 5 && hue >= x_vtx[index + 1] {
        index += 1;
    }
    let t = (hue - x_vtx[index]) / (x_vtx[index + 1] - x_vtx[index]);
    y_vtx[index] + t * (y_vtx[index + 1] - y_vtx[index])
}

fn ucs_hue_to_ryb_hue(hue: f32) -> f32 {
    let lightness_white = y_to_l_star(1.0_f32);
    let srgb = hue_to_srgb(hue, lightness_white);
    let linear = [
        if srgb[0] <= 0.04045_f32 {
            srgb[0] / 12.92_f32
        } else {
            ((srgb[0] + 0.055_f32) / 1.055_f32).powf(2.4_f32)
        },
        if srgb[1] <= 0.04045_f32 {
            srgb[1] / 12.92_f32
        } else {
            ((srgb[1] + 0.055_f32) / 1.055_f32).powf(2.4_f32)
        },
        if srgb[2] <= 0.04045_f32 {
            srgb[2] / 12.92_f32
        } else {
            ((srgb[2] + 0.055_f32) / 1.055_f32).powf(2.4_f32)
        },
    ];
    rgb_hue_to_ryb_hue(rgb_to_hcv(linear)[0])
}

#[must_use]
pub fn clampf(value: f32, minimum: f32, maximum: f32) -> f32 {
    if value >= minimum {
        if value <= maximum { value } else { maximum }
    } else {
        minimum
    }
}
