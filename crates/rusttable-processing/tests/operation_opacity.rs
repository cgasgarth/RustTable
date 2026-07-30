use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_processing::{
    CompiledPipeline, EvaluationError, FiniteF32, LinearRgb, OperationCompileError,
    RasterDimensions, WorkingRgbImage, evaluate,
};

fn operation(id: u128, opacity: f64, key: &str, parameter: (&str, f64)) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).unwrap(),
        OperationKey::new(key).unwrap(),
        true,
        OperationOpacity::new(opacity).unwrap(),
        [(
            ParameterName::new(parameter.0).unwrap(),
            ParameterValue::Scalar(FiniteF64::new(parameter.1).unwrap()),
        )],
    )
    .unwrap()
}

fn image(red: f32, green: f32, blue: f32) -> WorkingRgbImage {
    WorkingRgbImage::new(
        RasterDimensions::new(1, 1).unwrap(),
        vec![LinearRgb::new(
            FiniteF32::new(red).unwrap(),
            FiniteF32::new(green).unwrap(),
            FiniteF32::new(blue).unwrap(),
        )],
    )
    .unwrap()
}

fn pipeline(operations: Vec<Operation>) -> CompiledPipeline {
    CompiledPipeline::compile(
        &Edit::new(
            EditId::new(1).unwrap(),
            PhotoId::new(2).unwrap(),
            Revision::ZERO,
            operations,
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn zero_opacity_skips_overflowing_operation_arithmetic() {
    let input = image(0.25, 0.5, 0.75);
    let result = evaluate(
        &pipeline(vec![operation(
            1,
            0.0,
            "rusttable.exposure",
            ("stops", 1024.0),
        )]),
        &input,
    )
    .unwrap();

    assert_eq!(result, input);
}

#[test]
fn opaque_and_half_opacity_use_expected_normal_blend() {
    let input = image(0.25, 0.5, 0.75);
    let opaque = evaluate(
        &pipeline(vec![operation(
            1,
            1.0,
            "rusttable.linear_offset",
            ("value", 0.5),
        )]),
        &input,
    )
    .unwrap();
    let half = evaluate(
        &pipeline(vec![operation(
            1,
            0.5,
            "rusttable.linear_offset",
            ("value", 0.5),
        )]),
        &input,
    )
    .unwrap();

    assert_eq!(
        opaque.pixel(0).unwrap().red().get().to_bits(),
        0.75f32.to_bits()
    );
    assert_eq!(
        half.pixel(0).unwrap().red().get().to_bits(),
        0.5f32.to_bits()
    );
}

#[test]
fn positive_opacity_that_underflows_f32_is_rejected_at_compilation() {
    let operation = operation(
        7,
        f64::from_bits(1),
        "rusttable.linear_offset",
        ("value", 0.0),
    );

    assert_eq!(
        rusttable_processing::ProcessingOperation::compile(&operation),
        Err(OperationCompileError::OpacityNarrowingUnderflow {
            operation_id: OperationId::new(7).unwrap(),
        })
    );
}

#[test]
fn compilation_preserves_representable_opacity() {
    let source = operation(8, 0.25, "rusttable.linear_offset", ("value", 0.0));
    let compiled = rusttable_processing::ProcessingOperation::compile(&source).unwrap();

    assert_eq!(compiled.opacity().get().to_bits(), 0.25f32.to_bits());
}

#[test]
fn partially_blended_operations_follow_pipeline_order() {
    let input = image(0.25, 0.25, 0.25);
    let ordered = evaluate(
        &pipeline(vec![
            operation(1, 0.5, "rusttable.exposure", ("stops", 1.0)),
            operation(2, 0.5, "rusttable.linear_offset", ("value", 0.5)),
        ]),
        &input,
    )
    .unwrap();
    let reversed = evaluate(
        &pipeline(vec![
            operation(2, 0.5, "rusttable.linear_offset", ("value", 0.5)),
            operation(1, 0.5, "rusttable.exposure", ("stops", 1.0)),
        ]),
        &input,
    )
    .unwrap();

    assert_eq!(
        ordered.pixel(0).unwrap().red().get().to_bits(),
        0.625f32.to_bits()
    );
    assert_eq!(
        reversed.pixel(0).unwrap().red().get().to_bits(),
        0.75f32.to_bits()
    );
}

#[test]
fn blend_failures_keep_typed_context() {
    let input = image(f32::MAX, 0.0, 0.0);
    let result = evaluate(
        &pipeline(vec![operation(
            1,
            0.5,
            "rusttable.linear_offset",
            ("value", f64::from(f32::MAX)),
        )]),
        &input,
    );

    assert!(matches!(
        result,
        Err(EvaluationError::NonFiniteBlendResult { .. }
            | EvaluationError::NonFiniteChannelResult { .. },)
    ));
}
