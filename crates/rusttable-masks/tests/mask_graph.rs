use rusttable_masks::{
    CombinationMode, GeometryAncestry, GraphBuildError, MaskGeometry, MaskGraphBuilder, MaskGroup,
    MaskIdentity, MaskModifier, MaskNode, MaskRaster, MaskReference, MaskRoi, MaskSource,
    ProducerIdentity, RasterMaskDescriptor, RasterMaskPublication, RasterMaskStore,
};

fn identity(id: u128) -> MaskIdentity {
    MaskIdentity::new(7, 3, id, 1)
}
fn geometry() -> MaskGeometry {
    MaskGeometry::new(GeometryAncestry::identity(), MaskRoi::full(2, 1), false)
}
fn node(id: u128, values: Vec<f32>) -> MaskNode {
    MaskNode::new(
        identity(id),
        format!("mask-{id}"),
        MaskSource::Raster,
        geometry(),
        Some(MaskRaster::new(2, 1, values).expect("valid raster")),
        [],
    )
    .expect("valid node")
}

#[test]
fn graph_order_is_tied_to_identity_and_groups_apply_equations() {
    let first = identity(1);
    let second = identity(2);
    let group = MaskGroup::new(
        identity(3),
        "group",
        [
            MaskReference::new(first, 10, 1),
            MaskReference::new(second, 10, 1),
        ],
        CombinationMode::Union,
        [MaskModifier::Invert, MaskModifier::Opacity(0.5)],
    )
    .expect("valid group");
    let graph = MaskGraphBuilder::new()
        .add_mask(node(2, vec![0.75, 0.25]))
        .add_mask(node(1, vec![0.25, 0.5]))
        .add_group(group)
        .build()
        .expect("valid graph");

    assert_eq!(graph.order(), &[first, second, identity(3)]);
    assert_eq!(graph.consumer_use_count(first), 1);
    let result = graph.evaluate(identity(3)).expect("group evaluates");
    assert_eq!(result.values(), &[0.125, 0.25]);
    assert_eq!(
        graph.identity(),
        MaskGraphBuilder::new()
            .add_mask(node(2, vec![0.75, 0.25]))
            .add_mask(node(1, vec![0.25, 0.5]))
            .add_group(
                MaskGroup::new(
                    identity(3),
                    "group",
                    [
                        MaskReference::new(first, 10, 1),
                        MaskReference::new(second, 10, 1)
                    ],
                    CombinationMode::Union,
                    [MaskModifier::Invert, MaskModifier::Opacity(0.5)]
                )
                .expect("valid group")
            )
            .build()
            .expect("same graph")
            .identity()
    );
    let encoded = graph.canonical_bytes();
    let decoded = rusttable_masks::MaskGraph::from_canonical_bytes(&encoded).expect("round trip");
    assert_eq!(decoded, graph);
}

#[test]
fn graph_rejects_cycles_and_preserves_missing_reference_as_error() {
    let left = identity(1);
    let right = identity(2);
    let graph = MaskGraphBuilder::new()
        .add_group(
            MaskGroup::new(
                left,
                "left",
                [MaskReference::new(right, 1, 1)],
                CombinationMode::Union,
                [],
            )
            .expect("group"),
        )
        .add_group(
            MaskGroup::new(
                right,
                "right",
                [MaskReference::new(left, 1, 1)],
                CombinationMode::Union,
                [],
            )
            .expect("group"),
        )
        .build();
    assert_eq!(graph, Err(GraphBuildError::Cycle));
}

#[test]
fn publication_identity_contains_producer_ancestry_and_store_is_bounded() {
    let producer =
        ProducerIdentity::new(42, 9, [1; 32], [2; 32], [3; 32], MaskRoi::full(2, 1), 0.5)
            .expect("valid producer");
    let descriptor = RasterMaskDescriptor::new(identity(9), producer);
    let publication = RasterMaskPublication::new(
        descriptor.clone(),
        MaskRaster::new(2, 1, vec![0.0, 1.0]).expect("raster"),
    )
    .expect("publication");
    let mut store = RasterMaskStore::new(8);
    store.publish(publication).expect("within budget");
    assert_eq!(
        store
            .consume(&descriptor)
            .expect("exact descriptor")
            .raster()
            .values(),
        &[0.0, 1.0]
    );

    let changed =
        RasterMaskDescriptor::new(MaskIdentity::new(7, 4, 9, 1), descriptor.producer().clone());
    assert!(store.consume(&changed).is_err());
}
