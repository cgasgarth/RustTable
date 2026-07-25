#![allow(
    clippy::float_cmp,
    reason = "compatibility vectors intentionally assert exact f32 results"
)]

use rusttable_color::ColorEncoding;
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_masks::MaskRaster;
use rusttable_processing::descriptor::{
    AlphaPolicy, OperationFlags, ParameterKind, RoiKind, velvia_descriptor,
};
use rusttable_processing::operations::velvia::{
    VELVIA_DEFAULT_BIAS, VELVIA_DEFAULT_STRENGTH, VELVIA_GPU_TIER, VELVIA_V1_PARAMETER_BYTES,
    VELVIA_V2_PARAMETER_BYTES, VELVIA_WGPU_PASS_ID, VelviaConfig, VelviaHistory,
    VelviaParameterError, VelviaParametersV1, VelviaParametersV2, VelviaPixel, VelviaPlan,
    migrate_v1_to_v2, wgpu_passes,
};
use rusttable_processing::{
    CompiledOperationGraph, CompiledPipeline, DeviceCapabilitySnapshot, ExecutionBackend,
    FiniteF32, FrameBoundaryMode, FrameBoundaryOptions, LinearRgb, OperationCompileError,
    OperationMaskSet, ProcessingOperation, ProcessingOperationKind, RasterDimensions,
    WorkingRgbImage, builtin_registry, evaluate_graph_at_frame_boundaries,
    evaluate_graph_at_frame_boundaries_with_masks,
};

fn finite(value: f32) -> FiniteF32 {
    FiniteF32::new(value).expect("finite fixture")
}

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(finite(red), finite(green), finite(blue))
}

fn scalar(value: f64) -> ParameterValue {
    ParameterValue::Scalar(FiniteF64::new(value).expect("finite scalar"))
}

fn operation(
    id: u128,
    opacity: f64,
    parameters: impl IntoIterator<Item = (&'static str, ParameterValue)>,
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("operation ID"),
        OperationKey::new("rusttable.velvia").expect("operation key"),
        true,
        OperationOpacity::new(opacity).expect("opacity"),
        parameters
            .into_iter()
            .map(|(name, value)| (ParameterName::new(name).expect("parameter name"), value)),
    )
    .expect("operation")
}

fn pipeline(opacity: f64) -> CompiledPipeline {
    let operation = operation(
        7,
        opacity,
        [("strength", scalar(25.0)), ("bias", scalar(1.0))],
    );
    CompiledPipeline::compile(
        &Edit::from_parts(
            EditId::new(1).expect("edit ID"),
            PhotoId::new(2).expect("photo ID"),
            Revision::ZERO,
            Revision::ZERO,
            [operation],
        )
        .expect("edit"),
    )
    .expect("pipeline")
}

fn channel_bits(pixel: LinearRgb) -> [u32; 3] {
    [
        pixel.red().get().to_bits(),
        pixel.green().get().to_bits(),
        pixel.blue().get().to_bits(),
    ]
}

#[test]
fn v1_and_v2_codecs_are_exact_little_endian_and_preserve_unknown_history() {
    let v1 = VelviaParametersV1::new(50.0, 80.0, 0.25, 0.75);
    assert_eq!(v1.to_bytes().len(), VELVIA_V1_PARAMETER_BYTES);
    assert_eq!(
        v1.to_bytes(),
        [
            0x00, 0x00, 0x48, 0x42, 0x00, 0x00, 0xa0, 0x42, 0x00, 0x00, 0x80, 0x3e, 0x00, 0x00,
            0x40, 0x3f,
        ]
    );
    assert_eq!(VelviaParametersV1::from_bytes(&v1.to_bytes()), Ok(v1));

    let defaults = VelviaParametersV2::defaults();
    assert_eq!(defaults.strength, VELVIA_DEFAULT_STRENGTH);
    assert_eq!(defaults.bias, VELVIA_DEFAULT_BIAS);
    assert_eq!(defaults.to_bytes().len(), VELVIA_V2_PARAMETER_BYTES);
    assert_eq!(
        defaults.to_bytes(),
        [0x00, 0x00, 0xc8, 0x41, 0x00, 0x00, 0x80, 0x3f]
    );
    assert_eq!(
        VelviaParametersV2::from_bytes(&defaults.to_bytes()),
        Ok(defaults)
    );

    let opaque_bytes = vec![0xde, 0xad, 0xbe, 0xef, 0x80];
    let opaque = VelviaHistory::decode(9, &opaque_bytes).expect("unknown history");
    assert_eq!(opaque.version(), 9);
    assert_eq!(opaque.payload(), opaque_bytes);
    assert!(opaque.current().is_err());
}

#[test]
fn checked_benchmark_xmp_decodes_the_default_v2_payload_and_order_neighbors() {
    let xmp = include_str!("../../../src/tests/benchmark/darktable-bench-4.2.xmp");
    let velvia_history = xmp
        .split_once("darktable:operation=\"velvia\"")
        .expect("checked benchmark contains Velvia")
        .1;
    let payload = velvia_history
        .split_once("darktable:params=\"")
        .expect("Velvia history contains parameters")
        .1
        .split_once('"')
        .expect("Velvia parameter quote")
        .0;
    assert_eq!(payload, "0000c8410000803f");
    let (pairs, remainder) = payload.as_bytes().as_chunks::<2>();
    assert!(remainder.is_empty(), "checked hex has complete bytes");
    let bytes = pairs
        .iter()
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("ASCII hex"), 16)
                .expect("valid checked hex")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        VelviaHistory::decode(2, &bytes).expect("checked Velvia history"),
        VelviaHistory::V2(VelviaParametersV2::defaults())
    );
    assert!(xmp.contains("colorcontrast,0,velvia,0,vibrance,0"));
}

#[test]
fn v1_migration_uses_native_f32_order_and_ignores_clarity() {
    let migrated = migrate_v1_to_v2(VelviaParametersV1::new(50.0, 80.0, 0.25, 0.75));
    assert_eq!(migrated, VelviaParametersV2::new(40.0, 0.25));
    assert_eq!(
        migrated.to_bytes(),
        [0x00, 0x00, 0x20, 0x42, 0x00, 0x00, 0x80, 0x3e]
    );

    let order_discriminator = migrate_v1_to_v2(VelviaParametersV1::new(
        f32::from_bits(0x3cfd_5c5f),
        f32::from_bits(0x42b0_f024),
        0.75,
        f32::NAN,
    ));
    assert_eq!(order_discriminator.strength.to_bits(), 0x3ce0_2557);
    assert_eq!(order_discriminator.bias.to_bits(), 0.75_f32.to_bits());

    let history = VelviaHistory::decode(
        1,
        &VelviaParametersV1::new(50.0, 80.0, 0.25, 1.0).to_bytes(),
    )
    .expect("v1 history");
    assert_eq!(
        history.current().expect("known migration"),
        VelviaParametersV2::new(40.0, 0.25)
    );
}

#[test]
fn runtime_config_accepts_finite_history_values_and_rejects_nonfinite_values() {
    assert_eq!(
        VelviaConfig::new(VELVIA_DEFAULT_STRENGTH, VELVIA_DEFAULT_BIAS),
        Ok(VelviaConfig::defaults())
    );
    assert_eq!(
        VelviaConfig::new(101.0, -0.01)
            .expect("finite persisted values are not clamped")
            .parameters(),
        VelviaParametersV2::new(101.0, -0.01)
    );
    assert_eq!(
        VelviaConfig::new(-1.0, 2.0)
            .expect("finite persisted values are not rejected")
            .parameters(),
        VelviaParametersV2::new(-1.0, 2.0)
    );
    assert_eq!(
        VelviaConfig::new(f32::NAN, 1.0),
        Err(VelviaParameterError::NonFinite("strength"))
    );
    assert_eq!(
        VelviaConfig::new(25.0, f32::INFINITY),
        Err(VelviaParameterError::NonFinite("bias"))
    );
}

#[test]
fn scalar_math_matches_darktable_golden_vectors_and_clamps_hdr_rgb() {
    let default_plan = VelviaPlan::new(VelviaConfig::defaults());
    let output = default_plan.execute(&[pixel(0.4, 0.3, 0.2)]);
    assert_eq!(
        channel_bits(output[0]),
        [0x3ed6_6671, 0x3e99_999a, 0x3e39_9985]
    );

    let full_plan = VelviaPlan::new(VelviaConfig::new(100.0, 0.5).expect("config"));
    let output = full_plan.execute(&[pixel(0.8, 0.4, 0.2)]);
    assert_eq!(
        channel_bits(output[0]),
        [0x3f80_0000, 0x3eb8_51d8, 0x3d23_d488]
    );

    let clipped =
        default_plan.execute(&[pixel(2.0, -1.0, 0.5), pixel(f32::MAX, f32::MAX, f32::MAX)]);
    assert_eq!(clipped[0], pixel(1.0, 0.0, 0.5));
    assert_eq!(clipped[1], pixel(0.0, 0.0, 0.0));

    let strength_above_slider = VelviaPlan::new(
        VelviaConfig::new(101.0, 1.0).expect("finite persisted strength is executable"),
    )
    .execute(&[pixel(0.4, 0.3, 0.2)]);
    assert_eq!(
        channel_bits(strength_above_slider[0]),
        [0x3ef3_95ac, 0x3e99_999a, 0x3dfe_761f]
    );

    let bias_below_slider = VelviaPlan::new(
        VelviaConfig::new(25.0, -0.01).expect("finite persisted bias is executable"),
    )
    .execute(&[pixel(0.4, 0.3, 0.2)]);
    assert_eq!(
        channel_bits(bias_below_slider[0]),
        [0x3edf_1544, 0x3e99_999a, 0x3e28_3bdf]
    );
}

#[test]
fn nonpositive_normalized_strength_is_bit_identical_and_positive_strength_preserves_alpha() {
    let source = VelviaPixel::from_channels([
        f32::from_bits(0x8000_0000),
        f32::from_bits(0x7fc0_1234),
        f32::MAX,
        f32::from_bits(0x7fc0_5678),
    ]);
    let tiny_positive = f32::from_bits(1);
    assert_eq!(
        VelviaConfig::new(tiny_positive, 1.0)
            .expect("smallest positive strength")
            .normalized_strength()
            .to_bits(),
        0.0_f32.to_bits(),
        "native strength / 100 underflows before the identity branch"
    );
    for strength in [0.0, -1.0, tiny_positive] {
        let identity = VelviaPlan::new(VelviaConfig::new(strength, 1.0).expect("identity config"))
            .execute_rgba(&[source]);
        assert_eq!(
            identity[0].channels().map(f32::to_bits),
            source.channels().map(f32::to_bits)
        );
    }

    let alpha = f32::from_bits(0x8000_0000);
    let processed = VelviaPlan::new(VelviaConfig::defaults())
        .execute_rgba(&[VelviaPixel::new(0.4, 0.3, 0.2, alpha)]);
    assert_eq!(processed[0].alpha().to_bits(), alpha.to_bits());
}

#[test]
fn descriptor_registry_order_and_migration_evidence_are_explicit() {
    let descriptor = velvia_descriptor();
    descriptor.validate().expect("descriptor");
    for flag in [
        OperationFlags::MULTI_INSTANCE,
        OperationFlags::STYLE_ELIGIBLE,
        OperationFlags::HISTORY_VISIBLE,
        OperationFlags::TILEABLE,
        OperationFlags::DETERMINISTIC_CPU,
        OperationFlags::DETERMINISTIC_GPU,
        OperationFlags::COLOR,
        OperationFlags::MASKS,
        OperationFlags::BLENDING,
    ] {
        assert!(descriptor.flags.contains(flag), "missing flag {flag:?}");
    }
    assert_eq!(descriptor.stage, "scene-linear-rgb");
    assert_eq!(descriptor.roi, RoiKind::Identity);
    assert_eq!(descriptor.tiling.overlap_pixels, 0);
    assert_eq!(descriptor.io.input.alpha, AlphaPolicy::Preserve);
    assert_eq!(descriptor.io.output.alpha, AlphaPolicy::Preserve);
    assert!(descriptor.mask_blend.consumes_mask);
    assert!(descriptor.mask_blend.blend_if);
    assert_eq!(descriptor.migration.source_versions, [1, 2]);
    assert_eq!(descriptor.migration.target_version, 2);
    assert!(descriptor.migration.opaque_unknown_allowed);
    assert!(matches!(
        &descriptor.parameters[0].kind,
        ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 100.0
        }
    ));
    assert!(matches!(
        &descriptor.parameters[1].kind,
        ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 1.0
        }
    ));
    assert_eq!(descriptor.capability.gpu_tier, Some(VELVIA_GPU_TIER));
    assert_eq!(
        descriptor.capability.required_features,
        ["f32-storage", "deterministic-row-major"]
    );
    assert_eq!(descriptor.capability.required_formats, ["rgba32float"]);
    assert!(descriptor.capability.deterministic_gpu);
    assert!(descriptor.capability.fallback_to_cpu);
    assert_eq!(wgpu_passes(), [VELVIA_WGPU_PASS_ID]);

    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.velvia")
        .expect("registry definition");
    assert!(definition.cpu().is_some());
    let gpu = definition.gpu().expect("qualified Velvia GPU binding");
    assert_eq!(gpu.binding_id(), VELVIA_WGPU_PASS_ID);
    assert_eq!(gpu.tier(), VELVIA_GPU_TIER);
    assert_eq!(
        gpu.required_features(),
        ["f32-storage", "deterministic-row-major"]
    );
    assert_eq!(gpu.required_formats(), ["rgba32float"]);
    assert_eq!(definition.migrations().len(), 1);
    assert_eq!(definition.migrations()[0].from_version(), 1);
    assert_eq!(definition.migrations()[0].to_version(), 2);
    assert_eq!(
        definition.migrations()[0].evidence_id(),
        "velvia.migration.v1-v2"
    );
    assert!(
        definition
            .evidence_ids()
            .iter()
            .any(|evidence| evidence == "iop.velvia.order.v30-57")
    );
    assert!(
        definition
            .evidence_ids()
            .iter()
            .any(|evidence| evidence == "iop.velvia.wgpu.point.unmasked-full-opacity")
    );

    let capable_device = DeviceCapabilitySnapshot::gpu(
        VELVIA_GPU_TIER,
        [
            "f32-storage".to_owned(),
            "deterministic-row-major".to_owned(),
        ],
        ["rgba32float".to_owned()],
    );
    let capability = registry
        .capability(
            "rusttable.velvia",
            &capable_device,
            ColorEncoding::LinearSrgbD65,
            Some("preview"),
        )
        .expect("Velvia capability");
    assert_eq!(capability.backend, ExecutionBackend::Gpu);
    assert!(capability.available);

    let missing_point_requirement = DeviceCapabilitySnapshot::gpu(
        VELVIA_GPU_TIER,
        ["f32-storage".to_owned()],
        ["rgba32float".to_owned()],
    );
    let fallback = registry
        .capability(
            "rusttable.velvia",
            &missing_point_requirement,
            ColorEncoding::LinearSrgbD65,
            Some("preview"),
        )
        .expect("Velvia CPU fallback");
    assert_eq!(fallback.backend, ExecutionBackend::CpuFallback);
    assert!(fallback.available);

    let order = registry
        .definitions_in_declaration_order()
        .into_iter()
        .map(|definition| definition.descriptor().id.compatibility_name.as_str())
        .collect::<Vec<_>>();
    let relight = order
        .iter()
        .position(|id| *id == "relight")
        .expect("relight");
    let velvia = order.iter().position(|id| *id == "velvia").expect("velvia");
    assert_eq!(velvia, relight + 1);
}

#[test]
fn operation_kind_compilation_is_checked_and_defaults_are_materializable() {
    let compiled = ProcessingOperation::compile(&operation(
        11,
        1.0,
        [("strength", scalar(40.0)), ("bias", scalar(0.25))],
    ))
    .expect("compiled Velvia");
    assert_eq!(
        compiled.kind(),
        &ProcessingOperationKind::Velvia {
            config: VelviaConfig::new(40.0, 0.25).expect("config")
        }
    );

    let default = builtin_registry()
        .materialize_operation(
            "rusttable.velvia",
            OperationId::new(12).expect("operation ID"),
        )
        .expect("default operation");
    assert_eq!(
        ProcessingOperation::compile(&default)
            .expect("compiled defaults")
            .kind(),
        &ProcessingOperationKind::Velvia {
            config: VelviaConfig::defaults()
        }
    );

    let out_of_ui_range = operation(
        13,
        1.0,
        [("strength", scalar(101.0)), ("bias", scalar(-0.01))],
    );
    assert_eq!(
        ProcessingOperation::compile(&out_of_ui_range)
            .expect("finite persisted state compiles")
            .kind(),
        &ProcessingOperationKind::Velvia {
            config: VelviaConfig::new(101.0, -0.01).expect("finite history config")
        }
    );

    let unexpected = operation(
        14,
        1.0,
        [
            ("strength", scalar(25.0)),
            ("bias", scalar(1.0)),
            ("clarity", scalar(0.0)),
        ],
    );
    assert!(matches!(
        ProcessingOperation::compile(&unexpected),
        Err(OperationCompileError::UnexpectedParameter { .. })
    ));
}

#[test]
fn graph_evaluation_uses_shared_opacity_mask_reconstruction_and_preserves_alpha() {
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let input = WorkingRgbImage::new(dimensions, vec![pixel(0.4, 0.3, 0.2)]).expect("input");
    let alpha = vec![f32::from_bits(0x3eab_cdef)];
    let operation_id = OperationId::new(7).expect("operation ID");
    let masks = OperationMaskSet::from_entries([(
        operation_id,
        MaskRaster::new(1, 1, vec![0.5]).expect("mask"),
    )])
    .expect("mask set");

    let half_graph = CompiledOperationGraph::from_pipeline(&pipeline(0.5));
    let masked_half = evaluate_graph_at_frame_boundaries_with_masks(
        &half_graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        Some(&masks),
        || false,
    )
    .expect("masked half");
    let quarter_graph = CompiledOperationGraph::from_pipeline(&pipeline(0.25));
    let unmasked_quarter = evaluate_graph_at_frame_boundaries(
        &quarter_graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    )
    .expect("unmasked quarter");

    assert_eq!(
        masked_half.image().pixel_slice(),
        unmasked_quarter.image().pixel_slice()
    );
    assert_eq!(masked_half.alpha()[0].to_bits(), alpha[0].to_bits());
    assert_eq!(unmasked_quarter.alpha()[0].to_bits(), alpha[0].to_bits());
}
