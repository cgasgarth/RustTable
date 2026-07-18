use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_processing::{
    CompiledPipeline, EvaluationError, FiniteF32, LinearRgb, PipelineStepIndex, RasterDimensions,
    RgbChannel, SourceRgb, SourceRgbImage, SrgbChannel, WorkingRgbImage, evaluate, to_linear_srgb,
};

fn operation(id: u128, key: &str, enabled: bool, parameter: (&str, f64)) -> Operation {
    Operation::new(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new(key).expect("valid operation key"),
        enabled,
        [(
            ParameterName::new(parameter.0).expect("valid parameter name"),
            ParameterValue::Scalar(FiniteF64::new(parameter.1).expect("finite parameter")),
        )],
    )
    .expect("valid operation")
}

fn edit(operations: Vec<Operation>) -> Edit {
    Edit::from_parts(
        EditId::new(1).expect("nonzero edit ID"),
        PhotoId::new(2).expect("nonzero photo ID"),
        Revision::ZERO,
        Revision::ZERO,
        operations,
    )
    .expect("valid edit")
}

fn image(pixels: Vec<LinearRgb>) -> WorkingRgbImage {
    let width = u32::try_from(pixels.len()).expect("test image fits in u32");
    WorkingRgbImage::new(
        RasterDimensions::new(width, 1).expect("nonzero dimensions"),
        pixels,
    )
    .expect("matching image dimensions")
}

fn scalar(value: f32) -> FiniteF32 {
    FiniteF32::new(value).expect("finite test scalar")
}

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(scalar(red), scalar(green), scalar(blue))
}

fn exposure(id: u128, enabled: bool, stops: f64) -> Operation {
    operation(id, "rusttable.exposure", enabled, ("stops", stops))
}

fn offset(id: u128, enabled: bool, value: f64) -> Operation {
    operation(id, "rusttable.linear_offset", enabled, ("value", value))
}

#[test]
fn empty_pipeline_returns_equal_new_image() {
    let source = image(vec![pixel(0.25, 0.5, 0.75)]);
    let pipeline = CompiledPipeline::compile(&edit(Vec::new())).expect("empty pipeline");

    let result = evaluate(&pipeline, &source).expect("empty evaluation succeeds");

    assert_eq!(result, source);
    assert_eq!(result.dimensions(), source.dimensions());
    assert_eq!(result.space(), source.space());
    assert_ne!(result.pixel_slice().as_ptr(), source.pixel_slice().as_ptr());
}

#[test]
fn disabled_steps_have_no_effect() {
    let source = image(vec![pixel(0.25, 0.5, 0.75)]);
    let pipeline = CompiledPipeline::compile(&edit(vec![exposure(1, false, 1.0)]))
        .expect("disabled operation compiles");

    assert_eq!(
        evaluate(&pipeline, &source).expect("evaluation succeeds"),
        source
    );
}

#[test]
fn exposure_plus_one_stop_doubles_binary_fractions() {
    let source = image(vec![pixel(0.25, 0.5, 0.75)]);
    let pipeline =
        CompiledPipeline::compile(&edit(vec![exposure(1, true, 1.0)])).expect("exposure compiles");

    let result = evaluate(&pipeline, &source).expect("evaluation succeeds");

    assert_eq!(
        result.pixel(0).unwrap().red().get().to_bits(),
        0.5f32.to_bits()
    );
    assert_eq!(
        result.pixel(0).unwrap().green().get().to_bits(),
        1.0f32.to_bits()
    );
    assert_eq!(
        result.pixel(0).unwrap().blue().get().to_bits(),
        1.5f32.to_bits()
    );
}

#[test]
fn exposure_minus_one_stop_halves_binary_fractions() {
    let source = image(vec![pixel(0.25, 0.5, 0.75)]);
    let pipeline =
        CompiledPipeline::compile(&edit(vec![exposure(1, true, -1.0)])).expect("exposure compiles");

    let result = evaluate(&pipeline, &source).expect("evaluation succeeds");

    assert_eq!(
        result.pixel(0).unwrap().red().get().to_bits(),
        0.125f32.to_bits()
    );
    assert_eq!(
        result.pixel(0).unwrap().green().get().to_bits(),
        0.25f32.to_bits()
    );
    assert_eq!(
        result.pixel(0).unwrap().blue().get().to_bits(),
        0.375f32.to_bits()
    );
}

#[test]
fn linear_offset_preserves_extended_range_without_clipping() {
    let source = image(vec![pixel(-1.5, 2.0, 0.25)]);
    let pipeline =
        CompiledPipeline::compile(&edit(vec![offset(1, true, 0.5)])).expect("offset compiles");

    let result = evaluate(&pipeline, &source).expect("evaluation succeeds");

    assert_eq!(
        result.pixel(0).unwrap().red().get().to_bits(),
        (-1.0f32).to_bits()
    );
    assert_eq!(
        result.pixel(0).unwrap().green().get().to_bits(),
        2.5f32.to_bits()
    );
}

#[test]
fn pipeline_order_is_execution_order() {
    let source = image(vec![pixel(0.5, 0.5, 0.5)]);
    let first =
        CompiledPipeline::compile(&edit(vec![offset(1, true, 0.25), exposure(2, true, 1.0)]))
            .expect("first pipeline compiles");
    let second =
        CompiledPipeline::compile(&edit(vec![exposure(1, true, 1.0), offset(2, true, 0.25)]))
            .expect("second pipeline compiles");

    let first_result = evaluate(&first, &source).expect("first evaluation succeeds");
    let second_result = evaluate(&second, &source).expect("second evaluation succeeds");

    assert_eq!(
        first_result.pixel(0).unwrap().red().get().to_bits(),
        1.5f32.to_bits()
    );
    assert_eq!(
        second_result.pixel(0).unwrap().red().get().to_bits(),
        1.25f32.to_bits()
    );
    assert_ne!(first_result, second_result);
}

#[test]
fn reports_non_finite_exposure_multiplier() {
    let source = image(vec![pixel(0.5, 0.5, 0.5)]);
    let pipeline = CompiledPipeline::compile(&edit(vec![exposure(7, true, 1024.0)]))
        .expect("finite extreme exposure compiles");

    assert_eq!(
        evaluate(&pipeline, &source),
        Err(EvaluationError::NonFiniteExposureMultiplier {
            step_index: PipelineStepIndex::new(0),
            operation_id: OperationId::new(7).unwrap(),
        })
    );
}

#[test]
fn reports_first_non_finite_channel_deterministically() {
    let source = image(vec![
        pixel(f32::MAX, f32::MAX, f32::MAX),
        pixel(f32::MAX, 0.0, 0.0),
    ]);
    let pipeline =
        CompiledPipeline::compile(&edit(vec![exposure(8, true, 1.0)])).expect("exposure compiles");

    assert_eq!(
        evaluate(&pipeline, &source),
        Err(EvaluationError::NonFiniteChannelResult {
            step_index: PipelineStepIndex::new(0),
            operation_id: OperationId::new(8).unwrap(),
            pixel_index: 0,
            channel: RgbChannel::Red,
        })
    );
}

#[test]
fn retains_original_step_index_after_disabled_predecessor() {
    let source = image(vec![pixel(f32::MAX, 0.0, 0.0)]);
    let pipeline =
        CompiledPipeline::compile(&edit(vec![exposure(1, false, 1.0), exposure(2, true, 1.0)]))
            .expect("pipeline compiles");

    assert_eq!(
        evaluate(&pipeline, &source),
        Err(EvaluationError::NonFiniteChannelResult {
            step_index: PipelineStepIndex::new(1),
            operation_id: OperationId::new(2).unwrap(),
            pixel_index: 0,
            channel: RgbChannel::Red,
        })
    );
}

#[test]
fn success_and_failure_do_not_mutate_inputs() {
    let source = image(vec![pixel(0.25, 0.5, 0.75)]);
    let pipeline =
        CompiledPipeline::compile(&edit(vec![offset(1, true, 0.25)])).expect("pipeline compiles");
    let source_before = source.clone();
    let pipeline_before = pipeline.clone();

    let _ = evaluate(&pipeline, &source).expect("success");

    assert_eq!(source, source_before);
    assert_eq!(pipeline, pipeline_before);

    let failing_source = image(vec![pixel(f32::MAX, 0.0, 0.0)]);
    let failing_pipeline =
        CompiledPipeline::compile(&edit(vec![exposure(2, true, 1.0)])).expect("pipeline compiles");
    let failing_source_before = failing_source.clone();
    let failing_pipeline_before = failing_pipeline.clone();

    let _ = evaluate(&failing_pipeline, &failing_source).expect_err("overflow");

    assert_eq!(failing_source, failing_source_before);
    assert_eq!(failing_pipeline, failing_pipeline_before);
}

#[test]
fn equal_inputs_evaluate_equally() {
    let source = image(vec![pixel(0.25, 0.5, 0.75)]);
    let first = CompiledPipeline::compile(&edit(vec![offset(1, true, 0.25)])).unwrap();
    let second = CompiledPipeline::compile(&edit(vec![offset(1, true, 0.25)])).unwrap();

    assert_eq!(evaluate(&first, &source), evaluate(&second, &source));
}

#[test]
fn synthetic_srgb_source_compiles_and_renders() {
    let source = SourceRgbImage::new(
        RasterDimensions::new(1, 1).unwrap(),
        vec![SourceRgb::new(
            SrgbChannel::new(0.5).unwrap(),
            SrgbChannel::new(0.5).unwrap(),
            SrgbChannel::new(0.5).unwrap(),
        )],
    )
    .unwrap();
    let working = to_linear_srgb(&source);
    let pipeline =
        CompiledPipeline::compile(&edit(vec![offset(1, true, 0.25)])).expect("pipeline compiles");

    let result = evaluate(&pipeline, &working).expect("evaluation succeeds");

    assert!((result.pixel(0).unwrap().red().get() - 0.464_041_14).abs() < 1e-5);
}
