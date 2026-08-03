#[path = "../src/operations/bloom/leaf.rs"]
mod bloom_leaf;

use std::cell::Cell;

use bloom_leaf::{
    BLOOM_BOX_ITERATIONS, BLOOM_COMPATIBILITY_ID, BLOOM_CPU_TILING_FACTOR_MILLI,
    BLOOM_DEFAULT_SIZE, BLOOM_DEFAULT_STRENGTH, BLOOM_DEFAULT_THRESHOLD,
    BLOOM_INTROSPECTION_VERSION, BLOOM_MAXIMUM_RADIUS, BLOOM_MIGRATION_EDGES,
    BLOOM_NATIVE_BOX_FILTER_MAX_VECT, BLOOM_NATIVE_TARGET_CACHELINE_BYTES,
    BLOOM_OPENCL_NUM_BUCKETS, BLOOM_OPENCL_PROGRAM, BLOOM_OPENCL_TILING_FACTOR_MILLI,
    BLOOM_OVERLAP_RADIUS_MULTIPLIER, BLOOM_PARAMETER_BYTES, BLOOM_RUST_ID, BLOOM_TILING_ALIGNMENT,
    BLOOM_TILING_MAXBUF_MILLI, BloomAllocationMode, BloomConfig, BloomError, BloomHistory,
    BloomMemoryBudget, BloomMigrationError, BloomParameterError, BloomParametersV1, BloomPlan,
    BloomPublication, bloom_descriptor, capabilities,
};
use rusttable_color::ColorEncoding;
use rusttable_processing::{
    RasterDimensions,
    common::box_filters::BOX_ITERATIONS,
    descriptor::{AlphaPolicy, OperationFlags, ParameterDefault, ParameterKind, RoiKind},
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("nonzero test dimensions")
}

const fn lab(lightness: f32, a: f32, b: f32, fourth: f32) -> [f32; 4] {
    [lightness, a, b, fourth]
}

#[test]
fn v1_codec_is_the_exact_three_float_native_declaration() {
    let expected = [
        0x00, 0x00, 0xa0, 0x41, // size = 20
        0x00, 0x00, 0xb4, 0x42, // threshold = 90
        0x00, 0x00, 0xc8, 0x41, // strength = 25
    ];
    let parameters = BloomParametersV1::defaults();

    assert_eq!(BLOOM_COMPATIBILITY_ID, "bloom");
    assert_eq!(BLOOM_RUST_ID, "rusttable.bloom");
    assert_eq!(BLOOM_INTROSPECTION_VERSION, 1);
    assert_eq!(BLOOM_PARAMETER_BYTES, 12);
    assert_eq!(BLOOM_PARAMETER_BYTES, 3 * std::mem::size_of::<f32>());
    assert_eq!(parameters.to_bytes(), expected);
    assert_eq!(BloomParametersV1::from_bytes(&expected), Ok(parameters));
    assert!(BloomParametersV1::from_bytes(&expected[..11]).is_err());
    assert!(BloomParametersV1::from_bytes(&[0; 13]).is_err());
}

#[test]
fn signed_zero_round_trips_through_every_native_parameter_and_config_field() {
    let positive_zero = 0.0_f32.to_bits();
    let negative_zero = (-0.0_f32).to_bits();

    for negative_field in 0..3 {
        let mut source_bits = [positive_zero; 3];
        source_bits[negative_field] = negative_zero;
        let parameters = BloomParametersV1::new(
            f32::from_bits(source_bits[0]),
            f32::from_bits(source_bits[1]),
            f32::from_bits(source_bits[2]),
        );
        let payload = parameters.to_bytes();
        let decoded = BloomParametersV1::from_bytes(&payload).expect("signed-zero ABI payload");
        assert_eq!(
            [
                decoded.size.to_bits(),
                decoded.threshold.to_bits(),
                decoded.strength.to_bits(),
            ],
            source_bits
        );
        assert_eq!(
            BloomHistory::decode(BLOOM_INTROSPECTION_VERSION, &payload)
                .expect("signed-zero history")
                .payload(),
            payload
        );

        let config = BloomConfig::try_from(decoded).expect("signed zero is inside native bounds");
        assert_eq!(
            [
                config.size().to_bits(),
                config.threshold().to_bits(),
                config.strength().to_bits(),
            ],
            source_bits
        );
        let round_trip = config.parameters();
        assert_eq!(
            [
                round_trip.size.to_bits(),
                round_trip.threshold.to_bits(),
                round_trip.strength.to_bits(),
            ],
            source_bits
        );
    }
}

#[test]
fn codec_preserves_nonfinite_bits_before_executable_validation() {
    let source_bits = [0xc0a0_0000_u32, 0x7fc0_1234, 0x42cb_0000];
    let mut payload = [0_u8; BLOOM_PARAMETER_BYTES];
    for (field, bits) in source_bits.into_iter().enumerate() {
        payload[field * 4..field * 4 + 4].copy_from_slice(&bits.to_le_bytes());
    }

    let decoded = BloomParametersV1::from_bytes(&payload).expect("typed v1 payload");
    assert_eq!(decoded.size.to_bits(), source_bits[0]);
    assert_eq!(decoded.threshold.to_bits(), source_bits[1]);
    assert_eq!(decoded.strength.to_bits(), source_bits[2]);
    assert_eq!(decoded.to_bytes(), payload);
    assert_eq!(
        BloomHistory::decode(1, &payload)
            .expect("known history stays typed")
            .payload(),
        payload
    );
    assert_eq!(
        BloomConfig::try_from(decoded),
        Err(BloomParameterError::NonFinite("threshold"))
    );
}

#[test]
fn migration_is_v1_identity_and_unknown_histories_remain_opaque() {
    let current = BloomParametersV1::new(12.5, 67.0, 31.25);
    let history = BloomHistory::decode(1, &current.to_bytes()).expect("v1 history");
    assert_eq!(BLOOM_MIGRATION_EDGES, &[]);
    assert_eq!(history.version(), 1);
    assert_eq!(history.migrate_to_v1(), Ok(current));

    let future_bytes = vec![0xff, 0x00, 0x7a, 0x19, 0x44];
    let future = BloomHistory::decode(9, &future_bytes).expect("opaque future history");
    assert_eq!(future.version(), 9);
    assert_eq!(future.payload(), future_bytes);
    assert_eq!(
        future.migrate_to_v1(),
        Err(BloomMigrationError::OpaqueVersion(9))
    );
    assert!(matches!(
        BloomPlan::from_history(&future, dimensions(1, 1)),
        Err(BloomError::OpaqueHistory(9))
    ));
}

#[test]
fn commit_domain_accepts_every_finite_f32_independent_of_ui_ranges() {
    assert_eq!(BloomConfig::defaults().size().to_bits(), 20.0_f32.to_bits());
    assert_eq!(
        BloomConfig::defaults().threshold().to_bits(),
        90.0_f32.to_bits()
    );
    assert_eq!(
        BloomConfig::defaults().strength().to_bits(),
        25.0_f32.to_bits()
    );

    let finite_values = [
        f32::MIN,
        -101.0,
        -2.0,
        -1.0,
        -f32::EPSILON,
        -0.0,
        0.0,
        f32::from_bits(1),
        100.0,
        f32::from_bits(100.0_f32.to_bits() + 1),
        101.0,
        f32::MAX,
    ];
    for value in finite_values {
        let size = BloomConfig::new(value, 90.0, 25.0).expect("finite committed size");
        assert_eq!(size.size().to_bits(), value.to_bits());
        let threshold = BloomConfig::new(20.0, value, 25.0).expect("finite committed threshold");
        assert_eq!(threshold.threshold().to_bits(), value.to_bits());
        let strength = BloomConfig::new(20.0, 90.0, value).expect("finite committed strength");
        assert_eq!(strength.strength().to_bits(), value.to_bits());
    }

    for (parameters, expected) in [
        (
            BloomParametersV1::new(f32::NAN, 90.0, 25.0),
            BloomParameterError::NonFinite("size"),
        ),
        (
            BloomParametersV1::new(20.0, f32::INFINITY, 25.0),
            BloomParameterError::NonFinite("threshold"),
        ),
        (
            BloomParametersV1::new(20.0, 90.0, f32::NEG_INFINITY),
            BloomParameterError::NonFinite("strength"),
        ),
    ] {
        assert_eq!(BloomConfig::try_from(parameters), Err(expected));
        assert_eq!(
            BloomParametersV1::from_bytes(&parameters.to_bytes())
                .expect("history representation remains lossless")
                .to_bytes(),
            parameters.to_bytes()
        );
    }
}

#[test]
fn local_descriptor_preserves_source_order_defaults_precision_and_truthful_capabilities() {
    let descriptor = bloom_descriptor();
    descriptor.validate().expect("bounded bloom descriptor");

    assert_eq!(descriptor.id.compatibility_name, BLOOM_COMPATIBILITY_ID);
    assert_eq!(descriptor.id.rust_id, BLOOM_RUST_ID);
    assert_eq!(descriptor.id.parameter_version, 1);
    assert_eq!(descriptor.stage, "display-referred-lab");
    assert_eq!(descriptor.roi, RoiKind::FullImage);
    assert_eq!(
        descriptor
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect::<Vec<_>>(),
        ["size", "threshold", "strength"]
    );
    // `src/develop/imageop_gui.c::dt_bauhaus_slider_from_params` computes
    // `MAX(2, -floorf(log10f(top/100)+.1))`, which is 2 for every Bloom
    // 0..100 range; `src/bauhaus/bauhaus.c` stores those digits unchanged.
    assert_eq!(
        descriptor
            .parameters
            .iter()
            .map(|parameter| parameter.precision)
            .collect::<Vec<_>>(),
        [2, 2, 2]
    );
    for (parameter, default) in descriptor.parameters.iter().zip([
        BLOOM_DEFAULT_SIZE,
        BLOOM_DEFAULT_THRESHOLD,
        BLOOM_DEFAULT_STRENGTH,
    ]) {
        assert_eq!(
            parameter.kind,
            ParameterKind::Scalar {
                minimum: 0.0,
                maximum: 100.0,
            }
        );
        assert_eq!(
            parameter.default,
            ParameterDefault::Scalar(f64::from(default))
        );
        assert_eq!(parameter.unit.as_deref(), Some("%"));
    }
    assert_eq!(descriptor.io.input.channels, 4);
    assert_eq!(descriptor.io.input.encodings, vec![ColorEncoding::LabD50]);
    assert_eq!(descriptor.io.output.alpha, AlphaPolicy::Preserve);
    assert!(descriptor.flags.contains(OperationFlags::STYLE_ELIGIBLE));
    assert!(descriptor.flags.contains(OperationFlags::BLENDING));
    assert!(descriptor.flags.contains(OperationFlags::DETERMINISTIC_CPU));
    assert!(!descriptor.flags.contains(OperationFlags::TILEABLE));
    assert_eq!(descriptor.capability.gpu_tier, None);
    assert!(!descriptor.capability.fallback_to_cpu);
    assert!(!descriptor.mask_blend.consumes_mask);
    assert!(descriptor.ui.is_none());
    assert_eq!(descriptor.migration.source_versions, vec![1]);
    assert!(descriptor.migration.opaque_unknown_allowed);

    let capability = capabilities();
    assert!(capability.cpu);
    assert!(capability.typed_history);
    assert!(capability.scaled_roi_radius);
    assert!(capability.allocation_copy_through);
    assert!(!capability.gpu);
    assert!(!capability.tiling_publication);
    assert!(!capability.masks_and_blending);
    assert!(!capability.production_routing);
    assert!(!capability.ui);
}

#[test]
fn retained_cpu_opencl_and_tiling_constants_are_explicit_without_gpu_claims() {
    assert_eq!(BLOOM_BOX_ITERATIONS, 8);
    assert_eq!(BLOOM_BOX_ITERATIONS, BOX_ITERATIONS);
    assert_eq!(BLOOM_MAXIMUM_RADIUS, 256);
    assert_eq!(BLOOM_OVERLAP_RADIUS_MULTIPLIER, 5);
    assert_eq!(BLOOM_OPENCL_NUM_BUCKETS, 4);
    assert_eq!(BLOOM_OPENCL_PROGRAM, 12);
    assert_eq!(BLOOM_CPU_TILING_FACTOR_MILLI, 2_300);
    assert_eq!(BLOOM_OPENCL_TILING_FACTOR_MILLI, 3_000);
    assert_eq!(BLOOM_TILING_MAXBUF_MILLI, 1_000);
    assert_eq!(BLOOM_TILING_ALIGNMENT, 1);
    assert_eq!(BLOOM_NATIVE_BOX_FILTER_MAX_VECT, 16);
    assert_eq!(BLOOM_NATIVE_TARGET_CACHELINE_BYTES, 128);
}

#[test]
fn radius_keeps_native_truncate_then_scaled_ceil_then_cap_order() {
    for (size, expected) in [
        (0.0, 2),
        (0.5, 3),
        (20.0, 53),
        (50.0, 130),
        (98.0, 253),
        (99.0, 256),
        (100.0, 256),
    ] {
        let plan = BloomPlan::new(
            BloomConfig::new(size, 90.0, 25.0).expect("bounded config"),
            dimensions(1, 1),
        )
        .expect("scale-one plan");
        assert_eq!(plan.radius(), expected, "size {size}");
        assert_eq!(plan.overlap_pixels(), 5 * expected);
    }

    let config = BloomConfig::new(20.0, 90.0, 25.0).expect("config");
    assert_eq!(
        BloomPlan::new_with_scale(config, dimensions(1, 1), 0.5, 1.0)
            .expect("half-scale plan")
            .radius(),
        27
    );
    assert_eq!(
        BloomPlan::new_with_scale(config, dimensions(1, 1), 2.0, 1.0)
            .expect("double-scale plan")
            .radius(),
        106
    );
    assert_eq!(
        BloomPlan::new_with_scale(config, dimensions(1, 1), 10.0, 1.0)
            .expect("capped plan")
            .radius(),
        256
    );

    // `size + 1.0f` is rounded as float before `fmin` promotes it to
    // double. Promoting the addition itself would produce base radius 2.
    let float_addition_boundary = f32::from_bits(0x3e2f_fffc);
    assert_eq!(
        BloomPlan::new(
            BloomConfig::new(float_addition_boundary, 90.0, 25.0).expect("boundary config"),
            dimensions(1, 1),
        )
        .expect("float-addition boundary plan")
        .radius(),
        3
    );

    // Base radius 3 multiplied by this float rounds to exactly 1.0f before
    // `ceilf`. Keeping the multiplication in double would instead ceil to 2.
    let float_radius_scale = f32::from_bits(0x3eaa_aaab);
    assert_eq!(
        BloomPlan::new_with_scale(
            BloomConfig::new(0.5, 90.0, 25.0).expect("three-pixel base radius"),
            dimensions(1, 1),
            float_radius_scale,
            1.0,
        )
        .expect("float radius multiplication plan")
        .radius(),
        1
    );

    assert!(BloomPlan::new_with_scale(config, dimensions(1, 1), 0.0, 1.0).is_err());
    assert!(BloomPlan::new_with_scale(config, dimensions(1, 1), 1.0, f32::NAN).is_err());
}

#[test]
fn size_negative_one_has_the_native_zero_radius_semantics() {
    let plan = BloomPlan::new(
        BloomConfig::new(-1.0, 1_000.0, 25.0).expect("finite out-of-range config"),
        dimensions(2, 1),
    )
    .expect("size -1 plan");
    assert_eq!(plan.radius(), 0);
    assert_eq!(plan.overlap_pixels(), 0);
    assert_eq!(plan.native_box_scratch_samples_per_worker(), 16);
    assert_eq!(
        plan.native_box_scratch_requested_bytes_per_worker(),
        BLOOM_NATIVE_TARGET_CACHELINE_BYTES
    );
    assert_eq!(plan.sequential_box_scratch_bytes(), 0);

    let input = [lab(25.0, -0.0, 3.0, -0.0), lab(75.0, 4.0, -5.0, 0.5)];
    assert_eq!(
        plan.execute(&input).expect("radius-zero execution"),
        input,
        "the uncapped threshold blocks glow while radius zero remains executable"
    );
}

#[test]
fn size_negative_two_rejects_the_invalid_signed_box_filter_radius() {
    let payload = [
        0x00, 0x00, 0x00, 0xc0, // size = -2
        0x00, 0x00, 0xb4, 0x42, // threshold = 90
        0x00, 0x00, 0xc8, 0x41, // strength = 25
    ];
    let parameters = BloomParametersV1::from_bytes(&payload).expect("finite v1 payload");
    assert_eq!(parameters.size.to_bits(), 0xc000_0000);
    assert_eq!(
        parameters.to_bytes(),
        payload,
        "the history codec must preserve the committed finite bytes"
    );
    let config = BloomConfig::try_from(parameters).expect("finite committed size");
    assert_eq!(config.size().to_bits(), (-2.0_f32).to_bits());
    assert_eq!(
        BloomPlan::new(config, dimensions(1, 1)),
        Err(BloomError::InvalidBoxFilterRadius)
    );
}

#[test]
fn plan_preflights_private_output_lightness_and_box_scratch_memory() {
    let config = BloomConfig::new(20.0, 90.0, 25.0).expect("config");
    let plan = BloomPlan::new(config, dimensions(32, 16)).expect("default budget");
    assert!(plan.required_memory_bytes() > 32 * 16 * 20);

    let error = BloomPlan::new_with_scale_and_budget(
        config,
        dimensions(32, 16),
        1.0,
        1.0,
        BloomMemoryBudget::new(plan.required_memory_bytes() - 1),
    )
    .expect_err("one byte short must fail closed");
    assert_eq!(
        error,
        BloomError::MemoryBudgetExceeded {
            required: plan.required_memory_bytes(),
            budget: plan.required_memory_bytes() - 1,
        }
    );
}

#[test]
fn tall_narrow_budget_uses_the_retained_per_thread_scratch_height_term() {
    const WIDTH: usize = 1;
    const HEIGHT: usize = 1_024;
    const EFFECTIVE_HEIGHT: usize = 8;
    const NATIVE_SCRATCH_SAMPLES: usize = HEIGHT;
    const NATIVE_SCRATCH_BYTES: usize = NATIVE_SCRATCH_SAMPLES * std::mem::size_of::<f32>();
    const RUST_SEQUENTIAL_SCRATCH_SAMPLES: usize =
        BLOOM_NATIVE_BOX_FILTER_MAX_VECT * EFFECTIVE_HEIGHT;
    const RUST_SEQUENTIAL_LOWER_BUDGET: usize = WIDTH * HEIGHT * std::mem::size_of::<[f32; 4]>()
        + WIDTH * HEIGHT * std::mem::size_of::<f32>()
        + RUST_SEQUENTIAL_SCRATCH_SAMPLES * std::mem::size_of::<f32>();
    const SOURCE_REQUIRED_BYTES: usize = WIDTH * HEIGHT * std::mem::size_of::<[f32; 4]>()
        + WIDTH * HEIGHT * std::mem::size_of::<f32>()
        + NATIVE_SCRATCH_BYTES;

    // size 0 gives radius 2, so `_compute_effective_height` maps window 5 to
    // 8. `_alloc_scratch_space` then takes max(1, 1024, 16 * 8) floats: the
    // otherwise-missing native height term is the discriminating requirement.
    let config = BloomConfig::new(0.0, 90.0, 25.0).expect("config");
    let plan = BloomPlan::new(config, dimensions(1, 1_024)).expect("plan");
    assert_eq!(plan.radius(), 2);
    assert_eq!(
        plan.native_box_scratch_samples_per_worker(),
        NATIVE_SCRATCH_SAMPLES
    );
    assert_eq!(
        plan.native_box_scratch_requested_bytes_per_worker(),
        NATIVE_SCRATCH_BYTES
    );
    assert_eq!(
        plan.sequential_box_scratch_bytes(),
        RUST_SEQUENTIAL_SCRATCH_SAMPLES * std::mem::size_of::<f32>()
    );
    assert_eq!(plan.required_memory_bytes(), 24_576);
    assert_eq!(plan.required_memory_bytes(), SOURCE_REQUIRED_BYTES);

    BloomPlan::new_with_scale_and_budget(
        config,
        dimensions(1, 1_024),
        1.0,
        1.0,
        BloomMemoryBudget::new(SOURCE_REQUIRED_BYTES),
    )
    .expect("the exact source-derived budget is sufficient");
    assert_eq!(RUST_SEQUENTIAL_LOWER_BUDGET, 20_992);
    assert_eq!(
        BloomPlan::new_with_scale_and_budget(
            config,
            dimensions(1, 1_024),
            1.0,
            1.0,
            BloomMemoryBudget::new(RUST_SEQUENTIAL_LOWER_BUDGET),
        ),
        Err(BloomError::MemoryBudgetExceeded {
            required: SOURCE_REQUIRED_BYTES,
            budget: RUST_SEQUENTIAL_LOWER_BUDGET,
        })
    );
}

#[test]
fn cacheline_rounding_sets_the_exact_width_33_native_budget_boundary() {
    const WIDTH: usize = 33;
    const HEIGHT: usize = 2;
    const PIXELS: usize = WIDTH * HEIGHT;
    const EFFECTIVE_HEIGHT: usize = 2;
    const NATIVE_SCRATCH_SAMPLES: usize = WIDTH;
    const NATIVE_UNPADDED_SCRATCH_BYTES: usize =
        NATIVE_SCRATCH_SAMPLES * std::mem::size_of::<f32>();
    const NATIVE_REQUESTED_SCRATCH_BYTES: usize = 2 * BLOOM_NATIVE_TARGET_CACHELINE_BYTES;
    const SEQUENTIAL_SCRATCH_SAMPLES: usize = 33;
    const SEQUENTIAL_SCRATCH_BYTES: usize = SEQUENTIAL_SCRATCH_SAMPLES * std::mem::size_of::<f32>();
    const SEQUENTIAL_ADAPTATION_BYTES: usize = PIXELS * std::mem::size_of::<[f32; 4]>()
        + PIXELS * std::mem::size_of::<f32>()
        + SEQUENTIAL_SCRATCH_BYTES;
    const REQUIRED_BYTES: usize = PIXELS * std::mem::size_of::<[f32; 4]>()
        + PIXELS * std::mem::size_of::<f32>()
        + NATIVE_REQUESTED_SCRATCH_BYTES;

    // size 0 produces radius 2. The retained footprint is max(1*33, 2,
    // 16*2)=33 floats, or 132 bytes, and dt_alloc_perthread rounds that request
    // to two 128-byte Apple ARM cache lines. The sequential Rust helper remains
    // an explicitly separate unpadded 132-byte adaptation.
    let config = BloomConfig::new(0.0, 90.0, 25.0).expect("config");
    let plan = BloomPlan::new(config, dimensions(33, 2)).expect("plan");
    assert_eq!(plan.radius(), 2);
    assert_eq!(
        SEQUENTIAL_SCRATCH_SAMPLES,
        WIDTH.max(BLOOM_NATIVE_BOX_FILTER_MAX_VECT * EFFECTIVE_HEIGHT)
    );
    assert_eq!(NATIVE_UNPADDED_SCRATCH_BYTES, 132);
    assert_eq!(plan.native_box_scratch_samples_per_worker(), 33);
    assert_eq!(
        plan.native_box_scratch_requested_bytes_per_worker(),
        NATIVE_REQUESTED_SCRATCH_BYTES
    );
    assert_eq!(SEQUENTIAL_SCRATCH_BYTES, 132);
    assert_eq!(
        plan.sequential_box_scratch_bytes(),
        SEQUENTIAL_SCRATCH_BYTES
    );
    assert_eq!(REQUIRED_BYTES, 1_576);
    assert_eq!(plan.required_memory_bytes(), REQUIRED_BYTES);

    BloomPlan::new_with_scale_and_budget(
        config,
        dimensions(33, 2),
        1.0,
        1.0,
        BloomMemoryBudget::new(REQUIRED_BYTES),
    )
    .expect("exact cache-line-rounded budget");
    assert_eq!(
        BloomPlan::new_with_scale_and_budget(
            config,
            dimensions(33, 2),
            1.0,
            1.0,
            BloomMemoryBudget::new(REQUIRED_BYTES - 1),
        ),
        Err(BloomError::MemoryBudgetExceeded {
            required: REQUIRED_BYTES,
            budget: REQUIRED_BYTES - 1,
        })
    );
    assert_eq!(SEQUENTIAL_ADAPTATION_BYTES, 1_452);
    assert_eq!(
        BloomPlan::new_with_scale_and_budget(
            config,
            dimensions(33, 2),
            1.0,
            1.0,
            BloomMemoryBudget::new(SEQUENTIAL_ADAPTATION_BYTES),
        ),
        Err(BloomError::MemoryBudgetExceeded {
            required: REQUIRED_BYTES,
            budget: SEQUENTIAL_ADAPTATION_BYTES,
        })
    );
}

#[test]
fn source_vector_matches_threshold_eight_box_means_and_release_screen_equation() {
    let plan = BloomPlan::new(
        BloomConfig::new(0.0, 50.0, 25.0).expect("config"),
        dimensions(2, 2),
    )
    .expect("plan");
    let input = [
        lab(10.0, 1.0, -2.0, 0.1),
        lab(40.0, 3.0, -4.0, 0.2),
        lab(80.0, 5.0, -6.0, 0.3),
        lab(100.0, 7.0, -8.0, 0.4),
    ];
    let output = plan.execute(&input).expect("bloom CPU leaf");
    // bloom.c scalar boundaries, eight H/V shrinking-edge means, and its
    // Release-contracted screen expression. These hard-coded bits also pin
    // every copied channel rather than allowing a tolerance-only match.
    let expected_bits = [
        [0x4269_fdd6, 0x3f80_0000, 0xc000_0000, 0x3dcc_cccd],
        [0x4290_a9f2, 0x4040_0000, 0xc080_0000, 0x3e4c_cccd],
        [0x42b5_8dfb, 0x40a0_0000, 0xc0c0_0000, 0x3e99_999a],
        [0x42c8_0000, 0x40e0_0000, 0xc100_0000, 0x3ecc_cccd],
    ];
    assert_eq!(
        output
            .iter()
            .map(|pixel| pixel.map(f32::to_bits))
            .collect::<Vec<_>>(),
        expected_bits
    );
}

#[test]
fn eight_iteration_impulse_fixture_distinguishes_the_complete_blur_chain() {
    let plan = BloomPlan::new(
        BloomConfig::new(0.0, 0.0, 25.0).expect("config"),
        dimensions(9, 1),
    )
    .expect("plan");
    let mut input = vec![lab(0.0, 12.0, -8.0, 0.25); 9];
    input[4] = lab(100.0, -24.0, 32.0, 0.75);
    let output = plan.execute(&input).expect("eight-pass bloom");
    let ordinary_channels = [0x4140_0000, 0xc100_0000, 0x3e80_0000];
    let expected_bits = [
        [
            0x4173_c928,
            ordinary_channels[0],
            ordinary_channels[1],
            ordinary_channels[2],
        ],
        [
            0x4174_d1a8,
            ordinary_channels[0],
            ordinary_channels[1],
            ordinary_channels[2],
        ],
        [
            0x4175_8f20,
            ordinary_channels[0],
            ordinary_channels[1],
            ordinary_channels[2],
        ],
        [
            0x4176_c3a8,
            ordinary_channels[0],
            ordinary_channels[1],
            ordinary_channels[2],
        ],
        [0x42c8_0000, 0xc1c0_0000, 0x4200_0000, 0x3f40_0000],
        [
            0x4176_c3a8,
            ordinary_channels[0],
            ordinary_channels[1],
            ordinary_channels[2],
        ],
        [
            0x4175_8f20,
            ordinary_channels[0],
            ordinary_channels[1],
            ordinary_channels[2],
        ],
        [
            0x4174_d1a0,
            ordinary_channels[0],
            ordinary_channels[1],
            ordinary_channels[2],
        ],
        [
            0x4173_c928,
            ordinary_channels[0],
            ordinary_channels[1],
            ordinary_channels[2],
        ],
    ];
    assert_eq!(
        output
            .iter()
            .map(|pixel| pixel.map(f32::to_bits))
            .collect::<Vec<_>>(),
        expected_bits
    );
}

#[test]
fn selected_macos_release_strength_and_fused_mix_discriminators_are_bit_exact() {
    let strength_boundary = BloomPlan::new(
        BloomConfig::new(0.0, 0.0, 0.0).expect("config"),
        dimensions(1, 1),
    )
    .expect("plan");

    // packaging/macosx/2_build_hb_darktable_default.sh overrides build.sh's
    // RelWithDebInfo default with Release. src/CMakeLists.txt then appends
    // `-O3`, the `-f` + `fast-math` enable option, then
    // `-fno-finite-math-only`, in that order. Apple Clang leaves
    // `__FAST_MATH__` undefined, while the selected compiler output still folds
    // reciprocal exp2 to produce 0x3f80_e3eb rather than the strict
    // reciprocal-after-exp2 result 0x3f80_e3ec.
    assert_eq!(strength_boundary.lightness_scale().to_bits(), 0x3f80_e3eb);
    let signed_zero_input = [lab(37.25, -0.0, 0.0, -0.0)];
    let signed_zero_output = strength_boundary
        .execute(&signed_zero_input)
        .expect("selected-profile scalar fixture");
    assert_eq!(
        signed_zero_output[0].map(f32::to_bits),
        [0x4273_25d8, 0x8000_0000, 0x0000_0000, 0x8000_0000]
    );

    // This bounded one-pixel fixture reaches blurred L=0x4282_6656. Selected
    // Apple ARM Release compiler output contracts the screen expression into
    // two ordered FMAs, yielding 0x42ad_2ec0 instead of the strict source-rounding
    // result 0x42ad_2ec1. That compiler-output contraction is the leaf's explicit
    // FMA policy; it is not evidence that `__FAST_MATH__` was defined.
    let mix_plan = BloomPlan::new(
        BloomConfig::new(0.0, 0.0, 7.5).expect("mix config"),
        dimensions(1, 1),
    )
    .expect("mix plan");
    assert_eq!(mix_plan.lightness_scale().to_bits(), 0x3f87_c49e);
    let mix_input = [lab(f32::from_bits(0x4275_e0b2), 3.0, -4.0, 0.25)];
    let mix_output = mix_plan.execute(&mix_input).expect("fused mix fixture");
    assert_eq!(
        mix_output[0].map(f32::to_bits),
        [0x42ad_2ec0, 0x4040_0000, 0xc080_0000, 0x3e80_0000]
    );
}

#[test]
fn native_upper_caps_do_not_clamp_the_committed_threshold() {
    let input = [lab(60.0, 36.0, -22.0, 0.4)];
    let equality = BloomPlan::new(
        BloomConfig::new(10_000.0, 120.0, 10_000.0).expect("finite out-of-range config"),
        dimensions(1, 1),
    )
    .expect("upper-cap plan");

    assert_eq!(equality.radius(), BLOOM_MAXIMUM_RADIUS);
    assert_eq!(equality.lightness_scale().to_bits(), 2.0_f32.to_bits());
    assert_eq!(equality.config().threshold().to_bits(), 120.0_f32.to_bits());
    // Scaled L is exactly 120. The retained threshold is not UI-clamped to 100,
    // and its comparison is strict `L > threshold`, so equality stores zero.
    assert_eq!(equality.execute(&input).expect("strict threshold"), input);

    let below = BloomPlan::new(
        BloomConfig::new(10_000.0, f32::from_bits(120.0_f32.to_bits() - 1), 10_000.0)
            .expect("uncapped threshold"),
        dimensions(1, 1),
    )
    .expect("below-threshold plan");
    assert_ne!(below.execute(&input).expect("glow above threshold"), input);
}

#[test]
fn primary_lightness_allocation_failure_copies_input_through() {
    let plan = BloomPlan::new(
        BloomConfig::new(0.0, 0.0, 25.0).expect("config"),
        dimensions(3, 1),
    )
    .expect("plan");
    let input = [
        lab(10.0, 1.0, 2.0, 0.1),
        lab(80.0, 3.0, 4.0, 0.2),
        lab(20.0, 5.0, 6.0, 0.3),
    ];
    let mut output = [lab(-1.0, -1.0, -1.0, -1.0); 3];

    let publication = plan
        .execute_into_with_cancel_and_allocation_mode(
            &input,
            &mut output,
            BloomAllocationMode::FailLightnessBuffer,
            || false,
        )
        .expect("native copy-through");
    assert_eq!(publication, BloomPublication::CopiedInput);
    assert_eq!(output, input);
}

#[test]
fn box_scratch_failure_uses_unblurred_threshold_buffer_before_screen_mix() {
    let plan = BloomPlan::new(
        BloomConfig::new(0.0, 50.0, 25.0).expect("config"),
        dimensions(3, 1),
    )
    .expect("plan");
    let input = [
        lab(10.0, 11.0, -12.0, 0.1),
        lab(80.0, 21.0, -22.0, 0.2),
        lab(10.0, 31.0, -32.0, 0.3),
    ];
    let mut output = [[0.0; 4]; 3];

    assert_eq!(
        plan.execute_into_with_cancel_and_allocation_mode(
            &input,
            &mut output,
            BloomAllocationMode::FailBoxFilterScratch,
            || false,
        )
        .expect("source scratch-failure behavior"),
        BloomPublication::Filtered
    );
    assert_eq!(
        output.map(|pixel| pixel.map(f32::to_bits)),
        [
            [0x4120_0002, 0x4130_0000, 0xc140_0000, 0x3dcc_cccd],
            [0x42c6_51bf, 0x41a8_0000, 0xc1b0_0000, 0x3e4c_cccd],
            [0x4120_0002, 0x41f8_0000, 0xc200_0000, 0x3e99_999a],
        ]
    );

    let blurred = plan.execute(&input).expect("normal blur");
    assert_ne!(
        blurred
            .iter()
            .map(|pixel| pixel[0].to_bits())
            .collect::<Vec<_>>(),
        output
            .iter()
            .map(|pixel| pixel[0].to_bits())
            .collect::<Vec<_>>()
    );
}

#[test]
fn cancellation_after_work_starts_never_publishes_partial_output() {
    let plan = BloomPlan::new(
        BloomConfig::new(0.0, 10.0, 25.0).expect("config"),
        dimensions(16, 8),
    )
    .expect("plan");
    let input = (0..128)
        .map(|index| {
            let lightness =
                f32::from(u8::try_from(index % 101).expect("fixture lightness fits u8"));
            lab(lightness, 4.0, -7.0, 0.375)
        })
        .collect::<Vec<_>>();
    let sentinel = lab(-9.0, -8.0, -7.0, -6.0);
    let mut output = vec![sentinel; input.len()];
    let polls = Cell::new(0_u32);

    let error = plan
        .execute_into_with_cancel(&input, &mut output, || {
            let current = polls.get();
            polls.set(current + 1);
            current >= 25
        })
        .expect_err("deterministic cancellation");
    assert_eq!(error, BloomError::Cancelled);
    assert!(polls.get() > 25);
    assert_eq!(output, vec![sentinel; input.len()]);
}

#[test]
fn shape_and_finite_failures_leave_caller_destination_unchanged() {
    let plan = BloomPlan::new(BloomConfig::defaults(), dimensions(2, 1)).expect("plan");
    let input = [lab(20.0, 1.0, 2.0, 0.3), lab(40.0, 3.0, 4.0, 0.5)];
    let sentinel = lab(-1.0, -2.0, -3.0, -4.0);

    let mut short_output = [sentinel];
    assert!(matches!(
        plan.execute_into(&input, &mut short_output),
        Err(BloomError::DimensionsMismatch {
            buffer: "output",
            expected: 2,
            actual: 1,
        })
    ));
    assert_eq!(short_output, [sentinel]);

    let mut nonfinite_input = input;
    nonfinite_input[1][2] = f32::NAN;
    let mut output = [sentinel; 2];
    assert_eq!(
        plan.execute_into(&nonfinite_input, &mut output),
        Err(BloomError::NonFiniteInput {
            pixel: 1,
            channel: 2,
        })
    );
    assert_eq!(output, [sentinel; 2]);

    let overflowing = [lab(f32::MAX, 1.0, 2.0, 0.3), input[1]];
    assert_eq!(
        plan.execute_into(&overflowing, &mut output),
        Err(BloomError::NonFiniteIntermediate {
            stage: "threshold",
            pixel: 0,
        })
    );
    assert_eq!(output, [sentinel; 2]);
}
