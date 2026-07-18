use std::error::Error;

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_processing::{
    CompiledOperationGraph, EvaluationError, FiniteF32, GraphWindowEvaluationError, LinearRgb,
    RasterDimensions, RasterRowWindow, RgbChannel, WorkingRgbImage, evaluate_graph_window,
};

fn dimensions() -> RasterDimensions {
    RasterDimensions::new(2, 2).unwrap()
}

fn input(red: f32) -> WorkingRgbImage {
    let red = FiniteF32::new(red).unwrap();
    let zero = FiniteF32::new(0.0).unwrap();
    WorkingRgbImage::new(
        dimensions(),
        vec![
            LinearRgb::new(red, zero, zero),
            LinearRgb::new(zero, zero, zero),
            LinearRgb::new(red, zero, zero),
            LinearRgb::new(zero, zero, zero),
        ],
    )
    .unwrap()
}

fn graph_with_gain(gain: f64) -> CompiledOperationGraph {
    let operation = Operation::new(
        OperationId::new(7).unwrap(),
        OperationKey::new("rusttable.rgb_gain").unwrap(),
        true,
        [
            (
                ParameterName::new("red").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(gain).unwrap()),
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
    let edit = Edit::from_parts(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        Revision::ZERO,
        [operation],
    )
    .unwrap();
    CompiledOperationGraph::compile(&edit).unwrap()
}

#[test]
fn window_validation_reports_distinct_checked_errors() {
    let dimensions = dimensions();
    assert!(matches!(
        RasterRowWindow::new(0, 0).validate(dimensions),
        Err(rusttable_processing::RasterRowWindowError::ZeroRowCount { .. })
    ));
    assert!(matches!(
        RasterRowWindow::new(2, 1).validate(dimensions),
        Err(rusttable_processing::RasterRowWindowError::StartOutOfBounds { .. })
    ));
    assert!(matches!(
        RasterRowWindow::new(1, 2).validate(dimensions),
        Err(rusttable_processing::RasterRowWindowError::EndOutOfBounds { .. })
    ));
    assert!(matches!(
        RasterRowWindow::new(usize::MAX, 2).validate(dimensions),
        Err(rusttable_processing::RasterRowWindowError::ArithmeticOverflow { .. })
    ));
}

#[test]
fn graph_window_wraps_validation_before_evaluation() {
    let error = evaluate_graph_window(
        &graph_with_gain(1.0),
        &input(1.0),
        RasterRowWindow::new(2, 1),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        GraphWindowEvaluationError::Window {
            source: rusttable_processing::RasterRowWindowError::StartOutOfBounds { .. }
        }
    ));
    assert!(error.source().is_some());
}

#[test]
fn evaluation_errors_keep_global_pixel_context_for_nonzero_windows() {
    let error = evaluate_graph_window(
        &graph_with_gain(2.0),
        &input(f32::MAX),
        RasterRowWindow::new(1, 1),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        GraphWindowEvaluationError::Evaluation {
            source
        } if matches!(
            source.as_ref(),
            EvaluationError::NonFiniteChannelResult {
                step_index,
                pixel_index: 2,
                channel: RgbChannel::Red,
                ..
            } if step_index.get() == 0
        )
    ));
}
