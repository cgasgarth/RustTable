use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterText, ParameterValue, PhotoId, Revision,
};
use rusttable_image::{
    ColorEncoding as ImageColorEncoding, ImageDimensions, Orientation, Roi, SourceColor,
};
use rusttable_pixelpipe::{
    CancellationReason, CancellationScope, CpuPixelpipeError, CpuPixelpipeExecutor,
    CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan, PipelineGeneration,
    RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
    RgbaF32SourceRepresentation,
};
use rusttable_processing::operations::clipping::{
    ClippingConfig, ClippingInterpolation, ClippingParametersV5, ClippingPlan,
};
use rusttable_processing::{
    CompiledOperationGraph, DistortionBorderMode, DistortionPlan, DistortionSamplingPolicy,
    FiniteF32, FrameBoundaryMode, FrameBoundaryOptions, FrameBoundaryPlan, LinearRgb,
    RasterDimensions, WorkingRgbImage, evaluate_graph,
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
    image_with_orientation(width, height, Orientation::Normal)
}

fn image_with_orientation(width: u32, height: u32, orientation: Orientation) -> RgbaF32Image {
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
        )
        .with_source_orientation(orientation),
        pixels,
    )
    .expect("valid image")
}

fn encoded_image_with_evidence(width: u32, height: u32, orientation: Orientation) -> RgbaF32Image {
    let dimensions = RasterDimensions::new(width, height).expect("dimensions");
    let pixel_count = u16::try_from(dimensions.pixel_count()).expect("small test image");
    let denominator = f32::from(pixel_count + 1);
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let index = u16::try_from(index).expect("small test image");
            let value = f32::from(index + 1) / denominator;
            RgbaF32Pixel::new(
                value,
                value * 0.75,
                value * 0.25,
                f32::from(index) / denominator,
            )
        })
        .collect();
    let source_color = SourceColor::declared(ImageColorEncoding::SrgbD65).expect("declared sRGB");
    RgbaF32Image::new(
        RgbaF32Descriptor::with_source_representation(
            dimensions,
            RgbaF32ColorEncoding::SrgbD65,
            RgbaF32SourceRepresentation::U8,
        )
        .with_source_orientation(orientation)
        .with_source_color(source_color),
        pixels,
    )
    .expect("valid encoded image")
}

fn rec2020_colorin_operation(id: u128) -> Operation {
    typed_operation(
        id,
        "rusttable.colorin",
        vec![
            (
                "input_profile",
                ParameterValue::Text(ParameterText::new("builtin:srgb").expect("input profile")),
            ),
            (
                "working_profile",
                ParameterValue::Text(
                    ParameterText::new("builtin:linear-rec2020").expect("working profile"),
                ),
            ),
            ("intent", ParameterValue::Integer(0)),
            ("normalize", ParameterValue::Integer(0)),
            ("blue_mapping", ParameterValue::Bool(true)),
        ],
    )
}

#[test]
fn automatic_flip_executes_all_source_orientations_once_on_a_non_square_frame() {
    let orientations = [
        Orientation::Normal,
        Orientation::FlipHorizontal,
        Orientation::Rotate180,
        Orientation::FlipVertical,
        Orientation::Transpose,
        Orientation::Rotate90,
        Orientation::Transverse,
        Orientation::Rotate270,
    ];
    let graph = graph(vec![operation(
        875,
        "rusttable.flip",
        &[("mode", 0.0), ("orientation", 0.0)],
    )]);
    for orientation in orientations {
        let input = image_with_orientation(3, 2, orientation);
        let source = input.pixels().to_vec();
        let result = CpuPixelpipeExecutor
            .execute(&CpuPixelpipeSnapshot::new(
                input,
                graph.clone(),
                CpuPixelpipeOutputMode::FullExport,
            ))
            .expect("automatic source orientation");
        let output_dimensions = orientation.output_dimensions(
            rusttable_image::ImageDimensions::new(3, 2).expect("image dimensions"),
        );
        assert_eq!(
            result.image().descriptor().dimensions(),
            RasterDimensions::new(output_dimensions.width(), output_dimensions.height()).unwrap()
        );
        assert_eq!(
            result.image().descriptor().source_orientation(),
            Orientation::Normal
        );
        for y in 0..2 {
            for x in 0..3 {
                let (output_x, output_y) = orientation.map_source_to_output(
                    rusttable_image::ImageDimensions::new(3, 2).unwrap(),
                    x,
                    y,
                );
                let source_index = usize::try_from(y * 3 + x).unwrap();
                let output_index =
                    usize::try_from(output_y * output_dimensions.width() + output_x).unwrap();
                assert_eq!(
                    result.image().pixels()[output_index],
                    source[source_index],
                    "{orientation:?} source ({x}, {y})"
                );
            }
        }
    }
}

#[test]
fn automatic_flip_rotate90_is_idempotent_across_snapshot_reingress() {
    let input = encoded_image_with_evidence(3, 2, Orientation::Rotate90);
    let input_descriptor = input.descriptor();
    let automatic_flip = graph(vec![operation(
        879,
        "rusttable.flip",
        &[("mode", 0.0), ("orientation", 0.0)],
    )]);
    let executor = CpuPixelpipeExecutor;

    let first = executor
        .execute(&CpuPixelpipeSnapshot::new(
            input,
            automatic_flip.clone(),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("first automatic flip");
    let second = executor
        .execute(&CpuPixelpipeSnapshot::new(
            first.image().clone(),
            automatic_flip,
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("re-ingressed automatic flip");
    let output_dimensions = RasterDimensions::new(2, 3).expect("rotated dimensions");
    let expected_descriptor = input_descriptor
        .with_dimensions_and_color_encoding(output_dimensions, RgbaF32ColorEncoding::LinearSrgbD65)
        .with_source_orientation(Orientation::Normal);

    assert_eq!(first.image().descriptor(), expected_descriptor);
    assert_eq!(second.image(), first.image());
}

#[test]
fn crop_only_preserves_rotate90_for_automatic_flip_after_reingress() {
    let input = encoded_image_with_evidence(7, 5, Orientation::Rotate90);
    let input_descriptor = input.descriptor();
    let crop = operation(
        883,
        "rusttable.crop",
        &[
            ("cx", 0.0),
            ("cy", 0.0),
            ("cw", 0.5),
            ("ch", 1.0),
            ("ratio_n", 0.0),
            ("ratio_d", 0.0),
        ],
    );
    let automatic_flip = operation(
        884,
        "rusttable.flip",
        &[("mode", 0.0), ("orientation", 0.0)],
    );
    let executor = CpuPixelpipeExecutor;

    let cropped = executor
        .execute(&CpuPixelpipeSnapshot::new(
            input.clone(),
            graph(vec![crop.clone()]),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("source-derived crop");
    let reingressed = executor
        .execute(&CpuPixelpipeSnapshot::new(
            cropped.image().clone(),
            graph(vec![automatic_flip.clone()]),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("automatic flip after crop re-ingress");
    let direct = executor
        .execute(&CpuPixelpipeSnapshot::new(
            input,
            graph(vec![crop, automatic_flip]),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("single-snapshot crop and automatic flip");

    assert_eq!(
        cropped.image().descriptor(),
        input_descriptor.with_dimensions_and_color_encoding(
            cropped.image().descriptor().dimensions(),
            RgbaF32ColorEncoding::LinearSrgbD65,
        )
    );
    assert_eq!(
        cropped.image().descriptor().source_orientation(),
        Orientation::Rotate90
    );
    assert_eq!(
        reingressed.image().descriptor().source_orientation(),
        Orientation::Normal
    );
    assert_eq!(reingressed.image(), direct.image());
}

#[test]
fn rec2020_colorin_geometry_exports_canonical_linear_srgb_with_full_evidence() {
    let input = encoded_image_with_evidence(7, 5, Orientation::Rotate270);
    let input_descriptor = input.descriptor();
    let geometry = vec![
        operation(
            881,
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
        operation(
            882,
            "rusttable.flip",
            &[("mode", 1.0), ("orientation", 6.0)],
        ),
    ];
    let executor = CpuPixelpipeExecutor;

    let canonical = executor
        .execute(&CpuPixelpipeSnapshot::new(
            input.clone(),
            graph(vec![rec2020_colorin_operation(880)]),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("canonical non-geometry colorin");
    let expected = executor
        .execute(&CpuPixelpipeSnapshot::new(
            canonical.image().clone(),
            graph(geometry.clone()),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("canonical linear-sRGB geometry");
    let actual = executor
        .execute(&CpuPixelpipeSnapshot::new(
            input,
            graph(
                std::iter::once(rec2020_colorin_operation(880))
                    .chain(geometry)
                    .collect(),
            ),
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("Rec.2020 colorin geometry");

    assert_eq!(
        actual.image().descriptor(),
        input_descriptor
            .with_dimensions_and_color_encoding(
                expected.image().descriptor().dimensions(),
                RgbaF32ColorEncoding::LinearSrgbD65,
            )
            .with_source_orientation(Orientation::Normal)
    );
    assert_eq!(actual.image().descriptor(), expected.image().descriptor());
    for (actual, expected) in actual
        .image()
        .pixels()
        .iter()
        .zip(expected.image().pixels())
    {
        assert!((actual.red() - expected.red()).abs() <= 2.0e-6);
        assert!((actual.green() - expected.green()).abs() <= 2.0e-6);
        assert!((actual.blue() - expected.blue()).abs() <= 2.0e-6);
        assert_eq!(actual.alpha().to_bits(), expected.alpha().to_bits());
    }
}

#[test]
fn repeated_automatic_flip_nodes_consume_source_orientation_only_once() {
    let graph = graph(vec![
        operation(
            876,
            "rusttable.flip",
            &[("mode", 0.0), ("orientation", 0.0)],
        ),
        operation(
            877,
            "rusttable.flip",
            &[("mode", 0.0), ("orientation", 0.0)],
        ),
    ]);
    let input = image_with_orientation(3, 2, Orientation::Rotate90);
    let result = CpuPixelpipeExecutor
        .execute(&CpuPixelpipeSnapshot::new(
            input,
            graph,
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect("source orientation is consumed once");

    assert_eq!(
        result.image().descriptor().dimensions(),
        RasterDimensions::new(2, 3).unwrap()
    );
    let red = result
        .image()
        .pixels()
        .iter()
        .map(|pixel| pixel.red())
        .collect::<Vec<_>>();
    assert_eq!(
        red,
        vec![
            4.0 / 7.0,
            1.0 / 7.0,
            5.0 / 7.0,
            2.0 / 7.0,
            6.0 / 7.0,
            3.0 / 7.0
        ]
    );
}

#[test]
fn source_orientation_participates_in_snapshot_cache_identity() {
    let graph = graph(vec![operation(
        878,
        "rusttable.flip",
        &[("mode", 0.0), ("orientation", 0.0)],
    )]);
    let normal = CpuPixelpipeSnapshot::new(
        image_with_orientation(3, 2, Orientation::Normal),
        graph.clone(),
        CpuPixelpipeOutputMode::Preview,
    );
    let rotated = CpuPixelpipeSnapshot::new(
        image_with_orientation(3, 2, Orientation::Rotate90),
        graph,
        CpuPixelpipeOutputMode::Preview,
    );

    assert_eq!(normal.source_identity(), rotated.source_identity());
    assert_ne!(normal.identity(), rotated.identity());
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

#[test]
fn distortion_operations_publish_frame_plans_and_receipts() {
    let input = image(7, 5);
    let executor = CpuPixelpipeExecutor;
    for (id, key, parameters) in [
        (
            20,
            "rusttable.ashift",
            vec![(
                "rotation",
                ParameterValue::Scalar(FiniteF64::new(8.0).expect("finite")),
            )],
        ),
        (
            21,
            "rusttable.clipping",
            vec![
                (
                    "angle",
                    ParameterValue::Scalar(FiniteF64::new(90.0).expect("finite")),
                ),
                ("crop_auto", ParameterValue::Bool(false)),
            ],
        ),
        (22, "rusttable.lenscorrection", Vec::new()),
    ] {
        let graph = graph(vec![typed_operation(id, key, parameters)]);
        let expected = FrameBoundaryPlan::new(
            &graph,
            RasterDimensions::new(7, 5).expect("dimensions"),
            FrameBoundaryOptions::new(FrameBoundaryMode::Export),
        )
        .expect("distortion plan");
        assert_eq!(expected.boundary_count(), 1);
        assert_ne!(expected.identity(), [0; 32]);

        let result = executor
            .execute(&CpuPixelpipeSnapshot::new(
                input.clone(),
                graph,
                CpuPixelpipeOutputMode::FullExport,
            ))
            .expect("distortion execution");
        assert_eq!(
            result.image().descriptor().dimensions(),
            expected.output_dimensions()
        );
        assert_eq!(result.receipt().frame_plan_identity(), expected.identity());
        assert_eq!(
            result.receipt().output_descriptor(),
            result.image().descriptor()
        );
        assert!(
            result
                .image()
                .pixels()
                .iter()
                .all(|pixel| pixel.alpha().is_finite())
        );
    }
}

#[test]
fn distortion_plan_exposes_inverse_roi_policy_and_coordinate_alignment() {
    let dimensions = RasterDimensions::new(100, 60).expect("dimensions");
    let parameters = ClippingParametersV5 {
        angle: 90.0,
        crop_auto: false,
        ..ClippingParametersV5::default()
    };
    let clipping = ClippingPlan::new(
        dimensions,
        ClippingConfig::new(parameters).expect("clipping config"),
        ClippingInterpolation::Bilinear,
    )
    .expect("clipping plan");
    let plan = DistortionPlan::Clipping(clipping);
    assert_eq!(
        plan.sampling_policy(),
        DistortionSamplingPolicy::Clipping {
            interpolation: ClippingInterpolation::Bilinear,
            border: DistortionBorderMode::Clamp,
        }
    );

    let output = plan
        .output_roi(Roi::full(
            ImageDimensions::new(100, 60).expect("image dimensions"),
        ))
        .expect("output ROI");
    let input = plan.input_roi(output).expect("input ROI with halo");
    assert!(input.width() <= 100 && input.height() <= 60);

    let source_point = [20.0, 15.0];
    let output_point = plan.forward_point(source_point).expect("forward map");
    let restored = plan.back_point(output_point).expect("inverse map");
    assert!((restored[0] - source_point[0]).abs() < 1.0e-6);
    assert!((restored[1] - source_point[1]).abs() < 1.0e-6);
}

#[test]
fn distortion_full_frame_and_tiled_paths_are_consistent_and_cancel_cleanly() {
    let graph = graph(vec![typed_operation(
        23,
        "rusttable.clipping",
        vec![
            (
                "angle",
                ParameterValue::Scalar(FiniteF64::new(90.0).expect("finite")),
            ),
            ("crop_auto", ParameterValue::Bool(false)),
        ],
    )]);
    let input = image(7, 5);
    let snapshot =
        CpuPixelpipeSnapshot::new(input.clone(), graph, CpuPixelpipeOutputMode::FullExport);
    let executor = CpuPixelpipeExecutor;
    let full = executor.execute(&snapshot).expect("full frame");
    let tiled = executor
        .execute_tiled(&snapshot, CpuTilePlan::new(3, 2).expect("tile plan"))
        .expect("tiled frame");
    assert_eq!(full.image(), tiled.image());
    assert_eq!(
        full.receipt().output_identity(),
        tiled.receipt().output_identity()
    );
    assert_eq!(
        full.receipt().frame_plan_identity(),
        tiled.receipt().frame_plan_identity()
    );

    let scope = CancellationScope::root(PipelineGeneration::new(870).expect("generation"));
    scope.cancel(CancellationReason::EditChanged);
    let error = executor
        .execute_with_cancellation(&snapshot, &scope)
        .expect_err("cancelled distortion");
    assert!(matches!(error, CpuPixelpipeError::Cancelled(_)));
}

#[test]
fn invalid_distortion_fails_before_output_publication() {
    let graph = graph(vec![typed_operation(
        24,
        "rusttable.clipping",
        vec![
            ("k_type", ParameterValue::Integer(99)),
            ("crop_auto", ParameterValue::Bool(true)),
        ],
    )]);
    let error = CpuPixelpipeExecutor
        .execute(&CpuPixelpipeSnapshot::new(
            image(7, 5),
            graph,
            CpuPixelpipeOutputMode::FullExport,
        ))
        .expect_err("invalid clipping mode");
    assert!(matches!(error, CpuPixelpipeError::Evaluation { .. }));
    assert!(!error.to_string().contains("GeometryRequiresFrameBoundary"));
}

fn encode_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}
