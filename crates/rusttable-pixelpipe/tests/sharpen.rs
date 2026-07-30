//! Public CPU pixelpipe expectations for Darktable's `src/iop/sharpen.c`.
//!
//! These tests intentionally use only the production graph/snapshot API. They
//! become runnable when the integration owner registers `rusttable.sharpen` and
//! connects neighborhood-aware CPU tile extraction; no GPU implementation is
//! claimed by this milestone.

#![allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]

use std::time::Duration;

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{GpuInitError, GpuRuntime, GpuRuntimeConfig};
use rusttable_masks::{
    GeometryAncestry, MaskGeometry, MaskGraphBuilder, MaskIdentity, MaskNode, MaskRaster, MaskRoi,
    MaskSource,
};
use rusttable_pixelpipe::{
    CancellationDeadline, CancellationReason, CancellationScope, CancellationStage,
    CpuPixelpipeError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeScaleContext,
    CpuPixelpipeSnapshot, CpuTilePlan, PipelineGeneration, PixelpipeBackend,
    PixelpipeExecutionService, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions};

fn sharpen_operation(id: u128, radius: f64, amount: f64, threshold: f64) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new("rusttable.sharpen").expect("valid operation key"),
        true,
        OperationOpacity::ONE,
        [
            ("radius", radius),
            ("amount", amount),
            ("threshold", threshold),
        ]
        .into_iter()
        .map(|(name, value)| {
            (
                ParameterName::new(name).expect("valid parameter name"),
                ParameterValue::Scalar(FiniteF64::new(value).expect("finite parameter")),
            )
        }),
    )
    .expect("valid operation")
}

fn scalar_operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
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

fn graph(operation: Operation) -> CompiledOperationGraph {
    graph_from_operations(vec![operation])
}

fn graph_from_operations(operations: Vec<Operation>) -> CompiledOperationGraph {
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(3),
        operations,
    )
    .expect("valid edit");
    CompiledOperationGraph::compile(&edit).expect("registered Sharpen graph")
}

fn input(width: u32, height: u32) -> RgbaF32Image {
    let dimensions = RasterDimensions::new(width, height).expect("nonzero dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let x = f32::from(u16::try_from(index % u64::from(width)).expect("test x fits"));
            let y = f32::from(u16::try_from(index / u64::from(width)).expect("test y fits"));
            let detail = if (index + index / u64::from(width)).is_multiple_of(3) {
                0.17
            } else {
                -0.06
            };
            let alpha_step = f32::from(u8::try_from(index % 9).expect("alpha step fits"));
            RgbaF32Pixel::new(
                0.35 + x * 0.013 + y * 0.007 + detail,
                -0.12 + x * 0.005,
                0.09 - y * 0.004,
                0.21 + alpha_step * 0.071,
            )
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LabD50),
        pixels,
    )
    .expect("valid Lab image")
}

fn linear_input(width: u32, height: u32) -> RgbaF32Image {
    let dimensions = RasterDimensions::new(width, height).expect("nonzero dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let x = f32::from(u16::try_from(index % u64::from(width)).expect("test x fits"));
            let y = f32::from(u16::try_from(index / u64::from(width)).expect("test y fits"));
            RgbaF32Pixel::new(
                0.08 + x * 0.011 + y * 0.004,
                0.12 + x * 0.006 + y * 0.008,
                0.16 + x * 0.003 + y * 0.009,
                0.21 + f32::from(u8::try_from(index % 9).expect("alpha fits")) * 0.071,
            )
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LinearSrgbD65),
        pixels,
    )
    .expect("valid linear RGB image")
}

fn sharpen_snapshot(width: u32, height: u32, id: u128, radius: f64) -> CpuPixelpipeSnapshot {
    CpuPixelpipeSnapshot::new(
        input(width, height),
        graph(sharpen_operation(id, radius, 0.7, 0.025)),
        CpuPixelpipeOutputMode::FullExport,
    )
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
        "sharpen-mask",
        MaskSource::Raster,
        MaskGeometry::new(
            GeometryAncestry::identity(),
            MaskRoi::full(width, height),
            true,
        ),
        Some(MaskRaster::new(width, height, values).expect("valid Sharpen mask")),
        [],
    )
    .expect("valid Sharpen mask node");
    MaskGraphBuilder::new()
        .add_mask(node)
        .add_edge(identity, operation_id, 1)
        .build()
        .expect("valid Sharpen mask graph")
}

#[test]
fn zero_radius_and_native_undersized_images_are_identity() {
    let zero = sharpen_snapshot(17, 13, 10, 0.0);
    let zero_source = zero.input().clone();
    let zero_result = CpuPixelpipeExecutor
        .execute(&zero)
        .expect("zero-radius Sharpen");
    assert_eq!(zero_result.image(), &zero_source);

    // radius=2 commits to 5, producing an 11-wide kernel. sharpen.c passes
    // through when either dimension is below that width.
    let undersized = sharpen_snapshot(10, 13, 11, 2.0);
    let undersized_source = undersized.input().clone();
    let undersized_result = CpuPixelpipeExecutor
        .execute(&undersized)
        .expect("undersized Sharpen");
    assert_eq!(undersized_result.image(), &undersized_source);
}

#[test]
fn overlapped_tiled_execution_matches_full_frame_sharpen() {
    let snapshot = sharpen_snapshot(23, 17, 20, 1.6);
    let executor = CpuPixelpipeExecutor;
    let full = executor.execute(&snapshot).expect("full-frame Sharpen");
    let tiled = executor
        .execute_tiled(&snapshot, CpuTilePlan::new(4, 3).expect("tile plan"))
        .expect("overlapped tiled Sharpen");

    assert_eq!(tiled.image(), full.image());
    assert_eq!(tiled.receipt(), full.receipt());
}

#[test]
fn mixed_exposure_sharpen_vibrance_executes_in_authored_order_and_tiles_exactly() {
    let operations = vec![
        scalar_operation(60, "rusttable.exposure", &[("stops", 0.75)]),
        sharpen_operation(61, 1.6, 0.7, 0.025),
        scalar_operation(62, "rusttable.vibrance", &[("amount", 35.0)]),
    ];
    let snapshot = CpuPixelpipeSnapshot::new(
        linear_input(23, 17),
        graph_from_operations(operations),
        CpuPixelpipeOutputMode::FullExport,
    );
    let executor = CpuPixelpipeExecutor;
    let full = executor.execute(&snapshot).expect("mixed Sharpen graph");
    let tiled = executor
        .execute_tiled(&snapshot, CpuTilePlan::new(5, 4).expect("tile plan"))
        .expect("mixed tiled Sharpen graph");
    assert_eq!(tiled.image(), full.image());
    assert_eq!(
        full.receipt()
            .nodes()
            .iter()
            .map(|node| node.operation_id().get())
            .collect::<Vec<_>>(),
        [60, 61, 62]
    );

    let reordered = CpuPixelpipeSnapshot::new(
        linear_input(23, 17),
        graph_from_operations(vec![
            sharpen_operation(61, 1.6, 0.7, 0.025),
            scalar_operation(60, "rusttable.exposure", &[("stops", 0.75)]),
            scalar_operation(62, "rusttable.vibrance", &[("amount", 35.0)]),
        ]),
        CpuPixelpipeOutputMode::FullExport,
    );
    assert_ne!(
        executor
            .execute(&reordered)
            .expect("reordered graph")
            .image(),
        full.image()
    );
}

#[test]
fn nonunit_scale_multiple_neighborhoods_match_full_and_tiled_execution() {
    let scale = CpuPixelpipeScaleContext::new(0.5, 1.0).expect("native scale context");
    let snapshot = CpuPixelpipeSnapshot::new(
        input(29, 21),
        graph_from_operations(vec![
            sharpen_operation(70, 1.2, 0.65, 0.01),
            sharpen_operation(71, 1.6, 0.45, 0.03),
        ]),
        CpuPixelpipeOutputMode::FullExport,
    )
    .with_scale_context(scale);
    let executor = CpuPixelpipeExecutor;
    let full = executor.execute(&snapshot).expect("full neighborhoods");
    let tiled = executor
        .execute_tiled(&snapshot, CpuTilePlan::new(5, 4).expect("tile plan"))
        .expect("tiled neighborhoods");
    assert_eq!(tiled.image(), full.image());
}

#[test]
fn sharpen_mask_blends_only_lightness_and_matches_overlapped_tiles() {
    const WIDTH: u32 = 23;
    const HEIGHT: u32 = 17;
    let source = input(WIDTH, HEIGHT);
    let values = (0..WIDTH * HEIGHT)
        .map(|index| if index % WIDTH < WIDTH / 2 { 0.0 } else { 1.0 })
        .collect();
    let snapshot = CpuPixelpipeSnapshot::new(
        source.clone(),
        graph(sharpen_operation(80, 1.6, 0.7, 0.0)),
        CpuPixelpipeOutputMode::FullExport,
    )
    .with_mask_graph(mask_graph(80, WIDTH, HEIGHT, values));
    let executor = CpuPixelpipeExecutor;
    let full = executor.execute(&snapshot).expect("masked Sharpen");
    let tiled = executor
        .execute_tiled(&snapshot, CpuTilePlan::new(4, 3).expect("tile plan"))
        .expect("masked tiled Sharpen");
    assert_eq!(tiled.image(), full.image());

    let left_center = (HEIGHT / 2 * WIDTH + WIDTH / 4) as usize;
    assert_eq!(
        full.image().pixels()[left_center],
        source.pixels()[left_center]
    );
    let right_center = (HEIGHT / 2 * WIDTH + (WIDTH * 3 / 4)) as usize;
    assert_ne!(
        full.image().pixels()[right_center].red().to_bits(),
        source.pixels()[right_center].red().to_bits()
    );
    assert_eq!(
        full.image().pixels()[right_center].green().to_bits(),
        source.pixels()[right_center].green().to_bits()
    );
    assert_eq!(
        full.image().pixels()[right_center].blue().to_bits(),
        source.pixels()[right_center].blue().to_bits()
    );
    assert_eq!(
        full.image().pixels()[right_center].alpha().to_bits(),
        source.pixels()[right_center].alpha().to_bits()
    );
}

#[test]
fn production_lab_route_matches_native_luma_equation_and_physical_borders() {
    const WIDTH: u32 = 23;
    const HEIGHT: u32 = 17;
    const RADIUS: u32 = 4;
    let snapshot = sharpen_snapshot(WIDTH, HEIGHT, 30, 1.6);
    let source = snapshot.input().clone();
    let output = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("production Lab Sharpen")
        .image()
        .clone();

    let sigma2 = (1.6_f32 * 2.5).powi(2) / (2.5_f32 * 2.5);
    let mut kernel = (0..=RADIUS * 2)
        .map(|offset| {
            let distance = offset as f32 - RADIUS as f32;
            (-(distance * distance) / (2.0 * sigma2)).exp()
        })
        .collect::<Vec<_>>();
    let weight = kernel.iter().sum::<f32>();
    for value in &mut kernel {
        *value /= weight;
    }

    let center_x = WIDTH / 2;
    let center_y = HEIGHT / 2;
    let mut temporary = Vec::new();
    for x in center_x - RADIUS..=center_x + RADIUS {
        let mut vertical = 0.0_f32;
        for y in center_y - RADIUS..=center_y + RADIUS {
            vertical += kernel[(y - (center_y - RADIUS)) as usize]
                * source.pixels()[(y * WIDTH + x) as usize].red();
        }
        temporary.push(vertical);
    }
    let mut blurred = 0.0_f32;
    for (weight, value) in kernel.iter().zip(temporary) {
        blurred += weight * value;
    }
    let center = (center_y * WIDTH + center_x) as usize;
    let difference = source.pixels()[center].red() - blurred;
    let detail = if difference.abs() > 0.025 {
        (difference.abs() - 0.025).max(0.0).copysign(difference)
    } else {
        0.0
    };
    let expected = source.pixels()[center].red() + detail * 0.7;
    assert_eq!(output.pixels()[center].red().to_bits(), expected.to_bits());

    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let index = (y * WIDTH + x) as usize;
            let source_pixel = source.pixels()[index];
            let output_pixel = output.pixels()[index];
            if !(RADIUS..WIDTH - RADIUS).contains(&x) || !(RADIUS..HEIGHT - RADIUS).contains(&y) {
                assert_eq!(output_pixel, source_pixel, "physical border {x},{y}");
            }
            assert_eq!(
                output_pixel.green().to_bits(),
                source_pixel.green().to_bits()
            );
            assert_eq!(output_pixel.blue().to_bits(), source_pixel.blue().to_bits());
            assert_eq!(
                output_pixel.alpha().to_bits(),
                source_pixel.alpha().to_bits()
            );
        }
    }
}

#[test]
fn cpu_publication_binds_snapshot_and_sharpen_operation_identity() {
    let snapshot = sharpen_snapshot(19, 13, 40, 1.5);
    let identical = snapshot.clone();
    let changed_radius = sharpen_snapshot(19, 13, 40, 1.6);
    let changed_scale = snapshot
        .clone()
        .with_scale_context(CpuPixelpipeScaleContext::new(0.5, 1.0).expect("native scale context"));
    assert_eq!(snapshot.identity(), identical.identity());
    assert_ne!(snapshot.identity(), changed_radius.identity());
    assert_ne!(snapshot.identity(), changed_scale.identity());
    assert_ne!(
        CpuPixelpipeExecutor.execute(&snapshot).unwrap().image(),
        CpuPixelpipeExecutor
            .execute(&changed_scale)
            .unwrap()
            .image()
    );

    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("canonical Sharpen execution");
    assert_eq!(canonical.receipt().nodes().len(), 1);
    assert_eq!(canonical.receipt().nodes()[0].operation_id().get(), 40);

    let published = PixelpipeExecutionService::cpu_only()
        .execute(&snapshot)
        .expect("CPU-only Sharpen execution");
    assert_eq!(published.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(
        published.receipt().backend(),
        PixelpipeBackend::CpuCanonical
    );
    assert!(published.receipt().gpu_fallback().is_none());
    assert_eq!(published.receipt().dispatches(), 0);
    assert!(published.receipt().tiling().is_none());
}

#[tokio::test]
async fn active_sharpen_forces_canonical_cpu_instead_of_being_skipped_by_gpu_plan() {
    let runtime = match GpuRuntime::initialize(GpuRuntimeConfig::default()).await {
        Ok(runtime) => runtime,
        Err(GpuInitError::NoAdapter) => return,
        Err(error) => panic!("WGPU adapter initialization failed: {error}"),
    };
    let snapshot = CpuPixelpipeSnapshot::new(
        linear_input(23, 17),
        graph_from_operations(vec![
            scalar_operation(90, "rusttable.exposure", &[("stops", 0.5)]),
            sharpen_operation(91, 1.6, 0.7, 0.025),
            scalar_operation(92, "rusttable.vibrance", &[("amount", 25.0)]),
        ]),
        CpuPixelpipeOutputMode::FullExport,
    );
    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("canonical mixed Sharpen graph");
    let selected = PixelpipeExecutionService::with_gpu(runtime)
        .execute(&snapshot)
        .expect("selected mixed Sharpen graph");
    assert_eq!(selected.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert_eq!(selected.receipt().dispatches(), 0);
    assert_eq!(selected.image(), canonical.image());
}

#[test]
fn cancellation_during_neighborhood_tiling_exposes_no_partial_result() {
    let snapshot = sharpen_snapshot(1024, 1024, 49, 2.0);
    let scope = CancellationScope::root(PipelineGeneration::new(6).expect("generation"))
        .with_deadline(CancellationDeadline::after(Duration::from_millis(10)));
    let error = CpuPixelpipeExecutor
        .execute_tiled_with_cancellation(
            &snapshot,
            CpuTilePlan::new(1024, 1024).expect("single neighborhood tile"),
            &scope,
        )
        .expect_err("deadline must interrupt Sharpen neighborhood processing");
    let CpuPixelpipeError::Cancelled(error) = error else {
        panic!("neighborhood cancellation must remain typed");
    };
    assert!(matches!(
        error.stage(),
        Some(CancellationStage::Tile | CancellationStage::Node)
    ));

    let published = CpuPixelpipeExecutor
        .execute_tiled(
            &snapshot,
            CpuTilePlan::new(1024, 1024).expect("single neighborhood tile"),
        )
        .expect("a fresh uncancelled execution publishes the complete image");
    assert_eq!(published.image().pixels().len(), 1024 * 1024);
}

#[test]
fn cancellation_before_sharpen_publication_exposes_no_partial_result() {
    let snapshot = sharpen_snapshot(19, 13, 50, 1.5);
    let scope = CancellationScope::root(PipelineGeneration::new(7).expect("generation"));
    scope.cancel(CancellationReason::Shutdown);
    let service = PixelpipeExecutionService::cpu_only();

    assert!(matches!(
        service.execute_with_cancellation(&snapshot, &scope),
        Err(CpuPixelpipeError::Cancelled(_))
    ));

    let published = service
        .execute(&snapshot)
        .expect("uncancelled Sharpen remains publishable");
    assert_eq!(published.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(
        published.receipt().backend(),
        PixelpipeBackend::CpuCanonical
    );
}
