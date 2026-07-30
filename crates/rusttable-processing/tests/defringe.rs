use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_processing::{
    CompiledPipeline, DEFRINGE_PARAMETER_BYTES, DEFRINGE_SCHEMA_VERSION, DefringeConfig,
    DefringeHistory, DefringeMode, DefringeOutcome, DefringeParametersV1, DefringePixel,
    DefringePlan, FiniteF32, LinearRgb, RasterDimensions, WorkingFrameDescriptor, WorkingRgbImage,
    evaluate,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("nonzero dimensions")
}

fn flat(width: u32, height: u32) -> Vec<DefringePixel> {
    vec![DefringePixel::new(50.0, 0.0, 0.0, 0.25); (width * height) as usize]
}

#[test]
fn v1_codec_preserves_numeric_mode_and_defaults() {
    let parameters = DefringeParametersV1::defaults();
    let bytes = parameters.to_bytes();
    assert_eq!(bytes.len(), DEFRINGE_PARAMETER_BYTES);
    assert_eq!(DefringeParametersV1::from_bytes(&bytes), Ok(parameters));
    assert_eq!(DefringeConfig::defaults().parameters(), parameters);
    assert!(
        DefringeHistory::decode(DEFRINGE_SCHEMA_VERSION, &bytes)
            .unwrap()
            .executable()
    );
    for (mode, value) in [
        (DefringeMode::GlobalAverage, 0_u32),
        (DefringeMode::LocalAverage, 1_u32),
        (DefringeMode::Static, 2_u32),
    ] {
        assert_eq!(DefringeMode::try_from(value), Ok(mode));
        assert_eq!(
            DefringeParametersV1::new(4.0, 20.0, mode).to_bytes()[8..12],
            value.to_le_bytes()
        );
    }
}

#[test]
fn unknown_history_is_byte_preserved_and_blocking() {
    let history = DefringeHistory::decode(9, &[1, 2, 3]).expect("opaque future history");
    assert!(!history.executable());
    assert_eq!(history.version(), 9);
    assert_eq!(history.payload(), vec![1, 2, 3]);
}

#[test]
fn plan_freezes_support_overlap_and_fibonacci_sample_counts() {
    let plan = DefringePlan::new(DefringeConfig::defaults(), dimensions(32, 32), 1.0, 1.0)
        .expect("checked plan");
    assert_eq!(plan.sigma().to_bits(), 4.0_f32.to_bits());
    assert_eq!(plan.support_radius(), 8);
    assert_eq!(plan.overlap(), 8);
    assert_eq!(plan.average_sample_count(), 89);
    assert_eq!(plan.small_sample_count(), 55);
    assert_eq!(plan.average_lattice()[0], (-28_i32, -28_i32));
    assert_eq!(plan.cache_identity(), plan.cache_identity());
}

#[test]
fn static_flat_field_is_exact_copy_through_and_preserves_alpha() {
    let plan = DefringePlan::new(
        DefringeConfig::new(4.0, 20.0, DefringeMode::Static).expect("config"),
        dimensions(32, 32),
        1.0,
        1.0,
    )
    .expect("checked plan");
    let input = flat(32, 32);
    let (output, receipt) = plan
        .execute_with_receipt(&input, None, 1.0, || false)
        .expect("deterministic execution");
    assert_eq!(output, input);
    assert_eq!(receipt.threshold().to_bits(), 20.0_f32.to_bits());
    assert_eq!(receipt.outcome(), DefringeOutcome::Complete);
    assert!(
        output
            .iter()
            .all(|pixel| pixel.alpha().to_bits() == 0.25_f32.to_bits())
    );
}

#[test]
fn global_analysis_is_frozen_in_plan_and_receipt() {
    let config = DefringeConfig::defaults();
    let input = flat(32, 32);
    let plan = DefringePlan::new(config, dimensions(32, 32), 1.0, 1.0).expect("plan");
    let analyzed = plan
        .with_global_analysis(&input, || false)
        .expect("analysis");
    assert!(analyzed.analysis().is_some());
    assert_ne!(analyzed.cache_identity(), plan.cache_identity());
    let (_, receipt) = analyzed
        .execute_with_receipt(&input, None, 1.0, || false)
        .expect("execution");
    assert_eq!(
        receipt.analysis_identity(),
        analyzed.analysis().unwrap().identity()
    );
    assert_eq!(
        receipt.global_threshold(),
        analyzed
            .analysis()
            .map(rusttable_processing::DefringeAnalysis::global_threshold)
    );
}

#[test]
fn mask_and_opacity_are_post_operation_blending_and_cancellation_publishes_nothing() {
    let plan = DefringePlan::new(
        DefringeConfig::new(4.0, 20.0, DefringeMode::Static).expect("config"),
        dimensions(32, 32),
        1.0,
        1.0,
    )
    .expect("plan");
    let input = flat(32, 32);
    let mask = vec![0.0; input.len()];
    let output = plan
        .execute_with_mask(&input, Some(&mask), 1.0, || false)
        .expect("zero mask is identity");
    assert_eq!(output, input);
    let mut checks = 0;
    let error = plan.execute(&input, || {
        checks += 1;
        checks > 1
    });
    assert!(matches!(
        error,
        Err(rusttable_processing::DefringeExecutionError::Cancelled)
    ));
}

#[test]
fn too_small_images_copy_through_and_record_degraded_outcome() {
    let plan =
        DefringePlan::new(DefringeConfig::defaults(), dimensions(16, 16), 1.0, 1.0).expect("plan");
    let input = flat(16, 16);
    let (output, receipt) = plan
        .execute_with_receipt(&input, None, 1.0, || false)
        .expect("copy-through");
    assert_eq!(output, input);
    assert_eq!(receipt.outcome(), DefringeOutcome::ImageTooSmallForKernel);
}

#[test]
fn mixed_rgb_lab_graph_executes_with_an_explicit_roundtrip_boundary() {
    let dimensions = dimensions(32, 32);
    let width = u16::try_from(dimensions.width()).expect("test width fits u16");
    let height = u16::try_from(dimensions.height()).expect("test height fits u16");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let index = usize::try_from(index).expect("test index fits usize");
            let x = f32::from(u16::try_from(index % usize::from(width)).expect("test x fits u16"))
                / f32::from(width);
            let y = f32::from(u16::try_from(index / usize::from(width)).expect("test y fits u16"))
                / f32::from(height);
            LinearRgb::new(
                FiniteF32::new(0.15 + x * 0.6).expect("finite red"),
                FiniteF32::new(0.2 + y * 0.5).expect("finite green"),
                FiniteF32::new(0.25 + (x + y) * 0.25).expect("finite blue"),
            )
        })
        .collect();
    let input =
        WorkingRgbImage::new_with_frame(dimensions, pixels, WorkingFrameDescriptor::rec2020())
            .expect("matching pixels");
    let defringe = Operation::new_with_opacity(
        OperationId::new(2).expect("operation ID"),
        OperationKey::new("rusttable.defringe").expect("operation key"),
        true,
        OperationOpacity::ONE,
        [
            (
                ParameterName::new("radius").expect("radius name"),
                ParameterValue::Scalar(FiniteF64::new(4.0).expect("finite radius")),
            ),
            (
                ParameterName::new("threshold").expect("threshold name"),
                ParameterValue::Scalar(FiniteF64::new(20.0).expect("finite threshold")),
            ),
            (
                ParameterName::new("mode").expect("mode name"),
                ParameterValue::Integer(2),
            ),
        ],
    )
    .expect("defringe operation");
    let offset = Operation::new_with_opacity(
        OperationId::new(1).expect("operation ID"),
        OperationKey::new("rusttable.linear_offset").expect("operation key"),
        true,
        OperationOpacity::ONE,
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
        [offset, defringe],
    )
    .expect("edit");
    let pipeline = CompiledPipeline::compile(&edit).expect("mixed graph compiles");

    let output = evaluate(&pipeline, &input).expect("mixed graph evaluates");

    assert_eq!(output.frame(), input.frame());
    assert!(output.pixel_slice().iter().all(|pixel| {
        [pixel.red().get(), pixel.green().get(), pixel.blue().get()]
            .into_iter()
            .all(f32::is_finite)
    }));
}

#[test]
fn deprecated_lab_descriptor_and_registry_seam_are_explicit() {
    let registry = rusttable_processing::builtin_registry();
    let definition = registry
        .definition("rusttable.defringe")
        .expect("defringe registry entry");
    let descriptor = definition.descriptor();
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::DEPRECATED)
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::HIDDEN)
    );
    assert_eq!(descriptor.io.input.channels, 4);
    assert_eq!(
        descriptor.io.input.encodings,
        vec![rusttable_color::ColorEncoding::LabD50]
    );
    let operation = Operation::new(
        OperationId::new(475).expect("operation ID"),
        OperationKey::new("rusttable.defringe").expect("operation key"),
        true,
        [
            (
                ParameterName::new("radius").expect("name"),
                ParameterValue::Scalar(FiniteF64::new(4.0).expect("finite")),
            ),
            (
                ParameterName::new("threshold").expect("name"),
                ParameterValue::Scalar(FiniteF64::new(20.0).expect("finite")),
            ),
            (
                ParameterName::new("mode").expect("name"),
                ParameterValue::Integer(2),
            ),
        ],
    )
    .expect("operation");
    let prepared = registry
        .prepare_cpu(&operation)
        .expect("registry preparation");
    assert!(
        matches!(prepared.operation().kind(), rusttable_processing::ProcessingOperationKind::Defringe { config } if config.mode() == DefringeMode::Static)
    );
    assert!(
        registry
            .capability(
                "rusttable.defringe",
                &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
                rusttable_color::ColorEncoding::LabD50,
                Some("full"),
            )
            .is_some_and(|capability| capability.available)
    );
}
