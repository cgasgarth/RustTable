#![allow(
    clippy::assertions_on_constants,
    clippy::float_cmp,
    clippy::too_many_lines,
    clippy::unreadable_literal,
    reason = "source-derived ABI and scalar golden vectors intentionally assert exact f32 bits"
)]

// The bounded leaf is intentionally not exported through shared hubs. Re-export
// descriptor types so its operation-local descriptor can compile in isolation.
pub mod descriptor {
    pub use rusttable_processing::descriptor::*;
}

#[path = "../src/operations/colorcorrection/leaf.rs"]
mod colorcorrection_leaf;

use colorcorrection_leaf::source_map::{
    COLORCORRECTION_BUDGET_BOUNDARY_BYTES_PER_PIXEL, COLORCORRECTION_BUDGET_BOUNDARY_EDGE,
    COLORCORRECTION_BUDGET_BOUNDARY_FACTOR, COLORCORRECTION_BUDGET_MAPPING,
    COLORCORRECTION_FACTOR_TWO_BUDGET_SHORTFALL_BYTES,
    COLORCORRECTION_NATIVE_ALIGNED_PIXEL_ALIGNMENT_BYTES,
    COLORCORRECTION_NATIVE_ALLOCATOR_GENERIC_MAX_TAIL_PADDING_APPLE_AARCH64,
    COLORCORRECTION_NATIVE_ALLOCATOR_GENERIC_MAX_TAIL_PADDING_OTHER,
    COLORCORRECTION_NATIVE_CACHE_OWNS_ALIGNED_BUFFERS,
    COLORCORRECTION_NATIVE_CACHE_TRACKS_REQUESTED_PAYLOAD_BYTES,
    COLORCORRECTION_NATIVE_CACHELINE_BYTES_APPLE_AARCH64,
    COLORCORRECTION_NATIVE_CACHELINE_BYTES_OTHER,
    COLORCORRECTION_NATIVE_CACHELINE_PIXELS_APPLE_AARCH64,
    COLORCORRECTION_NATIVE_CACHELINE_PIXELS_OTHER, COLORCORRECTION_NATIVE_COMMON_NON_CUSTOM_C_FLAG,
    COLORCORRECTION_NATIVE_DEBUG_DEFINE, COLORCORRECTION_NATIVE_DEBUG_NON_CUSTOM_C_FLAGS,
    COLORCORRECTION_NATIVE_DEFAULT_BUILD_TYPE, COLORCORRECTION_NATIVE_F32_FACTOR_TWO_BUDGET_BITS,
    COLORCORRECTION_NATIVE_F32_FACTOR_TWO_BUDGET_BYTES, COLORCORRECTION_NATIVE_GNU_DEBUG_EXTRA,
    COLORCORRECTION_NATIVE_GNU_RELEASE_EXTRA,
    COLORCORRECTION_NATIVE_HOST_BUDGET_ALIGNMENT_PADDING_BYTES,
    COLORCORRECTION_NATIVE_IDENTITY_TILING, COLORCORRECTION_NATIVE_OPENMP_STATIC_SCHEDULE,
    COLORCORRECTION_NATIVE_POSIX_DEBUG_ALLOCATION_EXTRA_CACHELINES,
    COLORCORRECTION_NATIVE_PROFILE_BITS_PORTABLE,
    COLORCORRECTION_NATIVE_RASTER_ALLOCATION_MAX_TAIL_PADDING_APPLE_AARCH64,
    COLORCORRECTION_NATIVE_RASTER_ALLOCATION_MAX_TAIL_PADDING_OTHER,
    COLORCORRECTION_NATIVE_RASTER_PIXEL_BYTES, COLORCORRECTION_NATIVE_RELEASE_C_FLAGS,
    COLORCORRECTION_NATIVE_RELEASE_PERMITS_FMA,
    COLORCORRECTION_NATIVE_RELEASE_PERMITS_REASSOCIATION,
    COLORCORRECTION_NATIVE_RELWITHDEBINFO_C_FLAGS,
    COLORCORRECTION_RUST_BUDGETED_ALIGNMENT_PADDING_BYTES,
    COLORCORRECTION_RUST_BUDGETED_ALLOCATOR_METADATA_BYTES,
    COLORCORRECTION_RUST_CHECKED_FACTOR_TWO_BUDGET_BYTES, COLORCORRECTION_RUST_EXECUTION_IS_SERIAL,
    COLORCORRECTION_RUST_PIXEL_ALIGNMENT_BYTES, COLORCORRECTION_RUST_PIXEL_BYTES,
    COLORCORRECTION_RUST_TRANSACTIONAL_TILING, COLORCORRECTION_RUST_USES_NATIVE_ALIGNMENT_CONTRACT,
    COLORCORRECTION_SOURCE_MAP, ColorCorrectionPortStatus, ColorCorrectionTilingProvenance,
};
use colorcorrection_leaf::{
    COLORCORRECTION_ALLOW_TILING, COLORCORRECTION_COMPATIBILITY_ID,
    COLORCORRECTION_CPU_ARITHMETIC_PROFILE, COLORCORRECTION_DEFAULT_COLORSPACE,
    COLORCORRECTION_DEFAULT_GROUPS, COLORCORRECTION_ENDPOINT_DEFERRED_DESCRIPTOR_PRECISION,
    COLORCORRECTION_ENDPOINT_NATIVE_UI_PRECISION, COLORCORRECTION_GPU_EXECUTABLE,
    COLORCORRECTION_GPU_KERNEL, COLORCORRECTION_GPU_PROGRAM,
    COLORCORRECTION_GUI_INSET_LOGICAL_PIXELS, COLORCORRECTION_GUI_KEY_STEP,
    COLORCORRECTION_KERNEL_ARGUMENT_ORDER, COLORCORRECTION_MIGRATION_EDGES,
    COLORCORRECTION_NATIVE_DESCRIPTION, COLORCORRECTION_NATIVE_NAME,
    COLORCORRECTION_PARAMETER_ORDER, COLORCORRECTION_PRESET_BLEND_COLORSPACE,
    COLORCORRECTION_RUST_ID, COLORCORRECTION_SATURATION_NATIVE_UI_PRECISION,
    COLORCORRECTION_SCHEMA_VERSION, COLORCORRECTION_SUPPORTS_BLENDING,
    COLORCORRECTION_V1_PARAMETER_BYTES, ColorCorrectionCapabilityError, ColorCorrectionChannel,
    ColorCorrectionCodecError, ColorCorrectionConfig, ColorCorrectionCpuArithmeticProfile,
    ColorCorrectionExecutionError, ColorCorrectionHistory, ColorCorrectionParameterError,
    ColorCorrectionParametersV1, ColorCorrectionPixel, ColorCorrectionPlan,
    ColorCorrectionPlanError, capabilities, colorcorrection_leaf_descriptor,
    colorcorrection_leaf_presentation, colorcorrection_preset_evidence,
};
use rusttable_color::ColorEncoding;
use rusttable_processing::RasterDimensions;
use rusttable_processing::descriptor::{
    AlphaPolicy, OperationFlags, ParameterDefault, ParameterKind, RoiKind,
};

const BENCHMARK_FIXTURE: &str = include_str!("fixtures/colorcorrection-v1-benchmark.hex");

fn fixture_bytes() -> Vec<u8> {
    BENCHMARK_FIXTURE
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("fixture is hexadecimal"))
        .collect()
}

fn pixel_bits(pixels: &[ColorCorrectionPixel]) -> Vec<[u32; 4]> {
    pixels
        .iter()
        .map(|pixel| pixel.channels().map(f32::to_bits))
        .collect()
}

fn sentinel_pixel() -> ColorCorrectionPixel {
    ColorCorrectionPixel::from_channels([
        f32::from_bits(0x3f01_2345),
        f32::from_bits(0x3f12_3456),
        f32::from_bits(0x3f23_4567),
        f32::from_bits(0x3f34_5678),
    ])
}

#[test]
fn native_v1_abi_defaults_fixture_and_declaration_order_are_exact() {
    assert_eq!(COLORCORRECTION_SCHEMA_VERSION, 1);
    assert_eq!(COLORCORRECTION_V1_PARAMETER_BYTES, 20);
    assert_eq!(
        COLORCORRECTION_PARAMETER_ORDER,
        ["hia", "hib", "loa", "lob", "saturation"]
    );
    assert_eq!(
        std::mem::size_of::<ColorCorrectionParametersV1>(),
        COLORCORRECTION_V1_PARAMETER_BYTES
    );

    let defaults = ColorCorrectionParametersV1::defaults();
    assert_eq!(
        defaults.to_bytes(),
        [
            0x00, 0x00, 0x00, 0x00, // hia
            0x00, 0x00, 0x00, 0x00, // hib
            0x00, 0x00, 0x00, 0x00, // loa
            0x00, 0x00, 0x00, 0x00, // lob
            0x00, 0x00, 0x80, 0x3f, // saturation
        ]
    );
    assert_eq!(
        ColorCorrectionParametersV1::from_bytes(&defaults.to_bytes()),
        Ok(defaults)
    );

    let bytes = fixture_bytes();
    assert_eq!(
        bytes,
        [
            0x66, 0x3b, 0x9a, 0x40, 0x31, 0x53, 0x4c, 0x40, 0x7f, 0x04, 0x95, 0xc0, 0x0a, 0x72,
            0x7e, 0xc0, 0x00, 0x00, 0x80, 0x3f,
        ]
    );
    let expected = ColorCorrectionParametersV1::new(
        f32::from_bits(0x409a3b66),
        f32::from_bits(0x404c5331),
        f32::from_bits(0xc095047f),
        f32::from_bits(0xc07e720a),
        1.0,
    );
    assert_eq!(
        ColorCorrectionParametersV1::from_bytes(&bytes),
        Ok(expected)
    );
    assert_eq!(expected.to_bytes().as_slice(), bytes);
    assert_eq!(
        ColorCorrectionParametersV1::from_bytes(&bytes[..19]),
        Err(ColorCorrectionCodecError::InvalidLength {
            version: 1,
            expected: 20,
            actual: 19,
        })
    );

    let native_xmp = include_str!("../../../src/tests/benchmark/darktable-bench-3.4.xmp");
    assert!(native_xmp.contains(
        "darktable:operation=\"colorcorrection\"\n      darktable:enabled=\"1\"\n      darktable:modversion=\"1\"\n      darktable:params=\"663b9a4031534c407f0495c00a727ec00000803f\""
    ));
    assert!(native_xmp.contains("bilat,1,colorcorrection,0,colorcontrast,0"));
}

#[test]
fn migration_topology_is_empty_and_future_history_stays_opaque() {
    assert!(COLORCORRECTION_MIGRATION_EDGES.is_empty());
    let current_bytes = ColorCorrectionParametersV1::defaults().to_bytes();
    let current = ColorCorrectionHistory::decode(1, &current_bytes).expect("native v1 history");
    assert_eq!(current.version(), 1);
    assert_eq!(current.payload(), current_bytes);
    assert_eq!(
        current.migrate_to_current(),
        Ok(ColorCorrectionParametersV1::defaults())
    );

    let future_bytes = vec![0xde, 0xad, 0xbe, 0xef, 0x80];
    let future = ColorCorrectionHistory::decode(9, &future_bytes).expect("opaque future history");
    assert_eq!(future.version(), 9);
    assert_eq!(future.payload(), future_bytes);
    assert_eq!(
        future.migrate_to_current(),
        Err(ColorCorrectionCodecError::OpaqueVersion(9))
    );
}

#[test]
fn finite_history_is_unclamped_and_nonfinite_or_overflowed_plans_fail_closed() {
    let parameters = ColorCorrectionParametersV1::new(400.0, -500.0, 600.0, -700.0, 8.0);
    let config = ColorCorrectionConfig::try_from(parameters).expect("finite native history");
    assert_eq!(config.parameters(), parameters);
    assert_eq!(config.hia(), 400.0);
    assert_eq!(config.hib(), -500.0);
    assert_eq!(config.loa(), 600.0);
    assert_eq!(config.lob(), -700.0);
    assert_eq!(config.saturation(), 8.0);

    assert_eq!(
        ColorCorrectionConfig::new(f32::NAN, 0.0, 0.0, 0.0, 1.0),
        Err(ColorCorrectionParameterError::NonFinite("hia"))
    );
    assert_eq!(
        ColorCorrectionConfig::new(0.0, 0.0, 0.0, f32::INFINITY, 1.0),
        Err(ColorCorrectionParameterError::NonFinite("lob"))
    );

    let overflowed = ColorCorrectionConfig::new(f32::MAX, 0.0, -f32::MAX, 0.0, 1.0)
        .expect("persisted endpoints are individually finite");
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    assert_eq!(
        ColorCorrectionPlan::new(overflowed, dimensions),
        Err(ColorCorrectionPlanError::NonFiniteDerived("a_scale"))
    );
}

#[test]
fn preset_tuples_and_signed_zero_survive_scoped_separate_rounding_boundaries() {
    let presets = colorcorrection_preset_evidence();
    assert_eq!(
        presets.iter().map(|preset| preset.name).collect::<Vec<_>>(),
        ["warm tone", "warming filter", "cooling filter"]
    );
    assert!(presets.iter().all(|preset| preset.enabled));
    assert!(
        presets
            .iter()
            .all(|preset| preset.blend_color_space == COLORCORRECTION_PRESET_BLEND_COLORSPACE)
    );
    assert_eq!(
        presets[0].parameters.to_bytes(),
        [
            0x00, 0x00, 0x00, 0x00, // hia = +0
            0x00, 0x00, 0x40, 0x40, // hib = 3
            0x00, 0x00, 0x00, 0x00, // loa = +0
            0x00, 0x00, 0x00, 0x00, // lob = +0
            0x00, 0x00, 0x80, 0x3f, // saturation = 1
        ]
    );
    assert_eq!(
        presets[1].parameters.to_bytes(),
        [
            0x33, 0x33, 0x73, 0xbf, // hia = -0.95
            0x00, 0x00, 0x90, 0x40, // hib = 4.5
            0x33, 0x33, 0x63, 0x40, // loa = 3.55
            0x00, 0x00, 0x00, 0x00, // lob = +0
            0x00, 0x00, 0x80, 0x3f, // saturation = 1
        ]
    );
    assert_eq!(
        presets[2].parameters.to_bytes(),
        [
            0x33, 0x33, 0x73, 0x3f, // hia = 0.95
            0x00, 0x00, 0x90, 0xc0, // hib = -4.5
            0x33, 0x33, 0x63, 0xc0, // loa = -3.55
            0x00, 0x00, 0x00, 0x80, // lob = -0
            0x00, 0x00, 0x80, 0x3f, // saturation = 1
        ]
    );

    let cooling = ColorCorrectionConfig::try_from(presets[2].parameters)
        .expect("finite cooling-filter evidence");
    assert_eq!(cooling.lob().to_bits(), 0x8000_0000);
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let cooling_plan = ColorCorrectionPlan::new(cooling, dimensions).expect("cooling plan");
    assert_eq!(cooling_plan.coefficients().b_base().to_bits(), 0x8000_0000);

    let signed_bytes = [
        0x00, 0x00, 0x00, 0x80, // hia = -0
        0x00, 0x00, 0x00, 0x80, // hib = -0
        0x00, 0x00, 0x00, 0x00, // loa = +0
        0x00, 0x00, 0x00, 0x00, // lob = +0
        0x00, 0x00, 0x00, 0x80, // saturation = -0
    ];
    let signed_parameters = ColorCorrectionParametersV1::from_bytes(&signed_bytes)
        .expect("signed-zero history payload");
    assert_eq!(signed_parameters.to_bytes(), signed_bytes);
    let signed_config =
        ColorCorrectionConfig::try_from(signed_parameters).expect("signed zeros are finite");
    assert_eq!(
        signed_config.parameters().to_bytes(),
        signed_parameters.to_bytes()
    );
    assert_eq!(signed_config.saturation().to_bits(), 0x8000_0000);
    let signed_plan = ColorCorrectionPlan::new(signed_config, dimensions).expect("signed plan");
    assert_eq!(
        signed_plan
            .coefficients()
            .as_kernel_arguments()
            .map(f32::to_bits),
        [0x8000_0000, 0x8000_0000, 0, 0x8000_0000, 0]
    );

    let alpha = f32::from_bits(0x3f41_2345);
    let output = signed_plan
        .execute(&[ColorCorrectionPixel::new(50.0, 1.0, 2.0, alpha)])
        .expect("negative-zero output is finite");
    assert_eq!(
        output[0].channels().map(f32::to_bits),
        [0x4248_0000, 0x8000_0000, 0x8000_0000, 0x3f41_2345]
    );
}

#[test]
fn commit_uses_f32_subtraction_then_double_division_then_f32_narrowing() {
    let hia = f32::from_bits(0x3e07_2f4a);
    let loa = f32::from_bits(0xbf2d_3741);
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let plan = ColorCorrectionPlan::new(
        ColorCorrectionConfig::new(hia, 0.0, loa, 0.0, 1.0).expect("finite vector"),
        dimensions,
    )
    .expect("finite committed vector");

    // Promoting both endpoints before subtraction would produce 0x3c047cd9.
    assert_eq!(plan.coefficients().a_scale().to_bits(), 0x3c04_7cda);
    let alpha = f32::from_bits(0x3f41_2345);
    let output = plan
        .execute(&[ColorCorrectionPixel::new(100.0, 0.0, 3.0, alpha)])
        .expect("finite discriminating output");
    assert_eq!(
        output[0].channels().map(f32::to_bits),
        [0x42c8_0000, 0x3e07_2f50, 0x4040_0000, 0x3f41_2345]
    );
}

#[test]
fn cpu_profile_uses_discriminating_separate_roundings_in_debug_and_release() {
    assert_eq!(
        COLORCORRECTION_CPU_ARITHMETIC_PROFILE,
        ColorCorrectionCpuArithmeticProfile::SeparateRoundings
    );
    assert_eq!(COLORCORRECTION_NATIVE_DEFAULT_BUILD_TYPE, "RelWithDebInfo");
    assert_eq!(COLORCORRECTION_NATIVE_COMMON_NON_CUSTOM_C_FLAG, "-g");
    assert_eq!(COLORCORRECTION_NATIVE_DEBUG_NON_CUSTOM_C_FLAGS, ["-O0"]);
    assert_eq!(COLORCORRECTION_NATIVE_DEBUG_DEFINE, "-D_DEBUG");
    assert_eq!(COLORCORRECTION_NATIVE_GNU_DEBUG_EXTRA, ["-g3", "-ggdb3"]);
    assert_eq!(
        COLORCORRECTION_NATIVE_RELWITHDEBINFO_C_FLAGS,
        ["-O2", "-ftree-vectorize"]
    );
    assert_eq!(
        COLORCORRECTION_NATIVE_RELEASE_C_FLAGS,
        ["-O3", concat!("-ffast", "-math"), "-fno-finite-math-only"]
    );
    assert_eq!(
        COLORCORRECTION_NATIVE_GNU_RELEASE_EXTRA,
        "-fexpensive-optimizations"
    );
    assert!(COLORCORRECTION_NATIVE_RELEASE_PERMITS_FMA);
    assert!(COLORCORRECTION_NATIVE_RELEASE_PERMITS_REASSOCIATION);
    assert!(!COLORCORRECTION_NATIVE_PROFILE_BITS_PORTABLE);

    let contract = include_str!("../../../architecture/rusttable-colorcorrection-source-map.toml");
    assert!(contract.contains("[arithmetic_provenance.native_debug]"));
    assert!(contract.contains("[arithmetic_provenance.native_default_relwithdebinfo]"));
    assert!(contract.contains("selected_when_build_type_unspecified = true"));
    assert!(contract.contains("exact_native_debug_bits_claimed = false"));
    assert!(contract.contains("exact_native_relwithdebinfo_bits_claimed = false"));
    assert!(contract.contains("exact_native_release_bits_claimed = false"));

    let hia = f32::from_bits(0x40f3_f724);
    let loa = f32::from_bits(0xbd49_57c7);
    let saturation = f32::from_bits(0x4023_832c);
    let lightness = f32::from_bits(0x42c3_c571);
    let input_a = f32::from_bits(0xc2b3_cff2);
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let plan = ColorCorrectionPlan::new(
        ColorCorrectionConfig::new(hia, 0.0, loa, 0.0, saturation).expect("finite discriminator"),
        dimensions,
    )
    .expect("finite discriminator coefficients");

    assert_eq!(plan.coefficients().a_scale().to_bits(), 0x3d9d_2503);
    let output = plan
        .execute(&[ColorCorrectionPixel::new(
            lightness,
            input_a,
            0.0,
            f32::from_bits(0x3f41_2345),
        )])
        .expect("finite separate-rounding discriminator");
    let output_bits = output[0].channels().map(f32::to_bits);
    assert_eq!(
        output_bits,
        [0x42c3_c571, 0xc352_a2c4, 0x0000_0000, 0x3f41_2345]
    );

    // Contracting `L * a_scale + a` before adding `a_base` yields one ULP
    // lower for this vector. Native emission remains compiler/profile dependent
    // (and Release fast-math expressly permits it), while this bounded Rust leaf
    // deliberately and portably selects separate roundings.
    assert_ne!(output_bits[1], 0xc352_a2c3);
}

#[test]
fn local_descriptor_preserves_native_metadata_without_routing_claims() {
    assert_eq!(COLORCORRECTION_COMPATIBILITY_ID, "colorcorrection");
    assert_eq!(COLORCORRECTION_RUST_ID, "rusttable.colorcorrection");
    assert_eq!(COLORCORRECTION_NATIVE_NAME, "color correction");
    assert_eq!(
        COLORCORRECTION_NATIVE_DESCRIPTION,
        [
            "correct white balance selectively for blacks and whites",
            "corrective or creative",
            "non-linear, Lab, display-referred",
            "non-linear, Lab",
            "non-linear, Lab, display-referred",
        ]
    );
    let presentation = colorcorrection_leaf_presentation();
    assert_eq!(presentation.name, "color correction");
    assert_eq!(presentation.description, COLORCORRECTION_NATIVE_DESCRIPTION);
    assert_eq!(COLORCORRECTION_DEFAULT_COLORSPACE, "Lab");
    assert_eq!(COLORCORRECTION_DEFAULT_GROUPS, ["color", "grading"]);
    assert!(COLORCORRECTION_ALLOW_TILING);
    assert!(COLORCORRECTION_SUPPORTS_BLENDING);
    assert_eq!(COLORCORRECTION_GUI_INSET_LOGICAL_PIXELS, 5);
    assert_eq!(COLORCORRECTION_GUI_KEY_STEP, 0.5);
    assert_eq!(COLORCORRECTION_GPU_PROGRAM, 2);
    assert_eq!(COLORCORRECTION_GPU_KERNEL, "colorcorrection");
    assert!(!COLORCORRECTION_GPU_EXECUTABLE);
    assert_eq!(
        COLORCORRECTION_KERNEL_ARGUMENT_ORDER,
        ["saturation", "a_scale", "a_base", "b_scale", "b_base"]
    );

    let descriptor = colorcorrection_leaf_descriptor();
    descriptor.validate().expect("operation-local descriptor");
    assert_eq!(
        descriptor.id.compatibility_name,
        COLORCORRECTION_COMPATIBILITY_ID
    );
    assert_eq!(descriptor.id.rust_id, COLORCORRECTION_RUST_ID);
    assert_eq!(descriptor.stage, "display-referred-lab-d50");
    assert_eq!(descriptor.roi, RoiKind::Identity);
    assert_eq!(descriptor.tiling.overlap_pixels, 0);
    assert_eq!(descriptor.tiling.alignment_pixels, 1);
    assert_eq!(descriptor.tiling.minimum_tile_edge, 1);
    assert_eq!(descriptor.tiling.preferred_tile_edge, 256);
    assert_eq!(descriptor.tiling.input_multiplier_milli, 1_000);
    assert_eq!(descriptor.tiling.output_multiplier_milli, 1_000);
    assert_eq!(descriptor.tiling.temporary_multiplier_milli, 1_000);
    assert_eq!(
        descriptor
            .parameters
            .iter()
            .map(|parameter| parameter.id.as_str())
            .collect::<Vec<_>>(),
        COLORCORRECTION_PARAMETER_ORDER
    );
    assert_eq!(COLORCORRECTION_ENDPOINT_NATIVE_UI_PRECISION, None);
    assert_eq!(COLORCORRECTION_ENDPOINT_DEFERRED_DESCRIPTOR_PRECISION, 0);
    assert!(descriptor.parameters[..4].iter().all(|parameter| {
        parameter.precision == COLORCORRECTION_ENDPOINT_DEFERRED_DESCRIPTOR_PRECISION
    }));
    assert_eq!(COLORCORRECTION_SATURATION_NATIVE_UI_PRECISION, 2);
    assert_eq!(
        descriptor.parameters[4].precision,
        COLORCORRECTION_SATURATION_NATIVE_UI_PRECISION
    );
    assert!(matches!(
        descriptor.parameters[0].kind,
        ParameterKind::Scalar {
            minimum: -40.0,
            maximum: 40.0,
        }
    ));
    assert!(matches!(
        descriptor.parameters[4].kind,
        ParameterKind::Scalar {
            minimum: -3.0,
            maximum: 3.0,
        }
    ));
    assert_eq!(
        descriptor.parameters[4].default,
        ParameterDefault::Scalar(1.0)
    );
    for flag in [
        OperationFlags::MULTI_INSTANCE,
        OperationFlags::STYLE_ELIGIBLE,
        OperationFlags::HISTORY_VISIBLE,
        OperationFlags::TILEABLE,
        OperationFlags::DETERMINISTIC_CPU,
        OperationFlags::COLOR,
        OperationFlags::MASKS,
        OperationFlags::BLENDING,
    ] {
        assert!(
            descriptor.flags.contains(flag),
            "missing local flag {flag:?}"
        );
    }
    assert!(!descriptor.flags.contains(OperationFlags::DETERMINISTIC_GPU));
    assert_eq!(descriptor.io.input.channels, 4);
    assert_eq!(descriptor.io.input.alpha, AlphaPolicy::Preserve);
    assert_eq!(descriptor.io.input.encodings, [ColorEncoding::LabD50]);
    assert_eq!(descriptor.io.output.encodings, [ColorEncoding::LabD50]);
    assert!(descriptor.capability.cpu_supported);
    assert_eq!(descriptor.capability.gpu_tier, None);
    assert_eq!(descriptor.capability.modes, ["operation-local"]);
    assert!(!descriptor.mask_blend.consumes_mask);
    assert_eq!(descriptor.migration.source_versions, [1]);
    assert_eq!(descriptor.migration.target_version, 1);
    assert!(descriptor.migration.opaque_unknown_allowed);
    assert!(descriptor.ui.is_none());
}

#[test]
fn native_ui_precision_applies_only_to_the_generated_saturation_slider() {
    let native_operation = include_str!("../../../src/iop/colorcorrection.c");
    assert_eq!(
        native_operation
            .matches("dt_bauhaus_slider_from_params")
            .count(),
        1
    );
    assert!(
        native_operation
            .contains("g->slider = dt_bauhaus_slider_from_params(self, N_(\"saturation\"));")
    );
    assert!(
        native_operation.contains("float hia, hib, loa, lob;  // directly manipulated from gui")
    );

    let slider_factory = include_str!("../../../src/develop/imageop_gui.c");
    assert!(
        slider_factory.contains("const float top = fminf(max-min, fmaxf(fabsf(min), fabsf(max)));")
    );
    assert!(slider_factory.contains("const int digits = MAX(2, -floorf(log10f(top/100)+.1));"));
    let top = (3.0_f32 - (-3.0)).min((-3.0_f32).abs().max(3.0_f32.abs()));
    assert_eq!(top, 3.0);
    assert_eq!(-((top / 100.0).log10() + 0.1).floor(), 2.0);
    assert_eq!(COLORCORRECTION_SATURATION_NATIVE_UI_PRECISION, 2);
    assert_eq!(COLORCORRECTION_ENDPOINT_NATIVE_UI_PRECISION, None);
    assert_eq!(COLORCORRECTION_ENDPOINT_DEFERRED_DESCRIPTOR_PRECISION, 0);

    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("custom-grid endpoint fields")
            && entry.native_symbol.contains("saturation digits formula")
            && entry.native_file.contains("src/develop/imageop_gui.c")
            && entry.status == ColorCorrectionPortStatus::RustAdaptation
    }));
    let contract = include_str!("../../../architecture/rusttable-colorcorrection-source-map.toml");
    assert!(contract.contains("[ui_precision_provenance.native_controls]"));
    assert!(contract.contains("endpoint_native_numeric_precision = \"none\""));
    assert!(contract.contains("endpoint_descriptor_precision_status = \"explicitly-deferred"));
    assert!(contract.contains("saturation_native_numeric_precision = 2"));
}

#[test]
fn native_alignment_and_schedule_are_explicit_serial_vec_rust_adaptations() {
    assert_eq!(COLORCORRECTION_NATIVE_CACHELINE_BYTES_APPLE_AARCH64, 128);
    assert_eq!(COLORCORRECTION_NATIVE_CACHELINE_BYTES_OTHER, 64);
    assert_eq!(COLORCORRECTION_NATIVE_CACHELINE_PIXELS_APPLE_AARCH64, 8);
    assert_eq!(COLORCORRECTION_NATIVE_CACHELINE_PIXELS_OTHER, 4);
    assert_eq!(COLORCORRECTION_NATIVE_RASTER_PIXEL_BYTES, 16);
    assert_eq!(COLORCORRECTION_NATIVE_ALIGNED_PIXEL_ALIGNMENT_BYTES, 16);
    assert!(COLORCORRECTION_NATIVE_OPENMP_STATIC_SCHEDULE);
    assert!(COLORCORRECTION_NATIVE_CACHE_OWNS_ALIGNED_BUFFERS);
    assert!(COLORCORRECTION_NATIVE_CACHE_TRACKS_REQUESTED_PAYLOAD_BYTES);

    assert_eq!(COLORCORRECTION_RUST_PIXEL_BYTES, 16);
    assert_eq!(
        COLORCORRECTION_RUST_PIXEL_BYTES,
        std::mem::size_of::<ColorCorrectionPixel>()
    );
    assert_eq!(COLORCORRECTION_RUST_PIXEL_ALIGNMENT_BYTES, 4);
    assert_eq!(
        COLORCORRECTION_RUST_PIXEL_ALIGNMENT_BYTES,
        std::mem::align_of::<ColorCorrectionPixel>()
    );
    assert!(COLORCORRECTION_RUST_EXECUTION_IS_SERIAL);
    assert!(!COLORCORRECTION_RUST_USES_NATIVE_ALIGNMENT_CONTRACT);

    assert_eq!(
        COLORCORRECTION_NATIVE_HOST_BUDGET_ALIGNMENT_PADDING_BYTES,
        0
    );
    assert_eq!(
        COLORCORRECTION_NATIVE_ALLOCATOR_GENERIC_MAX_TAIL_PADDING_APPLE_AARCH64,
        127
    );
    assert_eq!(
        COLORCORRECTION_NATIVE_ALLOCATOR_GENERIC_MAX_TAIL_PADDING_OTHER,
        63
    );
    assert_eq!(
        COLORCORRECTION_NATIVE_RASTER_ALLOCATION_MAX_TAIL_PADDING_APPLE_AARCH64,
        112
    );
    assert_eq!(
        COLORCORRECTION_NATIVE_RASTER_ALLOCATION_MAX_TAIL_PADDING_OTHER,
        48
    );
    assert_eq!(128 - COLORCORRECTION_NATIVE_RASTER_PIXEL_BYTES, 112);
    assert_eq!(64 - COLORCORRECTION_NATIVE_RASTER_PIXEL_BYTES, 48);
    assert_eq!(
        COLORCORRECTION_NATIVE_POSIX_DEBUG_ALLOCATION_EXTRA_CACHELINES,
        1
    );
    assert_eq!(COLORCORRECTION_RUST_BUDGETED_ALIGNMENT_PADDING_BYTES, 0);
    assert_eq!(COLORCORRECTION_RUST_BUDGETED_ALLOCATOR_METADATA_BYTES, 0);

    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("dt_check_aligned")
            && entry.native_symbol.contains("DT_OMP_FOR")
            && entry.native_file.contains("src/develop/pixelpipe_hb.c")
            && entry.status == ColorCorrectionPortStatus::RustAdaptation
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry
            .native_symbol
            .contains("pixelpipe_hb bpp*width*height")
            && entry
                .native_symbol
                .contains("pixelpipe_cache dt_alloc_aligned")
            && entry.native_file.contains("src/develop/pixelpipe_cache.c")
            && entry.native_file.contains("src/common/darktable.c")
            && entry.status == ColorCorrectionPortStatus::RustAdaptation
    }));

    let pixelpipe = include_str!("../../../src/develop/pixelpipe_hb.c");
    assert!(
        pixelpipe
            .contains("const size_t bufsize = (size_t)bpp * roi_out->width * roi_out->height;")
    );
    assert!(pixelpipe.contains("if(!dt_check_aligned(input) || !dt_check_aligned(*output))"));
    assert!(pixelpipe.contains("dt_dev_pixelpipe_cache_get(pipe, hash, bufsize,"));
    let cache = include_str!("../../../src/develop/pixelpipe_cache.c");
    assert!(cache.contains("cache->data[cline] = (void *)dt_alloc_aligned(size);"));
    assert!(cache.contains("dt_free_align(cache->data[cline]);"));
    assert!(cache.contains("cache->allmem += size;"));

    let contract = include_str!("../../../architecture/rusttable-colorcorrection-source-map.toml");
    assert!(contract.contains("[execution_provenance.native_cpu_loop]"));
    assert!(contract.contains("[execution_provenance.native_pixelpipe_buffer_ownership]"));
    assert!(contract.contains("[execution_provenance.rust_serial_vec_adaptation]"));
    assert!(contract.contains("required_pixel_alignment_bytes = 4"));
    assert!(contract.contains("upper-left x/y tile-coordinate modulus"));
    assert!(contract.contains("native_host_fit_alignment_padding_bytes = 0"));
    assert!(contract.contains("rust_budgeted_alignment_padding_bytes = 0"));
    assert!(contract.contains("colorcorrection_raster_max_tail_padding_apple_aarch64 = 112"));
    assert!(contract.contains("colorcorrection_raster_max_tail_padding_other = 48"));
    assert!(contract.contains("native_posix_debug_allocator_extra"));
}

#[test]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the in-range f32-to-size conversion is the native boundary under test"
)]
fn native_f32_budget_and_checked_integer_boundary_are_deliberately_distinct() {
    assert_eq!(COLORCORRECTION_BUDGET_BOUNDARY_EDGE, 4_097);
    assert_eq!(COLORCORRECTION_BUDGET_BOUNDARY_FACTOR, 2);
    assert_eq!(COLORCORRECTION_BUDGET_BOUNDARY_BYTES_PER_PIXEL, 16);

    // Mirrors the C source-language evaluation type without becoming production
    // arithmetic: `factor` is f32, so every integer operand enters an f32
    // expression before assignment to size_t.
    let native_f32_total = ((2.0_f32 * 4_097.0) * 4_097.0) * 16.0 + 0.0;
    assert_eq!(
        native_f32_total.to_bits(),
        COLORCORRECTION_NATIVE_F32_FACTOR_TWO_BUDGET_BITS
    );
    assert_eq!(native_f32_total.to_bits(), 0x4e00_1000);
    let native_size_total = native_f32_total as usize;
    assert_eq!(
        native_size_total,
        COLORCORRECTION_NATIVE_F32_FACTOR_TWO_BUDGET_BYTES
    );
    assert_eq!(native_size_total, 537_133_056);

    let checked_integer_total = 2_usize
        .checked_mul(4_097)
        .and_then(|bytes| bytes.checked_mul(4_097))
        .and_then(|bytes| bytes.checked_mul(16))
        .expect("4097-square factor-two payload fits usize");
    assert_eq!(
        checked_integer_total,
        COLORCORRECTION_RUST_CHECKED_FACTOR_TWO_BUDGET_BYTES
    );
    assert_eq!(checked_integer_total, 537_133_088);
    assert_eq!(
        checked_integer_total - COLORCORRECTION_NATIVE_F32_FACTOR_TWO_BUDGET_BYTES,
        COLORCORRECTION_FACTOR_TWO_BUDGET_SHORTFALL_BYTES
    );
    assert_eq!(COLORCORRECTION_FACTOR_TWO_BUDGET_SHORTFALL_BYTES, 32);

    let tiling = include_str!("../../../src/develop/tiling.c");
    assert!(tiling.contains("const size_t total = factor * width * height * bpp + overhead;"));
    let contract = include_str!("../../../architecture/rusttable-colorcorrection-source-map.toml");
    assert!(contract.contains("native_host_fit_evaluation_type = \"f32: factor is float"));
    assert!(contract.contains("boundary_width = 4097"));
    assert!(contract.contains("boundary_native_f32_then_size_bytes = 537133056"));
    assert!(contract.contains("boundary_checked_integer_bytes = 537133088"));
    assert!(contract.contains("boundary_native_shortfall_bytes = 32"));
    assert!(contract.contains("deliberately not native f32-then-size_t arithmetic"));
}

#[test]
fn retained_default_tiling_and_rust_transactional_policy_are_distinct() {
    let native = COLORCORRECTION_NATIVE_IDENTITY_TILING;
    assert_eq!(
        native.provenance,
        ColorCorrectionTilingProvenance::RetainedDefaultCallback
    );
    assert_eq!(native.input_output_factor, 2.0);
    assert_eq!(native.input_output_factor_cl, 2.0);
    assert_eq!(native.maximum_buffer_factor, 1.0);
    assert_eq!(native.maximum_buffer_factor_cl, 1.0);
    assert_eq!(native.overhead_bytes, 0);
    assert_eq!(native.overlap_pixels, 0);
    assert_eq!(native.alignment_pixels, 1);

    let rust_policy = COLORCORRECTION_RUST_TRANSACTIONAL_TILING;
    assert_eq!(
        rust_policy.provenance,
        ColorCorrectionTilingProvenance::RustTransactionalPolicy
    );
    assert_eq!(rust_policy.overlap_pixels, 0);
    assert_eq!(rust_policy.alignment_pixels, 1);
    assert_eq!(rust_policy.minimum_tile_edge, 1);
    assert_eq!(rust_policy.preferred_tile_edge, 256);
    assert_eq!(rust_policy.input_multiplier_milli, 1_000);
    assert_eq!(rust_policy.output_multiplier_milli, 1_000);
    assert_eq!(rust_policy.temporary_multiplier_milli, 1_000);

    let budget = COLORCORRECTION_BUDGET_MAPPING;
    assert_eq!(budget.native_input_output, 2_000);
    assert_eq!(budget.preallocated_input, 1_000);
    assert_eq!(budget.preallocated_destination, 1_000);
    assert_eq!(budget.budgeted_staging, 1_000);
    assert_eq!(budget.rust_peak, 3_000);
    assert_eq!(
        rust_policy.input_multiplier_milli + rust_policy.output_multiplier_milli,
        budget.native_input_output
    );
    assert_eq!(
        budget.native_input_output + rust_policy.temporary_multiplier_milli,
        budget.rust_peak
    );
    assert_ne!(native.provenance, rust_policy.provenance);

    let contract = include_str!("../../../architecture/rusttable-colorcorrection-source-map.toml");
    assert!(contract.contains("preferred_tile_edge = \"not supplied by native callback\""));
    assert!(contract.contains(
        "origin = \"additive bounded-leaf policy, not a native default_tiling_callback value\""
    ));
}

#[test]
fn committed_coefficients_and_cpu_equation_match_separate_rounding_golden_bits() {
    let parameters = ColorCorrectionParametersV1::new(
        f32::from_bits(0x409a3b66),
        f32::from_bits(0x404c5331),
        f32::from_bits(0xc095047f),
        f32::from_bits(0xc07e720a),
        1.0,
    );
    let config = ColorCorrectionConfig::try_from(parameters).expect("finite benchmark history");
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let plan = ColorCorrectionPlan::new(config, dimensions).expect("finite committed state");
    assert_eq!(plan.config(), config);
    assert_eq!(plan.dimensions(), dimensions);
    assert_eq!(
        plan.coefficients().as_kernel_arguments().map(f32::to_bits),
        [0x3f800000, 0x3dc21469, 0xc095047f, 0x3d92ce7a, 0xc07e720a]
    );

    let alpha = f32::from_bits(0x3f412345);
    let input = ColorCorrectionPixel::new(f32::from_bits(0x42558f5f), -17.25, 23.75, alpha);
    let output = plan.execute(&[input]).expect("finite point result");
    assert_eq!(
        output[0].channels().map(f32::to_bits),
        [0x42558f5f, 0xc186c735, 0x41bccfc0, 0x3f412345]
    );
    assert_eq!(output[0].lightness().to_bits(), input.lightness().to_bits());
    assert_eq!(output[0].alpha_or_spare().to_bits(), alpha.to_bits());

    let unbounded = ColorCorrectionPlan::new(
        ColorCorrectionConfig::new(400.0, -400.0, -400.0, 400.0, 8.0)
            .expect("finite out-of-GUI-range history"),
        dimensions,
    )
    .expect("finite unbounded coefficients")
    .execute(&[ColorCorrectionPixel::new(100.0, 128.0, -128.0, 0.25)])
    .expect("finite unbounded result");
    assert_eq!(unbounded[0].channels(), [100.0, 4224.0, -4224.0, 0.25]);
}

#[test]
fn failures_and_cancellation_never_publish_partial_destination_pixels() {
    let dimensions = RasterDimensions::new(4, 2).expect("dimensions");
    let config =
        ColorCorrectionConfig::new(10.0, -20.0, -10.0, 30.0, 1.5).expect("finite parameters");
    let plan = ColorCorrectionPlan::new(config, dimensions).expect("plan");
    let input = vec![ColorCorrectionPixel::new(50.0, 20.0, -10.0, 0.75); 8];
    let sentinel = sentinel_pixel();
    let mut destination = vec![sentinel; 8];
    let before = pixel_bits(&destination);
    assert_eq!(
        plan.execute_into_with_cancel(&input, &mut destination, |processed| processed == 4),
        Err(ColorCorrectionExecutionError::Cancelled { processed: 4 })
    );
    assert_eq!(pixel_bits(&destination), before);

    let mut nonfinite = input.clone();
    nonfinite[3] = ColorCorrectionPixel::new(50.0, 20.0, f32::NAN, 0.75);
    assert_eq!(
        plan.execute_into(&nonfinite, &mut destination),
        Err(ColorCorrectionExecutionError::NonFiniteInput {
            pixel: 3,
            channel: ColorCorrectionChannel::B,
        })
    );
    assert_eq!(pixel_bits(&destination), before);

    let overflow_config =
        ColorCorrectionConfig::new(0.0, 0.0, 0.0, 0.0, f32::MAX).expect("finite saturation");
    let overflow_plan =
        ColorCorrectionPlan::new(overflow_config, dimensions).expect("finite coefficients");
    let overflow_input = vec![ColorCorrectionPixel::new(50.0, 2.0, 0.0, 0.75); 8];
    assert_eq!(
        overflow_plan.execute_into(&overflow_input, &mut destination),
        Err(ColorCorrectionExecutionError::NonFiniteOutput {
            pixel: 0,
            channel: ColorCorrectionChannel::A,
        })
    );
    assert_eq!(pixel_bits(&destination), before);

    assert_eq!(
        plan.execute(&input[..7]),
        Err(ColorCorrectionExecutionError::DimensionsMismatch {
            expected: 8,
            actual: 7,
        })
    );
    let mut short_destination = vec![sentinel; 7];
    assert_eq!(
        plan.execute_into(&input, &mut short_destination),
        Err(ColorCorrectionExecutionError::DestinationLengthMismatch {
            expected: 8,
            actual: 7,
        })
    );

    plan.execute_into(&input, &mut destination)
        .expect("complete result publishes");
    assert_ne!(pixel_bits(&destination), before);
    assert!(destination.iter().all(|pixel| {
        pixel.lightness().to_bits() == 50.0_f32.to_bits()
            && pixel.alpha_or_spare().to_bits() == 0.75_f32.to_bits()
    }));
}

#[test]
fn cancellation_polls_the_literal_1024_pixel_boundary_transactionally() {
    let dimensions = RasterDimensions::new(2048, 1).expect("dimensions");
    let plan = ColorCorrectionPlan::new(ColorCorrectionConfig::defaults(), dimensions)
        .expect("default plan");
    let input = vec![ColorCorrectionPixel::new(50.0, 1.0, 2.0, 0.75); 2048];
    let sentinel = sentinel_pixel();
    let mut destination = vec![sentinel; 2048];
    let before = pixel_bits(&destination);
    let mut polls = Vec::new();

    assert_eq!(
        plan.execute_into_with_cancel(&input, &mut destination, |processed| {
            polls.push(processed);
            processed == 1024
        }),
        Err(ColorCorrectionExecutionError::Cancelled { processed: 1024 })
    );
    assert_eq!(polls, [0, 1024]);
    assert_eq!(pixel_bits(&destination), before);
}

#[test]
fn cancellation_before_work_keeps_the_destination_unchanged() {
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let plan = ColorCorrectionPlan::new(ColorCorrectionConfig::defaults(), dimensions)
        .expect("default plan");
    let input = [ColorCorrectionPixel::new(50.0, 1.0, 2.0, 0.75)];
    let sentinel = sentinel_pixel();
    let mut destination = [sentinel];
    let before = pixel_bits(&destination);
    let mut polls = Vec::new();

    assert_eq!(
        plan.execute_into_with_cancel(&input, &mut destination, |processed| {
            polls.push(processed);
            true
        }),
        Err(ColorCorrectionExecutionError::Cancelled { processed: 0 })
    );
    assert_eq!(polls, [0]);
    assert_eq!(pixel_bits(&destination), before);
}

#[test]
fn cancellation_after_private_processing_keeps_the_destination_unchanged() {
    let dimensions = RasterDimensions::new(2, 1).expect("dimensions");
    let plan = ColorCorrectionPlan::new(
        ColorCorrectionConfig::new(10.0, -20.0, -10.0, 30.0, 1.5).expect("config"),
        dimensions,
    )
    .expect("plan");
    let input = [
        ColorCorrectionPixel::new(50.0, 1.0, 2.0, 0.75),
        ColorCorrectionPixel::new(25.0, 3.0, 4.0, 0.5),
    ];
    let sentinel = sentinel_pixel();
    let mut destination = [sentinel; 2];
    let before = pixel_bits(&destination);
    let mut polls = Vec::new();

    assert_eq!(
        plan.execute_into_with_cancel(&input, &mut destination, |processed| {
            polls.push(processed);
            processed == 2
        }),
        Err(ColorCorrectionExecutionError::Cancelled { processed: 2 })
    );
    assert_eq!(polls, [0, 2]);
    assert_eq!(pixel_bits(&destination), before);
}

#[test]
fn size_and_allocation_failures_keep_the_destination_unchanged() {
    let sentinel = sentinel_pixel();

    let overflow_dimensions =
        RasterDimensions::new(u32::MAX, u32::MAX).expect("nonzero dimensions");
    let overflow_plan =
        ColorCorrectionPlan::new(ColorCorrectionConfig::defaults(), overflow_dimensions)
            .expect("finite coefficients");
    let mut overflow_destination = [sentinel];
    let overflow_before = pixel_bits(&overflow_destination);
    assert_eq!(
        overflow_plan.execute_into(&[], &mut overflow_destination),
        Err(ColorCorrectionExecutionError::SizeOverflow)
    );
    assert_eq!(pixel_bits(&overflow_destination), overflow_before);

    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let allocation_plan = ColorCorrectionPlan::new(ColorCorrectionConfig::defaults(), dimensions)
        .expect("default plan")
        .with_memory_budget(15);
    assert_eq!(allocation_plan.memory_budget(), 15);
    let input = [ColorCorrectionPixel::new(50.0, 1.0, 2.0, 0.75)];
    let mut allocation_destination = [sentinel];
    let allocation_before = pixel_bits(&allocation_destination);
    assert_eq!(
        allocation_plan.execute_into(&input, &mut allocation_destination),
        Err(ColorCorrectionExecutionError::AllocationFailed { required_bytes: 16 })
    );
    assert_eq!(pixel_bits(&allocation_destination), allocation_before);
}

#[test]
fn capabilities_and_source_maps_keep_every_unowned_surface_fail_closed() {
    let support = capabilities();
    assert!(support.cpu_supported);
    assert!(support.history_codec_supported);
    assert!(support.local_descriptor_supported);
    assert!(!support.gpu_supported);
    assert!(!support.gtk_supported);
    assert!(!support.presets_registered);
    assert!(!support.format_copy_through_supported);
    assert!(!support.masks_consumed);
    assert!(support.outer_blending_deferred);
    assert!(support.production_routing_deferred);
    assert!(support.alpha_or_spare_preserved);
    assert_eq!(
        support.require_gpu(),
        Err(ColorCorrectionCapabilityError::GpuUnavailable)
    );
    assert_eq!(
        support.require_gtk(),
        Err(ColorCorrectionCapabilityError::GtkUnavailable)
    );
    assert_eq!(
        support.require_preset_registration(),
        Err(ColorCorrectionCapabilityError::PresetRegistrationDeferred)
    );
    assert_eq!(
        support.require_format_copy_through(),
        Err(ColorCorrectionCapabilityError::FormatCopyThroughDeferred)
    );
    assert_eq!(
        support.require_production_routing(),
        Err(ColorCorrectionCapabilityError::ProductionRoutingDeferred)
    );

    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol == "commit_params" && entry.status == ColorCorrectionPortStatus::Ported
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol == "process" && entry.status == ColorCorrectionPortStatus::Ported
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry
            .native_symbol
            .contains(concat!("Release -O3/-ffast", "-math"))
            && entry.native_file.contains("src/CMakeLists.txt")
            && entry.status == ColorCorrectionPortStatus::RustAdaptation
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("DEFAULT callback fallback")
            && entry.native_file.contains("src/common/module_api.h")
            && entry.status == ColorCorrectionPortStatus::SourceEvidenceOnly
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("four-channel float Lab")
            && entry.native_file.contains("src/develop/format.c")
            && entry.status == ColorCorrectionPortStatus::Ported
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("dt_iop_copy_image_roi")
            && entry.native_file.contains("src/common/imagebuf.c")
            && entry.status == ColorCorrectionPortStatus::ExplicitlyDeferred
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("init_pipe / cleanup_pipe")
            && entry.status == ColorCorrectionPortStatus::Ported
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry
            .native_symbol
            .contains("default_tiling_callback identity-ROI")
            && entry.status == ColorCorrectionPortStatus::SourceEvidenceOnly
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry
            .native_symbol
            .contains("f32 factor*width*height*bpp+overhead before size_t conversion")
            && entry.native_file.contains("src/develop/pixelpipe_hb.c")
            && entry.native_file.contains("src/develop/pixelpipe_cache.c")
            && entry.status == ColorCorrectionPortStatus::RustAdaptation
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("init_presets tuple values")
            && entry.status == ColorCorrectionPortStatus::SourceEvidenceOnly
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("dt_gui_presets_add_generic")
            && entry.status == ColorCorrectionPortStatus::ExplicitlyDeferred
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("process_cl")
            && entry.status == ColorCorrectionPortStatus::ExplicitlyDeferred
    }));
    assert!(COLORCORRECTION_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("history import")
            && entry.status == ColorCorrectionPortStatus::ExplicitlyDeferred
    }));

    let contract = include_str!("../../../architecture/rusttable-colorcorrection-source-map.toml");
    assert!(contract.contains("rusttable_baseline = \"7aaec20b8c845df5607e864130e23e643381d313\""));
    assert!(contract.contains("complete_native_source_read = true"));
    assert!(contract.contains("production_routing_claimed = false"));
    assert!(contract.contains("status = \"implemented-rust-boundary\""));
    assert!(contract.contains("status = \"implemented-rust-ownership-boundary\""));
    assert!(contract.contains("status = \"source-evidence-only\""));
    assert!(contract.contains("status = \"deferred-unmodified\""));
    assert!(contract.contains("crates/rusttable-processing/src/operations/colorcorrection.rs"));
    assert!(contract.contains("crates/rusttable-processing/tests/colorcorrection_leaf.rs"));
    assert!(contract.contains("lob=0x80000000"));
    assert!(contract.contains("native_pipe_allocation"));
    assert!(contract.contains("src/develop/tiling.c"));
    assert!(contract.contains("native_tiling_owner"));
    assert!(contract.contains("rust_tiling_policy_owner"));
    assert!(contract.contains("rust_budget_owner"));
    assert!(contract.contains("src/common/module_api.h"));
    assert!(contract.contains("src/develop/format.c"));
    assert!(contract.contains("src/common/imagebuf.c"));
    assert!(contract.contains("profile = \"SeparateRoundings\""));
    assert!(contract.contains("chosen_output_a_bits = \"0xc352a2c4\""));
    assert!(contract.contains("contracted_output_a_bits = \"0xc352a2c3\""));
    assert!(contract.contains("shared-roi-copy-deferred"));
    assert!(contract.contains("native_input_output_multiplier_milli = 2000"));
    assert!(contract.contains("rust_budgeted_private_staging_multiplier_milli = 1000"));
    assert!(contract.contains("rust_peak_multiplier_milli = 3000"));
    assert!(contract.contains("src/develop/pixelpipe_hb.c"));
    assert!(contract.contains("src/develop/pixelpipe_cache.c"));
    assert!(contract.contains("src/develop/imageop_gui.c"));
    assert!(contract.contains("native_pixelpipe_buffer_size_owner"));
    assert!(contract.contains("native_pixelpipe_cache_owner"));
    assert!(contract.contains("boundary_native_shortfall_bytes = 32"));
    assert!(contract.contains("saturation_native_numeric_precision = 2"));
    assert!(contract.contains("rust_endpoint_precision_owner"));
}
