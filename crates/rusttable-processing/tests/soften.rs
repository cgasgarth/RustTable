#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    reason = "source-derived tests intentionally assert native f32 layout and casts"
)]

use rusttable_processing::common::box_filters::{
    BOX_ITERATIONS, CancellableBoxFilterError, box_mean, box_mean_with_cancel,
};
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
        rusttable_processing::descriptor::RoiKind::Neighborhood
    );
    assert_eq!(descriptor.io.input.channels, 4);
    assert_eq!(descriptor.io.output.channels, 4);
    assert_eq!(
        descriptor.io.output.alpha,
        rusttable_processing::descriptor::AlphaPolicy::Replace
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
fn hsl_conversion_keeps_native_float_double_promotion_boundaries() {
    let dimensions = dimensions(1, 1);
    let input = vec![pixel(0.0, 0.2, 0.3)];
    let output = SoftenPlan::new(
        SoftenConfig::new(0.0, 100.0, 0.0, 100.0).expect("config"),
        dimensions,
    )
    .expect("plan")
    .execute(&input, dimensions)
    .expect("soften");

    // Native colorspaces.h promotes the double literals in the hue and m1
    // expressions, then truncates each temporary back to f32.
    assert_eq!(output[0].red().get().to_bits(), 0x0000_0000);
    assert_eq!(output[0].green().get().to_bits(), 0x3e4c_ccca);
    assert_eq!(output[0].blue().get().to_bits(), 0x3e99_999a);
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

#[test]
fn v1_parameter_order_and_each_native_range_are_enforced() {
    let parameters = SoftenParametersV1::new(1.25, 2.5, -1.5, 99.0);
    let bytes = parameters.to_bytes();
    assert_eq!(
        f32::from_le_bytes(bytes[0..4].try_into().expect("size")),
        1.25
    );
    assert_eq!(
        f32::from_le_bytes(bytes[4..8].try_into().expect("saturation")),
        2.5
    );
    assert_eq!(
        f32::from_le_bytes(bytes[8..12].try_into().expect("brightness")),
        -1.5
    );
    assert_eq!(
        f32::from_le_bytes(bytes[12..16].try_into().expect("amount")),
        99.0
    );

    for (parameters, name) in [
        (SoftenParametersV1::new(-0.01, 100.0, 0.0, 0.0), "size"),
        (SoftenParametersV1::new(0.0, 100.01, 0.0, 0.0), "saturation"),
        (SoftenParametersV1::new(0.0, 100.0, 2.01, 0.0), "brightness"),
        (SoftenParametersV1::new(0.0, 100.0, 0.0, 100.01), "amount"),
        (SoftenParametersV1::new(f32::NAN, 100.0, 0.0, 0.0), "size"),
    ] {
        assert!(
            matches!(
                SoftenConfig::try_from(parameters),
                Err(rusttable_processing::operations::soften::SoftenParameterError::OutOfRange(
                    actual
                )) if actual == name
            ) || matches!(
                SoftenConfig::try_from(parameters),
                Err(rusttable_processing::operations::soften::SoftenParameterError::NonFinite(actual))
                    if actual == name
            )
        );
    }
}

#[test]
fn native_radius_applies_roi_and_piece_scales_after_integer_radius_steps() {
    let dimensions = dimensions(1001, 801);
    let config = SoftenConfig::new(100.0, 100.0, 0.0, 100.0).expect("config");
    let plan = SoftenPlan::new_with_scale(config, dimensions, 0.25, 0.5).expect("scaled plan");

    let full_width = dimensions.width() as f32 * 0.5;
    let full_height = dimensions.height() as f32 * 0.5;
    let mrad = full_width.hypot(full_height) * 0.01;
    let mrad = mrad as u32;
    let rad = (mrad as f32 * 1.0) as u32;
    let expected = mrad.min((rad as f32 * 0.25_f32 / 0.5_f32).ceil() as u32);
    assert_eq!(plan.radius(), expected);

    assert!(SoftenPlan::new_with_scale(config, dimensions, 0.0, 1.0).is_err());
    assert!(SoftenPlan::new_with_scale(config, dimensions, 1.0, f32::INFINITY).is_err());
}

#[test]
fn native_radius_truncates_the_double_size_multiplier_at_the_source_point() {
    let dimensions = dimensions(5000, 1);
    let plan = SoftenPlan::new(
        SoftenConfig::new(57.0, 100.0, 0.0, 100.0).expect("config"),
        dimensions,
    )
    .expect("plan");

    // mrad is 50. Native `rad = mrad * (fmin(100.0, size + 1.0f) / 100.0)`
    // is evaluated in double and truncates 28.999... to 28. An all-f32
    // substitute incorrectly produces 29 here.
    assert_eq!(plan.radius(), 28);
}

#[test]
fn cancellable_box_mean_polls_inside_a_scanline() {
    let width = 129;
    let mut buffer = vec![0.0_f32; width];
    buffer[0] = 1.0;
    let polls = std::cell::Cell::new(0);
    let error = box_mean_with_cancel(&mut buffer, 1, width, 1, 1, 8, || {
        let next = polls.get() + 1;
        polls.set(next);
        next > 5
    })
    .expect_err("scanline cancellation");

    assert_eq!(error, CancellableBoxFilterError::Cancelled);
    assert!(polls.get() > 5);
}

#[test]
fn four_channel_blend_uses_native_zeroed_fourth_channel() {
    let dimensions = dimensions(1, 1);
    let input = vec![rusttable_processing::operations::soften::SoftenPixel::new(
        0.2, 0.4, 0.8, 0.75,
    )];
    let plan = SoftenPlan::new(
        SoftenConfig::new(0.0, 100.0, 0.0, 50.0).expect("config"),
        dimensions,
    )
    .expect("plan");
    let output = plan.execute_rgba(&input, dimensions).expect("soften");

    assert!((output[0].fourth() - 0.375).abs() < 1.0e-7);
    assert_eq!(output[0].channels()[3].to_bits(), 0.375f32.to_bits());
}

#[test]
fn undersized_vertical_frame_uses_edge_normalized_eight_passes() {
    let dimensions = dimensions(1, 1001);
    let mut input = vec![pixel(0.0, 0.0, 0.0); 1001];
    input[0] = pixel(1.0, 1.0, 1.0);
    let plan = SoftenPlan::new(
        SoftenConfig::new(100.0, 100.0, 0.0, 100.0).expect("config"),
        dimensions,
    )
    .expect("plan");
    assert!(plan.radius() > 1, "radius must exceed the frame width");

    let output = plan.execute(&input, dimensions).expect("soften");
    let expected = scalar_box_mean(
        input.iter().map(|pixel| pixel.red().get()).collect(),
        usize::try_from(plan.radius()).expect("radius fits"),
    );
    for (actual, expected) in output.iter().zip(expected) {
        assert!((actual.red().get() - expected).abs() < 1.0e-6);
        assert!((actual.green().get() - expected).abs() < 1.0e-6);
        assert!((actual.blue().get() - expected).abs() < 1.0e-6);
    }
}

#[test]
fn cancellation_and_budget_failures_publish_no_partial_soften_result() {
    let frame_dimensions = dimensions(101, 101);
    let input = vec![pixel(0.2, 0.4, 0.8); 101 * 101];
    let plan = SoftenPlan::new(
        SoftenConfig::new(100.0, 100.0, 0.33, 100.0).expect("config"),
        frame_dimensions,
    )
    .expect("plan");
    let polls = std::cell::Cell::new(0);
    let error = plan
        .execute_with_cancel(&input, frame_dimensions, || {
            let next = polls.get() + 1;
            polls.set(next);
            // The first 105 polls cover the initial source adjustment and
            // blur setup; this cancels from inside the first horizontal box
            // scanline rather than at an outer operation boundary.
            next > 105
        })
        .expect_err("mid-operation cancellation");
    assert_eq!(
        error,
        rusttable_processing::operations::OperationExecutionError::Cancelled
    );
    assert!(
        polls.get() > 105,
        "cancellation must be polled during raster work"
    );

    let huge = dimensions(100_000, 100_000);
    assert!(matches!(
        SoftenPlan::new(SoftenConfig::defaults(), huge),
        Err(rusttable_processing::operations::OperationExecutionError::MemoryBudgetExceeded { .. })
    ));
}

fn scalar_box_mean(mut values: Vec<f32>, radius: usize) -> Vec<f32> {
    for _ in 0..BOX_ITERATIONS {
        let source = values.clone();
        for (index, value) in values.iter_mut().enumerate() {
            let start = index.saturating_sub(radius);
            let end = index
                .saturating_add(radius)
                .saturating_add(1)
                .min(source.len());
            let sum: f32 = source[start..end].iter().copied().sum();
            *value = sum / (end - start) as f32;
        }
    }
    values
}
