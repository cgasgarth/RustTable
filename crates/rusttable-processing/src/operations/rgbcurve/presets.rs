//! Source-derived preset construction from `init_presets()` in
//! `src/iop/rgbcurve.c`. Presets remain operation-local until a later owner
//! ports history/style materialization.

#![forbid(unsafe_code)]
#![expect(
    clippy::imprecise_flops,
    clippy::suboptimal_flops,
    reason = "Native RGB Curve preset equations preserve source evaluation order and IEEE-754 parity."
)]

use super::parameters::{RgbCurveParametersV1, RgbCurveType};

/// Native `DEVELOP_BLEND_CS_RGB_DISPLAY` preset blend-space contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RgbCurvePresetBlendColorspace {
    RgbDisplay,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RgbCurvePreset {
    /// Display name and native localization key passed to `_()`.
    pub name: &'static str,
    pub localization_key: &'static str,
    /// Native `dt_gui_presets_add_generic(..., TRUE, ...)` marker.
    pub generic: bool,
    pub blend_colorspace: RgbCurvePresetBlendColorspace,
    pub parameters: RgbCurveParametersV1,
}

const fn preset(parameters: RgbCurveParametersV1, name: &'static str) -> RgbCurvePreset {
    RgbCurvePreset {
        name,
        localization_key: name,
        generic: true,
        blend_colorspace: RgbCurvePresetBlendColorspace::RgbDisplay,
        parameters,
    }
}

/// Reproduces the native preset sequence and f32 arithmetic.
#[must_use]
pub fn init_presets() -> Vec<RgbCurvePreset> {
    let mut parameters = RgbCurveParametersV1::default();
    parameters.curve_num_nodes = [6, 7, 7];
    parameters.curve_type = [RgbCurveType::CubicSpline; 3];
    parameters.compensate_middle_grey = true;

    let linear_ab = [0.0_f32, 0.08, 0.3, 0.5, 0.7, 0.92, 1.0];
    for channel in 1..=2 {
        for (index, value) in linear_ab.into_iter().enumerate() {
            parameters.curve_nodes[channel][index] =
                super::parameters::RgbCurveNode::new(value, value);
        }
    }
    parameters.curve_nodes[0][..6].copy_from_slice(&[
        super::parameters::RgbCurveNode::new(0.0, 0.0),
        super::parameters::RgbCurveNode::new(0.003862, 0.007782),
        super::parameters::RgbCurveNode::new(0.076613, 0.156182),
        super::parameters::RgbCurveNode::new(0.169355, 0.290352),
        super::parameters::RgbCurveNode::new(0.774194, 0.773852),
        super::parameters::RgbCurveNode::new(1.0, 1.0),
    ]);

    let mut presets = vec![preset(parameters.clone(), "contrast | compression")];

    parameters.curve_num_nodes[0] = 7;
    let linear_l = [0.0_f32, 0.08, 0.17, 0.50, 0.83, 0.92, 1.0];
    set_channel(&mut parameters, 0, &linear_l, &linear_l);
    presets.push(preset(parameters.clone(), "linear (gamma 1.0)"));

    let mut y = linear_l;
    y[1] -= 0.020;
    y[2] -= 0.030;
    y[4] += 0.030;
    y[5] += 0.020;
    set_channel(&mut parameters, 0, &linear_l, &y);
    presets.push(preset(parameters.clone(), "contrast | medium (linear)"));

    let mut y = linear_l;
    y[1] -= 0.040;
    y[2] -= 0.060;
    y[4] += 0.060;
    y[5] += 0.040;
    set_channel(&mut parameters, 0, &linear_l, &y);
    presets.push(preset(parameters.clone(), "contrast | high (linear)"));

    let mut x = linear_l;
    let mut y = linear_l;
    y[1] -= 0.020;
    y[2] -= 0.030;
    y[4] += 0.030;
    y[5] += 0.020;
    for index in 1..6 {
        x[index] = x[index].powf(2.2);
        y[index] = y[index].powf(2.2);
    }
    set_channel(&mut parameters, 0, &x, &y);
    presets.push(preset(parameters.clone(), "contrast | medium (gamma 2.2)"));

    let mut x = linear_l;
    let mut y = linear_l;
    y[1] -= 0.040;
    y[2] -= 0.060;
    y[4] += 0.060;
    y[5] += 0.040;
    for index in 1..6 {
        x[index] = x[index].powf(2.2);
        y[index] = y[index].powf(2.2);
    }
    set_channel(&mut parameters, 0, &x, &y);
    presets.push(preset(parameters.clone(), "contrast | high (gamma 2.2)"));

    parameters.curve_type[0] = RgbCurveType::MonotoneHermite;
    set_channel(&mut parameters, 0, &linear_l, &linear_l);
    let mut y = linear_l;
    for index in 1..6 {
        y[index] *= y[index];
    }
    set_channel(&mut parameters, 0, &linear_l, &y);
    presets.push(preset(parameters.clone(), "non-contrast curve | gamma 2.0"));

    let mut y = linear_l;
    for index in 1..6 {
        y[index] = linear_l[index].sqrt();
    }
    set_channel(&mut parameters, 0, &linear_l, &y);
    presets.push(preset(parameters.clone(), "non-contrast curve | gamma 0.5"));

    let mut y = linear_l;
    for index in 1..6 {
        y[index] = (linear_l[index] + 1.0).ln() / 2.0_f32.ln();
    }
    set_channel(&mut parameters, 0, &linear_l, &y);
    presets.push(preset(
        parameters.clone(),
        "non-contrast curve | logarithm (base 2)",
    ));

    let mut y = linear_l;
    for index in 1..6 {
        y[index] = 2.0_f32.powf(linear_l[index]) - 1.0;
    }
    set_channel(&mut parameters, 0, &linear_l, &y);
    presets.push(preset(
        parameters,
        "non-contrast curve | exponential (base 2)",
    ));
    presets
}

fn set_channel(parameters: &mut RgbCurveParametersV1, channel: usize, x: &[f32; 7], y: &[f32; 7]) {
    for index in 0..7 {
        parameters.curve_nodes[channel][index] =
            super::parameters::RgbCurveNode::new(x[index], y[index]);
    }
}
