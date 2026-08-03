#![expect(
    clippy::suboptimal_flops,
    reason = "Native Bloom blend test vectors preserve source evaluation order and IEEE-754 parity."
)]

use rusttable_core::{
    Edit, EditId, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_masks::MaskRaster;
use rusttable_processing::common::box_filters::{BOX_ITERATIONS, box_mean};
use rusttable_processing::operations::bloom::{
    BLOOM_BOX_ITERATIONS, BLOOM_CPU_REQUIRES_FULL_FRAME, BLOOM_CPU_ROI_SCALE, BLOOM_GPU_TIER,
    BLOOM_OPENCL_NUM_BUCKETS, BLOOM_PARAMETER_BYTES, BLOOM_WGPU_PASS_ID, BloomConfig, BloomHistory,
    BloomParametersV1, BloomPixel, BloomPlan,
};
use rusttable_processing::{
    CompiledOperationGraph, CompiledPipeline, FiniteF32, FrameBoundaryMode, FrameBoundaryOptions,
    LinearRgb, OperationMaskSet, RasterDimensions, WorkingFrameDescriptor, WorkingRgbImage,
    builtin_registry, descriptor, evaluate, evaluate_graph_at_frame_boundaries,
    evaluate_graph_at_frame_boundaries_with_masks,
};

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

const fn lab_pixel(lightness: f32, a: f32, b: f32, alpha: f32) -> BloomPixel {
    BloomPixel::new(lightness, a, b, alpha)
}

const _: () = assert!(BLOOM_CPU_REQUIRES_FULL_FRAME);

#[test]
fn v1_payload_is_three_little_endian_native_floats_in_declaration_order() {
    let parameters = BloomParametersV1::defaults();
    let expected = [
        0x00, 0x00, 0xa0, 0x41, // size = 20
        0x00, 0x00, 0xb4, 0x42, // threshold = 90
        0x00, 0x00, 0xc8, 0x41, // strength = 25
    ];
    assert_eq!(BLOOM_PARAMETER_BYTES, 3 * std::mem::size_of::<f32>());
    assert_eq!(parameters.to_bytes(), expected);
    assert_eq!(BloomParametersV1::from_bytes(&expected), Ok(parameters));
    assert_eq!(
        BloomHistory::decode(1, &expected).expect("v1 history"),
        BloomHistory::V1(parameters)
    );
    assert_eq!(
        BloomHistory::decode(9, &[1, 2, 3]).expect("future history"),
        BloomHistory::Opaque {
            version: 9,
            bytes: vec![1, 2, 3]
        }
    );
}

#[test]
fn v1_payload_preservation_is_distinct_from_executable_validation() {
    let source_bits = [0xc0a0_0000_u32, 0x7fc0_1234, 0x42cb_0000];
    let mut payload = [0_u8; BLOOM_PARAMETER_BYTES];
    for (field, bits) in source_bits.into_iter().enumerate() {
        payload[field * 4..field * 4 + 4].copy_from_slice(&bits.to_le_bytes());
    }

    let parameters = BloomParametersV1::from_bytes(&payload).expect("typed native payload");
    assert_eq!(parameters.size.to_bits(), source_bits[0]);
    assert_eq!(parameters.threshold.to_bits(), source_bits[1]);
    assert_eq!(parameters.strength.to_bits(), source_bits[2]);
    assert_eq!(parameters.to_bytes(), payload);
    assert!(BloomConfig::try_from(parameters).is_err());
    assert_eq!(
        BloomHistory::decode(1, &payload)
            .expect("known history remains typed")
            .payload(),
        payload
    );
    assert!(BloomParametersV1::from_bytes(&payload[..11]).is_err());
}

#[test]
fn descriptor_registry_and_validation_match_the_backend_contract() {
    let descriptor = descriptor::bloom_descriptor();
    descriptor.validate().expect("bloom descriptor");
    assert_eq!(descriptor.id.compatibility_name, "bloom");
    assert_eq!(descriptor.id.parameter_version, 1);
    assert_eq!(
        descriptor.roi,
        rusttable_processing::descriptor::RoiKind::FullImage
    );
    assert_eq!(
        descriptor.io.output.alpha,
        rusttable_processing::descriptor::AlphaPolicy::Preserve
    );
    assert_eq!(descriptor.io.input.channels, 4);
    assert_eq!(
        descriptor.io.input.encodings,
        vec![rusttable_color::ColorEncoding::LabD50]
    );
    assert_eq!(descriptor.stage, "display-referred-lab");
    assert!(descriptor.mask_blend.consumes_mask);
    assert!(!descriptor.mask_blend.analysis);
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::MULTI_INSTANCE)
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::FULL_IMAGE)
    );
    assert!(
        !descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::TILEABLE),
        "scaled overlap tiling execution remains explicitly deferred",
    );
    assert_eq!(descriptor.capability.gpu_tier, Some(BLOOM_GPU_TIER));
    let definition = builtin_registry()
        .definition("rusttable.bloom")
        .expect("Bloom definition");
    let gpu = definition.gpu().expect("Bloom GPU binding");
    assert_eq!(gpu.binding_id(), BLOOM_WGPU_PASS_ID);
    assert_eq!(gpu.tier(), BLOOM_GPU_TIER);
    let soften = descriptor::soften_descriptor();
    assert_eq!(soften.io.input.channels, 4);
    assert_eq!(
        soften.io.input.encodings,
        vec![rusttable_color::ColorEncoding::LinearSrgbD65]
    );
    assert_eq!(soften.stage, "display-linear");
    assert!(builtin_registry().definition("rusttable.bloom").is_some());
    assert!(BloomConfig::new(-1.0, 90.0, 25.0).is_err());
}

#[test]
fn bloom_registry_order_follows_color_zones() {
    let executable = builtin_registry()
        .definitions_in_declaration_order()
        .into_iter()
        .map(|definition| definition.descriptor().id.compatibility_name.as_str())
        .collect::<Vec<_>>();
    let colorzones = executable
        .iter()
        .position(|name| *name == "colorzones")
        .expect("Color Zones registry entry");
    let bloom = executable
        .iter()
        .position(|name| *name == "bloom")
        .expect("Bloom registry entry");
    assert!(colorzones < bloom);
}

#[test]
fn executable_validation_checks_every_finite_native_range() {
    for parameters in [
        BloomParametersV1::new(f32::NAN, 90.0, 25.0),
        BloomParametersV1::new(20.0, f32::INFINITY, 25.0),
        BloomParametersV1::new(20.0, 90.0, f32::NEG_INFINITY),
        BloomParametersV1::new(-f32::EPSILON, 90.0, 25.0),
        BloomParametersV1::new(20.0, f32::from_bits(100.0_f32.to_bits() + 1), 25.0),
        BloomParametersV1::new(20.0, 90.0, f32::from_bits(100.0_f32.to_bits() + 1)),
    ] {
        assert!(BloomConfig::try_from(parameters).is_err());
        assert_eq!(
            BloomParametersV1::from_bytes(&parameters.to_bytes())
                .expect("payload representation")
                .to_bytes(),
            parameters.to_bytes()
        );
    }
}

#[test]
fn retained_execution_constants_and_current_cpu_limits_are_explicit() {
    assert_eq!(BLOOM_OPENCL_NUM_BUCKETS, 4);
    assert_eq!(BLOOM_BOX_ITERATIONS, 8);
    assert_eq!(BLOOM_BOX_ITERATIONS, BOX_ITERATIONS);
    assert_eq!(BLOOM_CPU_ROI_SCALE.to_bits(), 1.0_f32.to_bits());
}

#[test]
fn shared_box_mean_normalizes_border_windows_by_their_available_samples() {
    // src/common/box_filters.cc divides by the moving `hits` count instead of
    // repeating edge pixels into a fixed-width window.
    let mut samples = vec![1.0, 0.0, 0.0, 0.0, 0.0];
    box_mean(&mut samples, 1, 5, 1, 1, 1).expect("box mean");
    let expected = [0.5, 1.0 / 3.0, 0.0, 0.0, 0.0];
    for (actual, expected) in samples.iter().zip(expected) {
        assert!((actual - expected).abs() < 1.0e-6);
    }
}

#[test]
fn scale_one_radius_vectors_preserve_float_to_int_truncation() {
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
            BloomConfig::new(size, 90.0, 25.0).expect("config"),
            dimensions(1, 1),
        )
        .expect("plan");
        assert_eq!(plan.radius(), expected, "size {size}");
    }
}

#[test]
fn production_bloom_runs_eight_horizontal_vertical_box_iterations() {
    let config = BloomConfig::new(0.0, 0.0, 25.0).expect("config");
    let plan = BloomPlan::new(config, dimensions(9, 1)).expect("plan");
    assert_eq!(plan.radius(), 2);
    let mut input = vec![lab_pixel(0.0, 12.0, -8.0, 0.25); 9];
    input[4] = lab_pixel(100.0, -24.0, 32.0, 0.75);
    let first = plan
        .execute_lab(&input, None, 1.0, || false)
        .expect("first");
    let second = plan.execute(&input).expect("second");
    assert_eq!(first, second);

    // bloom.c threshold/strength transforms followed by eight separable H/V
    // iterations and the final screen equation, evaluated in source f32 order.
    let expected_lightness = [
        15.236_61, 15.301_178, 15.347_45, 15.422_76, 100.0, 15.422_76, 15.347_443, 15.301_178,
        15.236_61,
    ];
    for ((actual, original), expected) in first.iter().zip(&input).zip(expected_lightness) {
        assert!((actual.lightness() - expected).abs() < 1.0e-5);
        assert_eq!(actual.a().to_bits(), original.a().to_bits());
        assert_eq!(actual.b().to_bits(), original.b().to_bits());
        assert_eq!(actual.alpha().to_bits(), original.alpha().to_bits());
    }
}

#[test]
fn source_vector_matches_threshold_strength_blur_and_screen_equations() {
    let plan = BloomPlan::new(
        BloomConfig::new(0.0, 50.0, 25.0).expect("config"),
        dimensions(2, 2),
    )
    .expect("plan");
    let input = [
        lab_pixel(10.0, 1.0, -2.0, 0.1),
        lab_pixel(40.0, 3.0, -4.0, 0.2),
        lab_pixel(80.0, 5.0, -6.0, 0.3),
        lab_pixel(100.0, 7.0, -8.0, 0.4),
    ];
    let output = plan.execute(&input).expect("bloom");
    let expected_lightness = [58.497_89, 72.331_924, 90.777_306, 100.0];

    for ((actual, source), expected) in output.iter().zip(input).zip(expected_lightness) {
        assert!((actual.lightness() - expected).abs() < 1.0e-5);
        assert_eq!(actual.a().to_bits(), source.a().to_bits());
        assert_eq!(actual.b().to_bits(), source.b().to_bits());
        assert_eq!(actual.alpha().to_bits(), source.alpha().to_bits());
    }
}

#[test]
fn colored_lab_input_preserves_chroma_and_blends_opacity_only_on_lightness() {
    let dimensions = dimensions(2, 1);
    let plan = BloomPlan::new(
        BloomConfig::new(0.0, 0.0, 25.0).expect("config"),
        dimensions,
    )
    .expect("plan");
    let input = [
        lab_pixel(45.0, 72.0, -61.0, 0.2),
        lab_pixel(45.0, -54.0, 38.0, 0.8),
    ];
    let full = plan
        .execute_lab(&input, None, 1.0, || false)
        .expect("full opacity");
    let half = plan
        .execute_lab(&input, None, 0.5, || false)
        .expect("half opacity");

    assert_eq!(full[0].lightness().to_bits(), full[1].lightness().to_bits());
    for ((source, full), half) in input.iter().zip(full).zip(half) {
        let expected_half = source.lightness() + (full.lightness() - source.lightness()) * 0.5;
        assert_eq!(half.lightness().to_bits(), expected_half.to_bits());
        assert_eq!(full.a().to_bits(), source.a().to_bits());
        assert_eq!(full.b().to_bits(), source.b().to_bits());
        assert_eq!(full.alpha().to_bits(), source.alpha().to_bits());
        assert_eq!(half.a().to_bits(), source.a().to_bits());
        assert_eq!(half.b().to_bits(), source.b().to_bits());
        assert_eq!(half.alpha().to_bits(), source.alpha().to_bits());
    }
}

#[test]
fn mask_and_opacity_are_combined_on_lab_lightness() {
    let plan = BloomPlan::new(
        BloomConfig::new(0.0, 0.0, 25.0).expect("config"),
        dimensions(2, 1),
    )
    .expect("plan");
    let input = [
        lab_pixel(45.0, 72.0, -61.0, 0.2),
        lab_pixel(45.0, -54.0, 38.0, 0.8),
    ];
    let full = plan
        .execute_lab(&input, None, 1.0, || false)
        .expect("full opacity");
    let mask = [0.25, 0.75];
    let masked = plan
        .execute_lab(&input, Some(&mask), 0.5, || false)
        .expect("masked half opacity");

    for (index, ((source, candidate), actual)) in input.iter().zip(full).zip(masked).enumerate() {
        let coverage = mask[index] * 0.5;
        let expected = source.lightness() + (candidate.lightness() - source.lightness()) * coverage;
        assert_eq!(actual.lightness().to_bits(), expected.to_bits());
        assert_eq!(actual.a().to_bits(), source.a().to_bits());
        assert_eq!(actual.b().to_bits(), source.b().to_bits());
        assert_eq!(actual.alpha().to_bits(), source.alpha().to_bits());
    }
}

#[test]
fn strength_100_caps_at_two_before_the_strict_threshold() {
    let input = [lab_pixel(50.0, 36.0, -22.0, 0.4)];
    let output = BloomPlan::new(
        BloomConfig::new(0.0, 100.0, 100.0).expect("config"),
        dimensions(1, 1),
    )
    .expect("plan")
    .execute(&input)
    .expect("bloom");

    // min(strength + 1, 100) makes the scaled L exactly 100. The retained
    // comparison is strict, so equality does not enter the glow buffer.
    assert_eq!(output, input);
}

#[test]
fn production_bloom_restores_rec2020_encoding_and_blends_in_lab() {
    let dimensions = dimensions(3, 1);
    let input = WorkingRgbImage::new_with_frame(
        dimensions,
        vec![
            pixel(0.75, 0.35, 0.08),
            pixel(0.18, 0.68, 0.32),
            pixel(0.24, 0.38, 0.82),
        ],
        WorkingFrameDescriptor::rec2020(),
    )
    .expect("input");
    let full = evaluate(&bloom_pipeline(1.0), &input).expect("full-opacity bloom");
    let half = evaluate(&bloom_pipeline(0.5), &input).expect("half-opacity bloom");

    assert_eq!(full.frame(), input.frame());
    assert_eq!(half.frame(), input.frame());
    assert_eq!(
        half.frame().encoding(),
        rusttable_color::ColorEncoding::LinearRec2020D65
    );
    assert_ne!(full.pixel_slice(), input.pixel_slice());
    let differs_from_rgb_midpoint = half
        .pixel_slice()
        .iter()
        .zip(full.pixel_slice())
        .zip(input.pixel_slice())
        .any(|((actual, candidate), source)| {
            let legacy = [
                source.red().get() + (candidate.red().get() - source.red().get()) * 0.5,
                source.green().get() + (candidate.green().get() - source.green().get()) * 0.5,
                source.blue().get() + (candidate.blue().get() - source.blue().get()) * 0.5,
            ];
            actual.red().get().to_bits() != legacy[0].to_bits()
                || actual.green().get().to_bits() != legacy[1].to_bits()
                || actual.blue().get().to_bits() != legacy[2].to_bits()
        });
    assert!(
        differs_from_rgb_midpoint,
        "colored fixture must distinguish Lab opacity from a later RGB blend"
    );
}

#[test]
fn graph_mask_and_opacity_are_combined_once_inside_the_bloom_lab_boundary() {
    let dimensions = dimensions(3, 1);
    let input = WorkingRgbImage::new_with_frame(
        dimensions,
        vec![
            pixel(0.75, 0.35, 0.08),
            pixel(0.18, 0.68, 0.32),
            pixel(0.24, 0.38, 0.82),
        ],
        WorkingFrameDescriptor::rec2020(),
    )
    .expect("input");
    let operation_id = OperationId::new(7).expect("operation ID");
    let alpha = vec![1.0; input.pixel_slice().len()];
    let mask = MaskRaster::new(
        dimensions.width(),
        dimensions.height(),
        vec![0.5; input.pixel_slice().len()],
    )
    .expect("mask");
    let masks = OperationMaskSet::from_entries([(operation_id, mask)]).expect("operation mask set");
    let half_graph = CompiledOperationGraph::from_pipeline(&bloom_pipeline(0.5));

    let masked_half = evaluate_graph_at_frame_boundaries_with_masks(
        &half_graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        Some(&masks),
        || false,
    )
    .expect("masked half-opacity Bloom")
    .image()
    .clone();
    let quarter_graph = CompiledOperationGraph::from_pipeline(&bloom_pipeline(0.25));
    let unmasked_quarter = evaluate_graph_at_frame_boundaries(
        &quarter_graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    )
    .expect("quarter-opacity Bloom")
    .image()
    .clone();

    assert_eq!(masked_half.pixel_slice(), unmasked_quarter.pixel_slice());

    let unmasked_half = evaluate_graph_at_frame_boundaries(
        &half_graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    )
    .expect("half-opacity Bloom")
    .image()
    .clone();
    let differs_from_post_rgb_blend = masked_half
        .pixel_slice()
        .iter()
        .zip(unmasked_half.pixel_slice())
        .zip(input.pixel_slice())
        .any(|((actual, candidate), source)| {
            let legacy = [
                source.red().get() + (candidate.red().get() - source.red().get()) * 0.5,
                source.green().get() + (candidate.green().get() - source.green().get()) * 0.5,
                source.blue().get() + (candidate.blue().get() - source.blue().get()) * 0.5,
            ];
            actual.red().get().to_bits() != legacy[0].to_bits()
                || actual.green().get().to_bits() != legacy[1].to_bits()
                || actual.blue().get().to_bits() != legacy[2].to_bits()
        });
    assert!(
        differs_from_post_rgb_blend,
        "colored fixture must distinguish native Lab coverage from a later RGB blend"
    );
}

fn bloom_pipeline(opacity: f64) -> CompiledPipeline {
    let operation = Operation::new_with_opacity(
        OperationId::new(7).expect("operation ID"),
        OperationKey::new("rusttable.bloom").expect("operation key"),
        true,
        OperationOpacity::new(opacity).expect("opacity"),
        std::iter::empty::<(ParameterName, ParameterValue)>(),
    )
    .expect("default bloom");
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::ZERO,
        [operation],
    )
    .expect("edit");
    CompiledPipeline::compile(&edit).expect("pipeline")
}
