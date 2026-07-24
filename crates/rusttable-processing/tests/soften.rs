use rusttable_processing::common::box_filters::{BOX_ITERATIONS, box_mean};
use rusttable_processing::operations::soften::{
    SOFTEN_PARAMETER_BYTES, SoftenConfig, SoftenHistory, SoftenParametersV1, SoftenPlan,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions, builtin_registry, descriptor};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("red"),
        FiniteF32::new(green).expect("green"),
        FiniteF32::new(blue).expect("blue"),
    )
}

#[test]
fn v1_payload_defaults_and_unknown_history_are_typed() {
    let parameters = SoftenParametersV1::defaults();
    assert_eq!(parameters.to_bytes().len(), SOFTEN_PARAMETER_BYTES);
    assert_eq!(
        SoftenParametersV1::from_bytes(&parameters.to_bytes()),
        Ok(parameters)
    );
    assert_eq!(
        SoftenHistory::decode(1, &parameters.to_bytes()).expect("v1 history"),
        SoftenHistory::V1(parameters)
    );
    assert_eq!(
        SoftenHistory::decode(9, &[4, 5]).expect("future history"),
        SoftenHistory::Opaque {
            version: 9,
            bytes: vec![4, 5]
        }
    );
}

#[test]
fn descriptor_registry_and_validation_match_the_backend_contract() {
    let descriptor = descriptor::soften_descriptor();
    descriptor.validate().expect("soften descriptor");
    assert_eq!(descriptor.id.compatibility_name, "soften");
    assert_eq!(descriptor.id.parameter_version, 1);
    assert_eq!(
        descriptor.roi,
        rusttable_processing::descriptor::RoiKind::FullImage
    );
    assert_eq!(
        descriptor.io.output.alpha,
        rusttable_processing::descriptor::AlphaPolicy::Preserve
    );
    assert!(builtin_registry().definition("rusttable.soften").is_some());
    assert!(SoftenConfig::new(50.0, 100.0, 3.0, 50.0).is_err());
}

#[test]
fn zero_mix_is_exact_pass_through_and_adjustment_is_source_immutable() {
    let dimensions = dimensions(4, 4);
    let input = vec![pixel(0.2, 0.4, 0.8); 16];
    let identity = SoftenPlan::new(
        SoftenConfig::new(50.0, 100.0, 0.33, 0.0).expect("identity config"),
        dimensions,
    )
    .expect("identity plan")
    .execute(&input, dimensions)
    .expect("identity execution");
    assert_eq!(identity, input);

    let plan = SoftenPlan::new(
        SoftenConfig::new(0.0, 0.0, 1.0, 100.0).expect("adjust config"),
        dimensions,
    )
    .expect("adjust plan");
    let first = plan.execute(&input, dimensions).expect("first");
    let second = plan.execute(&input, dimensions).expect("second");
    assert_eq!(first, second);
    assert_ne!(first, input);
}

#[test]
fn near_gray_input_uses_darktable_saturation_denominator_floor() {
    // soften.c delegates to colorspaces.h::rgb2hsl, which floors the
    // saturation denominator at 2^-16. Below that threshold, the HSL
    // round-trip deliberately pulls this almost-neutral black toward gray.
    let dimensions = dimensions(1, 1);
    let input = vec![pixel(1.0e-6, 1.1e-6, 1.0e-6)];
    let output = SoftenPlan::new(
        SoftenConfig::new(0.0, 100.0, 0.0, 100.0).expect("config"),
        dimensions,
    )
    .expect("plan")
    .execute(&input, dimensions)
    .expect("soften");

    let expected = [1.043_118_7e-6, 1.056_881_4e-6, 1.043_118_7e-6];
    let actual = [
        output[0].red().get(),
        output[0].green().get(),
        output[0].blue().get(),
    ];
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 5.0e-13);
    }
}

#[test]
fn production_soften_uses_the_shared_four_channel_eight_pass_box_mean() {
    // A 101x1 raster gives soften.c's full-image radius calculation radius 1.
    // An edge impulse distinguishes clipped-window normalization from both a
    // clamped-edge box and the former Gaussian substitute.
    let dimensions = dimensions(101, 1);
    let mut input = vec![pixel(0.0, 0.0, 0.0); 101];
    input[0] = pixel(1.0, 1.0, 1.0);
    let plan = SoftenPlan::new(
        SoftenConfig::new(100.0, 100.0, 0.0, 100.0).expect("config"),
        dimensions,
    )
    .expect("plan");
    assert_eq!(plan.radius(), 1);

    let output = plan.execute(&input, dimensions).expect("soften");
    let mut rgba = vec![0.0; 101 * 4];
    rgba[0..4].copy_from_slice(&[1.0, 1.0, 1.0, 0.0]);
    box_mean(&mut rgba, 1, 101, 4, 1, BOX_ITERATIONS).expect("shared soften mean");

    let (expected_pixels, remainder) = rgba.as_chunks::<4>();
    assert!(remainder.is_empty());
    for (actual, expected) in output.iter().zip(expected_pixels) {
        assert!((actual.red().get() - expected[0]).abs() < 1.0e-6);
        assert!((actual.green().get() - expected[1]).abs() < 1.0e-6);
        assert!((actual.blue().get() - expected[2]).abs() < 1.0e-6);
        assert_eq!(expected[3].to_bits(), 0.0f32.to_bits());
    }
    assert!(output[0].red().get() > output[1].red().get());
    assert!(output[1].red().get() > output[8].red().get());
}
