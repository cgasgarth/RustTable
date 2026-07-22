use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_masks::{
    GeometryAncestry, MaskGeometry, MaskGraphBuilder, MaskIdentity, MaskModifier, MaskNode,
    MaskRaster, MaskRoi, MaskSource, ProducerIdentity, RasterMaskDescriptor, RasterMaskPublication,
    RasterMaskStore,
};
use rusttable_pixelpipe::{
    CancellationReason, CancellationScope, CancellationStage, CpuImplementation, CpuPixelpipeError,
    CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan,
    PipelineGeneration, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::CompiledOperationGraph;

fn operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new(key).expect("valid operation key"),
        true,
        OperationOpacity::ONE,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("valid parameter name"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite parameter")),
            )
        }),
    )
    .expect("valid operation")
}

fn graph(operations: Vec<Operation>) -> CompiledOperationGraph {
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(3),
        operations,
    )
    .expect("valid edit");
    CompiledOperationGraph::compile(&edit).expect("registered graph")
}

fn input(width: u32, height: u32) -> RgbaF32Image {
    let dimensions =
        rusttable_processing::RasterDimensions::new(width, height).expect("nonzero dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let index = u16::try_from(index).expect("test index fits u16");
            RgbaF32Pixel::new(
                0.1 + f32::from(index) * 0.01,
                0.2 + f32::from(index) * 0.005,
                0.3 + f32::from(index) * 0.002,
                0.25 + f32::from(index % 3) * 0.25,
            )
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LinearSrgbD65),
        pixels,
    )
    .expect("valid image")
}

fn mask_node(identity: MaskIdentity, width: u32, height: u32, values: Vec<f32>) -> MaskNode {
    MaskNode::new(
        identity,
        "cpu-mask",
        MaskSource::Raster,
        MaskGeometry::new(
            GeometryAncestry::identity(),
            MaskRoi::full(width, height),
            true,
        ),
        Some(MaskRaster::new(width, height, values).expect("valid mask")),
        [],
    )
    .expect("valid mask node")
}

fn mask_graph(
    operation_id: u128,
    width: u32,
    height: u32,
    values: Vec<f32>,
    modifiers: impl IntoIterator<Item = MaskModifier>,
) -> rusttable_masks::MaskGraph {
    let identity = MaskIdentity::new(2, 3, 7, 1);
    let node = MaskNode::new(
        identity,
        "cpu-mask",
        MaskSource::Raster,
        MaskGeometry::new(
            GeometryAncestry::identity(),
            MaskRoi::full(width, height),
            true,
        ),
        Some(MaskRaster::new(width, height, values).expect("valid mask")),
        modifiers,
    )
    .expect("valid mask node");
    MaskGraphBuilder::new()
        .add_mask(node)
        .add_edge(identity, operation_id, 1)
        .build()
        .expect("valid mask graph")
}

#[test]
fn cpu_pixelpipe_applies_graph_mask_after_operation_and_preserves_alpha() {
    let source = input(4, 2);
    let graph = graph(vec![operation(
        10,
        "rusttable.linear_offset",
        &[("value", 1.0)],
    )]);
    let mask = mask_graph(10, 4, 2, vec![0.0, 0.25, 0.5, 1.0, 1.0, 0.5, 0.25, 0.0], []);
    let snapshot =
        CpuPixelpipeSnapshot::new(source.clone(), graph, CpuPixelpipeOutputMode::FullExport)
            .with_mask_graph(mask);
    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("masked execution");

    for (index, (source, output)) in source
        .pixels()
        .iter()
        .zip(result.image().pixels())
        .enumerate()
    {
        let coverage = [0.0, 0.25, 0.5, 1.0, 1.0, 0.5, 0.25, 0.0][index];
        let expected = source.red() + coverage;
        assert!((output.red() - expected).abs() < f32::EPSILON);
        assert_eq!(output.alpha().to_bits(), source.alpha().to_bits());
    }
}

#[test]
fn mask_modifiers_are_evaluated_before_cpu_blending() {
    let source = input(2, 1);
    let graph = graph(vec![operation(
        10,
        "rusttable.linear_offset",
        &[("value", 1.0)],
    )]);
    let mask = mask_graph(
        10,
        2,
        1,
        vec![0.0, 1.0],
        [MaskModifier::Invert, MaskModifier::Opacity(0.5)],
    );
    let result = CpuPixelpipeExecutor
        .execute(
            &CpuPixelpipeSnapshot::new(source.clone(), graph, CpuPixelpipeOutputMode::FullExport)
                .with_mask_graph(mask),
        )
        .expect("modified mask execution");

    assert!(
        (result.image().pixels()[0].red() - (source.pixels()[0].red() + 0.5)).abs() < f32::EPSILON
    );
    assert!((result.image().pixels()[1].red() - source.pixels()[1].red()).abs() < f32::EPSILON);
}

#[test]
fn tiled_cpu_mask_evaluation_matches_full_frame_and_receipts() {
    let source = input(5, 3);
    let graph = graph(vec![operation(
        10,
        "rusttable.rgb_gain",
        &[("red", 1.5), ("green", 0.75), ("blue", 1.25)],
    )]);
    let values = (0_u16..15)
        .map(|index| f32::from(index % 4) / 3.0)
        .collect();
    let mask = mask_graph(10, 5, 3, values, []);
    let snapshot = CpuPixelpipeSnapshot::new(source, graph, CpuPixelpipeOutputMode::FullExport)
        .with_mask_graph(mask);
    let full = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("full execution");
    let tiled = CpuPixelpipeExecutor
        .execute_tiled(&snapshot, CpuTilePlan::new(2, 2).expect("tile plan"))
        .expect("tiled execution");

    assert_eq!(full.image(), tiled.image());
    assert_eq!(full.receipt(), tiled.receipt());
    assert_eq!(
        full.receipt().implementation(),
        CpuImplementation::ScalarReferenceV1
    );
}

#[test]
fn generated_masks_use_the_bounded_store_and_bind_to_restart_receipts() {
    let source = input(2, 2);
    let graph = graph(vec![operation(
        10,
        "rusttable.linear_offset",
        &[("value", 1.0)],
    )]);
    let mask_identity = MaskIdentity::new(2, 3, 7, 1);
    let producer =
        ProducerIdentity::new(10, 1, [1; 32], [2; 32], [3; 32], MaskRoi::full(2, 2), 1.0)
            .expect("producer identity");
    let descriptor = RasterMaskDescriptor::new(mask_identity, producer);
    let publication = RasterMaskPublication::new(
        descriptor.clone(),
        MaskRaster::new(2, 2, vec![0.0, 0.5, 1.0, 0.25]).expect("raster"),
    )
    .expect("publication");
    let mut store = RasterMaskStore::new(1024);
    store.publish(publication).expect("publish mask");
    let mask = MaskNode::new(
        mask_identity,
        "generated",
        MaskSource::Generated(descriptor),
        MaskGeometry::new(GeometryAncestry::identity(), MaskRoi::full(2, 2), true),
        None,
        [],
    )
    .expect("generated node");
    let mask_graph = MaskGraphBuilder::new()
        .add_mask(mask)
        .add_edge(mask_identity, 10, 1)
        .build()
        .expect("generated graph");
    let snapshot = CpuPixelpipeSnapshot::new(source, graph, CpuPixelpipeOutputMode::FullExport)
        .with_mask_graph(mask_graph)
        .with_mask_store(store.clone());

    let first = CpuPixelpipeExecutor.execute(&snapshot).expect("first run");
    let restarted = CpuPixelpipeExecutor
        .execute(&snapshot.clone())
        .expect("restart run");
    assert_eq!(first.image(), restarted.image());
    assert_eq!(first.receipt(), restarted.receipt());
    assert_eq!(
        snapshot.mask_store().expect("store").identity(),
        store.identity()
    );
}

#[test]
fn mixed_lab_defringe_mask_has_full_and_tiled_parity() {
    let dimensions = rusttable_processing::RasterDimensions::new(32, 32).expect("dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let x = u16::try_from(index % u64::from(dimensions.width())).expect("test x fits u16");
            let y = u16::try_from(index / u64::from(dimensions.width())).expect("test y fits u16");
            RgbaF32Pixel::new(
                50.0,
                (f32::from(x) - 16.0) * 2.0,
                (f32::from(y) - 16.0) * 2.0,
                0.5,
            )
        })
        .collect();
    let source = RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LabD50),
        pixels,
    )
    .expect("Lab image");
    let graph = graph(vec![operation(
        475,
        "rusttable.defringe",
        &[("radius", 4.0), ("threshold", 20.0), ("mode", 2.0)],
    )]);
    let mask = mask_graph(475, 32, 32, vec![0.5; 1024], []);
    let snapshot = CpuPixelpipeSnapshot::try_new(source, graph, CpuPixelpipeOutputMode::FullExport)
        .expect("Lab snapshot")
        .with_mask_graph(mask);
    let full = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("full Lab run");
    let tiled = CpuPixelpipeExecutor
        .execute_tiled(&snapshot, CpuTilePlan::new(8, 8).expect("tile plan"))
        .expect("tiled Lab run");

    assert_eq!(full.image(), tiled.image());
    assert_eq!(full.receipt(), tiled.receipt());
    assert!(full.image().pixels().iter().all(|pixel| {
        [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
            .into_iter()
            .all(f32::is_finite)
    }));
    assert!(
        full.image()
            .pixels()
            .iter()
            .all(|pixel| (pixel.alpha() - 0.5).abs() < f32::EPSILON)
    );
}

#[test]
fn missing_generated_raster_and_ambiguous_edges_fail_closed() {
    let source = input(2, 1);
    let graph = graph(vec![operation(
        10,
        "rusttable.linear_offset",
        &[("value", 1.0)],
    )]);
    let identity = MaskIdentity::new(2, 3, 7, 1);
    let producer =
        ProducerIdentity::new(10, 1, [1; 32], [2; 32], [3; 32], MaskRoi::full(2, 1), 1.0)
            .expect("producer");
    let descriptor = RasterMaskDescriptor::new(identity, producer);
    let node = MaskNode::new(
        identity,
        "missing",
        MaskSource::Generated(descriptor),
        MaskGeometry::new(GeometryAncestry::identity(), MaskRoi::full(2, 1), true),
        None,
        [],
    )
    .expect("node");
    let missing = MaskGraphBuilder::new()
        .add_mask(node.clone())
        .add_edge(identity, 10, 1)
        .build()
        .expect("graph");
    let missing_error = CpuPixelpipeExecutor.execute(
        &CpuPixelpipeSnapshot::new(
            source.clone(),
            graph.clone(),
            CpuPixelpipeOutputMode::FullExport,
        )
        .with_mask_graph(missing),
    );
    assert!(matches!(
        missing_error,
        Err(CpuPixelpipeError::MaskEvaluation { .. })
    ));

    let second_identity = MaskIdentity::new(2, 3, 8, 1);
    let second = mask_node(second_identity, 2, 1, vec![1.0, 1.0]);
    let ambiguous = MaskGraphBuilder::new()
        .add_mask(node)
        .add_mask(second)
        .add_edge(identity, 10, 1)
        .add_edge(second_identity, 10, 2)
        .build()
        .expect("ambiguous graph is structurally valid");
    let ambiguous_error = CpuPixelpipeExecutor.execute(
        &CpuPixelpipeSnapshot::new(source, graph, CpuPixelpipeOutputMode::FullExport)
            .with_mask_graph(ambiguous),
    );
    assert!(matches!(
        ambiguous_error,
        Err(CpuPixelpipeError::MaskEvaluation { .. })
    ));
}

#[test]
fn cancellation_prevents_mask_raster_publication() {
    let generation = PipelineGeneration::new(1).expect("generation");
    let scope = CancellationScope::root(generation).child(CancellationStage::Allocation);
    scope.cancel(CancellationReason::Shutdown);
    let result = CpuPixelpipeExecutor.execute_with_cancellation(
        &CpuPixelpipeSnapshot::new(
            input(2, 2),
            graph(vec![operation(
                10,
                "rusttable.linear_offset",
                &[("value", 1.0)],
            )]),
            CpuPixelpipeOutputMode::FullExport,
        ),
        &scope,
    );
    assert!(matches!(result, Err(CpuPixelpipeError::Cancelled(_))));
}
