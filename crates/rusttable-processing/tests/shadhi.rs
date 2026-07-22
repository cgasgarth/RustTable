#![allow(
    clippy::float_cmp,
    reason = "compatibility tests assert stable scalar values"
)]

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_processing::descriptor::OperationFlags;
use rusttable_processing::operations::shadhi::{
    SHADHI_V1_PARAMETER_BYTES, SHADHI_V5_PARAMETER_BYTES, ShadhiAlgorithm, ShadhiConfig,
    ShadhiHistory, ShadhiParametersV1, ShadhiParametersV5, ShadhiPixel, ShadhiPlan,
    migrate_v1_to_v5,
};
use rusttable_processing::{
    CompiledOperationGraph, CompiledPipeline, FiniteF32, FrameBoundaryMode, FrameBoundaryOptions,
    LinearRgb, RasterDimensions, WorkingFrameDescriptor, WorkingRgbImage, builtin_registry,
    descriptor, evaluate_graph_at_frame_boundaries,
};

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("finite red"),
        FiniteF32::new(green).expect("finite green"),
        FiniteF32::new(blue).expect("finite blue"),
    )
}

fn gaussian_config() -> ShadhiConfig {
    ShadhiConfig::new(ShadhiParametersV5 {
        shadhi_algo: ShadhiAlgorithm::Gaussian.id(),
        radius: 10.0,
        ..ShadhiParametersV5::defaults()
    })
    .expect("Gaussian config")
}

#[test]
fn typed_legacy_layouts_migrate_and_unknown_payloads_round_trip() {
    let old = ShadhiParametersV1 {
        order: 0,
        radius: -100.0,
        shadows: 40.0,
        reserved1: 2.0,
        highlights: 20.0,
        reserved2: 0.0,
        compress: 50.0,
    };
    let migrated = migrate_v1_to_v5(old);
    assert_eq!(migrated.radius, 100.0);
    assert_eq!(migrated.shadows, 20.0);
    assert_eq!(migrated.highlights, -10.0);
    assert_eq!(migrated.shadhi_algo, ShadhiAlgorithm::Bilateral.id());
    assert_eq!(
        old.radius.to_le_bytes().len() + 24,
        SHADHI_V1_PARAMETER_BYTES
    );
    let current = ShadhiParametersV5::defaults();
    assert_eq!(current.to_bytes().len(), SHADHI_V5_PARAMETER_BYTES);
    assert_eq!(
        ShadhiHistory::decode(5, &current.to_bytes())
            .expect("v5 history")
            .payload(),
        current.to_bytes().to_vec()
    );
    let opaque = ShadhiHistory::decode(99, &[7, 8, 9]).expect("unknown history is retained");
    assert_eq!(opaque.payload(), vec![7, 8, 9]);
    assert_eq!(opaque.version(), 99);
}

#[test]
fn lab_plan_executes_both_algorithms_and_is_deterministic() {
    let dimensions = RasterDimensions::new(3, 3).expect("dimensions");
    let input = vec![
        pixel(0.02, 0.04, 0.08),
        pixel(0.2, 0.3, 0.4),
        pixel(0.8, 0.7, 0.6),
        pixel(0.1, 0.2, 0.3),
        pixel(0.4, 0.5, 0.6),
        pixel(0.9, 0.8, 0.7),
        pixel(0.05, 0.06, 0.07),
        pixel(0.3, 0.2, 0.1),
        pixel(0.95, 0.9, 0.85),
    ];
    let lab_input = input
        .iter()
        .map(|value| {
            ShadhiPixel::new(
                value.red().get() * 100.0,
                value.green().get() * 128.0,
                value.blue().get() * 128.0,
                1.0,
            )
        })
        .collect::<Vec<_>>();
    let gaussian = ShadhiPlan::new(gaussian_config(), dimensions).expect("Gaussian plan");
    let first = gaussian
        .execute_lab(&lab_input, None, 1.0, || false)
        .expect("first execution");
    assert_eq!(
        first,
        gaussian
            .execute_lab(&lab_input, None, 1.0, || false)
            .expect("second execution")
    );
    assert_ne!(first, lab_input);

    let bilateral =
        ShadhiPlan::new(ShadhiConfig::defaults(), dimensions).expect("default bilateral plan");
    let output = bilateral
        .execute_lab(&lab_input, None, 1.0, || false)
        .expect("default bilateral execution");
    assert_eq!(output.len(), lab_input.len());
    assert!(
        output
            .iter()
            .flat_map(|pixel| pixel.channels())
            .all(f32::is_finite)
    );
}

#[test]
fn historical_gaussian_orders_are_typed_and_executable() {
    let dimensions = RasterDimensions::new(5, 5).expect("dimensions");
    let input = vec![ShadhiPixel::new(35.0, 18.0, -12.0, 1.0); 25];
    for order in 0..=2 {
        let config = ShadhiConfig::new(ShadhiParametersV5 {
            order,
            shadhi_algo: ShadhiAlgorithm::Gaussian.id(),
            radius: 3.0,
            ..ShadhiParametersV5::defaults()
        })
        .expect("historical order");
        let plan = ShadhiPlan::new(config, dimensions).expect("plan");
        let output = plan
            .execute_lab(&input, None, 1.0, || false)
            .expect("Gaussian order execution");
        assert!(
            output
                .iter()
                .flat_map(|pixel| pixel.channels())
                .all(f32::is_finite)
        );
    }
    assert!(
        ShadhiConfig::new(ShadhiParametersV5 {
            order: 3,
            ..ShadhiParametersV5::defaults()
        })
        .is_err()
    );
}

#[test]
fn descriptor_and_registry_advertise_lab_cpu_fallback_and_blending() {
    let descriptor = descriptor::shadhi_descriptor();
    descriptor.validate().expect("descriptor");
    assert!(descriptor.flags.contains(OperationFlags::FULL_IMAGE));
    assert!(descriptor.flags.contains(OperationFlags::DETERMINISTIC_CPU));
    assert!(descriptor.flags.contains(OperationFlags::MASKS));
    assert!(descriptor.flags.contains(OperationFlags::BLENDING));
    assert_eq!(descriptor.io.input.alpha, descriptor::AlphaPolicy::Preserve);
    assert_eq!(descriptor.io.input.channels, 4);
    assert_eq!(
        descriptor.io.input.encodings,
        vec![rusttable_color::ColorEncoding::LabD50]
    );
    assert!(descriptor.mask_blend.consumes_mask);
    assert_eq!(descriptor.migration.source_versions, [1, 2, 3, 4, 5]);
    let definition = builtin_registry()
        .definition("rusttable.shadhi")
        .expect("registry");
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_none());
}

#[test]
fn masks_opacity_receipts_and_cancellation_are_part_of_the_plan_contract() {
    let dimensions = RasterDimensions::new(3, 3).expect("dimensions");
    let input = vec![ShadhiPixel::new(20.0, 5.0, -3.0, 0.25); 9];
    let plan = ShadhiPlan::new(
        ShadhiConfig::new(ShadhiParametersV5 {
            shadhi_algo: ShadhiAlgorithm::Gaussian.id(),
            radius: 2.0,
            ..ShadhiParametersV5::defaults()
        })
        .expect("config"),
        dimensions,
    )
    .expect("plan");
    let mask = vec![0.0; input.len()];
    let output = plan
        .execute_lab(&input, Some(&mask), 1.0, || false)
        .expect("masked execution");
    assert_eq!(output, input);
    let (_, receipt) = plan
        .execute_with_receipt(&input, None, 0.5, || false)
        .expect("receipt execution");
    assert_eq!(receipt.plan_identity(), plan.cache_identity());
    assert_ne!(receipt.input_identity(), receipt.output_identity());
    assert!(matches!(
        plan.execute_lab(&input, None, 1.0, || true),
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    ));
}

#[test]
fn mixed_rgb_lab_preview_and_export_share_the_same_shadhi_plan_and_alpha() {
    let dimensions = RasterDimensions::new(8, 8).expect("dimensions");
    let pixels = (0..64)
        .map(|index| {
            let x = index % 8;
            let y = index / 8;
            let x = f32::from(u16::try_from(x).expect("x fits u16"));
            let y = f32::from(u16::try_from(y).expect("y fits u16"));
            pixel(0.1 + x / 16.0, 0.15 + y / 16.0, 0.2 + (x + y) / 32.0)
        })
        .collect();
    let input =
        WorkingRgbImage::new_with_frame(dimensions, pixels, WorkingFrameDescriptor::rec2020())
            .expect("input");
    let shadhi = builtin_registry()
        .materialize_operation(
            "rusttable.shadhi",
            OperationId::new(2).expect("shadhi operation ID"),
        )
        .expect("default shadhi operation");
    let offset = Operation::new(
        OperationId::new(1).expect("offset operation ID"),
        OperationKey::new("rusttable.linear_offset").expect("offset key"),
        true,
        [(
            ParameterName::new("value").expect("value name"),
            ParameterValue::Scalar(FiniteF64::new(0.01).expect("finite value")),
        )],
    )
    .expect("offset operation");
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::ZERO,
        [offset, shadhi],
    )
    .expect("edit");
    let pipeline = CompiledPipeline::compile(&edit).expect("pipeline");
    let graph = CompiledOperationGraph::from_pipeline(&pipeline);
    let alpha = vec![0.25; 64];
    let preview = evaluate_graph_at_frame_boundaries(
        &graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    )
    .expect("preview");
    let export = evaluate_graph_at_frame_boundaries(
        &graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Export),
        || false,
    )
    .expect("export");
    assert_eq!(preview.image().frame(), input.frame());
    assert_eq!(preview.alpha(), alpha.as_slice());
    assert_eq!(preview.image().pixel_slice(), export.image().pixel_slice());
    assert!(
        preview
            .image()
            .pixel_slice()
            .iter()
            .flat_map(|p| { [p.red().get(), p.green().get(), p.blue().get()] })
            .all(f32::is_finite)
    );
}
