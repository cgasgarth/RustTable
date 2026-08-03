//! Source-derived tests for the standalone bounded Soften CPU leaf.

#![allow(
    clippy::assertions_on_constants,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::too_many_lines,
    reason = "hard-coded native ABI bits and f32 processing vectors are intentional"
)]

#[path = "../src/operations/soften/leaf.rs"]
mod soften_leaf;

use std::cell::Cell;
use std::mem::size_of;

use rusttable_processing::descriptor::{AlphaPolicy, OperationFlags, ParameterDefault, RoiKind};
use soften_leaf::{
    SOFTEN_ALLOW_TILING, SOFTEN_CHANNELS, SOFTEN_DEFAULT_BRIGHTNESS, SOFTEN_DEFAULT_COLORSPACE,
    SOFTEN_DEFAULT_ENABLED, SOFTEN_DEFAULT_GROUPS, SOFTEN_DEFAULT_VISIBLE, SOFTEN_DESCRIPTION,
    SOFTEN_GPU_EXECUTABLE, SOFTEN_GPU_KERNELS, SOFTEN_GPU_PROGRAM, SOFTEN_MIGRATION_EDGES,
    SOFTEN_OPERATION_ORDERS, SOFTEN_PARAMETER_BYTES, SOFTEN_SCHEMA_VERSION, SOFTEN_SOURCE_MAP,
    SoftenCodecError, SoftenConfig, SoftenDimensions, SoftenExecutionError, SoftenHistory,
    SoftenParameterError, SoftenParametersV1, SoftenPixel, SoftenPlan, SoftenPortStatus, SoftenRoi,
    capabilities, soften_descriptor,
};

const DEFAULT_FIXTURE: &str = include_str!("fixtures/soften/default-v1.hex");
const BENCHMARK_FIXTURE: &str = include_str!("fixtures/soften/benchmark-4.2-v1.hex");

fn dimensions(width: u32, height: u32) -> SoftenDimensions {
    SoftenDimensions::new(width, height).expect("valid dimensions")
}

fn config(size: f32, saturation: f32, brightness: f32, amount: f32) -> SoftenConfig {
    SoftenConfig::new(SoftenParametersV1::new(
        size, saturation, brightness, amount,
    ))
    .expect("valid soften parameters")
}

const fn pixel(red: f32, green: f32, blue: f32, fourth: f32) -> SoftenPixel {
    SoftenPixel::new(red, green, blue, fourth)
}

fn fixture_payload(source: &str) -> Vec<u8> {
    let hex = source
        .lines()
        .find_map(|line| line.strip_prefix("payload_hex="))
        .expect("payload_hex fixture field");
    let (pairs, remainder) = hex.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty());
    pairs
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII hex"), 16)
                .expect("valid hex byte")
        })
        .collect()
}

#[test]
fn native_v1_abi_defaults_and_benchmark_payload_are_exact() {
    assert_eq!(SOFTEN_SCHEMA_VERSION, 1);
    assert_eq!(SOFTEN_PARAMETER_BYTES, 16);
    assert_eq!(size_of::<SoftenParametersV1>(), 16);
    assert_eq!(SOFTEN_MIGRATION_EDGES, &[]);

    let defaults = SoftenParametersV1::defaults();
    assert_eq!(defaults.size.to_bits(), 50.0_f32.to_bits());
    assert_eq!(defaults.saturation.to_bits(), 100.0_f32.to_bits());
    assert_eq!(defaults.brightness.to_bits(), 0.33_f32.to_bits());
    assert_eq!(defaults.amount.to_bits(), 50.0_f32.to_bits());
    assert_eq!(
        defaults.to_bytes().as_slice(),
        fixture_payload(DEFAULT_FIXTURE)
    );
    assert!(DEFAULT_FIXTURE.contains("field_order=size,saturation,brightness,amount"));
    assert!(DEFAULT_FIXTURE.contains("migration_edges=[]"));

    // `src/tests/benchmark/darktable-bench-4.2.xmp` lines 621-624.
    let benchmark = fixture_payload(BENCHMARK_FIXTURE);
    let decoded = SoftenParametersV1::from_bytes(&benchmark).expect("native benchmark payload");
    assert_eq!(decoded.size.to_bits(), 0x40b2_3d71);
    assert_eq!(decoded.saturation.to_bits(), 0x42c8_0000);
    assert_eq!(decoded.brightness.to_bits(), 0x3ea8_f5c3);
    assert_eq!(decoded.amount.to_bits(), 0x408a_3d71);
    assert_eq!(decoded.to_bytes().as_slice(), benchmark);
}

#[test]
fn codec_has_no_invented_migration_and_future_history_stays_opaque() {
    let defaults = SoftenParametersV1::defaults();
    let current = SoftenHistory::decode(1, &defaults.to_bytes()).expect("v1 decode");
    assert_eq!(current.version(), 1);
    assert_eq!(current.payload(), defaults.to_bytes());
    assert_eq!(current.migrate_to_current(), Ok(defaults));

    let future = SoftenHistory::decode(9, &[0xde, 0xad, 0xbe, 0xef]).expect("opaque decode");
    assert_eq!(future.version(), 9);
    assert_eq!(future.payload(), [0xde, 0xad, 0xbe, 0xef]);
    assert_eq!(
        future.migrate_to_current(),
        Err(SoftenCodecError::UnsupportedVersion(9))
    );
    assert_eq!(
        SoftenHistory::decode(1, &[0; 15]),
        Err(SoftenCodecError::InvalidLength {
            expected: 16,
            actual: 15,
        })
    );
}

#[test]
fn executable_planning_accepts_finite_commit_values_and_rejects_nonfinite() {
    for parameters in [
        SoftenParametersV1::new(-0.01, 100.0, 0.0, 50.0),
        SoftenParametersV1::new(125.0, 100.0, 0.0, 50.0),
        SoftenParametersV1::new(50.0, -0.01, 0.0, 50.0),
        SoftenParametersV1::new(50.0, 100.01, 0.0, 50.0),
        SoftenParametersV1::new(50.0, 100.0, -2.01, 50.0),
        SoftenParametersV1::new(50.0, 100.0, 2.01, 50.0),
        SoftenParametersV1::new(50.0, 100.0, 0.0, -0.01),
        SoftenParametersV1::new(50.0, 100.0, 0.0, 100.01),
        SoftenParametersV1::new(f32::MAX, -f32::MAX, f32::MAX, -f32::MAX),
    ] {
        let committed = SoftenConfig::new(parameters).expect("finite commit_params values");
        assert_eq!(committed.parameters(), parameters);
        SoftenPlan::new(committed, dimensions(1, 1)).expect("finite values remain plannable");
    }

    for (parameters, field) in [
        (SoftenParametersV1::new(f32::NAN, 100.0, 0.0, 50.0), "size"),
        (
            SoftenParametersV1::new(50.0, f32::INFINITY, 0.0, 50.0),
            "saturation",
        ),
        (
            SoftenParametersV1::new(50.0, 100.0, f32::NEG_INFINITY, 50.0),
            "brightness",
        ),
        (
            SoftenParametersV1::new(50.0, 100.0, 0.0, f32::NAN),
            "amount",
        ),
    ] {
        assert_eq!(
            SoftenConfig::new(parameters),
            Err(SoftenParameterError::NonFinite(field))
        );
    }
}

#[test]
fn local_descriptor_preserves_source_order_without_claiming_shared_routing() {
    let descriptor = soften_descriptor();
    descriptor.validate().expect("bounded soften descriptor");
    assert_eq!(descriptor.id.compatibility_name, "soften");
    assert_eq!(descriptor.id.rust_id, "rusttable.soften");
    assert_eq!(descriptor.id.parameter_version, 1);
    assert_eq!(
        descriptor
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect::<Vec<_>>(),
        ["size", "saturation", "brightness", "amount"]
    );
    let ParameterDefault::Scalar(brightness_default) = &descriptor.parameters[2].default else {
        panic!("brightness default must remain scalar");
    };
    assert_eq!(
        brightness_default.to_bits(),
        f64::from(SOFTEN_DEFAULT_BRIGHTNESS).to_bits()
    );
    assert_ne!(brightness_default.to_bits(), 0.33_f64.to_bits());
    assert_eq!(descriptor.roi, RoiKind::Neighborhood);
    assert_eq!(
        descriptor.io.input.channels,
        u8::try_from(SOFTEN_CHANNELS).expect("four channels fit u8")
    );
    assert_eq!(descriptor.io.output.alpha, AlphaPolicy::Replace);
    assert!(descriptor.flags.contains(OperationFlags::MULTI_INSTANCE));
    assert!(descriptor.flags.contains(OperationFlags::STYLE_ELIGIBLE));
    assert!(descriptor.flags.contains(OperationFlags::TILEABLE));
    assert!(descriptor.flags.contains(OperationFlags::BLENDING));
    assert!(!descriptor.mask_blend.consumes_mask);
    assert!(!descriptor.mask_blend.blend_if);
    assert!(descriptor.migration.opaque_unknown_allowed);
    assert!(descriptor.ui.is_none());

    assert!(!SOFTEN_DEFAULT_ENABLED);
    assert!(!SOFTEN_DEFAULT_VISIBLE);
    assert_eq!(SOFTEN_DEFAULT_GROUPS, ["effect", "effects"]);
    assert_eq!(SOFTEN_DEFAULT_COLORSPACE, "RGB");
    assert_eq!(
        SOFTEN_DESCRIPTION.main_text,
        "create a softened image using the Orton effect"
    );
    assert_eq!(SOFTEN_DESCRIPTION.purpose, "creative");
    assert_eq!(SOFTEN_DESCRIPTION.input, "linear, RGB, display-referred");
    assert_eq!(SOFTEN_DESCRIPTION.process, "linear, RGB");
    assert_eq!(SOFTEN_DESCRIPTION.output, "linear, RGB, display-referred");
    assert_eq!(
        SOFTEN_OPERATION_ORDERS.map(|entry| (entry.table, entry.order.to_bits())),
        [
            ("legacy_order", 60.0_f32.to_bits()),
            ("v30_order", 66.0_f32.to_bits()),
            ("v50_order", 66.0_f32.to_bits()),
            ("v30_jpg_order", 66.0_f32.to_bits()),
            ("v50_jpg_order", 66.0_f32.to_bits()),
        ]
    );
    assert!(SOFTEN_ALLOW_TILING);
    assert_eq!(SOFTEN_GPU_PROGRAM, 9);
    assert_eq!(
        SOFTEN_GPU_KERNELS,
        [
            "soften_overexposed",
            "soften_hblur",
            "soften_vblur",
            "soften_mix"
        ]
    );
    assert!(!SOFTEN_GPU_EXECUTABLE);

    let capabilities = capabilities();
    assert!(capabilities.cpu);
    assert!(!capabilities.gpu);
    assert!(!capabilities.gtk);
    assert!(!capabilities.history_materialization);
    assert!(!capabilities.outer_blending_and_masks);
    assert!(!capabilities.production_routing);
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::Ported
            && entry.native_symbol == "dt_iop_soften_data_t"
            && entry.rust_symbol.contains("SoftenConfig")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::Ported
            && entry
                .native_symbol
                .contains("commit_params direct field copies")
            && entry.rust_symbol.contains("every finite parameter value")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::RustAdaptation
            && entry.native_symbol.contains("init_pipe calloc")
            && entry
                .rust_symbol
                .contains("owns initialized committed state")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::RustAdaptation
            && entry.native_symbol.contains("cleanup_pipe free")
            && entry.rust_symbol.contains("automatic Drop")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::RustAdaptation
            && entry.native_symbol.contains("non-finite payload bits")
            && entry.rust_symbol.contains("fail-closed")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::RustAdaptation
            && entry.native_symbol.contains("negative int radius")
            && entry.rust_symbol.contains("neighborhoods to zero")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::Ported
            && entry.native_file == "src/develop/imageop.c; src/iop/CMakeLists.txt"
            && entry.native_symbol.contains("without DEFAULT_VISIBLE")
            && entry.rust_symbol.contains("SOFTEN_DEFAULT_ENABLED")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::RustAdaptation
            && entry.native_file == "src/CMakeLists.txt; src/common/math.h"
            && entry.native_symbol.contains("__FAST_MATH__ dt_fast_hypotf")
            && entry.rust_symbol.contains("release-profile")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::Ported
            && entry.native_file == "src/common/math.h"
            && entry.native_symbol.contains("CLIP")
            && entry.rust_symbol == "clip_native"
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::ExistingDependency
            && entry.native_symbol.contains("dt_box_mean")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::RustAdaptation
            && entry
                .native_symbol
                .contains("radius-zero eight horizontal/vertical")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::ExplicitlyDeferred
            && entry.native_symbol.contains("process_cl")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::Ported
            && entry.native_symbol.contains("description")
            && entry.rust_symbol.contains("SOFTEN_DESCRIPTION")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::Ported
            && entry.native_file == "src/common/iop_order.c"
            && entry.rust_symbol == "SOFTEN_OPERATION_ORDERS"
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::Ported
            && entry.native_symbol.contains("dt_iop_copy_image_roi")
    }));
    assert!(SOFTEN_SOURCE_MAP.iter().any(|entry| {
        entry.status == SoftenPortStatus::ExplicitlyDeferred
            && entry.native_symbol.contains("trouble message")
    }));
}

#[test]
fn hsl_conversion_retains_native_float_double_promotion_points() {
    let plan =
        SoftenPlan::new(config(0.0, 100.0, 0.0, 100.0), dimensions(1, 1)).expect("one-pixel plan");
    let output = plan
        .execute(&[pixel(0.0, 0.2, 0.3, 0.75)])
        .expect("soften execution");
    let channels = output.pixels[0].channels();

    // Exact bits from colorspaces.h's mixed f32/double CPU expressions.
    assert_eq!(channels[0].to_bits(), 0x0000_0000);
    assert_eq!(channels[1].to_bits(), 0x3e4c_ccca);
    assert_eq!(channels[2].to_bits(), 0x3e99_999a);
    assert_eq!(channels[3].to_bits(), 0.0_f32.to_bits());
}

#[test]
fn zero_radius_runs_all_native_box_passes_for_signed_zero() {
    const NEGATIVE_ZERO_BITS: u32 = 0x8000_0000;
    const POSITIVE_ZERO_BITS: u32 = 0x0000_0000;

    let input = [pixel(
        f32::from_bits(NEGATIVE_ZERO_BITS),
        f32::from_bits(NEGATIVE_ZERO_BITS),
        f32::from_bits(NEGATIVE_ZERO_BITS),
        f32::from_bits(NEGATIVE_ZERO_BITS),
    )];
    assert_eq!(
        input[0].channels().map(f32::to_bits),
        [NEGATIVE_ZERO_BITS; SOFTEN_CHANNELS]
    );

    let plan = SoftenPlan::new(config(0.0, 100.0, 0.0, 100.0), dimensions(1, 1))
        .expect("zero-radius plan");
    assert_eq!(plan.radius(), 0);
    let output = plan
        .execute(&input)
        .expect("zero-radius signed-zero execution");

    // The first retained horizontal pass starts its sum at +0.0, adds -0.0,
    // and stores +0.0 / 1.0. Skipping all eight pass pairs leaves -0.0 here.
    assert_eq!(
        output.pixels[0].channels().map(f32::to_bits),
        [POSITIVE_ZERO_BITS; SOFTEN_CHANNELS]
    );
}

#[test]
fn non_default_hsl_controls_and_mix_match_independent_native_vector() {
    let plan = SoftenPlan::new(config(0.0, 40.0, -1.0, 75.0), dimensions(1, 1))
        .expect("one-pixel non-default plan");
    let output = plan
        .execute(&[pixel(0.2, 0.4, 0.8, 0.75)])
        .expect("non-default soften execution");

    // Independent retained-source oracle: `soften.c::process` gives saturation
    // 0.4, brightness 0.5, and amount 0.75. `colorspaces.h` converts the source
    // RGB to HSL, scales S and L, and converts it to (0.19, 0.23, 0.31, 0).
    // For this 1x1 raster the native radius is zero; each of the eight
    // `box_filters.cc::_box_mean` horizontal/vertical passes divides a one-hit
    // sum by one. `imagebuf.c::dt_iop_image_linear_blend` then produces these
    // source-ordered f32 bits. Ignoring saturation would instead begin with
    // 0x3e00_0000; ignoring brightness would begin with 0x3eab_851f.
    assert_eq!(
        output.pixels[0].channels().map(f32::to_bits),
        [0x3e45_1eb8, 0x3e8b_851f, 0x3edd_70a4, 0x3e40_0000]
    );
}

#[test]
fn near_black_saturation_uses_the_source_denominator_floor() {
    let plan =
        SoftenPlan::new(config(0.0, 100.0, 0.0, 100.0), dimensions(1, 1)).expect("one-pixel plan");
    let output = plan
        .execute(&[pixel(1.0e-6, 1.1e-6, 1.0e-6, 0.0)])
        .expect("soften execution");
    let actual = output.pixels[0].channels();
    let expected = [1.043_118_7e-6, 1.056_881_4e-6, 1.043_118_7e-6];
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 5.0e-13);
    }
}

#[test]
fn radius_and_tiling_keep_native_integer_truncation_order() {
    let promotion_boundary =
        SoftenPlan::new(config(57.0, 100.0, 0.0, 100.0), dimensions(5000, 1)).expect("radius plan");
    // Native double fmin/multiply truncates 28.999... to 28 here.
    assert_eq!(promotion_boundary.radius(), 28);

    // Retained Release builds select common/math.h's `__FAST_MATH__` branch.
    // Its source-ordered f32 products and sum produce 6499.9995 before the
    // 0.01f multiplier; unconditional `hypotf` rounds this boundary to 6500.
    let release_boundary = SoftenPlan::new(config(100.0, 100.0, 0.0, 100.0), dimensions(6499, 114))
        .expect("release dt_fast_hypotf boundary");
    assert_eq!(release_boundary.radius(), 64);
    assert_eq!(
        release_boundary.tiling().expect("boundary tiling").overlap,
        316
    );

    let capped_outside_slider =
        SoftenPlan::new(config(125.0, 100.0, 0.0, 100.0), dimensions(6499, 114))
            .expect("commit_params accepts size above the slider maximum");
    assert_eq!(capped_outside_slider.radius(), 64);
    assert_eq!(
        capped_outside_slider
            .tiling()
            .expect("capped boundary tiling")
            .overlap,
        316
    );

    let scaled = SoftenPlan::new_with_scale(
        config(100.0, 100.0, 0.0, 100.0),
        dimensions(1001, 801),
        0.25,
        0.5,
    )
    .expect("scaled plan");
    assert_eq!(scaled.radius(), 3);

    let radius_one = SoftenPlan::new(config(100.0, 100.0, 0.0, 100.0), dimensions(101, 1))
        .expect("radius-one plan");
    assert_eq!(radius_one.radius(), 1);
    let tiling = radius_one.tiling().expect("tiling values");
    assert_eq!(tiling.factor.to_bits(), 2.1_f32.to_bits());
    assert_eq!(tiling.factor_cl.to_bits(), 3.0_f32.to_bits());
    assert_eq!(tiling.maxbuf.to_bits(), 1.0_f32.to_bits());
    assert_eq!(tiling.overhead, 0);
    assert_eq!(tiling.overlap, 8);
    assert_eq!(tiling.align, 1);

    assert!(matches!(
        SoftenPlan::new_with_scale(config(50.0, 100.0, 0.0, 50.0), dimensions(10, 10), 0.0, 1.0,),
        Err(SoftenExecutionError::InvalidScale)
    ));
}

#[test]
fn edge_impulse_matches_hard_coded_eight_pass_shrinking_windows() {
    let dimensions = dimensions(101, 1);
    let plan =
        SoftenPlan::new(config(100.0, 100.0, 0.0, 100.0), dimensions).expect("radius-one plan");
    let mut input = vec![pixel(0.0, 0.0, 0.0, 0.0); 101];
    input[0] = pixel(1.0, 1.0, 1.0, 0.0);
    let output = plan.execute(&input).expect("soften edge impulse");

    // Independent source-equation vector: eight horizontal radius-one means,
    // each edge divided by only the samples intersecting the image. Height one
    // makes every vertical pass an exact identity.
    let expected = [
        0.223_802_94,
        0.199_551_57,
        0.149_593_71,
        0.093_435_645,
        0.047_849_033,
        0.019_528_273,
        0.006_058_527_6,
        0.001_295_534_2,
        0.000_152_415_8,
        0.0,
    ];
    for (index, expected) in expected.into_iter().enumerate() {
        let channels = output.pixels[index].channels();
        for actual in &channels[..3] {
            assert!((actual - expected).abs() < 2.0e-6, "pixel {index}");
        }
        assert_eq!(channels[3].to_bits(), 0.0_f32.to_bits());
    }
}

#[test]
fn final_cpu_blend_processes_the_zeroed_fourth_lane() {
    let plan =
        SoftenPlan::new(config(0.0, 100.0, 0.0, 50.0), dimensions(1, 1)).expect("blend plan");
    let output = plan
        .execute(&[pixel(0.2, 0.4, 0.8, 0.75)])
        .expect("soften blend");
    assert_eq!(output.pixels[0].fourth().to_bits(), 0.375_f32.to_bits());

    let identity =
        SoftenPlan::new(config(100.0, 0.0, 2.0, 0.0), dimensions(3, 1)).expect("zero-mix plan");
    let input = [
        pixel(0.2, 0.4, 0.8, 0.75),
        pixel(0.9, 0.1, 0.3, 0.25),
        pixel(0.0, 1.0, 0.5, 1.0),
    ];
    assert_eq!(identity.execute(&input).expect("zero mix").pixels, input);
}

#[test]
fn required_format_copy_through_preserves_three_channels_and_distinct_rois() {
    let plan = SoftenPlan::new(SoftenConfig::defaults(), dimensions(2, 2)).expect("plan");
    let roi_in = SoftenRoi::new(10, 20, 2, 2).expect("input ROI");
    let roi_out = SoftenRoi::new(9, 19, 4, 4).expect("output ROI");
    let input = [
        1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0,
    ];
    let output = plan
        .copy_through_unsupported_input(&input, 3, roi_in, roi_out)
        .expect("packed copy-through");

    assert!(output.input_format_problem);
    assert_eq!(output.channels, 3);
    assert_eq!(
        output.samples,
        [
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, // padded row
            0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 0.0, 0.0, 0.0, // first input row
            0.0, 0.0, 0.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 0.0, 0.0, 0.0, // second input row
            0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, // padded row
        ]
    );
}

#[test]
fn required_format_copy_through_uses_native_channel_min_and_fast_path() {
    let plan = SoftenPlan::new(SoftenConfig::defaults(), dimensions(2, 1)).expect("plan");
    let roi_in = SoftenRoi::new(0, 0, 2, 1).expect("input ROI");
    let shifted_equal_size = SoftenRoi::new(40, -20, 2, 1).expect("equal-sized output ROI");
    let one_channel = [f32::from_bits(0x7fc0_1234), -0.0];
    let copied = plan
        .copy_through_unsupported_input(&one_channel, 1, roi_in, shifted_equal_size)
        .expect("one-channel fast-path copy");
    assert_eq!(copied.channels, 1);
    assert_eq!(
        copied
            .samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>(),
        one_channel.map(f32::to_bits)
    );

    let one_pixel = SoftenRoi::new(0, 0, 1, 1).expect("one-pixel ROI");
    let more_than_required = plan
        .copy_through_unsupported_input(&[1.0, 2.0, 3.0, 4.0], 6, one_pixel, one_pixel)
        .expect("native min(channel, required) copy");
    assert_eq!(more_than_required.channels, 4);
    assert_eq!(more_than_required.samples, [1.0, 2.0, 3.0, 4.0]);

    assert_eq!(
        plan.copy_through_unsupported_input(&[], 0, one_pixel, one_pixel),
        Err(SoftenExecutionError::InvalidChannelCount { actual: 0 })
    );
    assert_eq!(
        plan.copy_through_unsupported_input(&[0.0; 4], 4, one_pixel, one_pixel),
        Err(SoftenExecutionError::RequiredFormatAlreadySatisfied)
    );
    assert_eq!(
        plan.copy_through_unsupported_input(&[0.0; 2], 3, one_pixel, one_pixel),
        Err(SoftenExecutionError::SampleCountMismatch {
            expected: 3,
            actual: 2,
        })
    );
}

#[test]
fn finite_shape_budget_and_dimension_failures_are_typed() {
    assert_eq!(
        SoftenDimensions::new(0, 1),
        Err(SoftenExecutionError::InvalidDimensions)
    );
    let small_dimensions = dimensions(2, 1);
    let plan = SoftenPlan::new(SoftenConfig::defaults(), small_dimensions).expect("plan");
    assert_eq!(
        plan.execute(&[pixel(0.1, 0.2, 0.3, 0.4)]),
        Err(SoftenExecutionError::DimensionsMismatch {
            expected: 2,
            actual: 1,
        })
    );
    assert_eq!(
        plan.execute(&[
            pixel(0.1, 0.2, f32::INFINITY, 0.4),
            pixel(0.5, 0.6, 0.7, 0.8),
        ]),
        Err(SoftenExecutionError::NonFiniteInput {
            pixel: 0,
            channel: 2,
        })
    );

    let budgeted = SoftenPlan::new_with_budget(
        SoftenConfig::defaults(),
        dimensions(101, 101),
        1.0,
        1.0,
        128,
    )
    .expect("plan construction allocates no raster");
    let budget_input = vec![pixel(0.1, 0.2, 0.3, 0.4); 101 * 101];
    assert!(matches!(
        budgeted.execute(&budget_input),
        Err(SoftenExecutionError::MemoryBudgetExceeded { budget: 128, .. })
    ));
}

#[test]
fn tileable_large_full_frame_budgets_only_the_executed_raster() {
    let descriptor = soften_descriptor();
    assert!(descriptor.flags.contains(OperationFlags::TILEABLE));

    // A full-frame allocation would require hundreds of gigabytes, but native
    // TILEABLE execution allocates only the expanded tile passed to `process`.
    let plan = SoftenPlan::new_with_budget(
        config(0.0, 100.0, 0.0, 100.0),
        dimensions(100_000, 100_000),
        1.0,
        1.0,
        64 * 1024,
    )
    .expect("full geometry controls radius without allocating the full frame");
    let tile = dimensions(8, 8);
    let input = vec![pixel(0.2, 0.4, 0.8, 0.75); 8 * 8];
    let output = plan
        .execute_raster(&input, tile)
        .expect("small expanded tile fits its allocation budget");
    assert_eq!(output.pixels.len(), 8 * 8);
}

#[test]
fn cancellation_inside_zero_radius_passes_never_publishes_a_partial_candidate() {
    let dimensions = dimensions(16, 16);
    let plan =
        SoftenPlan::new(config(0.0, 100.0, 0.33, 100.0), dimensions).expect("zero-radius plan");
    assert_eq!(plan.radius(), 0);
    let input = vec![pixel(0.2, 0.4, 0.8, 0.75); 16 * 16];
    let original = input.clone();
    let polls = Cell::new(0_u32);
    let result = plan.execute_with_cancel(&input, || {
        let next = polls.get() + 1;
        polls.set(next);
        next > 80
    });

    assert_eq!(result, Err(SoftenExecutionError::Cancelled));
    assert!(polls.get() > 80, "cancellation must reach radius-zero work");
    assert_eq!(input, original, "private work must not mutate the source");
}

#[test]
fn cancellation_inside_box_work_never_publishes_a_partial_candidate() {
    let dimensions = dimensions(101, 101);
    let plan =
        SoftenPlan::new(config(100.0, 100.0, 0.33, 100.0), dimensions).expect("cancellable plan");
    assert_eq!(plan.radius(), 1);
    let input = vec![pixel(0.2, 0.4, 0.8, 0.75); 101 * 101];
    let original = input.clone();
    let polls = Cell::new(0_u32);
    let result = plan.execute_with_cancel(&input, || {
        let next = polls.get() + 1;
        polls.set(next);
        next > 210
    });

    assert_eq!(result, Err(SoftenExecutionError::Cancelled));
    assert!(polls.get() > 210, "cancellation must reach box-filter work");
    assert_eq!(input, original, "private work must not mutate the source");
}
