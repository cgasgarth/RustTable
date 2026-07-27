use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{GpuInitError, GpuRuntime, GpuRuntimeConfig};
use rusttable_pixelpipe::{
    CancellationReason, CancellationScope, CpuPixelpipeError, CpuPixelpipeExecutor,
    CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan, PipelineGeneration,
    PixelpipeBackend, PixelpipeExecutionService, RgbaF32ColorEncoding, RgbaF32Descriptor,
    RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions, builtin_registry};

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

fn bloom_operation(opacity: OperationOpacity, strength: f64) -> Operation {
    let operation_id = OperationId::new(0xb100).expect("Bloom operation ID");
    let defaults = builtin_registry()
        .materialize_operation("rusttable.bloom", operation_id)
        .expect("Bloom defaults");
    let parameters = defaults
        .parameters()
        .map(|(name, value)| {
            let replacement = match name.as_str() {
                "size" | "threshold" => Some(0.0),
                "strength" => Some(strength),
                _ => None,
            };
            (
                name.clone(),
                replacement.map_or_else(
                    || value.clone(),
                    |value| {
                        ParameterValue::Scalar(FiniteF64::new(value).expect("finite Bloom value"))
                    },
                ),
            )
        })
        .collect::<Vec<_>>();
    Operation::new_with_opacity(
        operation_id,
        defaults.key().clone(),
        true,
        opacity,
        parameters,
    )
    .expect("checked Bloom operation")
}

fn snapshot(operation: Operation) -> CpuPixelpipeSnapshot {
    let dimensions = RasterDimensions::new(4, 3).expect("dimensions");
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            RgbaF32Pixel::new(
                50.0,
                -12.0,
                18.0,
                f32::from_bits(0x3e80_0000 + u32::try_from(index).expect("small fixture")),
            )
        })
        .collect::<Vec<_>>();
    snapshot_with_pixels(operation, dimensions, pixels)
}

fn snapshot_with_pixels(
    operation: Operation,
    dimensions: RasterDimensions,
    pixels: Vec<RgbaF32Pixel>,
) -> CpuPixelpipeSnapshot {
    let input = RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LabD50),
        pixels,
    )
    .expect("Lab input");
    let edit = Edit::from_parts(
        EditId::new(0xb101).expect("edit ID"),
        PhotoId::new(0xb102).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(4),
        [operation],
    )
    .expect("Bloom edit");
    CpuPixelpipeSnapshot::new(
        input,
        CompiledOperationGraph::compile(&edit).expect("Bloom graph"),
        CpuPixelpipeOutputMode::FullExport,
    )
}

#[tokio::test]
async fn dedicated_bloom_gpu_matches_cpu_interior_and_stays_full_frame_when_available() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let dimensions = RasterDimensions::new(41, 41).expect("interior differential dimensions");
    let pixels = (0..dimensions.height())
        .flat_map(|y| {
            (0..dimensions.width()).map(move |x| {
                let index = y * dimensions.width() + x;
                let lightness = f32::from(
                    u16::try_from((x * 17 + y * 29 + x * y * 3) % 101).expect("bounded lightness"),
                );
                let x = f32::from(u16::try_from(x).expect("small x"));
                let y = f32::from(u16::try_from(y).expect("small y"));
                RgbaF32Pixel::new(
                    lightness,
                    (x - 20.0) * 2.5,
                    (20.0 - y) * 2.0,
                    f32::from_bits(0x3e80_0000 + index),
                )
            })
        })
        .collect::<Vec<_>>();
    let snapshot = snapshot_with_pixels(
        bloom_operation(OperationOpacity::new(0.5).expect("partial opacity"), 25.0),
        dimensions,
        pixels,
    );
    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("canonical CPU Bloom result");
    let service = PixelpipeExecutionService::with_gpu(runtime);

    let direct = service
        .execute(&snapshot)
        .expect("dedicated Bloom GPU result");
    let tiled = service
        .execute_tiled(
            &snapshot,
            CpuTilePlan::new(1, 1).expect("one-pixel request tiles"),
        )
        .expect("qualified full-frame Bloom GPU result");

    assert_eq!(direct.receipt().backend(), PixelpipeBackend::WgpuBloom);
    assert_eq!(direct.receipt().dispatches(), 18);
    assert_eq!(direct.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(tiled.receipt().backend(), PixelpipeBackend::WgpuBloom);
    assert_eq!(tiled.receipt().dispatches(), 18);
    assert_eq!(tiled.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(
        tiled
            .receipt()
            .tiling()
            .expect("full-frame tiling receipt")
            .tile_count(),
        1,
        "scaled overlap tiling remains explicit and must not run Bloom as independent tiles",
    );

    let width = usize::try_from(dimensions.width()).expect("small width");
    for (index, (((source, cpu), gpu), tiled_gpu)) in snapshot
        .input()
        .pixels()
        .iter()
        .zip(canonical.image().pixels())
        .zip(direct.image().pixels())
        .zip(tiled.image().pixels())
        .enumerate()
    {
        assert_eq!(gpu.green().to_bits(), source.green().to_bits());
        assert_eq!(gpu.blue().to_bits(), source.blue().to_bits());
        assert_eq!(gpu.alpha().to_bits(), source.alpha().to_bits());
        assert_eq!(tiled_gpu.green().to_bits(), source.green().to_bits());
        assert_eq!(tiled_gpu.blue().to_bits(), source.blue().to_bits());
        assert_eq!(tiled_gpu.alpha().to_bits(), source.alpha().to_bits());

        let x = index % width;
        let y = index / width;
        if (16..width - 16).contains(&x) && (16..width - 16).contains(&y) {
            let tolerance = 0.003 * cpu.red().abs().max(1.0);
            for actual in [gpu.red(), tiled_gpu.red()] {
                assert!(
                    (actual - cpu.red()).abs() <= tolerance,
                    "interior ({x}, {y}) expected {}, got {actual} (tolerance {tolerance})",
                    cpu.red(),
                );
            }
        }
        assert_eq!(gpu.red().to_bits(), tiled_gpu.red().to_bits());
    }
}

#[test]
fn qualified_bloom_without_a_gpu_publishes_the_canonical_fallback() {
    let snapshot = snapshot(bloom_operation(OperationOpacity::ONE, 25.0));
    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("canonical Bloom");
    let result = PixelpipeExecutionService::cpu_only()
        .execute(&snapshot)
        .expect("Bloom CPU fallback");

    assert_eq!(result.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert_eq!(result.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(result.image(), canonical.image());
}

#[test]
fn bloom_cancellation_is_terminal_before_fallback_or_publication() {
    let snapshot = snapshot(bloom_operation(OperationOpacity::ONE, 25.0));
    let scope = CancellationScope::root(
        PipelineGeneration::new(0xb103).expect("nonzero Bloom pipeline generation"),
    );
    scope.cancel(CancellationReason::EditChanged);

    let error = PixelpipeExecutionService::cpu_only()
        .execute_with_cancellation(&snapshot, &scope)
        .expect_err("cancelled Bloom must not publish a fallback");
    let CpuPixelpipeError::Cancelled(error) = error else {
        panic!("Bloom cancellation must remain typed");
    };
    assert_eq!(error.reason(), CancellationReason::EditChanged);
}

#[test]
fn bloom_parameters_participate_in_immutable_snapshot_identity() {
    let first = snapshot(bloom_operation(OperationOpacity::ONE, 25.0));
    let second = snapshot(bloom_operation(OperationOpacity::ONE, 26.0));

    assert_ne!(first.identity(), second.identity());
}

#[test]
fn bloom_parameter_names_used_by_the_gpu_fixture_remain_canonical() {
    let operation = bloom_operation(OperationOpacity::ONE, 25.0);
    for name in ["size", "threshold", "strength"] {
        assert!(
            operation
                .parameter(&ParameterName::new(name).expect("parameter name"))
                .is_some(),
            "missing canonical Bloom parameter {name}",
        );
    }
}
