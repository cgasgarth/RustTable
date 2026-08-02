#![allow(
    clippy::float_cmp,
    reason = "compatibility tests assert stable scalar values"
)]

use rusttable_core::{Edit, EditId, OperationId, PhotoId, Revision};
use rusttable_processing::operations::relight::{
    RELIGHT_PARAMETER_BYTES, RELIGHT_PRESETS, RelightConfig, RelightHistory, RelightParametersV1,
    RelightPixel, RelightPlan,
};
use rusttable_processing::{
    CompiledOperationGraph, CompiledPipeline, FiniteF32, FrameBoundaryMode, FrameBoundaryOptions,
    LinearRgb, RasterDimensions, WorkingFrameDescriptor, WorkingRgbImage, builtin_registry,
    descriptor, evaluate_graph_at_frame_boundaries,
};

fn lab_pixel(lightness: f32, a: f32, b: f32, alpha: f32) -> RelightPixel {
    RelightPixel::new(lightness, a, b, alpha)
}

#[test]
fn v1_payload_defaults_presets_and_unknown_history_are_typed() {
    let defaults = RelightParametersV1::defaults();
    assert_eq!(defaults.to_bytes().len(), RELIGHT_PARAMETER_BYTES);
    assert_eq!(
        RelightParametersV1::from_bytes(&defaults.to_bytes()),
        Ok(defaults)
    );
    assert_eq!(
        RELIGHT_PRESETS[0].parameters,
        RelightParametersV1::new(0.25, 0.25, 4.0)
    );
    assert_eq!(
        RELIGHT_PRESETS[1].parameters,
        RelightParametersV1::new(-0.25, 0.25, 4.0)
    );
    let opaque = RelightHistory::decode(8, &[1, 2, 3]).expect("unknown history is retained");
    assert_eq!(opaque.payload(), vec![1, 2, 3]);
    assert_eq!(opaque.version(), 8);
}

#[test]
fn lab_fill_light_matches_native_bit_fixture_and_preserves_colored_channels() {
    // Derived from relight.c:123-143 and math.h:70-73.  The expected bits are
    // deliberately not computed by a Rust equation or a tolerance reference.
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let input = [lab_pixel(43.25, -13.5, 22.25, 0.75)];
    let plan = RelightPlan::new(
        RelightConfig::new(0.75, 0.18, 5.5).expect("config"),
        dimensions,
    );
    let output = plan
        .execute_lab(&input, 0.37, || false)
        .expect("native fixture execution");

    assert_eq!(
        output[0].channels().map(f32::to_bits),
        [0x422e_26ed, 0xc158_0000, 0x41b2_0000, 0x3f40_0000]
    );
}

#[test]
fn relight_zero_signs_validate_inputs_and_follow_native_clip() {
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");

    for ev in [0.0_f32, -0.0_f32] {
        let plan = RelightPlan::new(
            RelightConfig::new(ev, 0.0, 4.0).expect("zero EV configuration"),
            dimensions,
        );
        let negative = plan
            .execute_lab(&[lab_pixel(-10.0, 1.0, 2.0, 0.5)], 1.0, || false)
            .expect("finite negative lightness");
        assert_eq!(negative[0].lightness().to_bits(), 0.0_f32.to_bits());
        let positive = plan
            .execute_lab(&[lab_pixel(120.0, 1.0, 2.0, 0.5)], 1.0, || false)
            .expect("finite positive lightness");
        assert_eq!(positive[0].lightness().to_bits(), 100.0_f32.to_bits());

        let signed_zero = plan
            .execute_lab(&[lab_pixel(-0.0, 1.0, 2.0, 0.5)], 1.0, || false)
            .expect("signed-zero lightness");
        assert_eq!(signed_zero[0].lightness().to_bits(), (-0.0_f32).to_bits());

        for opacity in [0.0_f32, -0.0_f32] {
            for lightness in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert!(matches!(
                    plan.execute_lab(
                        &[lab_pixel(lightness, 1.0, 2.0, 0.5)],
                        opacity,
                        || false,
                    ),
                    Err(rusttable_processing::operations::OperationExecutionError::NonFiniteResult { .. })
                ));
            }
        }
    }
}

#[test]
fn relight_opacity_nonfinite_and_signed_zero_boundaries_are_explicit() {
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let input = [lab_pixel(43.25, -13.5, 22.25, 0.75)];
    let plan = RelightPlan::new(
        RelightConfig::new(0.75, 0.18, 5.5).expect("config"),
        dimensions,
    );

    for opacity in [0.0_f32, -0.0_f32] {
        let output = plan
            .execute_lab(&input, opacity, || false)
            .expect("signed-zero opacity is a no-op");
        assert_eq!(
            output[0].channels().map(f32::to_bits),
            [0x422d_0000, 0xc158_0000, 0x41b2_0000, 0x3f40_0000]
        );
    }

    for opacity in [f32::NAN, f32::INFINITY] {
        assert!(matches!(
            plan.execute_lab(&input, opacity, || false),
            Err(rusttable_processing::operations::OperationExecutionError::NonFiniteResult { .. })
        ));
    }

    for lightness in [f32::NAN, f32::INFINITY] {
        assert!(matches!(
            plan.execute_lab(&[lab_pixel(lightness, -13.5, 22.25, 0.75)], 1.0, || false,),
            Err(rusttable_processing::operations::OperationExecutionError::NonFiniteResult { .. })
        ));
    }
}

#[test]
fn relight_preserves_history_and_cancellation_contracts() {
    let dimensions = RasterDimensions::new(2, 2).expect("dimensions");
    let input = vec![lab_pixel(20.0, 4.0, -5.0, 0.2); 4];
    let plan = RelightPlan::new(RelightConfig::defaults(), dimensions);
    assert!(matches!(
        plan.execute_lab(&input, 1.0, || true),
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    ));
    assert_eq!(
        plan.execute_lab(&input, 0.0, || false)
            .expect("zero opacity"),
        input
    );
}

#[test]
fn descriptor_and_registry_claim_deprecated_cpu_lab_compatibility() {
    let descriptor = descriptor::relight_descriptor();
    descriptor.validate().expect("descriptor");
    assert!(
        descriptor
            .flags
            .contains(descriptor::OperationFlags::DEPRECATED)
    );
    assert!(
        descriptor
            .flags
            .contains(descriptor::OperationFlags::HIDDEN)
    );
    assert_eq!(descriptor.io.input.alpha, descriptor::AlphaPolicy::Preserve);
    assert_eq!(descriptor.io.input.channels, 4);
    assert_eq!(
        descriptor.io.input.encodings,
        vec![rusttable_color::ColorEncoding::LabD50]
    );
    assert_eq!(descriptor.stage, "display-referred-lab");
    assert!(descriptor.capability.fallback_to_cpu);
    let definition = builtin_registry()
        .definition("rusttable.relight")
        .expect("registry");
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_none());
}

#[test]
fn mixed_rgb_lab_preview_and_export_share_relight_output_and_alpha() {
    let dimensions = RasterDimensions::new(3, 1).expect("dimensions");
    let input = WorkingRgbImage::new_with_frame(
        dimensions,
        vec![
            LinearRgb::new(finite(0.05), finite(0.15), finite(0.3)),
            LinearRgb::new(finite(0.3), finite(0.15), finite(0.05)),
            LinearRgb::new(finite(0.6), finite(0.5), finite(0.4)),
        ],
        WorkingFrameDescriptor::rec2020(),
    )
    .expect("input");
    let operation = builtin_registry()
        .materialize_operation(
            "rusttable.relight",
            OperationId::new(2).expect("operation ID"),
        )
        .expect("default relight operation");
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::ZERO,
        [operation],
    )
    .expect("edit");
    let graph =
        CompiledOperationGraph::from_pipeline(&CompiledPipeline::compile(&edit).expect("pipeline"));
    let alpha = vec![0.2, 0.6, 0.9];
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
    assert_eq!(preview.image().pixel_slice(), export.image().pixel_slice());
    assert_eq!(preview.alpha(), alpha.as_slice());
    assert_eq!(export.alpha(), alpha.as_slice());
}

fn finite(value: f32) -> FiniteF32 {
    FiniteF32::new(value).expect("finite test channel")
}
