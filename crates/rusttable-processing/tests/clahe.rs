#![allow(clippy::float_cmp)]

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_processing::{
    CLAHE_HISTOGRAM_ENTRIES, CLAHE_PARAMETER_BYTES, CLAHE_SCHEMA_VERSION, ClaheConfig,
    ClaheHistory, ClaheParametersV1, ClahePixel, ClahePlan, CompiledOperationGraph,
    EvaluationError, FiniteF32, LinearRgb, RasterDimensions, WorkingRgbImage, builtin_registry,
    descriptor, prepare_basicadj_plans_with_cancellation,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("nonzero dimensions")
}

fn pixels(width: usize, height: usize) -> Vec<ClahePixel> {
    (0..width * height)
        .map(|index| {
            let value = f32::from(u16::try_from(index).expect("focused test image fits u16"))
                / f32::from(
                    u16::try_from((width * height).max(1)).expect("focused test image fits u16"),
                );
            ClahePixel::new(value - 0.25, value, value + 0.5, 0.125 + value * 0.25)
        })
        .collect()
}

#[test]
fn v1_codec_defaults_ranges_and_future_history_are_exact() {
    let parameters = ClaheParametersV1::defaults();
    assert_eq!(parameters.to_bytes().len(), CLAHE_PARAMETER_BYTES);
    assert_eq!(
        ClaheParametersV1::from_bytes(&parameters.to_bytes()),
        Ok(parameters)
    );
    assert_eq!(ClaheConfig::defaults().parameters(), parameters);
    assert!(ClaheConfig::new(0.0, 1.0).is_ok());
    assert!(ClaheConfig::new(256.0, 3.0).is_ok());
    assert!(ClaheConfig::new(-0.01, 1.0).is_err());
    assert!(ClaheConfig::new(64.0, 3.01).is_err());
    let future = ClaheHistory::decode(9, &[1, 2, 3]).expect("opaque future history");
    assert!(!future.executable());
    assert_eq!(future.version(), 9);
    assert_eq!(future.payload(), vec![1, 2, 3]);
    assert!(
        ClaheHistory::decode(CLAHE_SCHEMA_VERSION, &parameters.to_bytes())
            .expect("v1 history")
            .executable()
    );
}

#[test]
fn plan_freezes_radius_scale_full_frame_and_memory_budget() {
    let config = ClaheConfig::new(64.9, 1.25).expect("config");
    let plan = ClahePlan::with_budget(config, dimensions(9, 4), 2.0, 1.5, usize::MAX)
        .expect("checked plan");
    assert_eq!(plan.resolved_radius(), 48);
    assert_eq!(plan.histogram_entries(), CLAHE_HISTOGRAM_ENTRIES);
    assert!(plan.full_image());
    assert_eq!(plan.cache_identity(), plan.cache_identity());
    assert!(matches!(
        ClahePlan::with_budget(
            config,
            dimensions(9, 4),
            2.0,
            1.5,
            plan.memory_estimate() - 1
        ),
        Err(rusttable_processing::ClaheExecutionError::MemoryBudgetExceeded { .. })
    ));
}

#[test]
fn constant_and_tiny_fields_are_finite_hsl_identity_and_alpha_safe() {
    let input = vec![ClahePixel::new(0.25, 0.25, 0.25, 0.37); 4];
    let plan = ClahePlan::new(ClaheConfig::defaults(), dimensions(2, 2), 1.0, 1.0).expect("plan");
    let output = plan.execute(&input, || false).expect("constant field");
    assert_eq!(output, input);
    assert!(
        output
            .iter()
            .all(|pixel| pixel.channels().iter().all(|value| value.is_finite()))
    );
    let one = ClahePlan::new(ClaheConfig::defaults(), dimensions(1, 1), 1.0, 1.0)
        .expect("one-pixel plan");
    let one_input = vec![ClahePixel::new(-0.2, 1.2, 0.5, 0.8)];
    let one_output = one.execute(&one_input, || false).expect("one pixel");
    assert_eq!(one_output[0].alpha().to_bits(), 0.8_f32.to_bits());
    assert!(
        one_output[0].channels()[..3]
            .iter()
            .all(|value| value.is_finite())
    );
}

#[test]
fn tile_equivalence_mask_blend_and_cancellation_are_deterministic() {
    let input = pixels(11, 7);
    let plan = ClahePlan::new(
        ClaheConfig::new(2.0, 2.0).expect("config"),
        dimensions(11, 7),
        1.0,
        1.0,
    )
    .expect("plan");
    let full = plan.execute(&input, || false).expect("full");
    let tiled = plan
        .execute_tiled(&input, 3, || false)
        .expect("full-frame tiled");
    assert_eq!(full, tiled);
    let zero_mask = vec![0.0; input.len()];
    assert_eq!(
        plan.execute_with_mask(&input, Some(&zero_mask), 1.0, || false)
            .unwrap(),
        input
    );
    let mut checks = 0;
    assert!(matches!(
        plan.execute(&input, || {
            checks += 1;
            checks > 1
        }),
        Err(rusttable_processing::ClaheExecutionError::Cancelled)
    ));
    let (_, receipt) = plan
        .execute_with_receipt(&input, None, 0.5, || false)
        .expect("receipt");
    assert_eq!(receipt.histogram_entries(), CLAHE_HISTOGRAM_ENTRIES);
    assert_eq!(receipt.resolved_radius(), plan.resolved_radius());
    assert!(receipt.full_image());
}

#[test]
fn descriptor_registry_and_operation_compilation_are_deprecated_v1_seams() {
    let value = descriptor::clahe_descriptor();
    value.validate().expect("descriptor");
    assert!(value.flags.contains(descriptor::OperationFlags::DEPRECATED));
    assert!(value.flags.contains(descriptor::OperationFlags::HIDDEN));
    assert!(
        value
            .flags
            .contains(descriptor::OperationFlags::STYLE_ELIGIBLE)
    );
    assert_eq!(value.io.input.channels, 4);
    assert_eq!(value.roi, descriptor::RoiKind::FullImage);
    let operation = Operation::new(
        OperationId::new(473).expect("operation ID"),
        OperationKey::new("rusttable.clahe").expect("operation key"),
        true,
        [
            (
                ParameterName::new("radius").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(8.5).unwrap()),
            ),
            (
                ParameterName::new("slope").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(2.0).unwrap()),
            ),
        ],
    )
    .expect("operation");
    let prepared = builtin_registry()
        .prepare_cpu(&operation)
        .expect("registered operation");
    assert!(matches!(
        prepared.operation().kind(),
        rusttable_processing::ProcessingOperationKind::Clahe { config } if config.parameters() == ClaheParametersV1::new(8.5, 2.0)
    ));
    assert!(
        builtin_registry()
            .capability(
                "rusttable.clahe",
                &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
                rusttable_color::ColorEncoding::LinearSrgbD65,
                Some("full"),
            )
            .is_some_and(|capability| capability.available)
    );
}

#[test]
fn cancellable_plan_preparation_cancels_clahe_between_output_rows() {
    let operation_id = OperationId::new(474).expect("operation ID");
    let operation = Operation::new(
        operation_id,
        OperationKey::new("rusttable.clahe").expect("operation key"),
        true,
        [
            (
                ParameterName::new("radius").expect("parameter name"),
                ParameterValue::Scalar(FiniteF64::new(2.0).expect("finite radius")),
            ),
            (
                ParameterName::new("slope").expect("parameter name"),
                ParameterValue::Scalar(FiniteF64::new(2.0).expect("finite slope")),
            ),
        ],
    )
    .expect("operation");
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        [operation],
    )
    .expect("edit");
    let graph = CompiledOperationGraph::compile(&edit).expect("graph");
    let dimensions = dimensions(7, 5);
    let input = WorkingRgbImage::new(
        dimensions,
        (0_u8..35)
            .map(|index| {
                let value = f32::from(index) / 40.0;
                let value = FiniteF32::new(value).expect("finite sample");
                LinearRgb::new(value, value, value)
            })
            .collect(),
    )
    .expect("working image");
    let polls = std::cell::Cell::new(0_usize);

    let error = prepare_basicadj_plans_with_cancellation(&graph, &input, || {
        let next = polls.get() + 1;
        polls.set(next);
        next >= 4
    })
    .expect_err("mid-filter cancellation must not publish a partial plan set");

    assert_eq!(
        error,
        EvaluationError::Cancelled {
            step_index: rusttable_processing::PipelineStepIndex::new(0),
            operation_id,
        }
    );
    assert_eq!(polls.get(), 4);
}
