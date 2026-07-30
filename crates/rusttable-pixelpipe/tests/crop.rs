//! Source-derived CPU pixelpipe coverage for Darktable's `src/iop/crop.c`.
//!
//! The retained callbacks copy the selected ROI, preserve straight alpha, and
//! translate masks with the same integer source offset. These tests exercise
//! only the current public CPU frame/pixelpipe contract; Crop has no GPU path.

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_masks::{
    GeometryAncestry, MaskGeometry, MaskGraphBuilder, MaskIdentity, MaskNode, MaskRaster, MaskRoi,
    MaskSource,
};
use rusttable_pixelpipe::{
    CancellationReason, CancellationScope, CpuPixelpipeError, CpuPixelpipeExecutor,
    CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan, PipelineGeneration,
    PixelpipeBackend, PixelpipeExecutionService, RgbaF32ColorEncoding, RgbaF32Descriptor,
    RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions};

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

fn crop_operation(id: u128, cx: f64, cy: f64, cw: f64, ch: f64) -> Operation {
    operation(
        id,
        "rusttable.crop",
        &[
            ("cx", cx),
            ("cy", cy),
            ("cw", cw),
            ("ch", ch),
            ("ratio_n", 0.0),
            ("ratio_d", 0.0),
        ],
    )
}

fn input(width: u32, height: u32) -> RgbaF32Image {
    let dimensions = RasterDimensions::new(width, height).expect("nonzero dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let x = f32::from(u16::try_from(index % u64::from(width)).expect("test x fits in u16"));
            let y = f32::from(u16::try_from(index / u64::from(width)).expect("test y fits in u16"));
            let alpha_step = f32::from(u8::try_from(index % 7).expect("alpha step fits in u8"));
            RgbaF32Pixel::new(
                0.031 + x * 0.017 + y * 0.003,
                0.071 + x * 0.011 + y * 0.019,
                0.113 + x * 0.007 + y * 0.013,
                0.127 + alpha_step * 0.083,
            )
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LinearSrgbD65),
        pixels,
    )
    .expect("valid image")
}

fn mask_graph(operation_id: u128, width: u32, height: u32) -> rusttable_masks::MaskGraph {
    let identity = MaskIdentity::new(2, 3, 7, 1);
    let values = (0..u64::from(width) * u64::from(height))
        .map(|index| 0.1 + f32::from(u8::try_from(index % 9).expect("mask step fits in u8")) / 10.0)
        .collect();
    let node = MaskNode::new(
        identity,
        "source-roi-mask",
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
fn crop_preserves_nonuniform_rgba_bits_for_the_selected_roi() {
    let source = input(7, 5);
    let source_pixels = source.pixels().to_vec();
    let snapshot = CpuPixelpipeSnapshot::new(
        source,
        graph(vec![crop_operation(10, 0.2, 0.2, 0.8, 0.8)]),
        CpuPixelpipeOutputMode::FullExport,
    );

    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("crop execution");

    assert_eq!(
        result.image().descriptor().dimensions(),
        RasterDimensions::new(4, 4).unwrap()
    );
    let expected_indices = [
        8, 9, 10, 11, // source row 1, x 1..4
        15, 16, 17, 18, 22, 23, 24, 25, 29, 30, 31, 32,
    ];
    let actual = result.image().pixels();
    assert_eq!(actual.len(), expected_indices.len());
    for (pixel, source_index) in actual.iter().zip(expected_indices) {
        assert_eq!(
            *pixel, source_pixels[source_index],
            "source index {source_index}"
        );
    }
}

#[test]
fn crop_full_frame_and_legal_tiled_execution_have_identical_public_results() {
    let snapshot = CpuPixelpipeSnapshot::new(
        input(11, 7),
        graph(vec![crop_operation(20, 0.25, 0.0, 0.75, 1.0)]),
        CpuPixelpipeOutputMode::FullExport,
    );
    let executor = CpuPixelpipeExecutor;
    let full = executor.execute(&snapshot).expect("full-frame crop");
    let tiled = executor
        .execute_tiled(&snapshot, CpuTilePlan::new(3, 2).expect("legal tile plan"))
        .expect("legal tiled crop");

    assert_eq!(tiled.image(), full.image());
    assert_eq!(tiled.receipt(), full.receipt());
}

#[test]
fn crop_selects_the_source_roi_after_a_source_space_masked_operation() {
    let source = input(8, 6);
    let source_pixels = source.pixels().to_vec();
    let crop_x = 2_u32;
    let crop_y = 1_u32;
    let crop_width = 4_u32;
    let crop_height = 4_u32;
    let masked_operation_id = 30_u128;
    let snapshot = CpuPixelpipeSnapshot::new(
        source,
        graph(vec![
            operation(
                masked_operation_id,
                "rusttable.linear_offset",
                &[("value", 0.25)],
            ),
            crop_operation(31, 0.25, 1.0 / 6.0, 0.75, 5.0 / 6.0),
        ]),
        CpuPixelpipeOutputMode::FullExport,
    )
    .with_mask_graph(mask_graph(masked_operation_id, 8, 6));

    let result = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("masked crop execution");

    assert_eq!(
        result.image().descriptor().dimensions(),
        RasterDimensions::new(crop_width, crop_height).unwrap()
    );
    for (output_index, output) in result.image().pixels().iter().enumerate() {
        let output_index = u32::try_from(output_index).expect("test output index fits in u32");
        let x = output_index % crop_width;
        let y = output_index / crop_width;
        let source_index = usize::try_from((crop_y + y) * 8 + crop_x + x).unwrap();
        let coverage =
            0.1 + f32::from(u8::try_from(source_index % 9).expect("mask step fits in u8")) / 10.0;
        let source_pixel = source_pixels[source_index];
        for (actual, expected) in [
            (output.red(), source_pixel.red() + 0.25 * coverage),
            (output.green(), source_pixel.green() + 0.25 * coverage),
            (output.blue(), source_pixel.blue() + 0.25 * coverage),
        ] {
            assert!((actual - expected).abs() < 1.0e-6, "{actual} != {expected}");
        }
        assert_eq!(output.alpha().to_bits(), source_pixel.alpha().to_bits());
    }
}

#[test]
fn cpu_only_crop_publishes_canonical_backend_and_binds_snapshot_identity() {
    let snapshot = CpuPixelpipeSnapshot::new(
        input(9, 6),
        graph(vec![crop_operation(40, 0.125, 0.0, 0.875, 1.0)]),
        CpuPixelpipeOutputMode::FullExport,
    );
    let changed_crop = CpuPixelpipeSnapshot::new(
        input(9, 6),
        graph(vec![crop_operation(40, 0.125, 0.0, 0.75, 1.0)]),
        CpuPixelpipeOutputMode::FullExport,
    );
    assert_eq!(snapshot.identity(), snapshot.clone().identity());
    assert_ne!(snapshot.identity(), changed_crop.identity());

    let service = PixelpipeExecutionService::cpu_only();
    let result = service.execute(&snapshot).expect("CPU-only crop execution");

    assert_eq!(result.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(result.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert!(result.receipt().gpu_fallback().is_none());
    assert_eq!(result.receipt().dispatches(), 0);
    assert!(result.receipt().tiling().is_none());
}

#[test]
fn cancellation_before_crop_publication_exposes_no_partial_result() {
    let snapshot = CpuPixelpipeSnapshot::new(
        input(9, 6),
        graph(vec![crop_operation(50, 0.125, 0.0, 0.875, 1.0)]),
        CpuPixelpipeOutputMode::FullExport,
    );
    let generation = PipelineGeneration::new(1).expect("generation");
    let scope = CancellationScope::root(generation);
    scope.cancel(CancellationReason::Shutdown);
    let service = PixelpipeExecutionService::cpu_only();

    let cancelled = service.execute_with_cancellation(&snapshot, &scope);
    assert!(matches!(cancelled, Err(CpuPixelpipeError::Cancelled(_))));

    let published = service
        .execute(&snapshot)
        .expect("uncancelled crop is still publishable");
    assert_eq!(published.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(
        published.receipt().backend(),
        PixelpipeBackend::CpuCanonical
    );
}
