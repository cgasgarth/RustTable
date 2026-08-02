#![allow(
    clippy::float_cmp,
    reason = "source-derived f32 vectors intentionally assert exact scalar results"
)]

#[path = "../src/operations/colorcontrast/leaf.rs"]
mod colorcontrast_leaf;

use std::collections::BTreeSet;
use std::mem::{align_of, size_of};

use colorcontrast_leaf::source_map::{COLOR_CONTRAST_SOURCE_MAP, ColorContrastPortStatus};
use colorcontrast_leaf::{
    COLOR_CONTRAST_COMPATIBILITY_ID, COLOR_CONTRAST_DEFAULT_A_OFFSET,
    COLOR_CONTRAST_DEFAULT_A_STEEPNESS, COLOR_CONTRAST_DEFAULT_B_OFFSET,
    COLOR_CONTRAST_DEFAULT_B_STEEPNESS, COLOR_CONTRAST_DEFAULT_UNBOUND, COLOR_CONTRAST_GPU_KERNEL,
    COLOR_CONTRAST_GPU_PROGRAM, COLOR_CONTRAST_MIGRATION_EDGES, COLOR_CONTRAST_NATIVE_ALIASES,
    COLOR_CONTRAST_NATIVE_ALIGNED_PIXEL_BYTES, COLOR_CONTRAST_NATIVE_APPLE_AARCH64_CACHELINE_BYTES,
    COLOR_CONTRAST_NATIVE_COLORSPACE, COLOR_CONTRAST_NATIVE_FLAGS, COLOR_CONTRAST_NATIVE_GROUPS,
    COLOR_CONTRAST_NATIVE_NAME, COLOR_CONTRAST_NATIVE_NO_VECTORIZATION_LANES,
    COLOR_CONTRAST_NATIVE_OTHER_CACHELINE_BYTES, COLOR_CONTRAST_NATIVE_STEEPNESS_SLIDER_PRECISION,
    COLOR_CONTRAST_NATIVE_TILING_ALIGNMENT_PIXELS, COLOR_CONTRAST_NATIVE_TILING_FACTOR,
    COLOR_CONTRAST_NATIVE_TILING_FACTOR_CL, COLOR_CONTRAST_NATIVE_TILING_MAXBUF,
    COLOR_CONTRAST_NATIVE_TILING_MAXBUF_CL, COLOR_CONTRAST_NATIVE_TILING_OVERHEAD_BYTES,
    COLOR_CONTRAST_NATIVE_TILING_OVERLAP_PIXELS, COLOR_CONTRAST_NATIVE_VECTORIZED_LANES,
    COLOR_CONTRAST_RUST_EXECUTE_INTO_MULTIPLIER_MILLI, COLOR_CONTRAST_RUST_EXECUTE_INTO_RASTERS,
    COLOR_CONTRAST_RUST_ID, COLOR_CONTRAST_RUST_INPUT_MULTIPLIER_MILLI, COLOR_CONTRAST_RUST_LANES,
    COLOR_CONTRAST_RUST_MINIMUM_TILE_EDGE, COLOR_CONTRAST_RUST_OUTPUT_MULTIPLIER_MILLI,
    COLOR_CONTRAST_RUST_PIXEL_TYPE_ALIGNMENT_BYTES, COLOR_CONTRAST_RUST_PREFERRED_TILE_EDGE,
    COLOR_CONTRAST_RUST_STAGING_MULTIPLIER_MILLI, COLOR_CONTRAST_SCHEMA_VERSION,
    COLOR_CONTRAST_V1_PARAMETER_BYTES, COLOR_CONTRAST_V2_PARAMETER_BYTES,
    ColorContrastCapabilityError, ColorContrastChannel, ColorContrastCodecError,
    ColorContrastConfig, ColorContrastExecutionError, ColorContrastHistory,
    ColorContrastLanePolicy, ColorContrastParameterError, ColorContrastParametersV1,
    ColorContrastParametersV2, ColorContrastPixel, ColorContrastPlan, capabilities,
    colorcontrast_descriptor, lane_policy, migrate_v1_to_v2,
};
use rusttable_color::ColorEncoding;
use rusttable_processing::RasterDimensions;
use rusttable_processing::descriptor::{
    AlphaPolicy, NonFinitePolicy, OperationFlags, ParameterDefault, ParameterKind, RoiKind,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("nonzero source-derived dimensions")
}

fn pixel_bits(pixel: ColorContrastPixel) -> [u32; 4] {
    pixel.channels().map(f32::to_bits)
}

#[test]
fn source_abi_defaults_and_checked_benchmark_payload_are_exact() {
    assert_eq!(COLOR_CONTRAST_SCHEMA_VERSION, 2);
    assert_eq!(
        size_of::<ColorContrastParametersV1>(),
        COLOR_CONTRAST_V1_PARAMETER_BYTES
    );
    assert_eq!(
        size_of::<ColorContrastParametersV2>(),
        COLOR_CONTRAST_V2_PARAMETER_BYTES
    );

    let v1 = ColorContrastParametersV1::new(1.0, -2.0, 0.5, 4.0);
    let v1_bytes = [
        0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x00, 0x00, 0x3f, 0x00, 0x00, 0x80,
        0x40,
    ];
    assert_eq!(v1.to_bytes(), v1_bytes);
    assert_eq!(ColorContrastParametersV1::from_bytes(&v1_bytes), Ok(v1));

    let defaults = ColorContrastParametersV2::defaults();
    assert_eq!(
        defaults,
        ColorContrastParametersV2::new(1.0, 0.0, 1.0, 0.0, 1)
    );
    let v2_default_bytes = [
        0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x00,
    ];
    assert_eq!(defaults.to_bytes(), v2_default_bytes);
    assert_eq!(
        ColorContrastParametersV2::from_bytes(&v2_default_bytes),
        Ok(defaults)
    );

    let signed_zero_v1_bytes = [
        0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00,
        0x80,
    ];
    let signed_zero_v1 = ColorContrastParametersV1::from_bytes(&signed_zero_v1_bytes)
        .expect("hard-coded native signed-zero v1 payload");
    assert_eq!(
        [
            signed_zero_v1.a_steepness.to_bits(),
            signed_zero_v1.a_offset.to_bits(),
            signed_zero_v1.b_steepness.to_bits(),
            signed_zero_v1.b_offset.to_bits(),
        ],
        [0x8000_0000; 4]
    );
    assert_eq!(signed_zero_v1.to_bytes(), signed_zero_v1_bytes);

    let signed_zero_v2_bytes = [
        0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00,
        0x80, 0x01, 0x00, 0x00, 0x00,
    ];
    let signed_zero_v2 = ColorContrastParametersV2::from_bytes(&signed_zero_v2_bytes)
        .expect("hard-coded native signed-zero v2 payload");
    assert_eq!(
        [
            signed_zero_v2.a_steepness.to_bits(),
            signed_zero_v2.a_offset.to_bits(),
            signed_zero_v2.b_steepness.to_bits(),
            signed_zero_v2.b_offset.to_bits(),
        ],
        [0x8000_0000; 4]
    );
    assert_eq!(signed_zero_v2.unbound, 1);
    assert_eq!(signed_zero_v2.to_bytes(), signed_zero_v2_bytes);

    // Hard-coded from src/tests/benchmark/darktable-bench-4.2.xmp:544.
    let benchmark_v2 = [
        0x52, 0xb8, 0x9e, 0x3f, 0x00, 0x00, 0x00, 0x00, 0x14, 0xae, 0x87, 0x3f, 0x00, 0x00, 0x00,
        0x00, 0x01, 0x00, 0x00, 0x00,
    ];
    let decoded = ColorContrastParametersV2::from_bytes(&benchmark_v2).expect("native v2");
    assert_eq!(decoded.a_steepness.to_bits(), 0x3f9e_b852);
    assert_eq!(decoded.a_offset.to_bits(), 0);
    assert_eq!(decoded.b_steepness.to_bits(), 0x3f87_ae14);
    assert_eq!(decoded.b_offset.to_bits(), 0);
    assert_eq!(decoded.unbound, 1);

    assert_eq!(
        ColorContrastParametersV1::from_bytes(&[0; 15]),
        Err(ColorContrastCodecError::InvalidLength {
            expected: 16,
            actual: 15
        })
    );
    assert_eq!(
        ColorContrastParametersV2::from_bytes(&[0; 21]),
        Err(ColorContrastCodecError::InvalidLength {
            expected: 20,
            actual: 21
        })
    );
}

#[test]
fn only_v1_migrates_directly_to_v2_and_future_history_stays_opaque() {
    assert_eq!(COLOR_CONTRAST_MIGRATION_EDGES, &[(1, 2)]);
    let v1 = ColorContrastParametersV1::new(
        f32::from_bits(0x3fa1_2345),
        f32::from_bits(0xc020_0001),
        f32::from_bits(0x3f12_3456),
        f32::from_bits(0x40a0_0001),
    );
    let migrated = migrate_v1_to_v2(v1);
    assert_eq!(migrated.a_steepness.to_bits(), v1.a_steepness.to_bits());
    assert_eq!(migrated.a_offset.to_bits(), v1.a_offset.to_bits());
    assert_eq!(migrated.b_steepness.to_bits(), v1.b_steepness.to_bits());
    assert_eq!(migrated.b_offset.to_bits(), v1.b_offset.to_bits());
    assert_eq!(migrated.unbound, 0);
    assert_eq!(
        ColorContrastHistory::decode(1, &v1.to_bytes())
            .expect("known v1")
            .current(),
        Ok(migrated)
    );

    let opaque_bytes = vec![0xde, 0xad, 0xbe, 0xef, 0x80];
    let opaque = ColorContrastHistory::decode(19, &opaque_bytes).expect("opaque future history");
    assert_eq!(opaque.version(), 19);
    assert_eq!(opaque.payload(), opaque_bytes);
    assert_eq!(
        opaque.current(),
        Err(ColorContrastCodecError::UnsupportedVersion(19))
    );

    let nan_payload =
        ColorContrastParametersV2::new(f32::from_bits(0x7fc1_2345), 0.0, 1.0, 0.0, -7);
    let decoded_nan = ColorContrastHistory::decode(2, &nan_payload.to_bytes())
        .expect("codec preserves native bits")
        .current()
        .expect("known v2");
    assert_eq!(
        decoded_nan.a_steepness.to_bits(),
        nan_payload.a_steepness.to_bits()
    );
    assert_eq!(decoded_nan.unbound, -7);
}

#[test]
fn local_descriptor_preserves_source_order_without_claiming_shared_surfaces() {
    assert_eq!(COLOR_CONTRAST_NATIVE_NAME, "color contrast");
    assert_eq!(COLOR_CONTRAST_NATIVE_ALIASES, "saturation");
    assert_eq!(COLOR_CONTRAST_NATIVE_COLORSPACE, "Lab");
    assert_eq!(COLOR_CONTRAST_NATIVE_GROUPS, ["color", "grading"]);
    assert_eq!(
        COLOR_CONTRAST_NATIVE_FLAGS,
        ["include-in-styles", "supports-blending", "allow-tiling"]
    );
    assert_eq!(COLOR_CONTRAST_GPU_PROGRAM, 8);
    assert_eq!(COLOR_CONTRAST_GPU_KERNEL, "colorcontrast");

    let descriptor = colorcontrast_descriptor();
    descriptor.validate().expect("operation-local descriptor");
    assert_eq!(
        descriptor.id.compatibility_name,
        COLOR_CONTRAST_COMPATIBILITY_ID
    );
    assert_eq!(descriptor.id.rust_id, COLOR_CONTRAST_RUST_ID);
    assert_eq!(descriptor.id.schema_version, 2);
    assert_eq!(descriptor.id.parameter_version, 2);
    assert_eq!(descriptor.stage, "display-referred-lab-d50");
    assert_eq!(descriptor.roi, RoiKind::Identity);
    assert_eq!(descriptor.tiling.overlap_pixels, 0);
    assert_eq!(descriptor.tiling.alignment_pixels, 1);
    assert_eq!(
        descriptor
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect::<Vec<_>>(),
        [
            "a_steepness",
            "a_offset",
            "b_steepness",
            "b_offset",
            "unbound"
        ]
    );
    assert!(matches!(
        descriptor.parameters[0].kind,
        ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 5.0
        }
    ));
    assert!(matches!(
        descriptor.parameters[2].kind,
        ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 5.0
        }
    ));
    // colorcontrast.c introspection supplies 0..5; imageop_gui.c's
    // dt_bauhaus_slider_from_params formula resolves that range to two digits.
    assert_eq!(COLOR_CONTRAST_NATIVE_STEEPNESS_SLIDER_PRECISION, 2);
    assert_eq!(descriptor.parameters[0].precision, 2);
    assert_eq!(descriptor.parameters[2].precision, 2);
    assert_eq!(descriptor.parameters[0].ui_hint.as_deref(), Some("slider"));
    assert_eq!(descriptor.parameters[2].ui_hint.as_deref(), Some("slider"));
    assert_eq!(descriptor.parameters[1].precision, 0);
    assert_eq!(descriptor.parameters[3].precision, 0);
    assert_eq!(descriptor.parameters[1].ui_hint, None);
    assert_eq!(descriptor.parameters[3].ui_hint, None);
    assert_eq!(
        descriptor.parameters[0].default,
        ParameterDefault::Scalar(1.0)
    );
    assert_eq!(
        descriptor.parameters[1].default,
        ParameterDefault::Scalar(0.0)
    );
    assert_eq!(
        descriptor.parameters[2].default,
        ParameterDefault::Scalar(1.0)
    );
    assert_eq!(
        descriptor.parameters[3].default,
        ParameterDefault::Scalar(0.0)
    );
    assert_eq!(
        descriptor.parameters[4].default,
        ParameterDefault::Integer(1)
    );
    assert_eq!(descriptor.parameters[4].introduced_version, 2);
    assert_eq!(descriptor.migration.source_versions, [1, 2]);
    assert_eq!(descriptor.migration.target_version, 2);
    assert!(descriptor.migration.opaque_unknown_allowed);

    for flag in [
        OperationFlags::MULTI_INSTANCE,
        OperationFlags::STYLE_ELIGIBLE,
        OperationFlags::HISTORY_VISIBLE,
        OperationFlags::TILEABLE,
        OperationFlags::DETERMINISTIC_CPU,
        OperationFlags::COLOR,
    ] {
        assert!(
            descriptor.flags.contains(flag),
            "missing local flag {flag:?}"
        );
    }
    for unsupported in [
        OperationFlags::DETERMINISTIC_GPU,
        OperationFlags::MASKS,
        OperationFlags::BLENDING,
    ] {
        assert!(!descriptor.flags.contains(unsupported));
    }
    assert!(descriptor.capability.cpu_supported);
    assert_eq!(descriptor.capability.gpu_tier, None);
    assert!(!descriptor.capability.deterministic_gpu);
    assert!(!descriptor.capability.fallback_to_cpu);
    assert_eq!(descriptor.io.input.channels, 4);
    assert_eq!(descriptor.io.input.alpha, AlphaPolicy::Preserve);
    assert_eq!(descriptor.io.input.nonfinite, NonFinitePolicy::Reject);
    assert_eq!(descriptor.io.input.encodings, [ColorEncoding::LabD50]);
    assert_eq!(descriptor.io.output.encodings, [ColorEncoding::LabD50]);
    assert!(!descriptor.mask_blend.consumes_mask);
    assert!(!descriptor.mask_blend.blend_if);
    assert!(descriptor.ui.is_none());

    let capabilities = capabilities();
    assert!(capabilities.cpu_supported);
    assert!(!capabilities.gpu_supported);
    assert!(!capabilities.gtk_supported);
    assert!(!capabilities.masks_and_outer_blending_supported);
    assert!(!capabilities.native_required_format_image_buffer_supported);
    assert!(!capabilities.native_for_each_channel_policy_supported);
    assert!(capabilities.deterministic_four_lane_rust_adaptation);
    assert!(!capabilities.production_routing_supported);
    assert_eq!(COLOR_CONTRAST_NATIVE_NO_VECTORIZATION_LANES, 3);
    assert_eq!(COLOR_CONTRAST_NATIVE_VECTORIZED_LANES, 4);
    assert_eq!(COLOR_CONTRAST_RUST_LANES, 4);
    assert_eq!(
        lane_policy(),
        ColorContrastLanePolicy::DeterministicFourLaneRustAdaptation
    );
    assert!(
        descriptor
            .capability
            .required_features
            .iter()
            .any(|feature| feature == "explicit-fused-f32-rust-adaptation")
    );
    assert!(
        descriptor
            .capability
            .required_features
            .iter()
            .any(|feature| feature == "four-lane-rust-adaptation")
    );
    assert!(
        descriptor
            .capability
            .required_features
            .iter()
            .any(|feature| feature == "ordinary-vec-scalar-storage-rust-adaptation")
    );
    assert!(
        descriptor
            .capability
            .precision
            .contains("deterministic Rust adaptation")
    );
    assert!(
        descriptor
            .capability
            .precision
            .contains("native C contraction is compiler/target/profile dependent")
    );
    assert_eq!(
        capabilities.require_gpu(),
        Err(ColorContrastCapabilityError::GpuUnavailable)
    );
    assert_eq!(
        capabilities.require_gtk(),
        Err(ColorContrastCapabilityError::GtkUnavailable)
    );
    assert_eq!(
        capabilities.require_masks_and_outer_blending(),
        Err(ColorContrastCapabilityError::MasksAndBlendingUnavailable)
    );
    assert_eq!(
        capabilities.require_native_required_format_image_buffer(),
        Err(ColorContrastCapabilityError::RequiredFormatImageBufferBoundaryDeferred)
    );
    assert_eq!(
        capabilities.require_native_for_each_channel_policy(),
        Err(ColorContrastCapabilityError::NativeLanePolicyDeferred)
    );
    assert_eq!(
        capabilities.require_production_routing(),
        Err(ColorContrastCapabilityError::ProductionRoutingDeferred)
    );
}

#[test]
fn native_default_tiling_values_and_rust_staging_budget_are_distinct() {
    // Source-derived equal-ROI values from default_tiling_callback.
    assert_eq!(
        COLOR_CONTRAST_NATIVE_TILING_FACTOR.to_bits(),
        2.0_f32.to_bits()
    );
    assert_eq!(
        COLOR_CONTRAST_NATIVE_TILING_FACTOR_CL.to_bits(),
        2.0_f32.to_bits()
    );
    assert_eq!(
        COLOR_CONTRAST_NATIVE_TILING_MAXBUF.to_bits(),
        1.0_f32.to_bits()
    );
    assert_eq!(
        COLOR_CONTRAST_NATIVE_TILING_MAXBUF_CL.to_bits(),
        1.0_f32.to_bits()
    );
    assert_eq!(COLOR_CONTRAST_NATIVE_TILING_OVERHEAD_BYTES, 0);
    assert_eq!(COLOR_CONTRAST_NATIVE_TILING_OVERLAP_PIXELS, 0);
    assert_eq!(COLOR_CONTRAST_NATIVE_TILING_ALIGNMENT_PIXELS, 1);

    // Rust-only scheduler and transactional-publication policy.
    assert_eq!(COLOR_CONTRAST_RUST_MINIMUM_TILE_EDGE, 1);
    assert_eq!(COLOR_CONTRAST_RUST_PREFERRED_TILE_EDGE, 256);
    assert_eq!(COLOR_CONTRAST_RUST_INPUT_MULTIPLIER_MILLI, 1000);
    assert_eq!(COLOR_CONTRAST_RUST_OUTPUT_MULTIPLIER_MILLI, 1000);
    assert_eq!(COLOR_CONTRAST_RUST_STAGING_MULTIPLIER_MILLI, 1000);
    assert_eq!(COLOR_CONTRAST_RUST_EXECUTE_INTO_RASTERS, 3);
    assert_eq!(COLOR_CONTRAST_RUST_EXECUTE_INTO_MULTIPLIER_MILLI, 3000);

    let descriptor = colorcontrast_descriptor();
    assert_eq!(
        descriptor.tiling.overlap_pixels,
        COLOR_CONTRAST_NATIVE_TILING_OVERLAP_PIXELS
    );
    assert_eq!(
        descriptor.tiling.alignment_pixels,
        COLOR_CONTRAST_NATIVE_TILING_ALIGNMENT_PIXELS
    );
    assert_eq!(
        descriptor.tiling.minimum_tile_edge,
        COLOR_CONTRAST_RUST_MINIMUM_TILE_EDGE
    );
    assert_eq!(
        descriptor.tiling.preferred_tile_edge,
        COLOR_CONTRAST_RUST_PREFERRED_TILE_EDGE
    );
    assert_eq!(
        descriptor.tiling.input_multiplier_milli,
        COLOR_CONTRAST_RUST_INPUT_MULTIPLIER_MILLI
    );
    assert_eq!(
        descriptor.tiling.output_multiplier_milli,
        COLOR_CONTRAST_RUST_OUTPUT_MULTIPLIER_MILLI
    );
    assert_eq!(
        descriptor.tiling.temporary_multiplier_milli,
        COLOR_CONTRAST_RUST_STAGING_MULTIPLIER_MILLI
    );
    assert_eq!(
        descriptor.tiling.input_multiplier_milli + descriptor.tiling.output_multiplier_milli,
        2000,
        "native factor 2.0 accounts only input plus output"
    );
    assert_eq!(
        descriptor.tiling.input_multiplier_milli
            + descriptor.tiling.output_multiplier_milli
            + descriptor.tiling.temporary_multiplier_milli,
        COLOR_CONTRAST_RUST_EXECUTE_INTO_MULTIPLIER_MILLI,
        "Rust transactional publication adds one staging raster"
    );

    let budget = ColorContrastPlan::new(ColorContrastConfig::defaults(), dimensions(3, 2))
        .allocation_budget()
        .expect("six-pixel budget");
    assert_eq!(budget.pixel_count(), 6);
    assert_eq!(size_of::<ColorContrastPixel>(), 16);
    assert_eq!(budget.raster_bytes(), 96);
    assert_eq!(budget.input_bytes(), 96);
    assert_eq!(budget.output_bytes(), 96);
    assert_eq!(
        budget.staging_bytes(),
        96,
        "operation-owned element-payload budget is one full staging raster"
    );
    assert_eq!(budget.resident_bytes(), 288);
}

#[test]
fn native_memory_store_facts_are_separate_from_ordinary_vec_scalar_storage() {
    assert_eq!(COLOR_CONTRAST_NATIVE_APPLE_AARCH64_CACHELINE_BYTES, 128);
    assert_eq!(COLOR_CONTRAST_NATIVE_OTHER_CACHELINE_BYTES, 64);
    assert_eq!(COLOR_CONTRAST_NATIVE_ALIGNED_PIXEL_BYTES, 16);

    // Ordinary Vec storage requests only the element type's natural alignment;
    // it does not port DT_IS_ALIGNED, dt_aligned_pixel_t, or nontemporal stores.
    assert_eq!(
        COLOR_CONTRAST_RUST_PIXEL_TYPE_ALIGNMENT_BYTES,
        align_of::<ColorContrastPixel>()
    );
    assert_eq!(COLOR_CONTRAST_RUST_PIXEL_TYPE_ALIGNMENT_BYTES, 4);
    assert_ne!(
        COLOR_CONTRAST_RUST_PIXEL_TYPE_ALIGNMENT_BYTES,
        COLOR_CONTRAST_NATIVE_ALIGNED_PIXEL_BYTES
    );
    assert_ne!(
        usize::try_from(COLOR_CONTRAST_NATIVE_TILING_ALIGNMENT_PIXELS)
            .expect("one-pixel tile alignment"),
        COLOR_CONTRAST_NATIVE_ALIGNED_PIXEL_BYTES,
        "tile alignment is a pixel-grid fact, not a byte-alignment promise"
    );
}

#[test]
fn finite_commit_keeps_hidden_values_and_exact_signed_unbound_int() {
    assert_eq!(COLOR_CONTRAST_DEFAULT_A_STEEPNESS, 1.0);
    assert_eq!(COLOR_CONTRAST_DEFAULT_A_OFFSET, 0.0);
    assert_eq!(COLOR_CONTRAST_DEFAULT_B_STEEPNESS, 1.0);
    assert_eq!(COLOR_CONTRAST_DEFAULT_B_OFFSET, 0.0);
    assert_eq!(COLOR_CONTRAST_DEFAULT_UNBOUND, 1);

    let parameters = ColorContrastParametersV2::new(8.0, -300.0, -2.0, 400.0, -7);
    let config = ColorContrastConfig::try_from(parameters).expect("finite native state");
    assert_eq!(config.parameters(), parameters);
    assert!(config.is_unbound());
    assert_eq!(config.unbound(), -7);
    assert_eq!(
        ColorContrastConfig::defaults().parameters(),
        ColorContrastParametersV2::defaults()
    );

    for (parameters, field) in [
        (
            ColorContrastParametersV2::new(f32::NAN, 0.0, 1.0, 0.0, 1),
            "a_steepness",
        ),
        (
            ColorContrastParametersV2::new(1.0, f32::INFINITY, 1.0, 0.0, 1),
            "a_offset",
        ),
        (
            ColorContrastParametersV2::new(1.0, 0.0, f32::NEG_INFINITY, 0.0, 1),
            "b_steepness",
        ),
        (
            ColorContrastParametersV2::new(1.0, 0.0, 1.0, f32::NAN, 1),
            "b_offset",
        ),
    ] {
        assert_eq!(
            ColorContrastConfig::try_from(parameters),
            Err(ColorContrastParameterError::NonFinite(field))
        );
    }
}

#[test]
fn signed_zero_commit_reserialization_and_process_bits_are_exact() {
    let signed_zero_v2_bytes = [
        0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00, 0x80, 0x00, 0x00, 0x00,
        0x80, 0x01, 0x00, 0x00, 0x00,
    ];
    let parameters = ColorContrastParametersV2::from_bytes(&signed_zero_v2_bytes)
        .expect("hard-coded signed-zero executable payload");
    let config = ColorContrastConfig::try_from(parameters).expect("signed zero is finite");

    assert_eq!(config.a_steepness().to_bits(), 0x8000_0000);
    assert_eq!(config.a_offset().to_bits(), 0x8000_0000);
    assert_eq!(config.b_steepness().to_bits(), 0x8000_0000);
    assert_eq!(config.b_offset().to_bits(), 0x8000_0000);
    assert_eq!(config.parameters().to_bytes(), signed_zero_v2_bytes);

    let output = ColorContrastPlan::new(config, dimensions(1, 1))
        .execute(&[ColorContrastPixel::from_channels([
            f32::from_bits(0x41c8_0000),
            f32::from_bits(0x3f80_0000),
            f32::from_bits(0x3f80_0000),
            f32::from_bits(0x3f00_0000),
        ])])
        .expect("signed-zero process vector");
    assert_eq!(
        pixel_bits(output[0]),
        [0x41c8_0000, 0x8000_0000, 0x8000_0000, 0x3f00_0000]
    );
}

#[test]
fn unbound_cpu_equation_uses_native_lane_order_hidden_offsets_and_nonzero_branch() {
    let plan = ColorContrastPlan::new(
        ColorContrastConfig::new(2.0, 3.0, 0.5, -4.0, -9).expect("finite config"),
        dimensions(2, 1),
    );
    let input = [
        ColorContrastPixel::new(50.0, -20.0, 40.0, 0.75),
        ColorContrastPixel::new(12.5, 64.0, -32.0, 0.125),
    ];
    assert_eq!(
        plan.execute(&input).expect("unbound execution"),
        [
            ColorContrastPixel::new(50.0, -37.0, 16.0, 0.75),
            ColorContrastPixel::new(12.5, 131.0, -20.0, 0.125),
        ]
    );

    let identity = ColorContrastPlan::new(ColorContrastConfig::defaults(), dimensions(2, 1));
    assert_eq!(identity.execute(&input).expect("native defaults"), input);
}

#[test]
fn unbound_cpu_equation_pins_deterministic_fused_rust_adaptation() {
    let plan = ColorContrastPlan::new(
        ColorContrastConfig::new(
            f32::from_bits(0x3f9a_ead4),
            f32::from_bits(0x40c8_8889),
            1.0,
            0.0,
            1,
        )
        .expect("finite fused-adaptation config"),
        dimensions(1, 1),
    );
    let output = plan
        .execute(&[ColorContrastPixel::new(
            50.0,
            f32::from_bits(0xc2e6_0bbb),
            0.0,
            1.0,
        )])
        .expect("deterministic fused Rust adaptation vector");

    assert_eq!(output[0].a().to_bits(), 0xc304_f1cf);
    assert_ne!(
        output[0].a().to_bits(),
        0xc304_f1d0,
        "explicit mul_add must retain its fused discriminator against separate rounding"
    );
}

#[test]
fn bounded_cpu_equation_uses_native_clamps_order_and_contains_finite_overflow() {
    let bounded = ColorContrastPlan::new(
        ColorContrastConfig::new(2.0, 0.0, 2.0, 0.0, 0).expect("bounded config"),
        dimensions(2, 1),
    );
    let output = bounded
        .execute(&[
            ColorContrastPixel::new(50.0, 100.0, -100.0, 0.75),
            ColorContrastPixel::new(25.0, 64.0, -64.0, 0.25),
        ])
        .expect("bounded execution");
    assert_eq!(
        output,
        [
            ColorContrastPixel::new(50.0, 128.0, -128.0, 0.75),
            ColorContrastPixel::new(25.0, 128.0, -128.0, 0.25),
        ]
    );

    let bounded_overflow = ColorContrastPlan::new(
        ColorContrastConfig::new(f32::MAX, 0.0, f32::MAX, 0.0, 0)
            .expect("finite bounded overflow config"),
        dimensions(1, 1),
    );
    assert_eq!(
        bounded_overflow
            .execute(&[ColorContrastPixel::new(
                f32::MAX,
                f32::MAX,
                -f32::MAX,
                f32::MAX,
            )])
            .expect("CLAMPS contains arithmetic overflow"),
        [ColorContrastPixel::new(f32::MAX, 128.0, -128.0, f32::MAX)]
    );

    let unbound_overflow = ColorContrastPlan::new(
        ColorContrastConfig::new(f32::MAX, 0.0, 1.0, 0.0, 1)
            .expect("finite unbound overflow config"),
        dimensions(1, 1),
    );
    let overflow_input = [ColorContrastPixel::new(50.0, f32::MAX, 0.0, 1.0)];
    let overflow_sentinel = [ColorContrastPixel::new(-7.0, -6.0, -5.0, -4.0)];
    let mut overflow_destination = overflow_sentinel;
    assert_eq!(
        unbound_overflow.execute_into(&overflow_input, &mut overflow_destination),
        Err(ColorContrastExecutionError::NonFiniteOutput {
            pixel: 0,
            channel: ColorContrastChannel::A
        })
    );
    assert_eq!(overflow_destination, overflow_sentinel);
}

#[test]
fn failures_and_cancellation_never_publish_partial_destination_state() {
    let plan = ColorContrastPlan::new(
        ColorContrastConfig::new(2.0, 3.0, 0.5, -4.0, 1).expect("config"),
        dimensions(2, 1),
    );
    let input = [
        ColorContrastPixel::new(50.0, -20.0, 40.0, 0.75),
        ColorContrastPixel::new(25.0, 10.0, 8.0, 0.25),
    ];
    let sentinel = [
        ColorContrastPixel::new(-9.0, -9.0, -9.0, -9.0),
        ColorContrastPixel::new(-8.0, -8.0, -8.0, -8.0),
    ];

    let mut destination = sentinel;
    assert_eq!(
        plan.execute_into(&input[..1], &mut destination),
        Err(ColorContrastExecutionError::InputDimensionsMismatch {
            expected: 2,
            actual: 1
        })
    );
    assert_eq!(destination, sentinel);

    let mut nonfinite = input;
    nonfinite[1] = ColorContrastPixel::new(25.0, 10.0, f32::NAN, 0.25);
    assert_eq!(
        plan.execute_into(&nonfinite, &mut destination),
        Err(ColorContrastExecutionError::NonFiniteInput {
            pixel: 1,
            channel: ColorContrastChannel::B
        })
    );
    assert_eq!(destination, sentinel);

    let mut wrong_output = [ColorContrastPixel::new(1.0, 2.0, 3.0, 4.0)];
    assert_eq!(
        plan.execute_into(&input, &mut wrong_output),
        Err(ColorContrastExecutionError::OutputDimensionsMismatch {
            expected: 2,
            actual: 1
        })
    );
    assert_eq!(wrong_output, [ColorContrastPixel::new(1.0, 2.0, 3.0, 4.0)]);

    let wide_plan = ColorContrastPlan::new(ColorContrastConfig::defaults(), dimensions(1025, 1));
    let mut wide_input = vec![ColorContrastPixel::new(50.0, 1.0, -1.0, 0.5); 1025];
    wide_input[1024] = ColorContrastPixel::new(50.0, f32::NAN, -1.0, 0.5);
    let wide_sentinel = vec![ColorContrastPixel::new(-7.0, -6.0, -5.0, -4.0); 1025];
    let mut wide_destination = wide_sentinel.clone();
    let mut polls = 0_u32;
    assert_eq!(
        wide_plan.execute_into_with_cancel(&wide_input, &mut wide_destination, || {
            polls += 1;
            polls == 2
        }),
        Err(ColorContrastExecutionError::Cancelled)
    );
    assert_eq!(polls, 2, "second poll occurs by the 1024-pixel bound");
    assert_eq!(wide_destination, wide_sentinel);

    let mut final_gate_polls = 0_u32;
    assert_eq!(
        plan.execute_into_with_cancel(&input, &mut destination, || {
            final_gate_polls += 1;
            final_gate_polls == 2
        }),
        Err(ColorContrastExecutionError::Cancelled)
    );
    assert_eq!(final_gate_polls, 2, "second poll is the publication gate");
    assert_eq!(destination, sentinel);

    plan.execute_into(&input, &mut destination)
        .expect("complete success publishes");
    assert_eq!(
        destination,
        [
            ColorContrastPixel::new(50.0, -37.0, 16.0, 0.75),
            ColorContrastPixel::new(25.0, 23.0, 0.0, 0.25),
        ]
    );
}

#[test]
fn leaf_api_requires_prevalidated_equal_size_four_lane_rasters() {
    let plan = ColorContrastPlan::new(ColorContrastConfig::defaults(), dimensions(2, 1));
    assert_eq!(
        plan.execute(&[ColorContrastPixel::new(50.0, 1.0, -1.0, 0.5)]),
        Err(ColorContrastExecutionError::InputDimensionsMismatch {
            expected: 2,
            actual: 1
        })
    );

    let capabilities = capabilities();
    assert!(!capabilities.native_required_format_image_buffer_supported);
    assert_eq!(
        capabilities.require_native_required_format_image_buffer(),
        Err(ColorContrastCapabilityError::RequiredFormatImageBufferBoundaryDeferred)
    );
}

#[test]
fn point_operation_is_tile_equivalent_with_deterministic_fourth_lane_adaptation() {
    assert_eq!(
        lane_policy(),
        ColorContrastLanePolicy::DeterministicFourLaneRustAdaptation
    );
    let config = ColorContrastConfig::new(1.25, -3.0, 0.75, 2.0, 1).expect("config");
    let input = [
        ColorContrastPixel::new(5.0, -120.0, 80.0, 0.125),
        ColorContrastPixel::new(25.0, -20.0, 40.0, 0.25),
        ColorContrastPixel::new(50.0, 10.0, -8.0, 0.5),
        ColorContrastPixel::new(95.0, 100.0, -100.0, 0.875),
    ];
    let whole = ColorContrastPlan::new(config, dimensions(4, 1))
        .execute(&input)
        .expect("whole raster");
    let first = ColorContrastPlan::new(config, dimensions(2, 1))
        .execute(&input[..2])
        .expect("first tile");
    let second = ColorContrastPlan::new(config, dimensions(2, 1))
        .execute(&input[2..])
        .expect("second tile");
    let tiled = first.into_iter().chain(second).collect::<Vec<_>>();
    assert_eq!(whole, tiled);
    assert_eq!(
        whole.iter().map(|pixel| pixel.alpha()).collect::<Vec<_>>(),
        input.iter().map(|pixel| pixel.alpha()).collect::<Vec<_>>()
    );
}

#[test]
fn source_maps_inventory_ported_dependencies_and_explicit_deferrals() {
    let responsibilities = COLOR_CONTRAST_SOURCE_MAP
        .iter()
        .map(|entry| entry.responsibility)
        .collect::<BTreeSet<_>>();
    assert_eq!(responsibilities.len(), COLOR_CONTRAST_SOURCE_MAP.len());
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "cpu-process-equations"
            && entry.status == ColorContrastPortStatus::RustAdaptation
            && entry.native_lines == "166-221"
            && entry.notes.contains("explicit fused rounding")
            && entry.notes.contains("fail-closed finite checks")
            && entry.notes.contains("noncontracting")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "explicit-fused-rounding-rust-adaptation"
            && entry.status == ColorContrastPortStatus::RustAdaptation
            && entry.native_file.contains("src/iop/CMakeLists.txt")
            && entry.native_file.contains("src/common/math.h")
            && entry.native_lines.contains("iop/CMakeLists.txt:16-59,125")
            && entry
                .notes
                .contains("CMakeLists.txt:125 registers Color Contrast")
            && entry.notes.contains("compiler/target/profile dependent")
            && entry
                .notes
                .contains("Fused rounding is therefore a deterministic Rust adaptation")
            && entry
                .notes
                .contains("Noncontracting native profiles are explicitly deferred")
            && entry.notes.contains("0xc304f1cf")
            && entry.notes.contains("separate-rounding 0xc304f1d0")
            && entry.notes.contains("architecture/rusttable-numerics.toml")
            && entry
                .notes
                .contains("crates/rusttable-processing/src/operations/colorcontrast/leaf.rs")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "operation-local-descriptor-policy"
            && entry.status == ColorContrastPortStatus::RustAdaptation
            && entry
                .notes
                .contains("Native has no operation-local contract")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "visible-steepness-slider-precision"
            && entry.status == ColorContrastPortStatus::Ported
            && entry.native_file.contains("tools/introspection/parser.pm")
            && entry.native_file.contains("src/common/introspection.h")
            && entry.native_file.contains("src/develop/imageop_gui.c")
            && entry.notes.contains("Float.Min and Float.Max")
            && entry.notes.contains("digits=MAX(2")
            && entry.notes.contains("passes two digits")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "native-memory-alignment-and-store-policy"
            && entry.status == ColorContrastPortStatus::RustAdaptation
            && entry.native_symbol.contains("DT_IS_ALIGNED")
            && entry.native_symbol.contains("DT_CACHELINE_BYTES")
            && entry.native_symbol.contains("dt_aligned_pixel_t")
            && entry.native_symbol.contains("copy_pixel_nontemporal")
            && entry.notes.contains("128 on Apple AArch64, 64 otherwise")
            && entry.notes.contains("ordinary Vec scalar Rust storage")
            && entry.notes.contains("Rust adaptation")
            && entry
                .notes
                .contains("tile alignment and Rust allocation-budget facts remain separate")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "native-openmp-scheduling-and-declare-simd"
            && entry.status == ColorContrastPortStatus::RustAdaptation
            && entry.native_file == "src/iop/colorcontrast.c; src/common/darktable.h"
            && entry.native_lines == "colorcontrast.c:152-163,194-219; darktable.h:117-132"
            && entry
                .native_symbol
                .contains("DT_OMP_DECLARE_SIMD(aligned(in,out:64) aligned(slope,offset,low,high))")
            && entry.native_symbol.matches("DT_OMP_FOR()").count() == 2
            && entry.native_symbol.contains("DT_OMP_PRAGMA")
            && entry.rust_symbol.contains("serial row-major pixel loop")
            && entry.notes.contains("both the unbounded and bounded")
            && entry
                .notes
                .contains("parallel for default(firstprivate) schedule(static)")
            && entry.notes.contains("without _OPENMP")
            && entry.notes.contains("scheduling/SIMD")
            && entry.notes.contains("explicitly deferred")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "commit-and-piece-lifetime"
            && entry.status == ColorContrastPortStatus::RustAdaptation
            && entry.notes.contains("including NaN and infinity")
            && entry
                .notes
                .contains("rejects non-finite committed parameters")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "finite-cancellation-publication"
            && entry.status == ColorContrastPortStatus::RustAdaptation
            && entry.notes.contains("writes directly to its output")
            && entry.notes.contains("complete staged raster")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "native-default-tiling-and-rust-allocation-budget"
            && entry.status == ColorContrastPortStatus::RustAdaptation
            && entry.native_symbol.contains("default_tiling_callback")
            && entry.notes.contains("factor 2.0")
            && entry.notes.contains("preferred edge of 256")
            && entry.notes.contains("width*height*16 staging bytes")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "native-description"
            && entry.status == ColorContrastPortStatus::ExplicitlyDeferred
            && entry.native_lines == "81-89"
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "required-input-format-imagebuf-boundary"
            && entry.status == ColorContrastPortStatus::ExplicitlyDeferred
            && entry.native_symbol.contains("dt_iop_copy_image_roi")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "native-for-each-channel-lane-policy"
            && entry.status == ColorContrastPortStatus::ExplicitlyDeferred
            && entry.native_symbol.contains("DT_PIXEL_SIMD_CHANNELS")
    }));
    assert!(COLOR_CONTRAST_SOURCE_MAP.iter().any(|entry| {
        entry.responsibility == "raster-dimensions-and-descriptor-types"
            && entry.status == ColorContrastPortStatus::ExistingDependency
    }));
    let deferred = COLOR_CONTRAST_SOURCE_MAP
        .iter()
        .filter(|entry| entry.status == ColorContrastPortStatus::ExplicitlyDeferred)
        .map(|entry| entry.responsibility)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        deferred,
        BTreeSet::from([
            "gtk-controls",
            "masks-and-outer-blending",
            "native-description",
            "native-for-each-channel-lane-policy",
            "opencl-kernel",
            "production-routing",
            "required-input-format-imagebuf-boundary",
        ])
    );
    let ported = COLOR_CONTRAST_SOURCE_MAP
        .iter()
        .filter(|entry| entry.status == ColorContrastPortStatus::Ported)
        .map(|entry| entry.responsibility)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        ported,
        BTreeSet::from([
            "bounded-scaling-helper",
            "legacy-history-migration",
            "native-metadata-and-parameter-abi",
            "visible-steepness-slider-precision",
        ]),
        "Rust-only finite, publication, tiling, or lane policy must not be marked Ported"
    );

    let architecture =
        include_str!("../../../architecture/rusttable-colorcontrast-source-map.toml");
    for deferred_id in [
        "native-description",
        "required-input-format-imagebuf-boundary",
        "native-for-each-channel-lane-policy",
    ] {
        let deferred_entry = architecture
            .split("[[responsibility]]")
            .find(|entry| entry.contains(&format!("id = \"{deferred_id}\"")))
            .unwrap_or_else(|| panic!("missing deferred responsibility {deferred_id}"));
        assert!(deferred_entry.contains("status = \"deferred\""));
    }
    let precision_entry = architecture
        .split("[[responsibility]]")
        .find(|entry| entry.contains("id = \"visible-steepness-slider-precision\""))
        .expect("missing visible slider precision responsibility");
    assert!(precision_entry.contains("status = \"implemented\""));
    assert!(precision_entry.contains("tools/introspection/parser.pm:66-83"));
    assert!(precision_entry.contains("src/develop/imageop_gui.c:89-107"));
    assert!(precision_entry.contains("passes two digits"));

    let scheduling_entry = architecture
        .split("[[responsibility]]")
        .find(|entry| entry.contains("id = \"native-openmp-scheduling-and-declare-simd\""))
        .expect("missing native OpenMP scheduling and declare-SIMD responsibility");
    assert!(scheduling_entry.contains("status = \"adapted\""));
    assert!(scheduling_entry.contains("src/iop/colorcontrast.c; src/common/darktable.h"));
    assert!(
        scheduling_entry
            .contains("DT_OMP_DECLARE_SIMD(aligned(in,out:64) aligned(slope,offset,low,high))",)
    );
    assert!(scheduling_entry.contains("both the unbounded and bounded process branches"));
    assert!(scheduling_entry.contains("colorcontrast.c:196 and 210"));
    assert!(scheduling_entry.contains("src/common/darktable.h:117-132"));
    assert!(scheduling_entry.contains("parallel for default(firstprivate) schedule(static)"));
    assert!(scheduling_entry.contains("serial row-major pixel traversal"));
    assert!(scheduling_entry.contains("scheduling/SIMD choice is a Rust adaptation"));
    assert!(scheduling_entry.contains("remain explicitly deferred"));

    for adapted_id in [
        "metadata-defaults-and-local-descriptor",
        "explicit-fused-rounding-rust-adaptation",
        "source-ordered-cpu-equations",
        "native-memory-alignment-and-store-policy",
        "native-openmp-scheduling-and-declare-simd",
        "commit-and-piece-lifetime",
        "finite-cancellation-and-transactional-publication",
        "native-default-tiling-and-rust-allocation-budget",
    ] {
        let adapted_entry = architecture
            .split("[[responsibility]]")
            .find(|entry| entry.contains(&format!("id = \"{adapted_id}\"")))
            .unwrap_or_else(|| panic!("missing adapted responsibility {adapted_id}"));
        assert!(
            adapted_entry.contains("status = \"adapted\""),
            "responsibility {adapted_id} overclaims native parity"
        );
    }
    for expected in [
        "schema = \"rusttable.colorcontrast-source-map.v1\"",
        "leaf_baseline = \"7aaec20b8c845df5607e864130e23e643381d313\"",
        "production_routing = false",
        "numerics_policy_registry = \"architecture/rusttable-numerics.toml\"",
        "explicit_fma_source_path = \"crates/rusttable-processing/src/operations/colorcontrast/leaf.rs\"",
        "explicit_fma_policy = \"ExplicitFused\"",
        "explicit_fma_registration = \"present\"",
        "id = \"native-description\"",
        "id = \"required-input-format-imagebuf-boundary\"",
        "id = \"visible-steepness-slider-precision\"",
        "tools/introspection/parser.pm:66-83",
        "src/develop/imageop_gui.c:89-107",
        "id = \"explicit-fused-rounding-rust-adaptation\"",
        "src/iop/CMakeLists.txt:125 registers Color Contrast",
        "line 124 registers nlmeans",
        "compiler/target/profile dependent",
        "Fused rounding is a deterministic Rust adaptation",
        "Noncontracting native profiles are explicitly deferred",
        "0xc2e60bbb",
        "0x3f9aead4",
        "0x40c88889",
        "0xc304f1cf",
        "separately rounded 0xc304f1d0",
        "id = \"source-ordered-cpu-equations\"",
        "id = \"native-for-each-channel-lane-policy\"",
        "id = \"native-memory-alignment-and-store-policy\"",
        "DT_IS_ALIGNED",
        "DT_CACHELINE_BYTES",
        "dt_aligned_pixel_t",
        "copy_pixel_nontemporal",
        "ordinary Vec scalar Rust storage",
        "default tiling callback's one-pixel tile alignment",
        "status = \"adapted\"",
        "Native process writes directly to its destination",
        "id = \"commit-and-piece-lifetime\"",
        "id = \"finite-cancellation-and-transactional-publication\"",
        "id = \"native-default-tiling-and-rust-allocation-budget\"",
        "host factor 2.0 and factor_cl 2.0",
        "preferred edge 256",
        "width*height*16 staging bytes",
        "actual-channel copy-through with distinct input/output ROI cropping and zero-padding",
        "noncontracting native compiler/target/profile combinations",
        "id = \"opencl-host-and-kernel\"",
        "id = \"gtk-controls\"",
        "id = \"masks-and-outer-blending\"",
        "id = \"shared-production-routing\"",
    ] {
        assert!(
            architecture.contains(expected),
            "missing source-map fact: {expected}"
        );
    }

    let numerics = include_str!("../../../architecture/rusttable-numerics.toml");
    let fma_path = "path = \"crates/rusttable-processing/src/operations/colorcontrast/leaf.rs\"";
    assert_eq!(
        numerics.matches(fma_path).count(),
        1,
        "Color Contrast must have one explicit_fma registration"
    );
    let fma_entry = numerics
        .split("[[explicit_fma]]")
        .find(|entry| entry.contains(fma_path))
        .expect("missing Color Contrast explicit_fma entry");
    assert!(fma_entry.contains("policy = \"ExplicitFused\""));
}
