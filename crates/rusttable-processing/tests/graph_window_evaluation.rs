use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_processing::{
    CompiledOperationGraph, FiniteF32, LinearRgb, RasterDimensions, RasterRowWindow,
    WorkingRgbImage, evaluate, evaluate_graph, evaluate_graph_window,
};

fn dimensions() -> RasterDimensions {
    RasterDimensions::new(3, 3).unwrap()
}

fn pixel(value: f32) -> LinearRgb {
    let value = FiniteF32::new(value).unwrap();
    LinearRgb::new(value, value, value)
}

fn input() -> WorkingRgbImage {
    WorkingRgbImage::new(
        dimensions(),
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]
            .into_iter()
            .map(pixel)
            .collect(),
    )
    .unwrap()
}

fn operation(id: u128, key: &str, parameter: &str, value: f64) -> Operation {
    operation_with_state(id, key, parameter, value, true, 1.0)
}

fn operation_with_state(
    id: u128,
    key: &str,
    parameter: &str,
    value: f64,
    enabled: bool,
    opacity: f64,
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).unwrap(),
        OperationKey::new(key).unwrap(),
        enabled,
        OperationOpacity::new(opacity).unwrap(),
        [(
            ParameterName::new(parameter).unwrap(),
            ParameterValue::Scalar(FiniteF64::new(value).unwrap()),
        )],
    )
    .unwrap()
}

fn graph_from<I>(operations: I) -> CompiledOperationGraph
where
    I: IntoIterator<Item = Operation>,
{
    let edit = Edit::from_parts(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        Revision::ZERO,
        operations,
    )
    .unwrap();
    CompiledOperationGraph::compile(&edit).unwrap()
}

fn graph() -> CompiledOperationGraph {
    graph_from([
        operation(1, "rusttable.linear_offset", "value", 0.25),
        operation(2, "rusttable.exposure", "stops", 1.0),
    ])
}

fn window(start_row: usize, row_count: usize) -> RasterRowWindow {
    RasterRowWindow::new(start_row, row_count)
}

#[test]
fn full_graph_matches_compatibility_pipeline_evaluation() {
    let graph = graph();
    let source = input();
    let edit = Edit::from_parts(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        Revision::ZERO,
        [
            operation(1, "rusttable.linear_offset", "value", 0.25),
            operation(2, "rusttable.exposure", "stops", 1.0),
        ],
    )
    .unwrap();
    let pipeline = rusttable_processing::CompiledPipeline::compile(&edit).unwrap();

    assert_eq!(
        evaluate(&pipeline, &source),
        evaluate_graph(&graph, &source)
    );
}

#[test]
fn full_graph_preserves_disabled_and_zero_opacity_identity() {
    let operations = vec![
        operation_with_state(1, "rusttable.exposure", "stops", 1_000.0, false, 1.0),
        operation_with_state(2, "rusttable.exposure", "stops", 1_000.0, true, 0.0),
        operation(3, "rusttable.linear_offset", "value", 0.25),
    ];
    let edit = Edit::from_parts(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        Revision::ZERO,
        operations.clone(),
    )
    .unwrap();
    let pipeline = rusttable_processing::CompiledPipeline::compile(&edit).unwrap();
    let graph = graph_from(operations);
    let source = input();

    assert_eq!(
        evaluate(&pipeline, &source),
        evaluate_graph(&graph, &source)
    );
}

#[test]
fn uneven_windows_stitch_to_exact_full_frame_bits() {
    let graph = graph();
    let source = input();
    let full = evaluate_graph(&graph, &source).unwrap();
    let windows = [window(2, 1), window(0, 2)];
    let mut stitched = Vec::new();

    for window in windows.into_iter().rev() {
        let evaluated = evaluate_graph_window(&graph, &source, window).unwrap();
        assert_eq!(evaluated.dimensions(), source.dimensions());
        assert_eq!(evaluated.pixel_slice().len(), 3 * evaluated.row_count());
        stitched.extend_from_slice(evaluated.pixel_slice());
    }

    assert_eq!(stitched, full.pixel_slice());
}

#[test]
fn arbitrary_window_request_order_is_independent_and_owned() {
    let graph = graph();
    let source = input();
    let graph_before = graph.clone();
    let source_before = source.clone();
    let first = evaluate_graph_window(&graph, &source, window(1, 1)).unwrap();
    let second = evaluate_graph_window(&graph, &source, window(1, 1)).unwrap();

    assert_eq!(first, second);
    assert_eq!(first.start_row(), 1);
    assert_eq!(first.row_count(), 1);
    assert_eq!(first.pixel(0, 0), Some(&first.pixel_slice()[0]));
    assert_eq!(graph, graph_before);
    assert_eq!(source, source_before);
}

#[test]
fn evaluated_window_owns_pixels_after_inputs_are_dropped() {
    let evaluated = {
        let graph = graph();
        let source = input();
        evaluate_graph_window(&graph, &source, window(1, 2)).unwrap()
    };

    assert_eq!(evaluated.dimensions(), dimensions());
    assert_eq!(evaluated.start_row(), 1);
    assert_eq!(evaluated.row_count(), 2);
    assert_eq!(evaluated.pixel_slice().len(), 6);
}

#[test]
fn empty_graph_returns_only_requested_source_rows() {
    let edit = Edit::from_parts(
        EditId::new(3).unwrap(),
        PhotoId::new(4).unwrap(),
        Revision::ZERO,
        Revision::ZERO,
        [],
    )
    .unwrap();
    let graph = CompiledOperationGraph::compile(&edit).unwrap();
    let source = input();
    let evaluated = evaluate_graph_window(&graph, &source, window(1, 1)).unwrap();

    assert_eq!(evaluated.pixel_slice(), &source.pixel_slice()[3..6]);
}
