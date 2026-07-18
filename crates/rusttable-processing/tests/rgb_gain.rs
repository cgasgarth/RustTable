use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_processing::{
    CompiledPipeline, EvaluationError, FiniteF32, LinearRgb, OperationCompileError,
    ProcessingOperation, ProcessingOperationKind, RasterDimensions, WorkingRgbImage, evaluate,
};

fn scalar(value: f64) -> ParameterValue {
    ParameterValue::Scalar(FiniteF64::new(value).expect("finite scalar"))
}

fn operation(
    id: u128,
    opacity: f64,
    key: &str,
    parameters: Vec<(&str, ParameterValue)>,
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new(key).expect("valid operation key"),
        true,
        OperationOpacity::new(opacity).expect("valid opacity"),
        parameters.into_iter().map(|(name, value)| {
            (
                ParameterName::new(name).expect("valid parameter name"),
                value,
            )
        }),
    )
    .expect("unique parameters")
}

fn gain(id: u128, opacity: f64, red: f64, green: f64, blue: f64) -> Operation {
    operation(
        id,
        opacity,
        "rusttable.rgb_gain",
        vec![
            ("red", scalar(red)),
            ("green", scalar(green)),
            ("blue", scalar(blue)),
        ],
    )
}

fn image(pixels: &[(f32, f32, f32)]) -> WorkingRgbImage {
    let width = u32::try_from(pixels.len()).expect("test image width fits");
    let dimensions = RasterDimensions::new(width, 1).expect("valid dimensions");
    let pixels = pixels
        .iter()
        .map(|&(red, green, blue)| {
            LinearRgb::new(
                FiniteF32::new(red).expect("finite red"),
                FiniteF32::new(green).expect("finite green"),
                FiniteF32::new(blue).expect("finite blue"),
            )
        })
        .collect();
    WorkingRgbImage::new(dimensions, pixels).expect("matching pixel count")
}

fn pipeline(operations: Vec<Operation>) -> CompiledPipeline {
    CompiledPipeline::compile(
        &Edit::new(
            EditId::new(1).expect("nonzero edit ID"),
            PhotoId::new(2).expect("nonzero photo ID"),
            Revision::ZERO,
            operations,
        )
        .expect("valid edit"),
    )
    .expect("valid pipeline")
}

#[test]
fn compiles_rgb_gain_with_exact_kind_and_opacity() {
    let source = gain(7, 0.5, 0.25, 2.0, 0.0);
    let compiled = ProcessingOperation::compile(&source).expect("valid gain");

    assert_eq!(compiled.operation_id(), source.id());
    assert_eq!(compiled.opacity().get().to_bits(), 0.5f32.to_bits());
    assert_eq!(
        compiled.kind(),
        &ProcessingOperationKind::RgbGain {
            red: FiniteF32::new(0.25).expect("finite"),
            green: FiniteF32::new(2.0).expect("finite"),
            blue: FiniteF32::new(0.0).expect("finite"),
        }
    );
}

#[test]
fn missing_gain_parameters_use_fixed_order() {
    let cases = [
        (vec![], "red"),
        (vec![("red", scalar(1.0))], "green"),
        (vec![("red", scalar(1.0)), ("green", scalar(1.0))], "blue"),
    ];

    for (parameters, expected) in cases {
        assert!(matches!(
            ProcessingOperation::compile(&operation(1, 1.0, "rusttable.rgb_gain", parameters)),
            Err(OperationCompileError::MissingParameter { parameter, .. })
                if parameter.as_str() == expected
        ));
    }
}

#[test]
fn unexpected_gain_parameters_use_lexical_order() {
    let source = operation(
        2,
        1.0,
        "rusttable.rgb_gain",
        vec![
            ("red", scalar(1.0)),
            ("green", scalar(1.0)),
            ("blue", scalar(1.0)),
            ("zeta", scalar(1.0)),
            ("alpha", scalar(1.0)),
        ],
    );

    assert!(matches!(
        ProcessingOperation::compile(&source),
        Err(OperationCompileError::UnexpectedParameter { parameter, .. })
            if parameter.as_str() == "alpha"
    ));
}

#[test]
fn gain_parameter_failures_are_typed_and_ordered() {
    let wrong_type = operation(
        3,
        1.0,
        "rusttable.rgb_gain",
        vec![
            ("red", ParameterValue::Bool(true)),
            ("green", scalar(1.0)),
            ("blue", scalar(1.0)),
        ],
    );
    let overflow = gain(4, 1.0, f64::MAX, 1.0, 1.0);
    let underflow = gain(5, 1.0, 1.0, f64::from_bits(1), 1.0);
    let negative = gain(6, 1.0, 1.0, 1.0, -1.0);

    assert!(matches!(
        ProcessingOperation::compile(&wrong_type),
        Err(OperationCompileError::WrongParameterType { parameter, .. })
            if parameter.as_str() == "red"
    ));
    assert!(matches!(
        ProcessingOperation::compile(&overflow),
        Err(OperationCompileError::ScalarNarrowingOverflow { parameter, .. })
            if parameter.as_str() == "red"
    ));
    assert!(matches!(
        ProcessingOperation::compile(&underflow),
        Err(OperationCompileError::ScalarNarrowingUnderflow { parameter, .. })
            if parameter.as_str() == "green"
    ));
    assert!(matches!(
        ProcessingOperation::compile(&negative),
        Err(OperationCompileError::NegativeParameter { operation_id, key, parameter })
            if operation_id == negative.id()
                && key == *negative.key()
                && parameter.as_str() == "blue"
    ));
}

#[test]
fn identity_gains_leave_pixels_exactly_equal() {
    let input = image(&[(0.25, 0.5, 0.75), (1.0, -0.5, 2.0)]);
    let output = evaluate(&pipeline(vec![gain(1, 1.0, 1.0, 1.0, 1.0)]), &input)
        .expect("identity gain succeeds");

    assert_eq!(output, input);
}

#[test]
fn unequal_gains_scale_channels_independently() {
    let output = evaluate(
        &pipeline(vec![gain(1, 1.0, 2.0, 0.5, 0.0)]),
        &image(&[(0.25, 0.5, 0.75)]),
    )
    .expect("gain succeeds");
    let pixel = output.pixel(0).expect("first pixel");

    assert_eq!(pixel.red().get().to_bits(), 0.5f32.to_bits());
    assert_eq!(pixel.green().get().to_bits(), 0.25f32.to_bits());
    assert_eq!(pixel.blue().get().to_bits(), 0.0f32.to_bits());
}

#[test]
fn equal_power_of_two_gains_match_one_stop_exposure() {
    let input = image(&[(0.125, 0.25, 0.5)]);
    let gain_output =
        evaluate(&pipeline(vec![gain(1, 1.0, 2.0, 2.0, 2.0)]), &input).expect("gain succeeds");
    let exposure = operation(2, 1.0, "rusttable.exposure", vec![("stops", scalar(1.0))]);
    let exposure_output = evaluate(&pipeline(vec![exposure]), &input).expect("exposure succeeds");

    assert_eq!(gain_output, exposure_output);
}

#[test]
fn rgb_gain_respects_pipeline_order() {
    let input = image(&[(0.25, 0.25, 0.25)]);
    let gain_then_offset = evaluate(
        &pipeline(vec![
            gain(1, 1.0, 2.0, 2.0, 2.0),
            operation(
                2,
                1.0,
                "rusttable.linear_offset",
                vec![("value", scalar(0.25))],
            ),
        ]),
        &input,
    )
    .expect("gain then offset succeeds");
    let offset_then_gain = evaluate(
        &pipeline(vec![
            operation(
                2,
                1.0,
                "rusttable.linear_offset",
                vec![("value", scalar(0.25))],
            ),
            gain(1, 1.0, 2.0, 2.0, 2.0),
        ]),
        &input,
    )
    .expect("offset then gain succeeds");

    assert_eq!(
        gain_then_offset.pixel(0).unwrap().red().get().to_bits(),
        0.75f32.to_bits()
    );
    assert_eq!(
        offset_then_gain.pixel(0).unwrap().red().get().to_bits(),
        1.0f32.to_bits()
    );
}

#[test]
fn rgb_gain_uses_common_operation_opacity() {
    let half = evaluate(
        &pipeline(vec![gain(1, 0.5, 2.0, 0.5, 0.0)]),
        &image(&[(0.25, 0.5, 0.75)]),
    )
    .expect("half gain succeeds");
    let zero = evaluate(
        &pipeline(vec![gain(
            2,
            0.0,
            f64::from(f32::MAX),
            f64::from(f32::MAX),
            f64::from(f32::MAX),
        )]),
        &image(&[(f32::MAX, f32::MAX, f32::MAX)]),
    )
    .expect("zero opacity skips gain arithmetic");
    let half_pixel = half.pixel(0).unwrap();

    assert_eq!(half_pixel.red().get().to_bits(), 0.375f32.to_bits());
    assert_eq!(half_pixel.green().get().to_bits(), 0.375f32.to_bits());
    assert_eq!(half_pixel.blue().get().to_bits(), 0.375f32.to_bits());
    assert_eq!(zero, image(&[(f32::MAX, f32::MAX, f32::MAX)]));
}

#[test]
fn reports_first_non_finite_gain_result_deterministically() {
    let source = gain(9, 1.0, 2.0, 2.0, 2.0);
    let input = image(&[(f32::MAX, 0.0, 0.0), (0.0, f32::MAX, 0.0)]);
    let error = evaluate(&pipeline(vec![source]), &input).expect_err("gain overflows");

    assert_eq!(
        error,
        EvaluationError::NonFiniteChannelResult {
            step_index: rusttable_processing::PipelineStepIndex::new(0),
            operation_id: OperationId::new(9).unwrap(),
            pixel_index: 0,
            channel: rusttable_processing::RgbChannel::Red,
        }
    );
    assert_eq!(input, image(&[(f32::MAX, 0.0, 0.0), (0.0, f32::MAX, 0.0)]));
}
