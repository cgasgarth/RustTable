use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_pixelpipe::{
    CancellationReason, CancellationScope, CpuPixelpipeError, CpuPixelpipeExecutor,
    CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan, PipelineGeneration,
    RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
    RgbaF32SourceRepresentation,
};
use rusttable_processing::{
    CompiledOperationGraph, FiniteF32, FrameBoundaryMode, FrameBoundaryOptions, FrameBoundaryPlan,
    LinearRgb, RasterDimensions, WorkingRgbImage, evaluate_graph,
};

fn operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
    operation_with_opacity(id, key, parameters, 1.0)
}

fn operation_with_opacity(
    id: u128,
    key: &str,
    parameters: &[(&str, f64)],
    opacity: f64,
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero ID"),
        OperationKey::new(key).expect("valid key"),
        true,
        OperationOpacity::new(opacity).expect("valid opacity"),
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
        EditId::new(869).expect("nonzero edit ID"),
        PhotoId::new(1).expect("nonzero photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        operations,
    )
    .expect("valid edit");
    CompiledOperationGraph::compile(&edit).expect("registered operations")
}

fn rotate_operation(id: u128, rx: i64, ry: i64, angle: f64) -> Operation {
    typed_operation(
        id,
        "rusttable.rotatepixels",
        vec![
            ("rx", ParameterValue::Integer(rx)),
            ("ry", ParameterValue::Integer(ry)),
            (
                "angle",
                ParameterValue::Scalar(FiniteF64::new(angle).expect("finite angle")),
            ),
        ],
    )
}

fn typed_operation(id: u128, key: &str, parameters: Vec<(&str, ParameterValue)>) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero ID"),
        OperationKey::new(key).expect("valid key"),
        true,
        OperationOpacity::ONE,
        parameters
            .into_iter()
            .map(|(name, value)| (ParameterName::new(name).expect("name"), value)),
    )
    .expect("typed operation")
}

fn image(width: u32, height: u32) -> RgbaF32Image {
    let dimensions = RasterDimensions::new(width, height).expect("dimensions");
    let pixel_count = u16::try_from(dimensions.pixel_count()).expect("small test image");
    let denominator = f32::from(pixel_count + 1);
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let index = u16::try_from(index).expect("small test image");
            let value = f32::from(index + 1) / denominator;
            RgbaF32Pixel::new(
                value,
                value / 2.0,
                value / 4.0,
                f32::from(index) / denominator,
            )
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::with_source_representation(
            dimensions,
            RgbaF32ColorEncoding::LinearSrgbD65,
            RgbaF32SourceRepresentation::U16,
        ),
        pixels,
    )
    .expect("valid image")
}

#[test]
fn crop_and_quarter_turn_publish_odd_non_square_frame_and_alpha() {
    let input = image(7, 5);
    let source = input.pixels().to_vec();
    let graph = graph(vec![
        operation(
            1,
            "rusttable.crop",
            &[
                ("cx", 0.0),
                ("cy", 0.0),
                ("cw", 0.5),
                ("ch", 1.0),
                ("ratio_n", 0.0),
                ("ratio_d", 0.0),
            ],
        ),
        operation(2, "rusttable.flip", &[("mode", 1.0), ("orientation", 6.0)]),
        operation(3, "rusttable.linear_offset", &[("value", 0.01)]),
    ]);
    let snapshot = CpuPixelpipeSnapshot::new(input, graph, CpuPixelpipeOutputMode::FullExport);

    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("frame-boundary execution");

    assert_eq!(
        result.image().descriptor().dimensions(),
        RasterDimensions::new(5, 4).expect("dimensions")
    );
    assert_eq!(
        result.image().descriptor().source_representation(),
        RgbaF32SourceRepresentation::U16
    );
    let expected_source_indices = [
        28, 21, 14, 7, 0, 29, 22, 15, 8, 1, 30, 23, 16, 9, 2, 31, 24, 17, 10, 3,
    ];
    for (actual, source_index) in result.image().pixels().iter().zip(expected_source_indices) {
        let expected = source[source_index];
        assert!((actual.red() - (expected.red() + 0.01)).abs() < 1.0e-6);
        assert_eq!(actual.alpha().to_bits(), expected.alpha().to_bits());
    }
    assert_eq!(
        result.receipt().output_descriptor(),
        result.image().descriptor()
    );
}

#[test]
fn rotatepixels_matches_its_existing_frame_plan_for_rgb_and_alpha() {
    use rusttable_processing::operations::rotatepixels::{
        RotatePixelsConfig, RotatePixelsInterpolation, RotatePixelsParametersV1, RotatePixelsPlan,
    };

    let input = image(9, 7);
    let dimensions = input.descriptor().dimensions();
    let config =
        RotatePixelsConfig::new(RotatePixelsParametersV1::new(1, 3, 17.0)).expect("rotate config");
    let direct_plan =
        RotatePixelsPlan::new(dimensions, config, RotatePixelsInterpolation::Bilinear)
            .expect("direct plan");
    let rgb = input
        .pixels()
        .iter()
        .map(|pixel| {
            LinearRgb::new(
                FiniteF32::new(pixel.red()).expect("red"),
                FiniteF32::new(pixel.green()).expect("green"),
                FiniteF32::new(pixel.blue()).expect("blue"),
            )
        })
        .collect::<Vec<_>>();
    let expected_rgb = direct_plan.execute(&rgb).expect("direct RGB");
    let alpha = input
        .pixels()
        .iter()
        .map(|pixel| pixel.alpha())
        .collect::<Vec<_>>();
    let expected_alpha = direct_plan
        .execute_interleaved(&alpha, 1, dimensions.width() as usize)
        .expect("direct alpha");
    let graph = graph(vec![rotate_operation(4, 1, 3, 17.0)]);
    let snapshot = CpuPixelpipeSnapshot::new(input, graph, CpuPixelpipeOutputMode::FullExport);

    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("rotatepixels boundary");

    assert_eq!(
        result.image().descriptor().dimensions(),
        direct_plan.output_dimensions()
    );
    for ((actual, expected), alpha) in result
        .image()
        .pixels()
        .iter()
        .zip(expected_rgb.pixels())
        .zip(expected_alpha)
    {
        assert!((actual.red() - expected.red().get()).abs() < 1.0e-6);
        assert!((actual.green() - expected.green().get()).abs() < 1.0e-6);
        assert!((actual.blue() - expected.blue().get()).abs() < 1.0e-6);
        assert!((actual.alpha() - alpha).abs() < 1.0e-6);
    }
}

#[test]
fn legal_tiled_geometry_path_is_identical_to_full_frame() {
    let graph = graph(vec![
        operation(
            5,
            "rusttable.crop",
            &[
                ("cx", 0.0),
                ("cy", 0.0),
                ("cw", 0.5),
                ("ch", 1.0),
                ("ratio_n", 0.0),
                ("ratio_d", 0.0),
            ],
        ),
        operation(6, "rusttable.flip", &[("mode", 1.0), ("orientation", 2.0)]),
    ]);
    let executor = CpuPixelpipeExecutor;
    for mode in [
        CpuPixelpipeOutputMode::Preview,
        CpuPixelpipeOutputMode::FullExport,
    ] {
        let snapshot = CpuPixelpipeSnapshot::new(image(7, 5), graph.clone(), mode);
        let full = executor.execute(&snapshot).expect("full frame");
        let tiled = executor
            .execute_tiled(&snapshot, CpuTilePlan::new(2, 2).expect("tile plan"))
            .expect("legal full-frame tiled path");

        assert_eq!(tiled.image(), full.image());
        assert_eq!(tiled.receipt(), full.receipt());
    }
}

#[test]
fn partial_opacity_geometry_is_rejected_before_frame_replacement() {
    let graph = graph(vec![operation_with_opacity(
        7,
        "rusttable.flip",
        &[("mode", 1.0), ("orientation", 2.0)],
        0.5,
    )]);
    let snapshot =
        CpuPixelpipeSnapshot::new(image(5, 3), graph, CpuPixelpipeOutputMode::FullExport);

    let error = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect_err("partial geometry opacity is unsupported");

    assert!(matches!(error, CpuPixelpipeError::Evaluation { .. }));
    assert!(error.to_string().contains("requires full opacity"));
    assert!(!error.to_string().contains("GeometryRequiresFrameBoundary"));
}

#[test]
fn scale_finalscale_and_canvas_chain_resolves_each_replacement_frame() {
    let graph = graph(vec![
        operation(8, "rusttable.scalepixels", &[("pixel_aspect_ratio", 1.5)]),
        typed_operation(
            9,
            "rusttable.finalscale",
            vec![
                ("mode", ParameterValue::Integer(1)),
                ("width", ParameterValue::Integer(6)),
                ("height", ParameterValue::Integer(5)),
                ("allow_upscale", ParameterValue::Integer(1)),
                ("kernel", ParameterValue::Integer(1)),
                ("quality", ParameterValue::Integer(3)),
            ],
        ),
        typed_operation(
            10,
            "rusttable.enlargecanvas",
            vec![
                (
                    "percent_left",
                    ParameterValue::Scalar(FiniteF64::new(50.0).expect("finite")),
                ),
                (
                    "percent_right",
                    ParameterValue::Scalar(FiniteF64::new(25.0).expect("finite")),
                ),
                (
                    "percent_top",
                    ParameterValue::Scalar(FiniteF64::new(20.0).expect("finite")),
                ),
                (
                    "percent_bottom",
                    ParameterValue::Scalar(FiniteF64::new(40.0).expect("finite")),
                ),
                ("color", ParameterValue::Integer(2)),
            ],
        ),
    ]);
    let dimensions = RasterDimensions::new(5, 3).expect("dimensions");
    let plan = FrameBoundaryPlan::new(
        &graph,
        dimensions,
        FrameBoundaryOptions::new(FrameBoundaryMode::Export),
    )
    .expect("checked frame plan");
    assert_eq!(plan.boundary_count(), 3);
    assert_eq!(
        plan.output_dimensions(),
        RasterDimensions::new(10, 8).expect("dimensions")
    );
    let input = image(5, 3);
    let snapshot = CpuPixelpipeSnapshot::new(
        input.clone(),
        graph.clone(),
        CpuPixelpipeOutputMode::FullExport,
    );

    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("three replacement frames");

    assert_eq!(
        result.image().descriptor().dimensions(),
        plan.output_dimensions()
    );
    assert_eq!(result.image().pixels().len(), 80);
    let canvas_corner = result.image().pixels()[0];
    assert_eq!(
        (
            canvas_corner.red(),
            canvas_corner.green(),
            canvas_corner.blue()
        ),
        (0.0, 0.0, 1.0)
    );
    assert_eq!(canvas_corner.alpha().to_bits(), 1.0_f32.to_bits());

    let working = WorkingRgbImage::new(
        dimensions,
        input
            .pixels()
            .iter()
            .map(|pixel| {
                LinearRgb::new(
                    FiniteF32::new(pixel.red()).expect("red"),
                    FiniteF32::new(pixel.green()).expect("green"),
                    FiniteF32::new(pixel.blue()).expect("blue"),
                )
            })
            .collect(),
    )
    .expect("working input");
    assert_eq!(
        evaluate_graph(&graph, &working)
            .expect("public full-frame evaluator")
            .dimensions(),
        plan.output_dimensions()
    );
}

#[test]
fn preview_and_export_share_geometry_alpha_and_receipt_dimensions() {
    let graph = graph(vec![
        operation(11, "rusttable.scalepixels", &[("pixel_aspect_ratio", 0.5)]),
        typed_operation(
            12,
            "rusttable.enlargecanvas",
            vec![
                (
                    "percent_left",
                    ParameterValue::Scalar(FiniteF64::new(20.0).expect("finite")),
                ),
                (
                    "percent_right",
                    ParameterValue::Scalar(FiniteF64::new(0.0).expect("finite")),
                ),
                (
                    "percent_top",
                    ParameterValue::Scalar(FiniteF64::new(25.0).expect("finite")),
                ),
                (
                    "percent_bottom",
                    ParameterValue::Scalar(FiniteF64::new(0.0).expect("finite")),
                ),
                ("color", ParameterValue::Integer(4)),
            ],
        ),
    ]);
    let input = image(5, 3);
    let preview_snapshot = CpuPixelpipeSnapshot::new(
        input.clone(),
        graph.clone(),
        CpuPixelpipeOutputMode::Preview,
    );
    let export_snapshot =
        CpuPixelpipeSnapshot::new(input, graph, CpuPixelpipeOutputMode::FullExport);
    let executor = CpuPixelpipeExecutor;

    let preview = executor.execute(&preview_snapshot).expect("preview");
    let export = executor.execute(&export_snapshot).expect("export");

    assert_eq!(
        preview.image().descriptor().dimensions(),
        export.image().descriptor().dimensions()
    );
    assert_eq!(
        preview.receipt().output_descriptor().dimensions(),
        export.receipt().output_descriptor().dimensions()
    );
    for (preview, export) in preview.image().pixels().iter().zip(export.image().pixels()) {
        assert_eq!(preview.alpha().to_bits(), export.alpha().to_bits());
        assert!((preview.red() - encode_srgb(export.red())).abs() < 2.0e-6);
        assert!((preview.green() - encode_srgb(export.green())).abs() < 2.0e-6);
        assert!((preview.blue() - encode_srgb(export.blue())).abs() < 2.0e-6);
    }
}

#[test]
fn geometry_planning_rejects_oversized_output_before_allocation() {
    let graph = graph(vec![typed_operation(
        13,
        "rusttable.finalscale",
        vec![
            ("mode", ParameterValue::Integer(1)),
            ("width", ParameterValue::Integer(40_000)),
            ("height", ParameterValue::Integer(40_000)),
            ("allow_upscale", ParameterValue::Integer(1)),
        ],
    )]);
    let snapshot =
        CpuPixelpipeSnapshot::new(image(3, 1), graph, CpuPixelpipeOutputMode::FullExport);

    let error = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect_err("oversized plan");

    assert!(matches!(error, CpuPixelpipeError::Evaluation { .. }));
    assert!(error.to_string().contains("limit is"));
}

#[test]
fn cancellation_prevents_geometry_publication() {
    let graph = graph(vec![operation(
        14,
        "rusttable.scalepixels",
        &[("pixel_aspect_ratio", 1.5)],
    )]);
    let snapshot =
        CpuPixelpipeSnapshot::new(image(5, 3), graph, CpuPixelpipeOutputMode::FullExport);
    let scope = CancellationScope::root(PipelineGeneration::new(869).expect("generation"));
    scope.cancel(CancellationReason::EditChanged);

    let error = CpuPixelpipeExecutor
        .execute_with_cancellation(&snapshot, &scope)
        .expect_err("cancelled geometry");

    assert!(matches!(error, CpuPixelpipeError::Cancelled(_)));
}

fn encode_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}
