use rusttable_core::{
    Edit, EditId, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_masks::MaskRaster;
use rusttable_processing::common::box_filters::{BOX_ITERATIONS, box_mean};
use rusttable_processing::operations::bloom::{
    BLOOM_PARAMETER_BYTES, BloomConfig, BloomHistory, BloomParametersV1, BloomPixel, BloomPlan,
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

fn lab_pixel(lightness: f32, a: f32, b: f32, alpha: f32) -> BloomPixel {
    BloomPixel::new(lightness, a, b, alpha)
}

#[test]
fn v1_payload_defaults_and_unknown_history_are_typed() {
    let parameters = BloomParametersV1::defaults();
    assert_eq!(parameters.to_bytes().len(), BLOOM_PARAMETER_BYTES);
    assert_eq!(
        BloomParametersV1::from_bytes(&parameters.to_bytes()),
        Ok(parameters)
    );
    assert_eq!(
        BloomHistory::decode(1, &parameters.to_bytes()).expect("v1 history"),
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
    let soften = descriptor::soften_descriptor();
    assert_eq!(soften.io.input.channels, 3);
    assert_eq!(
        soften.io.input.encodings,
        vec![rusttable_color::ColorEncoding::LinearSrgbD65]
    );
    assert_eq!(soften.stage, "display-linear");
    assert!(builtin_registry().definition("rusttable.bloom").is_some());
    assert!(BloomConfig::new(-1.0, 90.0, 25.0).is_err());
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
fn production_bloom_uses_the_shared_eight_pass_box_mean() {
    let config = BloomConfig::new(0.0, 0.0, 25.0).expect("config");
    let plan = BloomPlan::new(config, dimensions(9, 1)).expect("plan");
    // bloom.c truncates `rad` before applying ROI scale.
    assert_eq!(plan.radius(), 2);
    let mut input = vec![lab_pixel(0.0, 12.0, -8.0, 0.25); 9];
    input[4] = lab_pixel(100.0, -24.0, 32.0, 0.75);
    let first = plan
        .execute_lab(&input, None, 1.0, || false)
        .expect("first");
    let second = plan.execute(&input).expect("second");
    assert_eq!(first, second);

    let scale = 1.0 / (-26.0f32 / 100.0).exp2();
    let mut lightness = vec![0.0; 9];
    lightness[4] = 100.0 * scale;
    box_mean(&mut lightness, 1, 9, 1, 2, BOX_ITERATIONS).expect("shared bloom mean");
    for (index, (actual, original)) in first.iter().zip(&input).enumerate() {
        let old = original.lightness();
        let expected = (100.0 - ((100.0 - old) * (100.0 - lightness[index]) / 100.0)) / 100.0;
        assert!((actual.lightness() / 100.0 - expected).abs() < 1.0e-6);
        assert_eq!(actual.a().to_bits(), original.a().to_bits());
        assert_eq!(actual.b().to_bits(), original.b().to_bits());
        assert_eq!(actual.alpha().to_bits(), original.alpha().to_bits());
    }
    assert!(first[0].lightness() > 0.0);
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
