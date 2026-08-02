use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_pixelpipe::{
    CancellationReason, CancellationScope, CancellationStage, CpuPixelpipeError,
    CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan,
    PipelineGeneration, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
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

fn graph(operation: Operation) -> CompiledOperationGraph {
    let edit = Edit::from_parts(
        EditId::new(1).expect("nonzero edit ID"),
        PhotoId::new(2).expect("nonzero photo ID"),
        Revision::ZERO,
        Revision::from_u64(3),
        [operation],
    )
    .expect("valid edit");
    CompiledOperationGraph::compile(&edit).expect("registered spatial operation")
}

fn image() -> RgbaF32Image {
    let dimensions = RasterDimensions::new(7, 5).expect("dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let x =
                f32::from(u16::try_from(index % u64::from(dimensions.width())).expect("small x"));
            let y =
                f32::from(u16::try_from(index / u64::from(dimensions.width())).expect("small y"));
            RgbaF32Pixel::new(
                0.15 + x * 0.01,
                0.25 + y * 0.02,
                0.35 + (x + y) * 0.01,
                0.4 + (x + y) * 0.01,
            )
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::SrgbD65),
        pixels,
    )
    .expect("valid image")
}

fn snapshot(id: u128, key: &str, parameters: &[(&str, f64)]) -> CpuPixelpipeSnapshot {
    CpuPixelpipeSnapshot::new(
        image(),
        graph(operation(id, key, parameters)),
        CpuPixelpipeOutputMode::FullExport,
    )
}

#[test]
fn production_snapshot_identity_covers_spatial_parameter_histories() {
    let relight = snapshot(
        0x7101,
        "rusttable.relight",
        &[("ev", 0.33), ("center", 0.0), ("width", 4.0)],
    );
    let relight_equal = snapshot(
        0x7101,
        "rusttable.relight",
        &[("ev", 0.33), ("center", 0.0), ("width", 4.0)],
    );
    let relight_changed = snapshot(
        0x7101,
        "rusttable.relight",
        &[("ev", 0.34), ("center", 0.0), ("width", 4.0)],
    );
    assert_eq!(relight.identity(), relight_equal.identity());
    assert_ne!(relight.identity(), relight_changed.identity());
    let vignette = snapshot(
        0x7102,
        "rusttable.vignette",
        &[("brightness", -0.5), ("dithering", 1.0)],
    );
    let vignette_equal = snapshot(
        0x7102,
        "rusttable.vignette",
        &[("brightness", -0.5), ("dithering", 1.0)],
    );
    let vignette_changed = snapshot(
        0x7102,
        "rusttable.vignette",
        &[("brightness", -0.49), ("dithering", 1.0)],
    );
    assert_eq!(vignette.identity(), vignette_equal.identity());
    assert_ne!(vignette.identity(), vignette_changed.identity());

    let graduated = snapshot(
        0x7103,
        "rusttable.graduatednd",
        &[("density", 1.0), ("hardness", 0.0), ("rotation", 0.0)],
    );
    let graduated_equal = snapshot(
        0x7103,
        "rusttable.graduatednd",
        &[("density", 1.0), ("hardness", 0.0), ("rotation", 0.0)],
    );
    let graduated_changed = snapshot(
        0x7103,
        "rusttable.graduatednd",
        &[("density", 1.01), ("hardness", 0.0), ("rotation", 0.0)],
    );
    assert_eq!(graduated.identity(), graduated_equal.identity());
    assert_ne!(graduated.identity(), graduated_changed.identity());
}

#[test]
fn spatial_tiled_execution_uses_one_full_raster_and_preserves_receipts() {
    let tile_plan = CpuTilePlan::new(2, 2).expect("tile plan");
    let requests = [
        snapshot(
            0x7201,
            "rusttable.vignette",
            &[
                ("scale", 0.0),
                ("falloff_scale", 100.0),
                ("brightness", -0.25),
                ("dithering", 1.0),
            ],
        ),
        snapshot(
            0x7202,
            "rusttable.graduatednd",
            &[
                ("density", 2.0),
                ("hardness", 75.0),
                ("rotation", 37.0),
                ("offset", 30.0),
            ],
        ),
    ];

    for request in &requests {
        let full = CpuPixelpipeExecutor
            .execute(request)
            .expect("full spatial execution");
        let tiled = CpuPixelpipeExecutor
            .execute_tiled(request, tile_plan)
            .expect("tiled spatial execution");
        assert_eq!(full.image(), tiled.image());
        assert_eq!(full.receipt(), tiled.receipt());
        assert_eq!(tiled.receipt().snapshot_identity(), request.identity());
    }
}

#[test]
fn relight_cancellation_after_tile_start_preserves_published_and_working_images() {
    let request = snapshot(
        0x7300,
        "rusttable.relight",
        &[("ev", 0.33), ("center", 0.0), ("width", 4.0)],
    );
    let tile_plan = CpuTilePlan::new(2, 2).expect("tile plan");
    let executor = CpuPixelpipeExecutor;
    let published = executor
        .execute_tiled(&request, tile_plan)
        .expect("publish initial Relight result");
    let published_image = published.image().clone();
    let working_before = request.input().pixels().to_vec();

    let generation = PipelineGeneration::new(0x7300).expect("generation");
    let scope = CancellationScope::root(generation);
    let callback_observed = Arc::new(AtomicBool::new(false));
    let callback_observed_clone = Arc::clone(&callback_observed);
    let cancellation_scope = scope.clone();
    let _work_started = scope.register_work_started(move || {
        callback_observed_clone.store(true, Ordering::Release);
        cancellation_scope.cancel(CancellationReason::EditChanged);
    });

    let result = executor.execute_tiled_with_cancellation(&request, tile_plan, &scope);
    assert!(callback_observed.load(Ordering::Acquire));
    let Err(CpuPixelpipeError::Cancelled(error)) = result else {
        panic!("cancellation after tile work must not publish a result");
    };
    assert_eq!(error.stage(), Some(CancellationStage::Tile));
    assert_eq!(published.image(), &published_image);
    assert_eq!(request.input().pixels(), working_before.as_slice());
}

#[test]
fn relight_remains_tileable_while_spatial_full_frame_requests_cancel_before_publication() {
    let relight = snapshot(
        0x7301,
        "rusttable.relight",
        &[("ev", 0.33), ("center", 0.0), ("width", 4.0)],
    );
    let tile_plan = CpuTilePlan::new(2, 2).expect("tile plan");
    let full = CpuPixelpipeExecutor
        .execute(&relight)
        .expect("full Relight execution");
    let tiled = CpuPixelpipeExecutor
        .execute_tiled(&relight, tile_plan)
        .expect("tiled Relight execution");
    assert_eq!(full.image(), tiled.image());
    assert_eq!(full.receipt(), tiled.receipt());

    for (id, key, parameters) in [
        (
            0x7302,
            "rusttable.vignette",
            vec![("brightness", -0.25), ("dithering", 1.0)],
        ),
        (
            0x7303,
            "rusttable.graduatednd",
            vec![("density", 2.0), ("rotation", 37.0)],
        ),
        (
            0x7304,
            "rusttable.relight",
            vec![("ev", 0.33), ("center", 0.0), ("width", 4.0)],
        ),
    ] {
        let request = snapshot(id, key, &parameters);
        let generation = PipelineGeneration::new(u64::try_from(id).expect("small generation"))
            .expect("generation");
        let scope = CancellationScope::root(generation);
        scope.cancel(CancellationReason::EditChanged);
        let result =
            CpuPixelpipeExecutor.execute_tiled_with_cancellation(&request, tile_plan, &scope);
        assert!(matches!(result, Err(CpuPixelpipeError::Cancelled(_))));
    }
}
