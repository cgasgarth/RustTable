use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
use rusttable_processing::{EvaluationError, PipelineStepIndex, RgbChannel};
use rusttable_render::{RenderError, SourceColorPolicy, render_edit};

fn image(encoding: ColorEncoding) -> DecodedImage {
    DecodedImage::new_with_color_encoding(
        ImageDimensions::new(1, 1).unwrap(),
        vec![255, 0, 0, 255],
        encoding,
    )
    .unwrap()
}

fn edit(operations: Vec<Operation>) -> Edit {
    Edit::new(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        operations,
    )
    .unwrap()
}

#[test]
fn source_color_policy_precedes_invalid_pipeline() {
    let invalid = Operation::new(
        OperationId::new(3).unwrap(),
        OperationKey::new("rusttable.unknown").unwrap(),
        true,
        [],
    )
    .unwrap();

    assert!(matches!(
        render_edit(
            &edit(vec![invalid]),
            &image(ColorEncoding::Unspecified),
            SourceColorPolicy::RequireDeclaredSrgb,
        ),
        Err(RenderError::SourceColor {
            actual: ColorEncoding::Unspecified
        })
    ));
}

#[test]
fn compilation_error_preserves_typed_source() {
    let invalid = Operation::new(
        OperationId::new(3).unwrap(),
        OperationKey::new("rusttable.unknown").unwrap(),
        true,
        [],
    )
    .unwrap();
    let error = render_edit(
        &edit(vec![invalid]),
        &image(ColorEncoding::Srgb),
        SourceColorPolicy::RequireDeclaredSrgb,
    )
    .expect_err("unknown operation must fail compilation");

    assert!(matches!(error, RenderError::Pipeline { .. }));
    assert!(std::error::Error::source(&error).is_some());
}

#[test]
fn evaluation_error_preserves_exact_context() {
    let max_gain = Operation::new(
        OperationId::new(3).unwrap(),
        OperationKey::new("rusttable.rgb_gain").unwrap(),
        true,
        [
            (
                ParameterName::new("red").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(f64::from(f32::MAX)).unwrap()),
            ),
            (
                ParameterName::new("green").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(1.0).unwrap()),
            ),
            (
                ParameterName::new("blue").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(1.0).unwrap()),
            ),
        ],
    )
    .unwrap();
    let overflow_gain = Operation::new(
        OperationId::new(4).unwrap(),
        OperationKey::new("rusttable.rgb_gain").unwrap(),
        true,
        [
            (
                ParameterName::new("red").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(2.0).unwrap()),
            ),
            (
                ParameterName::new("green").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(1.0).unwrap()),
            ),
            (
                ParameterName::new("blue").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(1.0).unwrap()),
            ),
        ],
    )
    .unwrap();
    let error = render_edit(
        &edit(vec![max_gain, overflow_gain]),
        &DecodedImage::new_with_color_encoding(
            ImageDimensions::new(1, 1).unwrap(),
            vec![255, 0, 0, 255],
            ColorEncoding::Srgb,
        )
        .unwrap(),
        SourceColorPolicy::RequireDeclaredSrgb,
    )
    .expect_err("gain should overflow linear red");

    assert!(matches!(
        error,
        RenderError::Evaluation {
            source: EvaluationError::NonFiniteChannelResult {
                step_index,
                operation_id,
                pixel_index: 0,
                channel: RgbChannel::Red,
            },
    } if step_index == PipelineStepIndex::new(1) && operation_id == OperationId::new(4).unwrap()
    ));
}
