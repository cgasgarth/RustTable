//! Source-derived frame/pixelpipe coverage for Darktable's
//! `src/iop/enlargecanvas.c` process and mask paths.
//!
//! The native module fills new RGBA pixels with one of five opaque colors,
//! copies the source at the resolved canvas offset, and fills enlarged mask
//! pixels with zero. It does not provide a `process_cl` callback.

use std::cell::Cell;

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{GpuInitError, GpuRuntime, GpuRuntimeConfig};
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ColorEncoding, ImageDescriptor, ImageDimensions,
    Orientation, PixelFormat, SampleType, StorageLayout,
};
use rusttable_masks::{
    GeometryAncestry, MaskGeometry, MaskGraphBuilder, MaskIdentity, MaskNode, MaskRaster, MaskRoi,
    MaskSource,
};
use rusttable_pixelpipe::{
    CancellationReason, CancellationScope, CpuPixelpipeError, CpuPixelpipeExecutor,
    CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, PipelineGeneration, PixelpipeBackend,
    PixelpipeExecutionService, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::operations::enlargecanvas::{
    CanvasColor, CanvasFill, EnlargeCanvasConfig, EnlargeCanvasExecutionError, EnlargeCanvasPlan,
};
use rusttable_processing::{
    CompiledOperationGraph, FiniteF32, FrameBoundaryMode, FrameBoundaryOptions, LinearRgb,
    OperationMaskSet, RasterDimensions, WorkingFrameDescriptor, WorkingRgbImage,
    evaluate_graph_at_frame_boundaries_with_masks,
};

async fn gpu_runtime() -> Option<GpuRuntime> {
    let config = GpuRuntimeConfig {
        allow_cpu_fallback: false,
        ..GpuRuntimeConfig::default()
    };
    match GpuRuntime::initialize(config).await {
        Ok(runtime) => Some(runtime),
        Err(GpuInitError::NoAdapter) => None,
        Err(error) => panic!("WGPU adapter initialization failed: {error}"),
    }
}

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

fn enlarge_operation(
    id: u128,
    left: f64,
    right: f64,
    top: f64,
    bottom: f64,
    color: CanvasColor,
) -> Operation {
    let scalar = |name: &'static str, value| {
        (
            ParameterName::new(name).expect("valid parameter name"),
            ParameterValue::Scalar(FiniteF64::new(value).expect("finite parameter")),
        )
    };
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new("rusttable.enlargecanvas").expect("valid operation key"),
        true,
        OperationOpacity::ONE,
        [
            scalar("percent_left", left),
            scalar("percent_right", right),
            scalar("percent_top", top),
            scalar("percent_bottom", bottom),
            (
                ParameterName::new("color").expect("valid parameter name"),
                ParameterValue::Integer(i64::from(color as u32)),
            ),
        ],
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

#[expect(
    clippy::suboptimal_flops,
    reason = "Preserve source-derived Enlarge Canvas fixture arithmetic order"
)]
fn input(width: u32, height: u32) -> RgbaF32Image {
    let dimensions = RasterDimensions::new(width, height).expect("nonzero dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let step = f32::from(u16::try_from(index).expect("small test image"));
            RgbaF32Pixel::new(
                0.125 + step * 0.03125,
                0.25 + step * 0.015_625,
                0.375 + step * 0.007_812_5,
                0.2 + step * 0.15,
            )
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LinearSrgbD65),
        pixels,
    )
    .expect("valid image")
}

fn snapshot_for(color: CanvasColor) -> CpuPixelpipeSnapshot {
    CpuPixelpipeSnapshot::new(
        input(2, 2),
        graph(vec![enlarge_operation(10, 50.0, 50.0, 50.0, 50.0, color)]),
        CpuPixelpipeOutputMode::FullExport,
    )
}

const fn expected_fill(color: CanvasColor) -> [f32; 3] {
    match color {
        CanvasColor::Green => [0.0, 1.0, 0.0],
        CanvasColor::Red => [1.0, 0.0, 0.0],
        CanvasColor::Blue => [0.0, 0.0, 1.0],
        CanvasColor::Black => [0.0, 0.0, 0.0],
        CanvasColor::White => [1.0, 1.0, 1.0],
    }
}

fn assert_rgb_bits(actual: RgbaF32Pixel, expected: [f32; 3]) {
    for (actual, expected) in [actual.red(), actual.green(), actual.blue()]
        .into_iter()
        .zip(expected)
    {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
}

fn assert_sample_bits(actual: [f32; 4], expected: [f32; 4]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert_eq!(actual.to_bits(), expected.to_bits());
    }
}

#[test]
fn all_five_native_colors_fill_opaque_canvas_and_preserve_source_rgba() {
    for color in [
        CanvasColor::Green,
        CanvasColor::Red,
        CanvasColor::Blue,
        CanvasColor::Black,
        CanvasColor::White,
    ] {
        let snapshot = snapshot_for(color);
        let source = snapshot.input().pixels().to_vec();
        let result = CpuPixelpipeExecutor
            .execute(&snapshot)
            .expect("enlarge canvas execution");
        assert_eq!(
            result.image().descriptor().dimensions(),
            RasterDimensions::new(5, 5).unwrap()
        );
        let fill = expected_fill(color);
        for (index, pixel) in result.image().pixels().iter().copied().enumerate() {
            if ![6, 7, 11, 12].contains(&index) {
                assert_rgb_bits(pixel, fill);
                assert_eq!(pixel.alpha().to_bits(), 1.0_f32.to_bits());
            }
        }
        for (output_index, source_index) in [(6, 0), (7, 1), (11, 2), (12, 3)] {
            assert_eq!(result.image().pixels()[output_index], source[source_index]);
        }
    }
}

#[test]
#[expect(
    clippy::suboptimal_flops,
    reason = "Preserve the native Enlarge Canvas mask blend arithmetic order"
)]
fn enlarged_mask_uses_source_placement_and_zero_coverage_on_canvas() {
    let masked_operation_id = 21_u128;
    let source = input(2, 1);
    let source_pixels = source.pixels().to_vec();
    let snapshot = CpuPixelpipeSnapshot::new(
        source,
        graph(vec![
            enlarge_operation(20, 100.0, 0.0, 0.0, 0.0, CanvasColor::Green),
            operation(
                masked_operation_id,
                "rusttable.linear_offset",
                &[("value", 0.4)],
            ),
        ]),
        CpuPixelpipeOutputMode::FullExport,
    )
    .with_mask_graph(mask_graph(masked_operation_id, 2, 1, vec![0.25, 1.0]));

    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("masked enlarge canvas execution");
    assert_eq!(
        result.image().descriptor().dimensions(),
        RasterDimensions::new(5, 5).unwrap()
    );
    for (index, pixel) in result.image().pixels().iter().copied().enumerate() {
        if ![13, 14].contains(&index) {
            assert_rgb_bits(pixel, [0.0, 1.0, 0.0]);
            assert_eq!(pixel.alpha().to_bits(), 1.0_f32.to_bits());
        }
    }
    for (index, coverage) in [0.25_f32, 1.0].into_iter().enumerate() {
        let actual = result.image().pixels()[index + 13];
        let source = source_pixels[index];
        for (actual, expected) in [
            (actual.red(), source.red() + 0.4 * coverage),
            (actual.green(), source.green() + 0.4 * coverage),
            (actual.blue(), source.blue() + 0.4 * coverage),
        ] {
            assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
        }
        assert_eq!(actual.alpha().to_bits(), source.alpha().to_bits());
    }
}

#[test]
#[expect(
    clippy::suboptimal_flops,
    reason = "Preserve the native Enlarge Canvas frame-mask blend arithmetic order"
)]
fn frame_evaluator_preserves_right_bottom_placement_for_a_following_masked_operation() {
    let masked_operation_id = OperationId::new(31).expect("operation ID");
    let dimensions = RasterDimensions::new(2, 1).expect("dimensions");
    let source_pixels = vec![
        LinearRgb::new(
            FiniteF32::new(0.1).unwrap(),
            FiniteF32::new(0.2).unwrap(),
            FiniteF32::new(0.3).unwrap(),
        ),
        LinearRgb::new(
            FiniteF32::new(0.4).unwrap(),
            FiniteF32::new(0.5).unwrap(),
            FiniteF32::new(0.6).unwrap(),
        ),
    ];
    let source = WorkingRgbImage::new_with_frame(
        dimensions,
        source_pixels.clone(),
        WorkingFrameDescriptor::rec2020(),
    )
    .expect("working source");
    let graph = graph(vec![
        enlarge_operation(30, 0.0, 100.0, 0.0, 100.0, CanvasColor::Green),
        operation(
            masked_operation_id.get(),
            "rusttable.linear_offset",
            &[("value", 0.4)],
        ),
    ]);
    let coverage = [0.25_f32, 1.0];
    let masks = OperationMaskSet::from_entries([(
        masked_operation_id,
        MaskRaster::new(2, 1, coverage.to_vec()).expect("source mask"),
    )])
    .expect("operation masks");

    let evaluated = evaluate_graph_at_frame_boundaries_with_masks(
        &graph,
        &source,
        &[0.2, 0.35],
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        Some(&masks),
        || false,
    )
    .expect("frame evaluation");

    assert_eq!(
        evaluated.image().dimensions(),
        RasterDimensions::new(5, 5).unwrap()
    );
    for (index, (source, coverage)) in source_pixels.into_iter().zip(coverage).enumerate() {
        let actual = evaluated.image().pixel_slice()[index];
        for (actual, expected) in [
            (actual.red().get(), source.red().get() + 0.4 * coverage),
            (actual.green().get(), source.green().get() + 0.4 * coverage),
            (actual.blue().get(), source.blue().get() + 0.4 * coverage),
        ] {
            assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
        }
    }
    for index in 2..evaluated.image().pixel_slice().len() {
        assert_eq!(
            evaluated.image().pixel_slice()[index],
            CanvasColor::Green.fill().rgb_pixel()
        );
    }
    assert_eq!(evaluated.alpha()[0].to_bits(), 0.2_f32.to_bits());
    assert_eq!(evaluated.alpha()[1].to_bits(), 0.35_f32.to_bits());
    assert!(
        evaluated.alpha()[2..]
            .iter()
            .all(|alpha| alpha.to_bits() == 1.0_f32.to_bits())
    );
}

#[test]
fn frame_evaluator_cancellation_during_mask_rewrite_publishes_no_frame() {
    let masked_operation_id = OperationId::new(31).expect("operation ID");
    let dimensions = RasterDimensions::new(2, 1).expect("dimensions");
    let source = WorkingRgbImage::new_with_frame(
        dimensions,
        vec![
            LinearRgb::new(
                FiniteF32::new(0.1).unwrap(),
                FiniteF32::new(0.2).unwrap(),
                FiniteF32::new(0.3).unwrap(),
            ),
            LinearRgb::new(
                FiniteF32::new(0.4).unwrap(),
                FiniteF32::new(0.5).unwrap(),
                FiniteF32::new(0.6).unwrap(),
            ),
        ],
        WorkingFrameDescriptor::rec2020(),
    )
    .expect("working source");
    let graph = graph(vec![
        enlarge_operation(30, 0.0, 100.0, 0.0, 100.0, CanvasColor::Green),
        operation(
            masked_operation_id.get(),
            "rusttable.linear_offset",
            &[("value", 0.4)],
        ),
    ]);
    let masks = OperationMaskSet::from_entries([(
        masked_operation_id,
        MaskRaster::new(2, 1, vec![0.25, 1.0]).expect("source mask"),
    )])
    .expect("operation masks");
    let polls = Cell::new(0_u8);

    let error = evaluate_graph_at_frame_boundaries_with_masks(
        &graph,
        &source,
        &[0.2, 0.35],
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        Some(&masks),
        || {
            let next = polls.get() + 1;
            polls.set(next);
            // RGB and alpha boundary execution consume the first seven polls;
            // this fires from the subsequent active-mask rewrite.
            next == 10
        },
    )
    .expect_err("cancelled mask rewrite must not publish a frame");

    assert!(error.is_cancelled());
    assert_eq!(polls.get(), 10);
}

fn mask_graph(
    operation_id: u128,
    width: u32,
    height: u32,
    values: Vec<f32>,
) -> rusttable_masks::MaskGraph {
    let identity = MaskIdentity::new(2, 3, 7, 1);
    let node = MaskNode::new(
        identity,
        "source-canvas-mask",
        MaskSource::Raster,
        MaskGeometry::new(
            GeometryAncestry::identity(),
            MaskRoi::full(width, height),
            true,
        ),
        Some(MaskRaster::new(width, height, values).expect("valid mask")),
        [],
    )
    .expect("valid mask node");
    MaskGraphBuilder::new()
        .add_mask(node)
        .add_edge(identity, operation_id, 1)
        .build()
        .expect("valid mask graph")
}

#[test]
fn native_f32_image_copy_skips_padded_source_row_bytes_and_maps_alpha_channels() {
    let dimensions = RasterDimensions::new(2, 2).unwrap();
    let plan = EnlargeCanvasPlan::new_with_fill(
        EnlargeCanvasConfig::new(50.0, 0.0, 50.0, 0.0, CanvasColor::Black).unwrap(),
        dimensions,
        CanvasFill::new(0.25, 0.5, 0.75, 0.625).unwrap(),
    )
    .unwrap();
    let format = PixelFormat::new(
        SampleType::F32,
        ChannelLayout::Rgba,
        AlphaMode::Straight,
        ByteOrder::Native,
        StorageLayout::Interleaved,
    )
    .unwrap();
    let descriptor = ImageDescriptor::with_strides(
        ImageDimensions::new(2, 2).unwrap(),
        format,
        ColorEncoding::LinearSrgb,
        None,
        Orientation::Normal,
        &[40],
    )
    .unwrap();
    let source_pixels = [
        [1.0, 2.0, 3.0, 0.1],
        [4.0, 5.0, 6.0, 0.2],
        [7.0, 8.0, 9.0, 0.3],
        [10.0, 11.0, 12.0, 0.4],
    ];
    let mut input = vec![0xa5; descriptor.byte_length()];
    for y in 0..2 {
        for x in 0..2 {
            let offset = descriptor.pixel_offset(x, y).unwrap();
            input[offset..offset + 16].copy_from_slice(&pixel_bytes(
                source_pixels[usize::try_from(y * 2 + x).unwrap()],
            ));
        }
    }

    let output = plan.execute_image(&descriptor, &input).unwrap();
    assert_eq!(
        output.descriptor().dimensions(),
        ImageDimensions::new(5, 5).unwrap()
    );
    for (x, y, expected) in [
        (3, 3, source_pixels[0]),
        (4, 3, source_pixels[1]),
        (3, 4, source_pixels[2]),
        (4, 4, source_pixels[3]),
    ] {
        assert_sample_bits(
            read_pixel(output.descriptor(), output.bytes(), x, y),
            expected,
        );
    }
    for (x, y) in [(0, 0), (1, 0), (2, 0), (0, 1), (0, 2)] {
        assert_sample_bits(
            read_pixel(output.descriptor(), output.bytes(), x, y),
            [0.25, 0.5, 0.75, 0.625],
        );
    }
}

fn pixel_bytes(values: [f32; 4]) -> [u8; 16] {
    let mut bytes = [0_u8; 16];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

fn read_pixel(descriptor: &ImageDescriptor, bytes: &[u8], x: u32, y: u32) -> [f32; 4] {
    let offset = descriptor.pixel_offset(x, y).unwrap();
    std::array::from_fn(|channel| {
        let start = offset + channel * 4;
        f32::from_ne_bytes(bytes[start..start + 4].try_into().unwrap())
    })
}

#[test]
fn snapshot_and_frame_plan_identity_bind_canvas_parameters() {
    let red = snapshot_for(CanvasColor::Red);
    let blue = snapshot_for(CanvasColor::Blue);
    assert_ne!(red.identity(), blue.identity());

    let executor = CpuPixelpipeExecutor;
    let red_result = executor.execute(&red).unwrap();
    let blue_result = executor.execute(&blue).unwrap();
    assert_ne!(
        red_result.receipt().frame_plan_identity(),
        blue_result.receipt().frame_plan_identity()
    );
}

#[test]
fn canvas_execution_remains_cpu_canonical_without_a_proven_gpu_dispatch() {
    let snapshot = snapshot_for(CanvasColor::White);
    let result = PixelpipeExecutionService::cpu_only()
        .execute(&snapshot)
        .expect("CPU canvas execution");

    assert_eq!(result.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(result.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert!(result.receipt().gpu_fallback().is_none());
    assert_eq!(result.receipt().dispatches(), 0);
}

#[tokio::test]
async fn available_gpu_still_qualifies_canvas_for_canonical_cpu() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let snapshot = snapshot_for(CanvasColor::Black);
    let result = PixelpipeExecutionService::with_gpu(runtime)
        .execute(&snapshot)
        .expect("canvas CPU fallback");

    assert_eq!(result.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(result.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert!(result.receipt().gpu_fallback().is_none());
    assert_eq!(result.receipt().dispatches(), 0);
}

#[test]
fn late_row_cancellation_rejects_completed_rgb_and_mask_buffers() {
    let plan = EnlargeCanvasPlan::new(
        EnlargeCanvasConfig::new(50.0, 50.0, 50.0, 50.0, CanvasColor::Blue).unwrap(),
        RasterDimensions::new(2, 2).unwrap(),
    )
    .unwrap();
    let rgb = vec![CanvasColor::Red.fill().rgb_pixel(); 4];
    let rgb_checks = Cell::new(0_u8);
    assert!(matches!(
        plan.execute_with_cancel(&rgb, || {
            let next = rgb_checks.get() + 1;
            rgb_checks.set(next);
            next == 4
        }),
        Err(EnlargeCanvasExecutionError::Cancelled)
    ));

    let mask_checks = Cell::new(0_u8);
    assert!(matches!(
        plan.execute_mask_with_cancel(&[0.1, 0.2, 0.3, 0.4], || {
            let next = mask_checks.get() + 1;
            mask_checks.set(next);
            next == 4
        }),
        Err(EnlargeCanvasExecutionError::Cancelled)
    ));
}

#[test]
fn cancellation_before_canvas_publication_exposes_no_partial_result() {
    let snapshot = snapshot_for(CanvasColor::Blue);
    let generation = PipelineGeneration::new(1).unwrap();
    let scope = CancellationScope::root(generation);
    scope.cancel(CancellationReason::Shutdown);
    let service = PixelpipeExecutionService::cpu_only();

    assert!(matches!(
        service.execute_with_cancellation(&snapshot, &scope),
        Err(CpuPixelpipeError::Cancelled(_))
    ));
    let published = service.execute(&snapshot).expect("uncancelled execution");
    assert_eq!(published.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(
        published.receipt().backend(),
        PixelpipeBackend::CpuCanonical
    );
}
