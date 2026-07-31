//! Source-shaped scalar color math for `src/iop/colorbalancergb.c`.
//!
//! The constants and operation ordering are copied from
//! `src/common/colorspaces_inline_conversions.h`,
//! `src/common/chromatic_adaptation.h`, and
//! `src/common/darktable_ucs_22_helpers.h`.  This is intentionally not the
//! generic Color Balance implementation or a generic RGB conversion layer.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::double_must_use,
    clippy::excessive_precision,
    clippy::float_cmp,
    clippy::if_not_else,
    clippy::items_after_statements,
    clippy::unreadable_literal,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::too_many_arguments,
    reason = "native conversion constants and source-shaped equations retain their names and precision"
)]

pub type Matrix3 = [[f32; 3]; 3];
pub type Pixel = [f32; 4];

pub const LUT_ELEM: usize = 512;
pub const GAMUT_LUT_STEPS: usize = 92;

pub const D65_X: f32 = 0.31271;
pub const D65_Y: f32 = 0.32902;

const PI: f32 = std::f32::consts::PI;
const TWO_PI: f32 = 2.0 * PI;
const FLT_MIN: f32 = f32::MIN_POSITIVE;

/// Native `XYZ_D50_to_D65_CAT16` in conventional output-row form.
pub const XYZ_D50_TO_D65_CAT16: Matrix3 = [
    [0.989466254, -0.0400304626, 0.0440530317],
    [-0.00540518733, 1.00666069, -0.00175551955],
    [-0.000403920992, 0.0150768030, 1.30210211],
];

/// Native `XYZ_D65_to_D50_CAT16` in conventional output-row form.
pub const XYZ_D65_TO_D50_CAT16: Matrix3 = [
    [1.01085433, 0.0407086103, -0.0341445825],
    [0.00542814201, 0.993581926, 0.00115592039],
    [0.000250722468, -0.0114918759, 0.767964947],
];

#[must_use]
pub fn matrix_mul(left: Matrix3, right: Matrix3) -> Matrix3 {
    let mut result = [[0.0; 3]; 3];
    for row in 0..3 {
        for column in 0..3 {
            // Keep the native dt_colormatrix_mul accumulation order: each
            // product is added to the f32 accumulator before the next one.
            let mut value = left[row][0] * right[0][column];
            value += left[row][1] * right[1][column];
            value += left[row][2] * right[2][column];
            result[row][column] = value;
        }
    }
    result
}

#[must_use]
pub fn apply(matrix: Matrix3, input: [f32; 3]) -> [f32; 3] {
    [
        matrix[0][0] * input[0] + matrix[0][1] * input[1] + matrix[0][2] * input[2],
        matrix[1][0] * input[0] + matrix[1][1] * input[1] + matrix[1][2] * input[2],
        matrix[2][0] * input[0] + matrix[2][1] * input[1] + matrix[2][2] * input[2],
    ]
}

#[must_use]
pub fn apply_pixel(matrix: Matrix3, input: Pixel) -> Pixel {
    let values = apply(matrix, [input[0], input[1], input[2]]);
    [values[0], values[1], values[2], 0.0]
}

/// Native profile RGB → XYZ D65 composition used by gamut-LUT sampling.
#[must_use]
pub fn xyz_d65_input_matrix(profile_input_rgb_to_xyz_d50: Matrix3) -> Matrix3 {
    matrix_mul(XYZ_D50_TO_D65_CAT16, profile_input_rgb_to_xyz_d50)
}

/// Native process input composition from `colorbalancergb.c:615-616`.
///
/// The CAT16 result is composed with the CIE 2006 XYZ-D65 → LMS-D65 matrix
/// before per-pixel evaluation, matching the native f32 accumulation order.
#[must_use]
pub fn input_matrix(profile_input_rgb_to_xyz_d50: Matrix3) -> Matrix3 {
    matrix_mul(
        XYZ_D65_TO_LMS_2006_D65,
        xyz_d65_input_matrix(profile_input_rgb_to_xyz_d50),
    )
}

#[must_use]
pub fn output_matrix(profile_xyz_d50_to_rgb: Matrix3) -> Matrix3 {
    matrix_mul(profile_xyz_d50_to_rgb, XYZ_D65_TO_D50_CAT16)
}

const XYZ_D65_TO_LMS_2006_D65: Matrix3 = [
    [0.257085, 0.859943, -0.031061],
    [-0.394427, 1.175800, 0.106423],
    [0.064856, -0.076250, 0.559067],
];

const LMS_2006_D65_TO_XYZ_D65: Matrix3 = [
    [1.80794659, -1.29971660, 0.34785879],
    [0.61783960, 0.39595453, -0.04104687],
    [-0.12546960, 0.20478038, 1.74274183],
];

const FILMLIGHT_RGB_D65_TO_LMS_D65: Matrix3 =
    [[0.95, 0.05, 0.00], [0.38, 0.62, 0.00], [0.00, 0.03, 0.97]];

const LMS_D65_TO_FILMLIGHT_RGB_D65: Matrix3 = [
    [1.08771930, -0.0877193, 0.0],
    [-0.66666667, 1.66666667, 0.0],
    [0.02061856, -0.05154639, 1.03092784],
];

#[must_use]
pub fn xyz_to_lms(xyz: Pixel) -> Pixel {
    apply_pixel(XYZ_D65_TO_LMS_2006_D65, xyz)
}

#[must_use]
pub fn lms_to_xyz(lms: Pixel) -> Pixel {
    apply_pixel(LMS_2006_D65_TO_XYZ_D65, lms)
}

#[must_use]
pub fn grading_rgb_to_lms(rgb: Pixel) -> Pixel {
    apply_pixel(FILMLIGHT_RGB_D65_TO_LMS_D65, rgb)
}

#[must_use]
pub fn lms_to_grading_rgb(lms: Pixel) -> Pixel {
    apply_pixel(LMS_D65_TO_FILMLIGHT_RGB_D65, lms)
}

#[must_use]
pub fn lms_to_yrg(lms: Pixel) -> Pixel {
    let luminance = 0.68990272 * lms[0] + 0.34832189 * lms[1];
    let total = lms[0] + lms[1] + lms[2];
    let normalized = if total == 0.0 {
        [0.0; 3]
    } else {
        [lms[0] / total, lms[1] / total, lms[2] / total]
    };
    let rgb = lms_to_grading_rgb([normalized[0], normalized[1], normalized[2], 0.0]);
    [luminance, rgb[0], rgb[1], 0.0]
}

#[must_use]
pub fn yrg_to_lms(yrg: Pixel) -> Pixel {
    let rgb = [yrg[1], yrg[2], 1.0 - yrg[1] - yrg[2], 0.0];
    let normalized = grading_rgb_to_lms(rgb);
    let denominator = 0.68990272 * normalized[0] + 0.34832189 * normalized[1];
    let scale = if denominator == 0.0 {
        0.0
    } else {
        yrg[0] / denominator
    };
    [
        normalized[0] * scale,
        normalized[1] * scale,
        normalized[2] * scale,
        0.0,
    ]
}

#[must_use]
pub fn yrg_to_ych(yrg: Pixel) -> Pixel {
    let r = yrg[1] - 0.21902143;
    let g = yrg[2] - 0.54371398;
    let chroma = g.hypot(r);
    let (cos_h, sin_h) = if chroma != 0.0 {
        (r / chroma, g / chroma)
    } else {
        (1.0, 0.0)
    };
    [yrg[0], chroma, cos_h, sin_h]
}

#[must_use]
pub fn ych_to_yrg(ych: Pixel) -> Pixel {
    [
        ych[0],
        ych[1] * ych[2] + 0.21902143,
        ych[1] * ych[3] + 0.54371398,
        0.0,
    ]
}

#[must_use]
pub fn make_ych(y: f32, chroma: f32, hue: f32) -> Pixel {
    [y, chroma, hue.cos(), hue.sin()]
}

#[must_use]
pub fn ych_to_grading_rgb(ych: Pixel) -> Pixel {
    lms_to_grading_rgb(yrg_to_lms(ych_to_yrg(ych)))
}

#[must_use]
pub fn grading_rgb_to_ych(rgb: Pixel) -> Pixel {
    yrg_to_ych(lms_to_yrg(grading_rgb_to_lms(rgb)))
}

pub fn gamut_check_yrg(ych: &mut Pixel) {
    let yrg = ych_to_yrg(*ych);
    const D65_R: f32 = 0.21902143;
    const D65_G: f32 = 0.54371398;
    let mut max_c = ych[1];
    let cos_h = ych[2];
    let sin_h = ych[3];
    if yrg[1] < 0.0 {
        max_c = (-D65_R / cos_h).min(max_c);
    }
    if yrg[2] < 0.0 {
        max_c = (-D65_G / sin_h).min(max_c);
    }
    if yrg[1] + yrg[2] > 1.0 {
        max_c = ((1.0 - D65_R - D65_G) / (cos_h + sin_h)).min(max_c);
    }
    ych[1] = max_c;
}

/// Native CIE 2006/JzAzBz conversion.  Unlike the public shared color crate,
/// this deliberately keeps darktable's fmaxf guards and does not reject
/// negative intermediate cone values.
#[must_use]
pub fn xyz_to_jzazbz(xyz_d65: Pixel) -> Pixel {
    let b = 1.15;
    let g = 0.66;
    let c1 = 0.8359375;
    let c2 = 18.8515625;
    let c3 = 18.6875;
    let n = 0.159301758;
    let p = 134.034375;
    let d = -0.56;
    let d0 = 1.6295499532821566e-11;
    let adjusted = [
        b * xyz_d65[0] - (b - 1.0) * xyz_d65[2],
        g * xyz_d65[1] - (g - 1.0) * xyz_d65[0],
        xyz_d65[2],
    ];
    let lms = apply(
        [
            [0.41478972, 0.57999900, 0.01464800],
            [-0.2015100, 1.1206490, 0.0531008],
            [-0.0166008, 0.2648000, 0.6684799],
        ],
        adjusted,
    );
    let encoded = lms.map(|value| {
        let value = (value / 10000.0).max(0.0).powf(n);
        ((c1 + c2 * value) / (1.0 + c3 * value)).powf(p)
    });
    let mut jab = apply(
        [
            [0.5, 0.5, 0.0],
            [3.524000, -4.066708, 0.542708],
            [0.199076, 1.096799, -1.295875],
        ],
        encoded,
    );
    jab[0] = (((1.0 + d) * jab[0]) / (1.0 + d * jab[0]) - d0).max(0.0);
    [jab[0], jab[1], jab[2], 0.0]
}

#[must_use]
pub fn jzazbz_to_xyz(jzazbz: Pixel) -> Pixel {
    let b = 1.15;
    let g = 0.66;
    let c1 = 0.8359375;
    let c2 = 18.8515625;
    let c3 = 18.6875;
    let n_inv = 1.0 / 0.159301758;
    let p_inv = 1.0 / 134.034375;
    let d = -0.56;
    let d0 = 1.6295499532821566e-11;
    let iz = ((jzazbz[0] + d0) / (1.0 + d - d * (jzazbz[0] + d0))).max(0.0);
    let lms_p = apply(
        [
            [1.0, 0.1386050432715393, 0.0580473161561189],
            [1.0, -0.1386050432715393, -0.0580473161561189],
            [1.0, -0.0960192420263190, -0.8118918960560390],
        ],
        [iz, jzazbz[1], jzazbz[2]],
    )
    .map(|value| value.max(0.0));
    let lms = lms_p.map(|value| {
        let value = value.powf(p_inv);
        ((c1 - value) / (c3 * value - c2)).max(0.0)
    });
    let lms = lms.map(|value| 10000.0 * value.powf(n_inv));
    let adjusted = apply(
        [
            [1.9242264357876067, -1.0047923125953657, 0.0376514040306180],
            [0.3503167620949991, 0.7264811939316552, -0.0653844229480850],
            [-0.0909828109828475, -0.3127282905230739, 1.5227665613052603],
        ],
        lms,
    );
    let x = (adjusted[0] + (b - 1.0) * adjusted[2]) / b;
    let y = (adjusted[1] + (g - 1.0) * x) / g;
    [x, y, adjusted[2], jzazbz[3]]
}

#[must_use]
pub fn xyz_to_xyy(xyz: Pixel) -> Pixel {
    let xyz = [xyz[0].max(0.0), xyz[1].max(0.0), xyz[2].max(0.0)];
    let sum = xyz[0] + xyz[1] + xyz[2];
    if sum > 0.0 {
        [xyz[0] / sum, xyz[1] / sum, xyz[1], 0.0]
    } else {
        [D65_X, D65_Y, 0.0, 0.0]
    }
}

#[must_use]
pub fn xyy_to_xyz(xyy: Pixel) -> Pixel {
    if xyy[1] == 0.0 {
        [0.0, 0.0, 0.0, 0.0]
    } else {
        [
            xyy[2] * xyy[0] / xyy[1],
            xyy[2],
            xyy[2] * (1.0 - xyy[0] - xyy[1]) / xyy[1],
            0.0,
        ]
    }
}

#[must_use]
pub fn y_to_ucs_lstar(y: f32) -> f32 {
    let y_hat = y.powf(0.631651345306265);
    2.098883786377 * y_hat / (y_hat + 1.12426773749357)
}

#[must_use]
pub fn ucs_lstar_to_y(lstar: f32) -> f32 {
    (1.12426773749357 * lstar / (2.098883786377 - lstar)).powf(1.5831518565279648)
}

#[must_use]
pub fn xyy_to_ucs_uv(xyy: Pixel) -> [f32; 2] {
    let x_factors = [-0.783941002840055, 0.745273540913283, 0.318707282433486];
    let y_factors = [0.277512987809202, -0.205375866083878, 2.16743692732158];
    let offsets = [0.153836578598858, -0.165478376301988, 0.291320554395942];
    let mut uvd = [0.0; 3];
    for c in 0..3 {
        uvd[c] = x_factors[c] * xyy[0] + y_factors[c] * xyy[1] + offsets[c];
    }
    let div = if uvd[2] >= 0.0 {
        FLT_MIN.max(uvd[2])
    } else {
        (-FLT_MIN).min(uvd[2])
    };
    uvd[0] /= div;
    uvd[1] /= div;
    let factors = [1.39656225667, 1.4513954287];
    let half_values = [1.49217352929, 1.52488637914];
    let uv_star = [
        factors[0] * uvd[0] / (uvd[0].abs() + half_values[0]),
        factors[1] * uvd[1] / (uvd[1].abs() + half_values[1]),
    ];
    [
        -1.124983854323892 * uv_star[0] - 0.980483721769325 * uv_star[1],
        1.86323315098672 * uv_star[0] + 1.971853092390862 * uv_star[1],
    ]
}

#[must_use]
pub fn ucs_luv_to_jch(lstar: f32, l_white: f32, uv: [f32; 2]) -> Pixel {
    let m2 = uv[0] * uv[0] + uv[1] * uv[1];
    [
        lstar / l_white,
        15.932993652962535 * lstar.powf(0.6523997524738018) * m2.powf(0.6007557017508491) / l_white,
        uv[1].atan2(uv[0]),
        0.0,
    ]
}

#[must_use]
pub fn xyy_to_ucs_jch(xyy: Pixel, l_white: f32) -> Pixel {
    ucs_luv_to_jch(y_to_ucs_lstar(xyy[2]), l_white, xyy_to_ucs_uv(xyy))
}

#[must_use]
pub fn ucs_jch_to_xyy(jch: Pixel, l_white: f32) -> Pixel {
    let lstar = (jch[0] * l_white).clamp(0.0, 2.09885);
    let m = if lstar != 0.0 {
        (jch[1] * l_white / (15.932993652962535 * lstar.powf(0.6523997524738018)))
            .powf(0.8322850678616855)
    } else {
        0.0
    };
    let uv_star_prime = [m * jch[2].cos(), m * jch[2].sin()];
    let uv_star = [
        -5.037522385190711 * uv_star_prime[0] - 2.504856328185843 * uv_star_prime[1],
        4.760029407436461 * uv_star_prime[0] + 2.874012963239247 * uv_star_prime[1],
    ];
    let factors = [1.39656225667, 1.4513954287];
    let half_values = [1.49217352929, 1.52488637914];
    let uv = [
        -half_values[0] * uv_star[0] / (uv_star[0].abs() - factors[0]),
        -half_values[1] * uv_star[1] / (uv_star[1].abs() - factors[1]),
    ];
    let u_factors = [0.167171472114775, -0.150959086409163, 0.940254742367256];
    let v_factors = [0.141299802443708, -0.155185060382272, 1.0];
    let offsets = [
        -0.00801531300850582,
        -0.00843312433578007,
        -0.0256325967652889,
    ];
    let xy_d = [
        u_factors[0] * uv[0] + v_factors[0] * uv[1] + offsets[0],
        u_factors[1] * uv[0] + v_factors[1] * uv[1] + offsets[1],
        u_factors[2] * uv[0] + v_factors[2] * uv[1] + offsets[2],
    ];
    let div = if xy_d[2] >= 0.0 {
        FLT_MIN.max(xy_d[2])
    } else {
        (-FLT_MIN).min(xy_d[2])
    };
    [xy_d[0] / div, xy_d[1] / div, ucs_lstar_to_y(lstar), 0.0]
}

#[must_use]
pub fn ucs_jch_to_hsb(jch: Pixel) -> Pixel {
    let brightness = jch[0] * (jch[1].powf(1.33654221029386) + 1.0);
    [
        jch[2],
        if brightness > 0.0 {
            jch[1] / brightness
        } else {
            0.0
        },
        brightness,
        0.0,
    ]
}

#[must_use]
pub fn ucs_hsb_to_jch(hsb: Pixel) -> Pixel {
    let chroma = hsb[1] * hsb[2];
    [
        hsb[2] / (chroma.powf(1.33654221029386) + 1.0),
        chroma,
        hsb[0],
        0.0,
    ]
}

#[must_use]
pub fn ucs_jch_to_hcb(jch: Pixel) -> Pixel {
    [
        jch[2],
        jch[1],
        jch[0] * (jch[1].powf(1.33654221029386) + 1.0),
        0.0,
    ]
}

#[must_use]
pub fn ucs_hcb_to_jch(hcb: Pixel) -> Pixel {
    [
        hcb[2] / (hcb[1].powf(1.33654221029386) + 1.0),
        hcb[1],
        hcb[0],
        0.0,
    ]
}

#[must_use]
pub fn lookup_gamut(gamut_lut: &[f32; LUT_ELEM], hue: f32) -> f32 {
    let x_test = LUT_ELEM as f32 * (hue + PI) / TWO_PI;
    let x_prev = x_test.floor();
    let x_next = x_test.ceil();
    let xi = (x_prev as i32 & (LUT_ELEM as i32 - 1)) as usize;
    let xii = (x_next as i32 & (LUT_ELEM as i32 - 1)) as usize;
    let y_prev = gamut_lut[xi];
    y_prev
        + if xi != xii {
            (x_test - x_prev) * (gamut_lut[xii] - y_prev)
        } else {
            0.0
        }
}

#[must_use]
pub fn soft_clip(x: f32, soft_threshold: f32, hard_threshold: f32) -> f32 {
    let norm = hard_threshold - soft_threshold;
    if x > soft_threshold {
        soft_threshold + (1.0 - (-(x - soft_threshold) / norm).exp()) * norm
    } else {
        x
    }
}

#[must_use]
pub fn build_jz_gamut_lut(
    input_matrix: Matrix3,
    mut cancelled: impl FnMut() -> bool,
) -> Result<[f32; LUT_ELEM], ()> {
    let mut sampler = [0.0_f32; LUT_ELEM];
    for r in 0..GAMUT_LUT_STEPS {
        if cancelled() {
            return Err(());
        }
        for g in 0..GAMUT_LUT_STEPS {
            for b in 0..GAMUT_LUT_STEPS {
                let rgb = [
                    r as f32 / (GAMUT_LUT_STEPS - 1) as f32,
                    g as f32 / (GAMUT_LUT_STEPS - 1) as f32,
                    b as f32 / (GAMUT_LUT_STEPS - 1) as f32,
                ];
                let xyz = apply(input_matrix, rgb);
                let jab = xyz_to_jzazbz([xyz[0], xyz[1], xyz[2], 0.0]);
                let chroma = jab[2].hypot(jab[1]);
                let saturation = if jab[0] > 0.0 { chroma / jab[0] } else { 0.0 };
                let hue = jab[2].atan2(jab[1]);
                let mut index = ((LUT_ELEM - 1) as f32 * (hue + PI) / TWO_PI).round() as i32;
                if index < 0 {
                    index += LUT_ELEM as i32;
                }
                if index >= LUT_ELEM as i32 {
                    index -= LUT_ELEM as i32;
                }
                sampler[index as usize] = sampler[index as usize].max(saturation);
            }
        }
    }
    let mut lut = [0.0; LUT_ELEM];
    for k in 2..LUT_ELEM - 2 {
        lut[k] =
            (sampler[k - 2] + sampler[k - 1] + sampler[k] + sampler[k + 1] + sampler[k + 2]) / 5.0;
    }
    lut[0] = (sampler[LUT_ELEM - 2] + sampler[LUT_ELEM - 1] + sampler[0] + sampler[1] + sampler[2])
        / 5.0;
    lut[1] = (sampler[LUT_ELEM - 1] + sampler[0] + sampler[1] + sampler[2] + sampler[3]) / 5.0;
    lut[LUT_ELEM - 1] = (sampler[LUT_ELEM - 3]
        + sampler[LUT_ELEM - 2]
        + sampler[LUT_ELEM - 1]
        + sampler[0]
        + sampler[1])
        / 5.0;
    lut[LUT_ELEM - 2] = (sampler[LUT_ELEM - 4]
        + sampler[LUT_ELEM - 3]
        + sampler[LUT_ELEM - 2]
        + sampler[LUT_ELEM - 1]
        + sampler[0])
        / 5.0;
    Ok(lut)
}

#[must_use]
pub fn build_ucs_gamut_lut(
    input_matrix: Matrix3,
    mut cancelled: impl FnMut() -> bool,
) -> Result<[f32; LUT_ELEM], ()> {
    let d65_xyy = [D65_X, D65_Y, 1.0, 0.0];
    let red = apply(input_matrix, [1.0, 0.0, 0.0]);
    let green = apply(input_matrix, [0.0, 1.0, 0.0]);
    let blue = apply(input_matrix, [0.0, 0.0, 1.0]);
    let red_xyy = xyz_to_xyy([red[0], red[1], red[2], 0.0]);
    let green_xyy = xyz_to_xyy([green[0], green[1], green[2], 0.0]);
    let blue_xyy = xyz_to_xyy([blue[0], blue[1], blue[2], 0.0]);
    let h_red = (red_xyy[1] - d65_xyy[1]).atan2(red_xyy[0] - d65_xyy[0]);
    let h_green = (green_xyy[1] - d65_xyy[1]).atan2(green_xyy[0] - d65_xyy[0]);
    let h_blue = (blue_xyy[1] - d65_xyy[1]).atan2(blue_xyy[0] - d65_xyy[0]);
    let mut lut = [0.0; LUT_ELEM];
    let mut sampler = [0.0_f32; LUT_ELEM];
    for i in 0..50 * LUT_ELEM {
        if i % LUT_ELEM == 0 && cancelled() {
            return Err(());
        }
        let angle = -PI + i as f32 / (50 * LUT_ELEM) as f32 * TWO_PI;
        let tan_angle = angle.tan();
        let t1 = delta_h(angle, h_blue) / delta_h(h_red, h_blue);
        let t2 = delta_h(angle, h_red) / delta_h(h_green, h_red);
        let t3 = delta_h(angle, h_green) / delta_h(h_blue, h_green);
        let (x_t, y_t) = if t1 == t1.clamp(0.0, 1.0) {
            let t = (d65_xyy[1] - blue_xyy[1] + tan_angle * (blue_xyy[0] - d65_xyy[0]))
                / (red_xyy[1] - blue_xyy[1] + tan_angle * (blue_xyy[0] - red_xyy[0]));
            (
                blue_xyy[0] + t * (red_xyy[0] - blue_xyy[0]),
                blue_xyy[1] + t * (red_xyy[1] - blue_xyy[1]),
            )
        } else if t2 == t2.clamp(0.0, 1.0) {
            let t = (d65_xyy[1] - red_xyy[1] + tan_angle * (red_xyy[0] - d65_xyy[0]))
                / (green_xyy[1] - red_xyy[1] + tan_angle * (red_xyy[0] - green_xyy[0]));
            (
                red_xyy[0] + t * (green_xyy[0] - red_xyy[0]),
                red_xyy[1] + t * (green_xyy[1] - red_xyy[1]),
            )
        } else if t3 == t3.clamp(0.0, 1.0) {
            let t = (d65_xyy[1] - green_xyy[1] + tan_angle * (green_xyy[0] - d65_xyy[0]))
                / (blue_xyy[1] - green_xyy[1] + tan_angle * (green_xyy[0] - blue_xyy[0]));
            (
                green_xyy[0] + t * (blue_xyy[0] - green_xyy[0]),
                green_xyy[1] + t * (blue_xyy[1] - green_xyy[1]),
            )
        } else {
            (0.0, 0.0)
        };
        let uv = xyy_to_ucs_uv([x_t, y_t, 1.0, 0.0]);
        let hue = uv[1].atan2(uv[0]);
        let mut index = ((LUT_ELEM - 1) as f32 * (hue + PI) / TWO_PI).round() as i32;
        if index < 0 {
            index += LUT_ELEM as i32;
        }
        if index >= LUT_ELEM as i32 {
            index -= LUT_ELEM as i32;
        }
        lut[index as usize] += uv[0] * uv[0] + uv[1] * uv[1];
        sampler[index as usize] += 1.0;
    }
    for k in 0..LUT_ELEM {
        lut[k] /= 1.0_f32.max(sampler[k]);
    }
    Ok(lut)
}

#[must_use]
pub fn delta_h(h1: f32, h2: f32) -> f32 {
    let mut diff = h1 - h2;
    if diff < -PI {
        diff += TWO_PI;
    }
    if diff > PI {
        diff -= TWO_PI;
    }
    diff
}
