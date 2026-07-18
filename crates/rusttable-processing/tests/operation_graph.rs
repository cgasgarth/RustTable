use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_processing::{
    CompiledOperationGraph, CompiledPipeline, OperationGraphInput, OperationGraphNodeIndex,
    OperationGraphOutput,
};

fn operation(id: u128, enabled: bool, opacity: f64) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).unwrap(),
        OperationKey::new("rusttable.exposure").unwrap(),
        enabled,
        OperationOpacity::new(opacity).unwrap(),
        [(
            ParameterName::new("stops").unwrap(),
            ParameterValue::Scalar(FiniteF64::new(1.0).unwrap()),
        )],
    )
    .unwrap()
}

fn edit(operations: Vec<Operation>) -> Edit {
    Edit::from_parts(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::from_u64(3),
        Revision::from_u64(4),
        operations,
    )
    .unwrap()
}

#[test]
fn empty_graph_preserves_provenance_and_source_output() {
    let source = edit(Vec::new());
    let graph = CompiledOperationGraph::compile(&source).unwrap();

    assert_eq!(graph.source_edit_id(), source.id());
    assert_eq!(graph.source_photo_id(), source.photo_id());
    assert_eq!(graph.base_photo_revision(), source.base_photo_revision());
    assert_eq!(graph.revision(), source.revision());
    assert_eq!(graph.nodes().count(), 0);
    assert_eq!(graph.output(), OperationGraphOutput::Source);
}

#[test]
fn graph_topology_is_authored_order_with_explicit_predecessors() {
    let graph = CompiledOperationGraph::compile(&edit(vec![
        operation(1, true, 1.0),
        operation(2, true, 1.0),
        operation(3, true, 1.0),
    ]))
    .unwrap();

    assert_eq!(
        graph
            .nodes()
            .map(|node| node.index().get())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(
        graph
            .nodes()
            .map(|node| node.pipeline_step_index().get())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert_eq!(
        graph
            .nodes()
            .map(rusttable_processing::OperationGraphNode::input)
            .collect::<Vec<_>>(),
        [
            OperationGraphInput::Source,
            OperationGraphInput::Node(OperationGraphNodeIndex::new(0)),
            OperationGraphInput::Node(OperationGraphNodeIndex::new(1)),
        ]
    );
    assert_eq!(
        graph
            .nodes()
            .map(|node| node.operation().operation_id().get())
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(
        graph.output(),
        OperationGraphOutput::Node(OperationGraphNodeIndex::new(2))
    );
}

#[test]
fn disabled_and_zero_opacity_nodes_remain_in_topology() {
    let graph = CompiledOperationGraph::compile(&edit(vec![
        operation(1, true, 1.0),
        operation(2, false, 1.0),
        operation(3, true, 0.0),
        operation(4, true, 1.0),
    ]))
    .unwrap();

    assert_eq!(graph.nodes().count(), 4);
    assert!(
        !graph
            .node(OperationGraphNodeIndex::new(1))
            .unwrap()
            .operation()
            .is_enabled()
    );
    assert!(
        graph
            .node(OperationGraphNodeIndex::new(2))
            .unwrap()
            .operation()
            .opacity()
            .get()
            .abs()
            < f32::EPSILON
    );
    assert_eq!(
        graph.node(OperationGraphNodeIndex::new(3)).unwrap().input(),
        OperationGraphInput::Node(OperationGraphNodeIndex::new(2))
    );
}

#[test]
fn lookup_and_pipeline_conversion_are_deterministic() {
    let source = edit(vec![operation(1, true, 1.0), operation(2, true, 1.0)]);
    let pipeline = CompiledPipeline::compile(&source).unwrap();
    let from_pipeline = CompiledOperationGraph::from_pipeline(&pipeline);
    let compiled = CompiledOperationGraph::compile(&source).unwrap();

    assert_eq!(compiled, from_pipeline);
    assert_eq!(
        compiled.node_by_operation_id(OperationId::new(2).unwrap()),
        compiled.node(OperationGraphNodeIndex::new(1))
    );
    assert!(compiled.node(OperationGraphNodeIndex::new(99)).is_none());
    assert!(
        compiled
            .node_by_operation_id(OperationId::new(99).unwrap())
            .is_none()
    );
}
