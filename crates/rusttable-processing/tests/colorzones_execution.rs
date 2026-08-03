#![expect(
    clippy::imprecise_flops,
    clippy::suboptimal_flops,
    reason = "Native Color Zones test vectors preserve source evaluation order and IEEE-754 parity."
)]
#![allow(
    clippy::cast_precision_loss,
    reason = "source-derived tests construct native f32 curve coordinates from bounded node indices"
)]

use rusttable_processing::operations::colorzones::{
    COLORZONES_LUT_RESOLUTION, ColorZonesChannel, ColorZonesCompileError, ColorZonesConfig,
    ColorZonesMode, ColorZonesNode, ColorZonesParametersV5, ColorZonesPixel, ColorZonesPlan,
    ColorZonesSplinesVersion,
};

fn constant_config(
    selection: ColorZonesChannel,
    mode: ColorZonesMode,
    values: [f32; 3],
    strength: f32,
) -> ColorZonesConfig {
    let mut parameters = ColorZonesParametersV5::defaults();
    parameters.channel = selection.raw();
    parameters.mode = mode.raw();
    parameters.curve_num_nodes = [2, 2, 2];
    parameters.strength = strength;
    // V2 nonperiodic sampling truncates the held endpoint value outside the
    // active x span but rounds evaluated samples inside it. Cover the complete
    // unit interval there. Periodic sampling folds x=1 onto x=0, so use two
    // distinct interior anchors instead.
    let [first_x, last_x] = if selection == ColorZonesChannel::Hue {
        [0.25, 0.75]
    } else {
        [0.0, 1.0]
    };
    for (curve, value) in values.into_iter().enumerate() {
        parameters.curves[curve][0] = ColorZonesNode::new(first_x, value);
        parameters.curves[curve][1] = ColorZonesNode::new(last_x, value);
    }
    ColorZonesConfig::try_from(parameters).expect("checked constant Color Zones config")
}

fn pixel_bits(pixel: ColorZonesPixel) -> [u32; 4] {
    pixel.channels().map(f32::to_bits)
}

#[test]
fn compiles_three_exact_channel_major_quantized_luts() {
    let plan = ColorZonesPlan::new(constant_config(
        ColorZonesChannel::Lightness,
        ColorZonesMode::Smooth,
        [0.25, 0.5, 0.75],
        0.0,
    ))
    .expect("constant LUT plan");
    let expected: [f32; 3] = [0.25, 0.5, 49_151.0 / 65_536.0];

    for (channel, expected) in [
        (ColorZonesChannel::Lightness, expected[0]),
        (ColorZonesChannel::Chroma, expected[1]),
        (ColorZonesChannel::Hue, expected[2]),
    ] {
        let lut = plan.lut(channel);
        assert_eq!(lut.len(), COLORZONES_LUT_RESOLUTION);
        assert!(
            lut.iter()
                .all(|value| value.to_bits() == expected.to_bits())
        );
    }

    let strengthened = ColorZonesPlan::new(constant_config(
        ColorZonesChannel::Lightness,
        ColorZonesMode::Smooth,
        [0.25, 0.5, 0.75],
        100.0,
    ))
    .expect("strength-adjusted LUT plan");
    assert!(
        strengthened
            .lut(ColorZonesChannel::Lightness)
            .iter()
            .all(|value| value.to_bits() == 0.0_f32.to_bits())
    );
    assert!(
        strengthened
            .lut(ColorZonesChannel::Chroma)
            .iter()
            .all(|value| value.to_bits() == 0.5_f32.to_bits())
    );
    assert!(
        strengthened
            .lut(ColorZonesChannel::Hue)
            .iter()
            .all(|value| value.to_bits() == (65_535.0_f32 / 65_536.0).to_bits())
    );
}

#[test]
fn spline_v1_capacity_failure_is_typed_at_the_safe_rust_boundary() {
    let mut parameters = ColorZonesParametersV5::defaults();
    parameters.splines_version = ColorZonesSplinesVersion::V1.raw();
    parameters.curve_num_nodes[0] = 19;
    for node in 0..19 {
        parameters.curves[0][node] = ColorZonesNode::new(node as f32 / 18.0, 0.5);
    }
    let config = ColorZonesConfig::try_from(parameters).expect("checked v1 config");
    assert_eq!(
        ColorZonesPlan::new(config),
        Err(ColorZonesCompileError::LegacyAnchorCapacityExceeded {
            curve: ColorZonesChannel::Lightness,
            active_nodes: 19,
            required_anchors: 21,
            maximum_anchors: 20,
        })
    );

    parameters.curve_num_nodes[0] = 18;
    let config = ColorZonesConfig::try_from(parameters).expect("checked v1 capacity boundary");
    assert!(ColorZonesPlan::new(config).is_ok());
}

#[test]
fn v2_strength_preserves_native_nonperiodic_endpoint_wrapping() {
    let mut parameters = ColorZonesParametersV5::defaults();
    parameters.channel = ColorZonesChannel::Lightness.raw();
    parameters.curve_num_nodes[0] = 1;
    parameters.curves[0][0] = ColorZonesNode::new(0.25, 1.0);
    parameters.strength = 100.0;

    let plan = ColorZonesPlan::new(
        ColorZonesConfig::try_from(parameters).expect("checked strengthened V2 config"),
    )
    .expect("native-defined endpoint wrapping compiles");
    let lightness = plan.lut(ColorZonesChannel::Lightness);

    assert_eq!(lightness[0].to_bits(), (32_766.0_f32 / 65_536.0).to_bits());
    assert_eq!(
        lightness[16_383].to_bits(),
        (65_535.0_f32 / 65_536.0).to_bits()
    );
    assert_eq!(
        lightness[16_384].to_bits(),
        (32_766.0_f32 / 65_536.0).to_bits()
    );
}

#[test]
fn smooth_hue_selection_retains_native_low_chroma_blending() {
    let alpha = f32::from_bits(0x7fc1_2345);
    let input = [ColorZonesPixel::new(50.0, 0.0, 0.0, alpha)];
    let smooth_hue = ColorZonesPlan::new(constant_config(
        ColorZonesChannel::Hue,
        ColorZonesMode::Smooth,
        [1.0, 0.5, 0.5],
        0.0,
    ))
    .expect("smooth hue plan")
    .execute_lab(&input)[0];
    assert_eq!(smooth_hue.lightness().to_bits(), 50.0_f32.to_bits());
    assert_eq!(smooth_hue.alpha().to_bits(), alpha.to_bits());

    let smooth_lightness = ColorZonesPlan::new(constant_config(
        ColorZonesChannel::Lightness,
        ColorZonesMode::Smooth,
        [1.0, 0.5, 0.5],
        0.0,
    ))
    .expect("smooth lightness plan")
    .execute_lab(&input)[0];
    let strong = ColorZonesPlan::new(constant_config(
        ColorZonesChannel::Hue,
        ColorZonesMode::Strong,
        [1.0, 0.5, 0.5],
        0.0,
    ))
    .expect("strong plan")
    .execute_lab(&input)[0];
    let committed_one = 65_535.0_f32 / 65_536.0;
    let expected_lightness = 50.0 * 2.0_f32.powf(4.0 * (committed_one - 0.5));

    assert_eq!(
        smooth_lightness.lightness().to_bits(),
        expected_lightness.to_bits()
    );
    assert_eq!(strong.lightness().to_bits(), expected_lightness.to_bits());
    assert_eq!(smooth_lightness.alpha().to_bits(), alpha.to_bits());
    assert_eq!(strong.alpha().to_bits(), alpha.to_bits());
}

#[test]
fn smooth_and_strong_keep_their_distinct_chroma_selection_scales() {
    let mut parameters = ColorZonesParametersV5::defaults();
    parameters.channel = ColorZonesChannel::Chroma.raw();
    parameters.curve_num_nodes = [2, 1, 1];
    parameters.curves[0][0] = ColorZonesNode::new(0.0, 0.0);
    parameters.curves[0][1] = ColorZonesNode::new(1.0, 1.0);
    parameters.curves[1][0] = ColorZonesNode::new(0.25, 0.5);
    parameters.curves[2][0] = ColorZonesNode::new(0.25, 0.5);

    parameters.mode = ColorZonesMode::Smooth.raw();
    let smooth =
        ColorZonesPlan::new(ColorZonesConfig::try_from(parameters).expect("smooth checked config"))
            .expect("smooth compiled plan");
    parameters.mode = ColorZonesMode::Strong.raw();
    let strong =
        ColorZonesPlan::new(ColorZonesConfig::try_from(parameters).expect("strong checked config"))
            .expect("strong compiled plan");

    let input = [ColorZonesPixel::new(50.0, 128.0, 0.0, -0.0)];
    let smooth_output = smooth.execute_lab(&input)[0];
    let strong_output = strong.execute_lab(&input)[0];
    assert!(smooth_output.lightness() > strong_output.lightness());
    assert_eq!(smooth_output.alpha().to_bits(), (-0.0_f32).to_bits());
    assert_eq!(strong_output.alpha().to_bits(), (-0.0_f32).to_bits());
}

#[test]
fn both_cpu_branches_retain_source_order_chroma_and_hue_equations() {
    let input = [ColorZonesPixel::new(42.0, 3.0, 4.0, -0.0)];
    let committed_chroma = 65_535.0_f32 / 65_536.0;
    let committed_hue = 49_151.0_f32 / 65_536.0;
    let hue_modification = committed_hue - 0.5;

    let smooth = ColorZonesPlan::new(constant_config(
        ColorZonesChannel::Lightness,
        ColorZonesMode::Smooth,
        [0.5, 1.0, 0.75],
        0.0,
    ))
    .expect("smooth formula plan")
    .execute_lab(&input)[0];
    let smooth_hue = ((4.0_f32.atan2(3.0) + std::f32::consts::TAU) % std::f32::consts::TAU)
        / std::f32::consts::TAU;
    let smooth_chroma = (4.0_f32 * 4.0 + 3.0 * 3.0).sqrt();
    let smooth_angle = std::f32::consts::TAU * (smooth_hue + hue_modification);
    let expected_smooth = ColorZonesPixel::new(
        42.0,
        smooth_angle.cos() * (2.0 * committed_chroma) * smooth_chroma,
        smooth_angle.sin() * (2.0 * committed_chroma) * smooth_chroma,
        -0.0,
    );
    assert_eq!(pixel_bits(smooth), pixel_bits(expected_smooth));

    let strong = ColorZonesPlan::new(constant_config(
        ColorZonesChannel::Lightness,
        ColorZonesMode::Strong,
        [0.5, 1.0, 0.75],
        0.0,
    ))
    .expect("strong formula plan")
    .execute_lab(&input)[0];
    let strong_hue = 4.0_f32.atan2(3.0) / std::f32::consts::TAU;
    let strong_chroma = 3.0_f32.hypot(4.0);
    let adjusted_strong_chroma = strong_chroma * (2.0 * committed_chroma);
    let strong_angle = std::f32::consts::TAU * (strong_hue + hue_modification);
    let expected_strong = ColorZonesPixel::new(
        42.0,
        strong_angle.cos() * adjusted_strong_chroma,
        strong_angle.sin() * adjusted_strong_chroma,
        -0.0,
    );
    assert_eq!(pixel_bits(strong), pixel_bits(expected_strong));
}

#[test]
fn execution_is_bitwise_chunk_invariant() {
    let plan = ColorZonesPlan::new(constant_config(
        ColorZonesChannel::Lightness,
        ColorZonesMode::Smooth,
        [0.5, 1.0, 0.75],
        0.0,
    ))
    .expect("compiled point plan");
    let input = [
        ColorZonesPixel::new(-10.0, 3.0, 4.0, 0.0),
        ColorZonesPixel::new(0.0, -8.0, 2.0, 0.25),
        ColorZonesPixel::new(42.0, 0.0, 0.0, 0.5),
        ColorZonesPixel::new(100.0, 64.0, -64.0, 0.75),
        ColorZonesPixel::new(150.0, -12.0, -9.0, 1.0),
    ];

    let whole: Vec<_> = plan
        .execute_lab(&input)
        .into_iter()
        .map(pixel_bits)
        .collect();
    let chunked: Vec<_> = input
        .chunks(2)
        .flat_map(|chunk| plan.execute_lab(chunk))
        .map(pixel_bits)
        .collect();
    assert_eq!(whole, chunked);
}

#[test]
fn authored_normal_blend_uses_lab_coverage_and_preserves_source_alpha() {
    let plan = ColorZonesPlan::new(constant_config(
        ColorZonesChannel::Lightness,
        ColorZonesMode::Strong,
        [0.75, 0.75, 0.5],
        0.0,
    ))
    .expect("compiled blend plan");
    let source_alpha = f32::from_bits(0x7fc1_2345);
    let input = [ColorZonesPixel::new(40.0, 8.0, -4.0, source_alpha)];
    let candidate = plan.execute_lab(&input)[0];
    let coverage = 0.25_f32 * 0.5_f32;
    let blended = plan.execute_lab_normal_blend(&input, Some(&[0.5]), 0.25)[0];
    let source = input[0].channels();
    let candidate = candidate.channels();
    let inverse_scale = [1.0_f32 / 100.0, 1.0_f32 / 128.0, 1.0_f32 / 128.0];
    let scale = [100.0_f32, 128.0_f32, 128.0_f32];
    let expected = ColorZonesPixel::from_channels([
        (source[0] * inverse_scale[0] * (1.0 - coverage)
            + candidate[0] * inverse_scale[0] * coverage)
            * scale[0],
        (source[1] * inverse_scale[1] * (1.0 - coverage)
            + candidate[1] * inverse_scale[1] * coverage)
            * scale[1],
        (source[2] * inverse_scale[2] * (1.0 - coverage)
            + candidate[2] * inverse_scale[2] * coverage)
            * scale[2],
        source[3],
    ]);

    assert_eq!(pixel_bits(blended), pixel_bits(expected));
}
