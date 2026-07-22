use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_pixelpipe::{
    CpuImplementation, CpuPipelineReceiptError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode,
    CpuPixelpipeRequest, CpuPixelpipeSnapshot, CpuPixelpipeSnapshotError, CpuTilePlan,
    CpuTilePlanError, RgbaF32Channel, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image,
    RgbaF32ImageError, RgbaF32Pixel,
};
use rusttable_processing::{
    CompiledOperationGraph, RasterDimensions, SourceRgb, SourceRgbImage, SrgbChannel,
    to_linear_srgb,
};

fn operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero ID"),
        OperationKey::new(key).expect("valid key"),
        true,
        OperationOpacity::ONE,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("valid parameter"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite value")),
            )
        }),
    )
    .expect("valid operation")
}

fn graph(operations: Vec<Operation>) -> CompiledOperationGraph {
    let edit = Edit::from_parts(
        EditId::new(1).expect("nonzero edit ID"),
        PhotoId::new(2).expect("nonzero photo ID"),
        Revision::ZERO,
        Revision::from_u64(3),
        operations,
    )
    .expect("valid edit");
    CompiledOperationGraph::compile(&edit).expect("registered operations")
}

fn image() -> RgbaF32Image {
    let descriptor = RgbaF32Descriptor::new(
        RasterDimensions::new(2, 1).expect("nonzero dimensions"),
        RgbaF32ColorEncoding::SrgbD65,
    );
    RgbaF32Image::new(
        descriptor,
        vec![
            RgbaF32Pixel::new(0.5, 0.25, 0.75, 0.4),
            RgbaF32Pixel::new(0.1, 0.2, 0.3, 1.0),
        ],
    )
    .expect("valid input")
}

fn tiled_image() -> RgbaF32Image {
    let descriptor = RgbaF32Descriptor::new(
        RasterDimensions::new(5, 3).expect("nonzero dimensions"),
        RgbaF32ColorEncoding::SrgbD65,
    );
    let pixels = (0_u16..15)
        .map(|index| {
            let index = f32::from(index);
            RgbaF32Pixel::new(
                (index + 1.0) / 20.0,
                (index + 2.0) / 20.0,
                (index + 3.0) / 20.0,
                (index + 4.0) / 20.0,
            )
        })
        .collect();
    RgbaF32Image::new(descriptor, pixels).expect("valid tiled input")
}

fn lab_image() -> RgbaF32Image {
    let dimensions = RasterDimensions::new(32, 32).expect("nonzero dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let x =
                f32::from(u16::try_from(index % u64::from(dimensions.width())).expect("small x"));
            let y =
                f32::from(u16::try_from(index / u64::from(dimensions.width())).expect("small y"));
            RgbaF32Pixel::new(50.0, (x - 16.0) * 2.0, (y - 16.0) * 2.0, 0.5)
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LabD50),
        pixels,
    )
    .expect("valid Lab input")
}

#[test]
fn executes_registered_operations_in_authored_order_and_preserves_alpha() {
    let graph = graph(vec![
        operation(7, "rusttable.exposure", &[("stops", 1.0)]),
        operation(8, "rusttable.linear_offset", &[("value", 0.1)]),
        operation(
            9,
            "rusttable.rgb_gain",
            &[("red", 0.5), ("green", 1.5), ("blue", 2.0)],
        ),
    ]);
    let request = CpuPixelpipeRequest::new(image(), graph, CpuPixelpipeOutputMode::FullExport);

    let result = CpuPixelpipeExecutor
        .execute(&request)
        .expect("CPU execution succeeds");

    assert_eq!(
        result.image().descriptor().color_encoding(),
        RgbaF32ColorEncoding::LinearSrgbD65
    );
    assert!((result.image().pixels()[0].alpha() - 0.4).abs() < f32::EPSILON);
    assert!((result.image().pixels()[1].alpha() - 1.0).abs() < f32::EPSILON);
    assert_eq!(
        result
            .receipt()
            .nodes()
            .iter()
            .map(|node| (node.index(), node.operation_id().get()))
            .collect::<Vec<_>>(),
        [(0, 7), (1, 8), (2, 9)]
    );

    let first = result.image().pixels()[0];
    assert!((first.red() - 0.264_041_13).abs() < 0.000_001);
    assert!((first.green() - 0.302_628_25).abs() < 0.000_001);
    assert!((first.blue() - 2.290_086_3).abs() < 0.000_001);
}

#[test]
fn executes_deprecated_defringe_only_at_the_typed_lab_boundary() {
    let graph = graph(vec![operation(
        475,
        "rusttable.defringe",
        &[("radius", 4.0), ("threshold", 20.0), ("mode", 2.0)],
    )]);
    let request =
        CpuPixelpipeSnapshot::try_new(lab_image(), graph, CpuPixelpipeOutputMode::FullExport)
            .expect("Lab compatibility snapshot");
    let result = CpuPixelpipeExecutor
        .execute(&request)
        .expect("typed Lab defringe execution");
    assert_eq!(
        result.image().descriptor().color_encoding(),
        RgbaF32ColorEncoding::LabD50
    );
    assert!(
        result
            .image()
            .pixels()
            .iter()
            .all(|pixel| pixel.alpha().to_bits() == 0.5_f32.to_bits())
    );
}

#[test]
fn exposure_applies_darktable_black_level_scale_without_clipping() {
    let graph = graph(vec![operation(
        10,
        "rusttable.exposure",
        &[("stops", 1.0), ("black", 0.125)],
    )]);
    let request = CpuPixelpipeRequest::new(image(), graph, CpuPixelpipeOutputMode::FullExport);
    let result = CpuPixelpipeExecutor
        .execute(&request)
        .expect("black-level exposure executes");

    let source = SourceRgbImage::new(
        RasterDimensions::new(2, 1).expect("dimensions"),
        vec![
            SourceRgb::new(
                SrgbChannel::new(0.5).expect("channel"),
                SrgbChannel::new(0.25).expect("channel"),
                SrgbChannel::new(0.75).expect("channel"),
            ),
            SourceRgb::new(
                SrgbChannel::new(0.1).expect("channel"),
                SrgbChannel::new(0.2).expect("channel"),
                SrgbChannel::new(0.3).expect("channel"),
            ),
        ],
    )
    .expect("source image");
    let linear = to_linear_srgb(&source);
    let scale = 1.0 / (2.0_f32.powi(-1) - 0.125);
    let first = *linear.pixels().next().expect("first pixel");
    let output = result.image().pixels();
    assert!((output[0].red() - (first.red().get() - 0.125) * scale).abs() < 0.000_001);
    assert!((output[0].green() - (first.green().get() - 0.125) * scale).abs() < 0.000_001);
    assert!((output[0].blue() - (first.blue().get() - 0.125) * scale).abs() < 0.000_001);
    assert!((output[0].alpha() - 0.4).abs() < f32::EPSILON);
    assert!(
        output[1].red() < 0.0,
        "black correction preserves negative values"
    );
    assert!(
        output[0].blue() > 1.0,
        "exposure preserves scene-linear headroom"
    );
}

#[test]
fn receipt_is_deterministic_and_records_scalar_cpu_provenance() {
    let graph = graph(vec![operation(
        7,
        "rusttable.linear_offset",
        &[("value", 0.05)],
    )]);
    let request = CpuPixelpipeRequest::new(image(), graph, CpuPixelpipeOutputMode::FullExport);

    let first = CpuPixelpipeExecutor
        .execute(&request)
        .expect("first execution");
    let second = CpuPixelpipeExecutor
        .execute(&request)
        .expect("second execution");

    assert_eq!(
        first.receipt().implementation(),
        CpuImplementation::ScalarReferenceV1
    );
    assert_eq!(first.receipt(), second.receipt());
    assert_ne!(
        first.receipt().input_identity(),
        first.receipt().output_identity()
    );
}

#[test]
fn immutable_snapshot_identity_is_deterministic_and_bound_to_receipt() {
    let snapshot = CpuPixelpipeSnapshot::new(
        image(),
        graph(vec![operation(
            7,
            "rusttable.linear_offset",
            &[("value", 0.05)],
        )]),
        CpuPixelpipeOutputMode::FullExport,
    );
    let clone = snapshot.clone();
    assert_eq!(snapshot, clone);
    assert_eq!(snapshot.identity(), clone.identity());
    assert_eq!(
        snapshot.source_identity(),
        snapshot.input().source_identity()
    );

    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("snapshot execution succeeds");
    assert_eq!(result.receipt().snapshot_identity(), snapshot.identity());
}

#[test]
fn snapshot_identity_changes_for_pixel_affecting_preparation_changes() {
    let source = image();
    let base = CpuPixelpipeSnapshot::new(
        source.clone(),
        graph(vec![operation(
            7,
            "rusttable.linear_offset",
            &[("value", 0.05)],
        )]),
        CpuPixelpipeOutputMode::FullExport,
    );
    let changed_operation = CpuPixelpipeSnapshot::new(
        source.clone(),
        graph(vec![operation(
            7,
            "rusttable.linear_offset",
            &[("value", 0.06)],
        )]),
        CpuPixelpipeOutputMode::FullExport,
    );
    let changed_mode = CpuPixelpipeSnapshot::new(
        source,
        graph(vec![operation(
            7,
            "rusttable.linear_offset",
            &[("value", 0.05)],
        )]),
        CpuPixelpipeOutputMode::Preview,
    );

    assert_ne!(base.identity(), changed_operation.identity());
    assert_ne!(base.identity(), changed_mode.identity());
}

#[test]
fn checked_snapshot_rejects_unsupported_input_before_execution() {
    let descriptor = RgbaF32Descriptor::new(
        RasterDimensions::new(1, 1).expect("nonzero dimensions"),
        RgbaF32ColorEncoding::LinearSrgbD65,
    );
    let input = RgbaF32Image::new(descriptor, vec![RgbaF32Pixel::new(1.5, 0.0, 0.0, 1.0)])
        .expect("linear extended range is valid");

    assert_eq!(
        CpuPixelpipeSnapshot::try_new(input, graph(Vec::new()), CpuPixelpipeOutputMode::FullExport),
        Err(CpuPixelpipeSnapshotError::UnsupportedInputEncoding {
            actual: RgbaF32ColorEncoding::LinearSrgbD65,
        })
    );
}

#[test]
fn source_identity_evidence_rejects_replaced_input_before_execution() {
    let original = image();
    let expected = original.source_identity();
    let descriptor = original.descriptor();
    let replacement = vec![
        RgbaF32Pixel::new(0.6, 0.25, 0.75, 0.4),
        RgbaF32Pixel::new(0.1, 0.2, 0.3, 1.0),
    ];

    assert!(matches!(
        RgbaF32Image::new_with_source_identity(descriptor, replacement, expected),
        Err(RgbaF32ImageError::SourceIdentityMismatch {
            expected: rejected_expected,
            actual,
        }) if rejected_expected == expected && actual != expected
    ));
}

#[test]
fn receipt_refuses_publication_when_source_evidence_is_replaced() {
    let original = image();
    let original_identity = original.source_identity();
    let result = CpuPixelpipeExecutor
        .execute(&CpuPixelpipeRequest::new(
            original,
            graph(Vec::new()),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("CPU execution succeeds");
    let replacement = RgbaF32Image::new(
        RgbaF32Descriptor::new(
            RasterDimensions::new(2, 1).expect("nonzero dimensions"),
            RgbaF32ColorEncoding::SrgbD65,
        ),
        vec![
            RgbaF32Pixel::new(0.6, 0.25, 0.75, 0.4),
            RgbaF32Pixel::new(0.1, 0.2, 0.3, 1.0),
        ],
    )
    .expect("valid replacement");

    assert_eq!(result.receipt().source_identity(), original_identity);
    assert_eq!(
        result
            .receipt()
            .authorize_publication_for(original_identity),
        Ok(())
    );
    assert_eq!(
        result
            .receipt()
            .authorize_publication_for(replacement.source_identity()),
        Err(CpuPipelineReceiptError::SourceIdentityMismatch {
            expected: replacement.source_identity(),
            actual: original_identity,
        })
    );
}

#[test]
fn rejects_a_nonfinite_rgba_component_at_the_descriptor_boundary() {
    let descriptor = RgbaF32Descriptor::new(
        RasterDimensions::new(1, 1).expect("nonzero dimensions"),
        RgbaF32ColorEncoding::SrgbD65,
    );

    assert_eq!(
        RgbaF32Image::new(descriptor, vec![RgbaF32Pixel::new(f32::NAN, 0.0, 0.0, 1.0)]),
        Err(RgbaF32ImageError::NonFiniteComponent {
            pixel_index: 0,
            channel: RgbaF32Channel::Red,
        })
    );
}

#[test]
fn preserves_extended_linear_input_for_full_export() {
    let descriptor = RgbaF32Descriptor::new(
        RasterDimensions::new(1, 1).expect("nonzero dimensions"),
        RgbaF32ColorEncoding::LinearSrgbD65,
    );
    let input = RgbaF32Image::new(descriptor, vec![RgbaF32Pixel::new(1.5, 0.0, 0.0, 1.0)])
        .expect("linear extended range is valid");
    let request =
        CpuPixelpipeRequest::new(input, graph(Vec::new()), CpuPixelpipeOutputMode::FullExport);

    let result = CpuPixelpipeExecutor
        .execute(&request)
        .expect("linear input");
    assert!((result.image().pixels()[0].red() - 1.5).abs() < f32::EPSILON);
}

#[test]
fn output_modes_have_known_linear_and_srgb_boundaries_with_identical_alpha() {
    let input = image();
    let graph = graph(Vec::new());
    let full = CpuPixelpipeExecutor
        .execute(&CpuPixelpipeRequest::new(
            input.clone(),
            graph.clone(),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("full export succeeds");
    let preview = CpuPixelpipeExecutor
        .execute(&CpuPixelpipeRequest::new(
            input,
            graph,
            CpuPixelpipeOutputMode::Preview,
        ))
        .expect("preview succeeds");

    assert_eq!(
        full.image().descriptor().color_encoding(),
        RgbaF32ColorEncoding::LinearSrgbD65
    );
    assert_eq!(
        preview.image().descriptor().color_encoding(),
        RgbaF32ColorEncoding::SrgbD65
    );
    let full_pixel = full.image().pixels()[0];
    assert!((full_pixel.red() - 0.214_041_14).abs() < 0.000_001);
    assert!((full_pixel.green() - 0.050_876_09).abs() < 0.000_001);
    assert!((full_pixel.blue() - 0.522_521_56).abs() < 0.000_001);
    let preview_pixel = preview.image().pixels()[0];
    assert!((preview_pixel.red() - 0.5).abs() < 0.000_001);
    assert!((preview_pixel.green() - 0.25).abs() < 0.000_001);
    assert!((preview_pixel.blue() - 0.75).abs() < 0.000_001);
    assert!((full_pixel.alpha() - 0.4).abs() < f32::EPSILON);
    assert!((preview_pixel.alpha() - 0.4).abs() < f32::EPSILON);
    assert_eq!(
        full.receipt().output_mode(),
        CpuPixelpipeOutputMode::FullExport
    );
    assert_eq!(
        preview.receipt().output_mode(),
        CpuPixelpipeOutputMode::Preview
    );
    assert_ne!(
        full.receipt().output_identity(),
        preview.receipt().output_identity()
    );
}

#[test]
fn checked_tile_grid_is_row_major_and_includes_edge_tiles() {
    let plan = CpuTilePlan::new(2, 2).expect("valid tile plan");
    let grid = plan
        .grid_for(RasterDimensions::new(5, 3).expect("nonzero dimensions"))
        .expect("valid grid");

    assert_eq!((grid.columns(), grid.rows(), grid.tile_count()), (3, 2, 6));
    let tiles = (0..grid.tile_count())
        .map(|index| grid.tile_at(index).expect("checked tile").expect("in grid"))
        .map(|tile| {
            (
                tile.origin_x(),
                tile.origin_y(),
                tile.dimensions().width(),
                tile.dimensions().height(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tiles,
        [
            (0, 0, 2, 2),
            (2, 0, 2, 2),
            (4, 0, 1, 2),
            (0, 2, 2, 1),
            (2, 2, 2, 1),
            (4, 2, 1, 1),
        ]
    );
    assert_eq!(grid.tile_at(6).expect("checked boundary"), None);
}

#[test]
fn tile_plan_rejects_zero_extents() {
    assert_eq!(CpuTilePlan::new(0, 1), Err(CpuTilePlanError::ZeroTileWidth));
    assert_eq!(
        CpuTilePlan::new(1, 0),
        Err(CpuTilePlanError::ZeroTileHeight)
    );
}

#[test]
fn tile_grid_preserves_checked_coordinates_at_u32_boundaries() {
    let dimensions = RasterDimensions::new(u32::MAX, u32::MAX).expect("nonzero dimensions");
    let grid = CpuTilePlan::new(1, 1)
        .expect("valid tile plan")
        .grid_for(dimensions)
        .expect("checked grid");

    let last = grid
        .tile_at(grid.tile_count() - 1)
        .expect("checked tile")
        .expect("final tile");
    assert_eq!(
        (last.origin_x(), last.origin_y()),
        (u32::MAX - 1, u32::MAX - 1)
    );
    assert_eq!(
        last.dimensions(),
        RasterDimensions::new(1, 1).expect("nonzero tile")
    );
}

#[test]
fn tiled_execution_matches_full_frame_image_and_receipt() {
    let executor = CpuPixelpipeExecutor;
    let operation_graph = graph(vec![
        operation(7, "rusttable.exposure", &[("stops", 0.75)]),
        operation(8, "rusttable.linear_offset", &[("value", 0.03)]),
        operation(
            9,
            "rusttable.rgb_gain",
            &[("red", 1.1), ("green", 0.8), ("blue", 1.3)],
        ),
    ]);

    for output_mode in [
        CpuPixelpipeOutputMode::Preview,
        CpuPixelpipeOutputMode::FullExport,
    ] {
        let request = CpuPixelpipeRequest::new(tiled_image(), operation_graph.clone(), output_mode);
        let full_frame = executor.execute(&request).expect("full-frame execution");
        let tiled = executor
            .execute_tiled(&request, CpuTilePlan::new(2, 2).expect("valid tile plan"))
            .expect("tiled execution");

        assert_eq!(tiled.image(), full_frame.image());
        assert_eq!(tiled.receipt(), full_frame.receipt());
    }
}

#[test]
fn neighborhood_effects_use_full_frame_cpu_path_and_preserve_alpha() {
    let executor = CpuPixelpipeExecutor;
    let operation_graph = graph(vec![
        operation(
            10,
            "rusttable.bloom",
            &[("size", 0.0), ("threshold", 0.0), ("strength", 25.0)],
        ),
        operation(
            11,
            "rusttable.soften",
            &[
                ("size", 0.0),
                ("saturation", 100.0),
                ("brightness", 0.33),
                ("amount", 50.0),
            ],
        ),
    ]);
    let request = CpuPixelpipeRequest::new(
        tiled_image(),
        operation_graph,
        CpuPixelpipeOutputMode::FullExport,
    );
    let full_frame = executor.execute(&request).expect("full-frame execution");
    let tiled = executor
        .execute_tiled(&request, CpuTilePlan::new(2, 2).expect("valid tile plan"))
        .expect("full-frame neighborhood fallback");

    assert_eq!(tiled.image(), full_frame.image());
    for (actual, source) in tiled.image().pixels().iter().zip(tiled_image().pixels()) {
        assert_eq!(actual.alpha().to_bits(), source.alpha().to_bits());
    }
}
