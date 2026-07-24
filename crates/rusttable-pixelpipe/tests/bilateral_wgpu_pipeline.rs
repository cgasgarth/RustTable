use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{BilateralGridError, GpuRuntime, GpuRuntimeConfig};
use rusttable_masks::{
    GeometryAncestry, MaskGeometry, MaskGraphBuilder, MaskIdentity, MaskNode, MaskRaster, MaskRoi,
    MaskSource,
};
use rusttable_pixelpipe::{
    CancellationReason, CancellationScope, CancellationStage, CpuPixelpipeError,
    CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan,
    PipelineGeneration, PixelpipeBackend, PixelpipeExecutionService, PixelpipeGpuFallback,
    RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions};

fn operation(id: u128, key: &str, opacity: f64, parameters: &[(&str, f64)]) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new(key).expect("valid operation key"),
        true,
        OperationOpacity::new(opacity).expect("valid opacity"),
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

fn image() -> RgbaF32Image {
    let dimensions = RasterDimensions::new(8, 8).expect("dimensions");
    let pixels = (0_u16..64)
        .map(|index| {
            let x = f32::from(index % 8);
            let y = f32::from(index / 8);
            RgbaF32Pixel::new(
                0.08 + x * 0.09,
                0.12 + y * 0.08,
                0.16 + (x + y) * 0.045,
                0.2 + f32::from(index % 5) * 0.17,
            )
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::SrgbD65),
        pixels,
    )
    .expect("valid image")
}

fn image_with_encoding(encoding: RgbaF32ColorEncoding) -> RgbaF32Image {
    let source = image();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(source.descriptor().dimensions(), encoding),
        source.pixels().to_vec(),
    )
    .expect("valid image")
}

fn shadhi_operation(id: u128, algorithm: u32, opacity: f64) -> Operation {
    complete_shadhi_operation(
        id,
        opacity,
        &[
            ("radius", 2.0),
            ("shadows", 35.0),
            ("highlights", -30.0),
            ("shadhi_algo", f64::from(algorithm)),
        ],
    )
}

fn complete_shadhi_operation(id: u128, opacity: f64, overrides: &[(&str, f64)]) -> Operation {
    let mut parameters = [
        ("order", 0.0),
        ("radius", 100.0),
        ("shadows", 50.0),
        ("whitepoint", 0.0),
        ("highlights", -50.0),
        ("reserved2", 0.0),
        ("compress", 50.0),
        ("shadows_ccorrect", 100.0),
        ("highlights_ccorrect", 50.0),
        ("flags", 127.0),
        ("low_approximation", 0.000_001),
        ("shadhi_algo", 1.0),
    ];
    for (name, value) in overrides {
        parameters
            .iter_mut()
            .find(|(candidate, _)| candidate == name)
            .expect("known Shadhi parameter")
            .1 = *value;
    }
    operation(id, "rusttable.shadhi", opacity, &parameters)
}

fn shadhi_snapshot(
    algorithm: u32,
    opacity: f64,
    mode: CpuPixelpipeOutputMode,
) -> CpuPixelpipeSnapshot {
    CpuPixelpipeSnapshot::new(
        image(),
        graph(vec![shadhi_operation(10, algorithm, opacity)]),
        mode,
    )
}

fn assert_hybrid_close(
    actual: &rusttable_pixelpipe::RgbaF32Image,
    expected: &rusttable_pixelpipe::RgbaF32Image,
) {
    assert_eq!(actual.descriptor(), expected.descriptor());
    for (actual, expected) in actual.pixels().iter().zip(expected.pixels()) {
        assert!((actual.red() - expected.red()).abs() <= 0.01);
        assert!((actual.green() - expected.green()).abs() <= 0.01);
        assert!((actual.blue() - expected.blue()).abs() <= 0.01);
        assert_eq!(actual.alpha().to_bits(), expected.alpha().to_bits());
    }
}

#[test]
fn pre_cancelled_service_never_executes_or_publishes_a_fallback() {
    let snapshot = shadhi_snapshot(1, 1.0, CpuPixelpipeOutputMode::Preview);
    let scope = CancellationScope::root(PipelineGeneration::new(19).expect("nonzero generation"));
    scope.cancel(CancellationReason::UserRequested);
    let service = PixelpipeExecutionService::cpu_only();

    let full = service
        .execute_with_cancellation(&snapshot, &scope)
        .expect_err("pre-cancelled full-frame request");
    let tiled = service
        .execute_tiled_with_cancellation(
            &snapshot,
            CpuTilePlan::new(2, 2).expect("tile plan"),
            &scope,
        )
        .expect_err("pre-cancelled tiled request");

    for error in [full, tiled] {
        let CpuPixelpipeError::Cancelled(cancelled) = error else {
            panic!("pre-cancellation must be terminal: {error:?}");
        };
        assert_eq!(cancelled.reason(), CancellationReason::UserRequested);
        assert_eq!(cancelled.stage(), Some(CancellationStage::Preparation));
    }
}

#[test]
fn shadhi_snapshot_identity_is_deterministic_and_covers_the_v5_payload() {
    let snapshot = |overrides: &[(&str, f64)]| {
        CpuPixelpipeSnapshot::new(
            image(),
            graph(vec![complete_shadhi_operation(10, 1.0, overrides)]),
            CpuPixelpipeOutputMode::Preview,
        )
    };
    let base = snapshot(&[]);
    assert_eq!(base.identity(), snapshot(&[]).identity());

    for (parameter, value) in [
        ("order", 1.0),
        ("radius", 101.0),
        ("shadows", 49.0),
        ("whitepoint", 1.0),
        ("highlights", -49.0),
        ("reserved2", 1.0),
        ("compress", 49.0),
        ("shadows_ccorrect", 99.0),
        ("highlights_ccorrect", 49.0),
        ("flags", 126.0),
        ("low_approximation", 0.000_002),
        ("shadhi_algo", 0.0),
    ] {
        assert_ne!(
            base.identity(),
            snapshot(&[(parameter, value)]).identity(),
            "snapshot identity omitted Shadhi parameter {parameter}"
        );
    }
}

#[tokio::test]
async fn singleton_bilateral_uses_hybrid_gpu_for_preview_export_and_partial_opacity() {
    for (mode, opacity) in [
        (CpuPixelpipeOutputMode::Preview, 1.0),
        (CpuPixelpipeOutputMode::FullExport, 0.5),
    ] {
        let Ok(runtime) = GpuRuntime::initialize(GpuRuntimeConfig::default()).await else {
            return;
        };
        if runtime.is_cpu_only() {
            return;
        }
        let snapshot = shadhi_snapshot(1, opacity, mode);
        let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
        let selected = PixelpipeExecutionService::with_gpu(runtime)
            .execute(&snapshot)
            .expect("hybrid result");

        assert_eq!(
            selected.receipt().backend(),
            PixelpipeBackend::WgpuBilateralHybrid,
            "unexpected receipt: {:?}",
            selected.receipt()
        );
        assert_eq!(selected.receipt().dispatches(), 6);
        assert_hybrid_close(selected.image(), canonical.image());
    }
}

#[tokio::test]
async fn gaussian_mixed_and_masked_shadhi_graphs_remain_canonical_cpu() {
    let Ok(runtime) = GpuRuntime::initialize(GpuRuntimeConfig::default()).await else {
        return;
    };
    if runtime.is_cpu_only() {
        return;
    }
    let service = PixelpipeExecutionService::with_gpu(runtime);

    let gaussian = shadhi_snapshot(0, 1.0, CpuPixelpipeOutputMode::Preview);
    let mixed = CpuPixelpipeSnapshot::new(
        image(),
        graph(vec![
            operation(9, "rusttable.linear_offset", 1.0, &[("value", 0.01)]),
            shadhi_operation(10, 1, 1.0),
        ]),
        CpuPixelpipeOutputMode::Preview,
    );
    let repeated = CpuPixelpipeSnapshot::new(
        image(),
        graph(vec![
            shadhi_operation(10, 1, 1.0),
            shadhi_operation(11, 1, 1.0),
        ]),
        CpuPixelpipeOutputMode::Preview,
    );
    let mask_identity = MaskIdentity::new(2, 3, 7, 1);
    let mask_node = MaskNode::new(
        mask_identity,
        "bilateral-mask",
        MaskSource::Raster,
        MaskGeometry::new(GeometryAncestry::identity(), MaskRoi::full(8, 8), true),
        Some(MaskRaster::new(8, 8, vec![0.5; 64]).expect("mask")),
        [],
    )
    .expect("mask node");
    let mask_graph = MaskGraphBuilder::new()
        .add_mask(mask_node)
        .add_edge(mask_identity, 10, 1)
        .build()
        .expect("mask graph");
    let masked =
        shadhi_snapshot(1, 1.0, CpuPixelpipeOutputMode::Preview).with_mask_graph(mask_graph);

    for snapshot in [&gaussian, &mixed, &repeated, &masked] {
        let canonical = CpuPixelpipeExecutor
            .execute(snapshot)
            .expect("canonical CPU");
        let selected = service.execute(snapshot).expect("selected result");
        assert_eq!(selected.receipt().backend(), PixelpipeBackend::CpuCanonical);
        assert_eq!(selected.image(), canonical.image());

        let tiled = service
            .execute_tiled(snapshot, CpuTilePlan::new(2, 2).expect("tile plan"))
            .expect("full-frame CPU result");
        assert_eq!(tiled.receipt().backend(), PixelpipeBackend::CpuCanonical);
        assert!(tiled.receipt().tiling().is_none());
        assert_eq!(tiled.image(), canonical.image());
    }
}

#[tokio::test]
async fn unavailable_backend_falls_back_atomically_with_diagnostic() {
    let mut config = GpuRuntimeConfig::default();
    config.policy.backends.clear();
    let runtime = GpuRuntime::initialize(config)
        .await
        .expect("CPU-only runtime");
    let snapshot = shadhi_snapshot(1, 1.0, CpuPixelpipeOutputMode::Preview);
    let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
    let service = PixelpipeExecutionService::with_gpu(runtime);
    let selected = service.execute(&snapshot).expect("fallback result");

    assert_eq!(selected.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert_eq!(
        selected.receipt().gpu_fallback(),
        Some(&PixelpipeGpuFallback::Bilateral(
            BilateralGridError::CpuOnly
        ))
    );
    assert_eq!(selected.image(), canonical.image());

    let tiled = service
        .execute_tiled(&snapshot, CpuTilePlan::new(2, 2).expect("tile plan"))
        .expect("full-frame CPU fallback");
    assert_eq!(tiled.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert_eq!(
        tiled.receipt().gpu_fallback(),
        Some(&PixelpipeGpuFallback::Bilateral(
            BilateralGridError::CpuOnly
        ))
    );
    assert!(tiled.receipt().tiling().is_none());
    assert_eq!(tiled.image(), canonical.image());
}

#[tokio::test]
async fn zero_opacity_bilateral_routes_directly_to_canonical_cpu() {
    let mut config = GpuRuntimeConfig::default();
    config.policy.backends.clear();
    let runtime = GpuRuntime::initialize(config)
        .await
        .expect("CPU-only runtime");
    let snapshot = shadhi_snapshot(1, 0.0, CpuPixelpipeOutputMode::Preview);
    let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
    let service = PixelpipeExecutionService::with_gpu(runtime);
    let selected = service.execute(&snapshot).expect("canonical result");

    assert_eq!(selected.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert!(selected.receipt().gpu_fallback().is_none());
    assert_eq!(selected.receipt().dispatches(), 0);
    assert_eq!(selected.receipt().basicadj_plan_identity(), [0; 32]);
    assert_eq!(selected.image(), canonical.image());

    let tiled = service
        .execute_tiled(&snapshot, CpuTilePlan::new(2, 2).expect("tile plan"))
        .expect("full-frame canonical result");
    assert_eq!(tiled.receipt().backend(), PixelpipeBackend::CpuCanonical);
    assert!(tiled.receipt().tiling().is_none());
    assert_eq!(tiled.image(), canonical.image());
}

#[tokio::test]
async fn hybrid_rejects_unsupported_input_encoding_before_conversion() {
    let Ok(runtime) = GpuRuntime::initialize(GpuRuntimeConfig::default()).await else {
        return;
    };
    if runtime.is_cpu_only() {
        return;
    }
    let snapshot = CpuPixelpipeSnapshot::new(
        image_with_encoding(RgbaF32ColorEncoding::Rec2020D65),
        graph(vec![shadhi_operation(10, 1, 1.0)]),
        CpuPixelpipeOutputMode::Preview,
    );
    let error = PixelpipeExecutionService::with_gpu(runtime)
        .execute(&snapshot)
        .expect_err("unsupported encoding must not cross the GPU bridge");

    assert_eq!(
        error,
        CpuPixelpipeError::UnsupportedInputEncoding {
            actual: RgbaF32ColorEncoding::Rec2020D65,
        }
    );
}

#[tokio::test]
async fn tiled_bilateral_request_executes_one_full_frame_grid_without_seams() {
    let Ok(runtime) = GpuRuntime::initialize(GpuRuntimeConfig::default()).await else {
        return;
    };
    if runtime.is_cpu_only() {
        return;
    }
    let snapshot = shadhi_snapshot(1, 1.0, CpuPixelpipeOutputMode::Preview);
    let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
    let selected = PixelpipeExecutionService::with_gpu(runtime)
        .execute_tiled(&snapshot, CpuTilePlan::new(3, 2).expect("tile plan"))
        .expect("full-frame hybrid result");

    assert_eq!(
        selected.receipt().backend(),
        PixelpipeBackend::WgpuBilateralHybrid,
        "unexpected receipt: {:?}",
        selected.receipt()
    );
    assert_eq!(selected.receipt().dispatches(), 6);
    let tiling = selected.receipt().tiling().expect("full-frame receipt");
    assert_eq!(tiling.tile_count(), 1);
    assert_eq!(tiling.attempts(), 1);
    assert_hybrid_close(selected.image(), canonical.image());
}
