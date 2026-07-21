use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{GpuRuntime, GpuRuntimeConfig};
use rusttable_pixelpipe::{
    CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan,
    PixelpipeBackend, PixelpipeExecutionService, RgbaF32ColorEncoding, RgbaF32Descriptor,
    RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions};

fn operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero ID"),
        OperationKey::new(key).expect("valid key"),
        true,
        OperationOpacity::ONE,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("valid parameter"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite parameter")),
            )
        }),
    )
    .expect("valid operation")
}

#[test]
fn tiled_service_falls_back_without_partial_publication() {
    let snapshot = snapshot();
    let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
    let selected = PixelpipeExecutionService::cpu_only()
        .execute_tiled(&snapshot, CpuTilePlan::new(1, 1).expect("tile plan"))
        .expect("tiled service result");

    assert_eq!(selected.image(), canonical.image());
    assert_eq!(
        selected.receipt().backend(),
        PixelpipeBackend::CpuTiledFallback
    );
    let tiling = selected.receipt().tiling().expect("tiling receipt");
    assert_eq!(tiling.tile_count(), 2);
    assert_eq!(tiling.attempts(), 0);
}

fn snapshot() -> CpuPixelpipeSnapshot {
    let dimensions = RasterDimensions::new(2, 1).expect("dimensions");
    let image = RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::SrgbD65),
        vec![
            RgbaF32Pixel::new(0.5, 0.25, 0.75, 0.4),
            RgbaF32Pixel::new(0.1, 0.2, 0.3, 1.0),
        ],
    )
    .expect("image");
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(3),
        vec![
            operation(7, "rusttable.exposure", &[("stops", 1.0)]),
            operation(8, "rusttable.linear_offset", &[("value", 0.1)]),
            operation(
                9,
                "rusttable.rgb_gain",
                &[("red", 0.5), ("green", 1.5), ("blue", 2.0)],
            ),
        ],
    )
    .expect("edit");
    CpuPixelpipeSnapshot::new(
        image,
        CompiledOperationGraph::compile(&edit).expect("graph"),
        CpuPixelpipeOutputMode::Preview,
    )
}

#[test]
fn cpu_only_grain_service_is_pixel_identical_to_canonical_reference() {
    let snapshot = grain_snapshot(2.0);
    let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
    let selected = PixelpipeExecutionService::cpu_only()
        .execute(&snapshot)
        .expect("service result");

    assert_eq!(selected.image(), canonical.image());
    assert_eq!(selected.receipt().backend(), PixelpipeBackend::CpuCanonical);
}

#[tokio::test]
async fn qualified_wgpu_grain_service_matches_cpu_reference_when_gpu_is_available() {
    for channel in 0..4 {
        let Ok(runtime) = GpuRuntime::initialize(GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let snapshot = grain_snapshot(f64::from(channel));
        let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
        let selected = PixelpipeExecutionService::with_gpu(runtime)
            .execute(&snapshot)
            .expect("GPU grain result");

        assert_eq!(selected.receipt().backend(), PixelpipeBackend::WgpuBasic);
        assert_eq!(selected.receipt().dispatches(), 1);
        for (actual, expected) in selected
            .image()
            .pixels()
            .iter()
            .zip(canonical.image().pixels())
        {
            assert!(
                (actual.red() - expected.red()).abs() < 0.0015,
                "channel {channel}: red {actual:?} != {expected:?}"
            );
            assert!(
                (actual.green() - expected.green()).abs() < 0.0015,
                "channel {channel}: green {actual:?} != {expected:?}"
            );
            assert!(
                (actual.blue() - expected.blue()).abs() < 0.0015,
                "channel {channel}: blue {actual:?} != {expected:?}"
            );
            assert_eq!(actual.alpha().to_bits(), expected.alpha().to_bits());
        }
    }
}

#[tokio::test]
async fn tiled_wgpu_grain_dispatch_matches_full_frame_cpu_coordinates() {
    let Ok(runtime) = GpuRuntime::initialize(GpuRuntimeConfig::default()).await else {
        return;
    };
    if runtime.is_cpu_only() {
        return;
    }
    let snapshot = grain_snapshot(3.0);
    let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
    let selected = PixelpipeExecutionService::with_gpu(runtime)
        .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
        .expect("tiled GPU grain result");

    assert_eq!(selected.receipt().backend(), PixelpipeBackend::WgpuTiled);
    assert_eq!(selected.receipt().dispatches(), 4);
    assert_eq!(
        selected.image().pixels().len(),
        canonical.image().pixels().len()
    );
    for (actual, expected) in selected
        .image()
        .pixels()
        .iter()
        .zip(canonical.image().pixels())
    {
        assert!((actual.red() - expected.red()).abs() < 0.0015);
        assert!((actual.green() - expected.green()).abs() < 0.0015);
        assert!((actual.blue() - expected.blue()).abs() < 0.0015);
        assert_eq!(actual.alpha().to_bits(), expected.alpha().to_bits());
    }
}

fn grain_snapshot(channel: f64) -> CpuPixelpipeSnapshot {
    let dimensions = RasterDimensions::new(4, 2).expect("dimensions");
    let image = RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::SrgbD65),
        vec![
            RgbaF32Pixel::new(0.45, 0.35, 0.55, 0.4),
            RgbaF32Pixel::new(0.25, 0.5, 0.4, 1.0),
            RgbaF32Pixel::new(0.65, 0.3, 0.2, 0.8),
            RgbaF32Pixel::new(0.2, 0.4, 0.7, 0.6),
            RgbaF32Pixel::new(0.55, 0.45, 0.3, 0.9),
            RgbaF32Pixel::new(0.35, 0.6, 0.25, 0.7),
            RgbaF32Pixel::new(0.7, 0.4, 0.5, 0.5),
            RgbaF32Pixel::new(0.3, 0.3, 0.6, 1.0),
        ],
    )
    .expect("image");
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(4),
        vec![operation(
            0x1234,
            "rusttable.grain",
            &[
                ("channel", channel),
                ("scale", 1600.0 / 213.2),
                ("strength", 25.0),
                ("midtones_bias", 100.0),
            ],
        )],
    )
    .expect("edit");
    CpuPixelpipeSnapshot::new(
        image,
        CompiledOperationGraph::compile(&edit).expect("graph"),
        CpuPixelpipeOutputMode::Preview,
    )
}

#[test]
fn cpu_only_basic_service_is_pixel_identical_to_canonical_reference() {
    let snapshot = snapshot();
    let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
    let selected = PixelpipeExecutionService::cpu_only()
        .execute(&snapshot)
        .expect("service result");

    assert_eq!(selected.image(), canonical.image());
    assert_eq!(selected.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert_eq!(selected.receipt().dispatches(), 0);
}

#[tokio::test]
async fn qualified_wgpu_basic_service_matches_cpu_reference_when_gpu_is_available() {
    let Ok(runtime) = GpuRuntime::initialize(GpuRuntimeConfig::default()).await else {
        return;
    };
    if runtime.is_cpu_only() {
        return;
    }
    let snapshot = snapshot();
    let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
    let selected = PixelpipeExecutionService::with_gpu(runtime)
        .execute(&snapshot)
        .expect("GPU service result");

    assert_eq!(selected.receipt().backend(), PixelpipeBackend::WgpuBasic);
    assert_eq!(selected.receipt().dispatches(), 3);
    for (actual, expected) in selected
        .image()
        .pixels()
        .iter()
        .zip(canonical.image().pixels())
    {
        assert!((actual.red() - expected.red()).abs() < 0.00001);
        assert!((actual.green() - expected.green()).abs() < 0.00001);
        assert!((actual.blue() - expected.blue()).abs() < 0.00001);
        assert_eq!(actual.alpha().to_bits(), expected.alpha().to_bits());
    }
}
