use rusttable_color::{
    AdaptationMethod, AlphaTransform, BlackPointCompensation, BuiltinColorTransformPlanner,
    ColorEncoding, ColorRole, ColorTransformPlanner, ColorTransformRequest, ExtendedRange, Pcs,
    Precision, Primaries, ProfileClass, ProfileId, ProfileModel, ProfileParserVersion,
    RenderingIntent, TransferFunction, TransformPlan,
};
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{
    BasicPointColorSpace, BasicPointError, GpuInitError, GpuRuntime, GpuRuntimeConfig,
};
use rusttable_image::{Orientation, SourceColor, SourceColorEvidence};
use rusttable_masks::{
    GeometryAncestry, MaskGeometry, MaskGraphBuilder, MaskIdentity, MaskNode, MaskRaster, MaskRoi,
    MaskSource,
};
use rusttable_pixelpipe::{
    CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan,
    PixelpipeBackend, PixelpipeExecutionService, PixelpipeGpuFallback, RgbaF32ColorEncoding,
    RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel, RgbaF32SourceRepresentation,
};
use rusttable_processing::{
    ColorContrastConfig, ColorContrastPixel, ColorContrastPlan, CompiledOperationGraph,
    RasterDimensions,
};

fn operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
    operation_with_opacity(id, key, OperationOpacity::ONE, parameters)
}

fn operation_with_opacity(
    id: u128,
    key: &str,
    opacity: OperationOpacity,
    parameters: &[(&str, f64)],
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero ID"),
        OperationKey::new(key).expect("valid key"),
        true,
        opacity,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("valid parameter"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite parameter")),
            )
        }),
    )
    .expect("valid operation")
}

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
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let service = PixelpipeExecutionService::with_gpu(runtime);
    for channel in 0..4 {
        let snapshot = grain_snapshot(f64::from(channel));
        let canonical = CpuPixelpipeExecutor.execute(&snapshot).expect("CPU result");
        let selected = service.execute(&snapshot).expect("GPU grain result");

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
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
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
    grain_snapshot_for_image(image, channel)
}

fn grain_snapshot_for_image(image: RgbaF32Image, channel: f64) -> CpuPixelpipeSnapshot {
    grain_snapshot_for_image_with_mode(image, channel, CpuPixelpipeOutputMode::Preview)
}

fn grain_snapshot_for_image_with_mode(
    image: RgbaF32Image,
    channel: f64,
    output_mode: CpuPixelpipeOutputMode,
) -> CpuPixelpipeSnapshot {
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
        output_mode,
    )
}

fn with_uniform_mask(
    snapshot: CpuPixelpipeSnapshot,
    consumer_operation: u128,
) -> CpuPixelpipeSnapshot {
    let dimensions = snapshot.input().descriptor().dimensions();
    let identity = MaskIdentity::new(7, 11, 13, 1);
    let pixel_count = usize::try_from(dimensions.pixel_count()).expect("mask pixel count");
    let mask = MaskNode::new(
        identity,
        "GPU qualification mask",
        MaskSource::Raster,
        MaskGeometry::new(
            GeometryAncestry::identity(),
            MaskRoi::full(dimensions.width(), dimensions.height()),
            true,
        ),
        Some(
            MaskRaster::new(
                dimensions.width(),
                dimensions.height(),
                vec![0.25; pixel_count],
            )
            .expect("uniform mask"),
        ),
        [],
    )
    .expect("mask node");
    let graph = MaskGraphBuilder::new()
        .add_mask(mask)
        .add_edge(identity, consumer_operation, 1)
        .build()
        .expect("mask graph");
    snapshot.with_mask_graph(graph)
}

#[tokio::test]
async fn masked_basic_and_grain_graphs_remain_canonical_cpu() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let service = PixelpipeExecutionService::with_gpu(runtime);
    let snapshots = [
        (
            "basic",
            with_uniform_mask(snapshot(), 7),
            PixelpipeBackend::CpuTiledFallback,
        ),
        (
            "grain",
            with_uniform_mask(grain_snapshot(3.0), 0x1234),
            PixelpipeBackend::CpuCanonical,
        ),
    ];

    for (label, snapshot, tiled_backend) in &snapshots {
        let canonical = CpuPixelpipeExecutor
            .execute(snapshot)
            .unwrap_or_else(|error| panic!("{label}: canonical CPU failed: {error}"));
        let selected = service
            .execute(snapshot)
            .unwrap_or_else(|error| panic!("{label}: selected execution failed: {error}"));
        assert_eq!(
            selected.receipt().backend(),
            PixelpipeBackend::CpuCanonical,
            "{label}: masked graph must not be GPU-qualified"
        );
        assert_eq!(selected.image(), canonical.image(), "{label}: full frame");

        let tiled = service
            .execute_tiled(snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
            .unwrap_or_else(|error| panic!("{label}: tiled execution failed: {error}"));
        assert_eq!(
            tiled.receipt().backend(),
            *tiled_backend,
            "{label}: masked tiled graph must not be GPU-qualified"
        );
        assert_eq!(tiled.image(), canonical.image(), "{label}: tiled");
    }
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
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
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

fn velvia_snapshot(
    opacity: OperationOpacity,
    include_surrounding_operations: bool,
) -> CpuPixelpipeSnapshot {
    velvia_snapshot_with_strength(opacity, include_surrounding_operations, 85.0)
}

fn velvia_snapshot_with_strength(
    opacity: OperationOpacity,
    include_surrounding_operations: bool,
    strength: f64,
) -> CpuPixelpipeSnapshot {
    velvia_snapshot_with_parameters(opacity, include_surrounding_operations, strength, 0.2)
}

fn velvia_snapshot_with_parameters(
    opacity: OperationOpacity,
    include_surrounding_operations: bool,
    strength: f64,
    bias: f64,
) -> CpuPixelpipeSnapshot {
    let dimensions = RasterDimensions::new(4, 2).expect("dimensions");
    let image = RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LinearSrgbD65),
        vec![
            RgbaF32Pixel::new(0.10, 0.20, 0.30, 0.0),
            RgbaF32Pixel::new(0.99, 0.90, 0.90, 0.37),
            RgbaF32Pixel::new(2.00, 2.00, 2.00, 1.0),
            RgbaF32Pixel::new(-0.25, 0.50, 1.25, f32::from_bits(1)),
            RgbaF32Pixel::new(0.30, 0.30, 0.30, 0.25),
            RgbaF32Pixel::new(0.75, 0.20, 0.45, 0.50),
            RgbaF32Pixel::new(0.01, 0.02, 0.01, 0.75),
            RgbaF32Pixel::new(1.20, 0.95, 0.90, f32::from_bits(0x3eaa_aaab)),
        ],
    )
    .expect("Velvia input");
    let mut operations = Vec::new();
    if include_surrounding_operations {
        operations.push(operation(
            0x3001,
            "rusttable.exposure",
            &[("stops", 0.25), ("black", 0.01)],
        ));
    }
    operations.push(operation_with_opacity(
        0x3002,
        "rusttable.velvia",
        opacity,
        &[("strength", strength), ("bias", bias)],
    ));
    if include_surrounding_operations {
        operations.push(operation(
            0x3003,
            "rusttable.linear_offset",
            &[("value", -0.025)],
        ));
    }
    let edit = Edit::from_parts(
        EditId::new(0x3000).expect("edit ID"),
        PhotoId::new(0x3010).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        operations,
    )
    .expect("Velvia edit");
    CpuPixelpipeSnapshot::new(
        image,
        CompiledOperationGraph::compile(&edit).expect("Velvia graph"),
        CpuPixelpipeOutputMode::FullExport,
    )
}

#[test]
fn velvia_snapshot_identity_includes_strength_and_bias_bits() {
    let baseline =
        velvia_snapshot_with_parameters(OperationOpacity::ONE, false, 25.0, 0.75).identity();
    let changed_strength =
        velvia_snapshot_with_parameters(OperationOpacity::ONE, false, 25.5, 0.75).identity();
    let changed_bias =
        velvia_snapshot_with_parameters(OperationOpacity::ONE, false, 25.0, 0.5).identity();

    assert_ne!(baseline, changed_strength);
    assert_ne!(baseline, changed_bias);
    assert_ne!(changed_strength, changed_bias);
}

#[tokio::test]
async fn wgpu_velvia_matches_cpu_corpus_full_frame_and_tiled() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let snapshot = velvia_snapshot(OperationOpacity::ONE, false);
    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("CPU Velvia result");
    let service = PixelpipeExecutionService::with_gpu(runtime);
    let full = service
        .execute(&snapshot)
        .expect("full-frame GPU Velvia result");
    let tiled = service
        .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
        .expect("tiled GPU Velvia result");

    assert_eq!(full.receipt().backend(), PixelpipeBackend::WgpuBasic);
    assert_eq!(full.receipt().dispatches(), 1);
    assert_eq!(tiled.receipt().backend(), PixelpipeBackend::WgpuTiled);
    assert_eq!(tiled.receipt().dispatches(), 4);
    assert_gpu_image_matches_cpu(
        "Velvia full frame",
        full.image(),
        canonical.image(),
        snapshot.input().descriptor(),
        0.00001,
    );
    assert_gpu_image_matches_cpu(
        "Velvia tiled",
        tiled.image(),
        canonical.image(),
        snapshot.input().descriptor(),
        0.00001,
    );
    assert_eq!(
        canonical.image().pixels()[2].red().to_bits(),
        1.0_f32.to_bits(),
        "positive-strength Velvia clips output RGB"
    );
}

#[tokio::test]
async fn zero_strength_velvia_is_cpu_and_wgpu_bit_exact_identity() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let snapshot = velvia_snapshot_with_strength(OperationOpacity::ONE, false, 0.0);
    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("zero-strength CPU Velvia result");
    let selected = PixelpipeExecutionService::with_gpu(runtime)
        .execute(&snapshot)
        .expect("zero-strength GPU Velvia result");

    assert_eq!(selected.receipt().backend(), PixelpipeBackend::WgpuBasic);
    assert_eq!(selected.receipt().dispatches(), 1);
    for (index, ((source, cpu), gpu)) in snapshot
        .input()
        .pixels()
        .iter()
        .zip(canonical.image().pixels())
        .zip(selected.image().pixels())
        .enumerate()
    {
        let bits = |pixel: &RgbaF32Pixel| {
            [
                pixel.red().to_bits(),
                pixel.green().to_bits(),
                pixel.blue().to_bits(),
                pixel.alpha().to_bits(),
            ]
        };
        assert_eq!(bits(cpu), bits(source), "CPU pixel {index}");
        assert_eq!(bits(gpu), bits(source), "WGPU pixel {index}");
    }
}

#[tokio::test]
async fn wgpu_velvia_preserves_authored_multi_operation_order() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let snapshot = velvia_snapshot(OperationOpacity::ONE, true);
    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("ordered CPU result");
    let selected = PixelpipeExecutionService::with_gpu(runtime)
        .execute(&snapshot)
        .expect("ordered GPU result");

    assert_eq!(selected.receipt().backend(), PixelpipeBackend::WgpuBasic);
    assert_eq!(selected.receipt().dispatches(), 3);
    assert_gpu_image_matches_cpu(
        "Velvia point-chain order",
        selected.image(),
        canonical.image(),
        snapshot.input().descriptor(),
        0.00001,
    );
}

#[tokio::test]
async fn masked_and_partial_opacity_velvia_remain_truthful_cpu_fallbacks() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let service = PixelpipeExecutionService::with_gpu(runtime);
    let cases = [
        (
            "masked",
            with_uniform_mask(velvia_snapshot(OperationOpacity::ONE, false), 0x3002),
        ),
        (
            "partial opacity",
            velvia_snapshot(OperationOpacity::new(0.5).expect("partial opacity"), false),
        ),
    ];

    for (label, snapshot) in cases {
        let canonical = CpuPixelpipeExecutor
            .execute(&snapshot)
            .unwrap_or_else(|error| panic!("{label} CPU result: {error}"));
        let full = service
            .execute(&snapshot)
            .unwrap_or_else(|error| panic!("{label} selected result: {error}"));
        let tiled = service
            .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
            .unwrap_or_else(|error| panic!("{label} tiled result: {error}"));

        assert_eq!(
            full.receipt().backend(),
            PixelpipeBackend::CpuCanonical,
            "{label} full-frame qualification"
        );
        assert_eq!(
            tiled.receipt().backend(),
            PixelpipeBackend::CpuTiledFallback,
            "{label} tiled qualification"
        );
        assert_eq!(full.image(), canonical.image(), "{label} full frame");
        assert_eq!(tiled.image(), canonical.image(), "{label} tiled");
    }
}

fn colorcontrast_operation(
    id: u128,
    opacity: OperationOpacity,
    parameters: [f64; 4],
    unbound: i64,
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("nonzero ID"),
        OperationKey::new("rusttable.colorcontrast").expect("valid key"),
        true,
        opacity,
        [
            (
                ParameterName::new("a_steepness").expect("valid parameter"),
                ParameterValue::Scalar(FiniteF64::new(parameters[0]).expect("finite parameter")),
            ),
            (
                ParameterName::new("a_offset").expect("valid parameter"),
                ParameterValue::Scalar(FiniteF64::new(parameters[1]).expect("finite parameter")),
            ),
            (
                ParameterName::new("b_steepness").expect("valid parameter"),
                ParameterValue::Scalar(FiniteF64::new(parameters[2]).expect("finite parameter")),
            ),
            (
                ParameterName::new("b_offset").expect("valid parameter"),
                ParameterValue::Scalar(FiniteF64::new(parameters[3]).expect("finite parameter")),
            ),
            (
                ParameterName::new("unbound").expect("valid parameter"),
                ParameterValue::Integer(unbound),
            ),
        ],
    )
    .expect("valid Color Contrast operation")
}

fn colorcontrast_snapshot(
    encoding: RgbaF32ColorEncoding,
    opacity: OperationOpacity,
    include_surrounding_operation: bool,
    parameters: [f64; 4],
    unbound: i64,
) -> CpuPixelpipeSnapshot {
    let dimensions = RasterDimensions::new(4, 2).expect("dimensions");
    let pixels = if encoding == RgbaF32ColorEncoding::LabD50 {
        vec![
            RgbaF32Pixel::new(50.0, 10.0, -20.0, 0.0),
            RgbaF32Pixel::new(80.0, 100.0, -100.0, 0.37),
            RgbaF32Pixel::new(0.0, -128.0, 128.0, 1.0),
            RgbaF32Pixel::new(100.0, 0.0, 0.0, f32::from_bits(1)),
            RgbaF32Pixel::new(25.0, -30.0, 40.0, 0.25),
            RgbaF32Pixel::new(65.0, 75.0, -85.0, 0.50),
            RgbaF32Pixel::new(42.0, -4.0, 7.0, 0.75),
            RgbaF32Pixel::new(91.0, 3.0, -2.0, f32::from_bits(0x3eaa_aaab)),
        ]
    } else {
        vec![
            RgbaF32Pixel::new(0.10, 0.20, 0.30, 0.0),
            RgbaF32Pixel::new(0.99, 0.90, 0.90, 0.37),
            RgbaF32Pixel::new(2.00, 2.00, 2.00, 1.0),
            RgbaF32Pixel::new(-0.25, 0.50, 1.25, f32::from_bits(1)),
            RgbaF32Pixel::new(0.30, 0.30, 0.30, 0.25),
            RgbaF32Pixel::new(0.75, 0.20, 0.45, 0.50),
            RgbaF32Pixel::new(0.01, 0.02, 0.01, 0.75),
            RgbaF32Pixel::new(1.20, 0.95, 0.90, f32::from_bits(0x3eaa_aaab)),
        ]
    };
    let image = RgbaF32Image::new(RgbaF32Descriptor::new(dimensions, encoding), pixels)
        .expect("Color Contrast input");
    let mut operations = Vec::new();
    if include_surrounding_operation {
        operations.push(operation(
            0x4001,
            "rusttable.exposure",
            &[("stops", 0.25), ("black", 0.01)],
        ));
    }
    operations.push(colorcontrast_operation(
        0x4002, opacity, parameters, unbound,
    ));
    let edit = Edit::from_parts(
        EditId::new(0x4000).expect("edit ID"),
        PhotoId::new(0x4010).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        operations,
    )
    .expect("Color Contrast edit");
    CpuPixelpipeSnapshot::new(
        image,
        CompiledOperationGraph::compile(&edit).expect("Color Contrast graph"),
        CpuPixelpipeOutputMode::FullExport,
    )
}

#[test]
fn colorcontrast_snapshot_identity_includes_every_native_parameter() {
    let baseline = colorcontrast_snapshot(
        RgbaF32ColorEncoding::LabD50,
        OperationOpacity::ONE,
        false,
        [1.0, 0.0, 1.0, 0.0],
        1,
    )
    .identity();
    for (changed, unbound) in [
        ([1.25, 0.0, 1.0, 0.0], 1),
        ([1.0, 0.25, 1.0, 0.0], 1),
        ([1.0, 0.0, 1.25, 0.0], 1),
        ([1.0, 0.0, 1.0, 0.25], 1),
        ([1.0, 0.0, 1.0, 0.0], -1),
    ] {
        assert_ne!(
            baseline,
            colorcontrast_snapshot(
                RgbaF32ColorEncoding::LabD50,
                OperationOpacity::ONE,
                false,
                changed,
                unbound,
            )
            .identity()
        );
    }
}

fn native_colorcontrast_channel(value: f32, steepness: f32, offset: f32, unbound: bool) -> f32 {
    let scaled = value * steepness + offset;
    if unbound {
        scaled
    } else if scaled > -128.0 {
        if scaled < 128.0 { scaled } else { 128.0 }
    } else {
        -128.0
    }
}

fn working_color_transform(source: ColorEncoding, target: ColorEncoding) -> TransformPlan {
    let request = ColorTransformRequest::new(
        source,
        target,
        ColorRole::Working,
        RenderingIntent::Relative,
        BlackPointCompensation::Disabled,
        AdaptationMethod::Bradford,
        Precision::F32,
        AlphaTransform::Preserve,
        ExtendedRange::Extended,
        1,
    )
    .expect("working color-transform request");
    BuiltinColorTransformPlanner
        .plan(&request)
        .expect("built-in working color transform")
}

#[test]
fn cpu_colorcontrast_executes_explicit_lab_without_an_rgb_round_trip() {
    for unbound in [false, true] {
        let snapshot = colorcontrast_snapshot(
            RgbaF32ColorEncoding::LabD50,
            OperationOpacity::ONE,
            false,
            [2.0, 3.0, 1.5, -4.0],
            i64::from(u8::from(unbound)),
        );
        let canonical = CpuPixelpipeExecutor
            .execute(&snapshot)
            .expect("CPU Color Contrast result");
        let tiled = CpuPixelpipeExecutor
            .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
            .expect("tiled CPU Color Contrast result");

        for (index, (source, actual)) in snapshot
            .input()
            .pixels()
            .iter()
            .zip(canonical.image().pixels())
            .enumerate()
        {
            assert_eq!(
                actual.red().to_bits(),
                source.red().to_bits(),
                "pixel {index} L"
            );
            assert_eq!(
                actual.green().to_bits(),
                native_colorcontrast_channel(source.green(), 2.0, 3.0, unbound).to_bits(),
                "pixel {index} a"
            );
            assert_eq!(
                actual.blue().to_bits(),
                native_colorcontrast_channel(source.blue(), 1.5, -4.0, unbound).to_bits(),
                "pixel {index} b"
            );
            assert_eq!(
                actual.alpha().to_bits(),
                source.alpha().to_bits(),
                "pixel {index} separately carried alpha"
            );
        }
        assert_eq!(tiled.image(), canonical.image());
    }
}

#[test]
fn cpu_colorcontrast_keeps_multiple_instances_in_native_lab_order() {
    let input = colorcontrast_snapshot(
        RgbaF32ColorEncoding::LabD50,
        OperationOpacity::ONE,
        false,
        [1.0, 0.0, 1.0, 0.0],
        1,
    )
    .input()
    .clone();
    let edit = Edit::from_parts(
        EditId::new(0x4100).expect("edit ID"),
        PhotoId::new(0x4110).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        [
            colorcontrast_operation(0x4101, OperationOpacity::ONE, [2.0, 3.0, 1.5, -4.0], 1),
            colorcontrast_operation(0x4102, OperationOpacity::ONE, [0.5, -2.0, 2.0, 5.0], 0),
        ],
    )
    .expect("two-instance Color Contrast edit");
    let snapshot = CpuPixelpipeSnapshot::new(
        input,
        CompiledOperationGraph::compile(&edit).expect("two-instance graph"),
        CpuPixelpipeOutputMode::FullExport,
    );
    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("CPU two-instance Color Contrast result");
    let tiled = CpuPixelpipeExecutor
        .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
        .expect("tiled CPU two-instance Color Contrast result");

    for (index, (source, actual)) in snapshot
        .input()
        .pixels()
        .iter()
        .zip(canonical.image().pixels())
        .enumerate()
    {
        let first_a = native_colorcontrast_channel(source.green(), 2.0, 3.0, true);
        let first_b = native_colorcontrast_channel(source.blue(), 1.5, -4.0, true);
        let expected_a = native_colorcontrast_channel(first_a, 0.5, -2.0, false);
        let expected_b = native_colorcontrast_channel(first_b, 2.0, 5.0, false);
        assert_eq!(
            actual.red().to_bits(),
            source.red().to_bits(),
            "pixel {index} L"
        );
        assert_eq!(
            actual.green().to_bits(),
            expected_a.to_bits(),
            "pixel {index} a"
        );
        assert_eq!(
            actual.blue().to_bits(),
            expected_b.to_bits(),
            "pixel {index} b"
        );
        assert_eq!(
            actual.alpha().to_bits(),
            source.alpha().to_bits(),
            "pixel {index} separately carried alpha"
        );
    }
    assert_eq!(tiled.image(), canonical.image());
}

#[test]
fn cpu_linear_srgb_colorcontrast_instances_share_one_exact_lab_boundary() {
    let input = colorcontrast_snapshot(
        RgbaF32ColorEncoding::LinearSrgbD65,
        OperationOpacity::ONE,
        false,
        [1.0, 0.0, 1.0, 0.0],
        1,
    )
    .input()
    .clone();
    let edit = Edit::from_parts(
        EditId::new(0x4200).expect("edit ID"),
        PhotoId::new(0x4210).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        [
            colorcontrast_operation(0x4201, OperationOpacity::ONE, [1.5, 2.0, 0.75, -4.0], 1),
            colorcontrast_operation(
                0x4202,
                OperationOpacity::new(0.5).expect("half opacity"),
                [0.5, -2.0, 1.25, 3.0],
                0,
            ),
        ],
    )
    .expect("two-instance RGB Color Contrast edit");
    let snapshot = with_uniform_mask(
        CpuPixelpipeSnapshot::new(
            input,
            CompiledOperationGraph::compile(&edit).expect("two-instance RGB graph"),
            CpuPixelpipeOutputMode::FullExport,
        ),
        0x4202,
    );

    let to_lab = working_color_transform(ColorEncoding::LinearSrgbD65, ColorEncoding::LabD50);
    let from_lab = working_color_transform(ColorEncoding::LabD50, ColorEncoding::LinearSrgbD65);
    let mut reference = snapshot
        .input()
        .pixels()
        .iter()
        .enumerate()
        .map(|(pixel_index, source)| {
            let lab = to_lab
                .apply_rgb([source.red(), source.green(), source.blue()], || false)
                .unwrap_or_else(|error| panic!("reference ingress pixel {pixel_index}: {error}"));
            ColorContrastPixel::new(lab[0], lab[1], lab[2], source.alpha())
        })
        .collect::<Vec<_>>();
    reference = ColorContrastPlan::new(
        ColorContrastConfig::new(1.5, 2.0, 0.75, -4.0, 1).expect("first config"),
    )
    .execute_lab(&reference);
    let mask = vec![0.25; reference.len()];
    reference = ColorContrastPlan::new(
        ColorContrastConfig::new(0.5, -2.0, 1.25, 3.0, 0).expect("second config"),
    )
    .execute_lab_normal_blend(&reference, Some(&mask), 0.5);
    let reference = reference
        .iter()
        .zip(snapshot.input().pixels())
        .enumerate()
        .map(|(pixel_index, (lab, source))| {
            let lab = lab.channels();
            let rgb = from_lab
                .apply_rgb([lab[0], lab[1], lab[2]], || false)
                .unwrap_or_else(|error| panic!("reference egress pixel {pixel_index}: {error}"));
            RgbaF32Pixel::new(rgb[0], rgb[1], rgb[2], source.alpha())
        })
        .collect::<Vec<_>>();
    let reference = RgbaF32Image::new(
        RgbaF32Descriptor::new(
            snapshot.input().descriptor().dimensions(),
            RgbaF32ColorEncoding::LinearSrgbD65,
        ),
        reference,
    )
    .expect("single-boundary reference image");

    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("CPU two-instance RGB Color Contrast result");
    let tiled = CpuPixelpipeExecutor
        .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
        .expect("tiled two-instance RGB Color Contrast result");

    assert_eq!(canonical.image(), &reference);
    assert_eq!(tiled.image(), &reference);
}

#[tokio::test]
async fn wgpu_colorcontrast_matches_native_lab_formula_full_frame_and_tiled() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let service = PixelpipeExecutionService::with_gpu(runtime);
    for unbound in [false, true] {
        let snapshot = colorcontrast_snapshot(
            RgbaF32ColorEncoding::LabD50,
            OperationOpacity::ONE,
            false,
            [2.0, 3.0, 1.5, -4.0],
            i64::from(u8::from(unbound)),
        );
        let canonical = CpuPixelpipeExecutor
            .execute(&snapshot)
            .expect("CPU Color Contrast result");
        let full = service
            .execute(&snapshot)
            .expect("full-frame GPU Color Contrast result");
        let tiled = service
            .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
            .expect("tiled GPU Color Contrast result");

        assert_eq!(full.receipt().backend(), PixelpipeBackend::WgpuBasic);
        assert_eq!(full.receipt().dispatches(), 1);
        assert_eq!(tiled.receipt().backend(), PixelpipeBackend::WgpuTiled);
        assert_eq!(tiled.receipt().dispatches(), 4);
        assert_gpu_image_matches_cpu(
            "Color Contrast full frame",
            full.image(),
            canonical.image(),
            snapshot.input().descriptor(),
            0.000_001,
        );
        assert_gpu_image_matches_cpu(
            "Color Contrast tiled",
            tiled.image(),
            canonical.image(),
            snapshot.input().descriptor(),
            0.000_001,
        );
        for (index, ((source, full), tiled)) in snapshot
            .input()
            .pixels()
            .iter()
            .zip(full.image().pixels())
            .zip(tiled.image().pixels())
            .enumerate()
        {
            let expected_a = native_colorcontrast_channel(source.green(), 2.0, 3.0, unbound);
            let expected_b = native_colorcontrast_channel(source.blue(), 1.5, -4.0, unbound);
            for (label, actual) in [("full", full), ("tiled", tiled)] {
                assert_eq!(
                    actual.red().to_bits(),
                    source.red().to_bits(),
                    "{label} pixel {index} L"
                );
                assert_eq!(
                    actual.green().to_bits(),
                    expected_a.to_bits(),
                    "{label} pixel {index} a"
                );
                assert_eq!(
                    actual.blue().to_bits(),
                    expected_b.to_bits(),
                    "{label} pixel {index} b"
                );
                assert_eq!(
                    actual.alpha().to_bits(),
                    source.alpha().to_bits(),
                    "{label} pixel {index} alpha"
                );
            }
        }
    }
}

#[tokio::test]
async fn colorcontrast_without_a_proven_lab_chain_reports_cpu_fallback() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let service = PixelpipeExecutionService::with_gpu(runtime);
    let fallback = PixelpipeGpuFallback::Basic(BasicPointError::ColorSpaceBoundaryUnavailable {
        required: BasicPointColorSpace::LabD50,
    });
    for (label, snapshot) in [
        (
            "RGB source",
            colorcontrast_snapshot(
                RgbaF32ColorEncoding::LinearSrgbD65,
                OperationOpacity::ONE,
                false,
                [2.0, 3.0, 1.5, -4.0],
                0,
            ),
        ),
        (
            "mixed Lab chain",
            colorcontrast_snapshot(
                RgbaF32ColorEncoding::LabD50,
                OperationOpacity::ONE,
                true,
                [2.0, 3.0, 1.5, -4.0],
                0,
            ),
        ),
    ] {
        let canonical = CpuPixelpipeExecutor
            .execute(&snapshot)
            .unwrap_or_else(|error| panic!("{label} CPU result: {error}"));
        let full = service
            .execute(&snapshot)
            .unwrap_or_else(|error| panic!("{label} selected result: {error}"));
        let tiled = service
            .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
            .unwrap_or_else(|error| panic!("{label} tiled result: {error}"));
        assert_eq!(full.receipt().backend(), PixelpipeBackend::CpuCanonical);
        assert_eq!(full.receipt().gpu_fallback(), Some(&fallback));
        assert_eq!(
            tiled.receipt().backend(),
            PixelpipeBackend::CpuTiledFallback
        );
        assert_eq!(tiled.receipt().gpu_fallback(), Some(&fallback));
        assert_eq!(full.image(), canonical.image());
        assert_eq!(tiled.image(), canonical.image());
    }
}

#[tokio::test]
async fn masked_and_partial_opacity_colorcontrast_remain_cpu_only() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let service = PixelpipeExecutionService::with_gpu(runtime);
    let cases = [
        (
            "masked",
            with_uniform_mask(
                colorcontrast_snapshot(
                    RgbaF32ColorEncoding::LabD50,
                    OperationOpacity::ONE,
                    false,
                    [2.0, 3.0, 1.5, -4.0],
                    0,
                ),
                0x4002,
            ),
        ),
        (
            "partial opacity",
            colorcontrast_snapshot(
                RgbaF32ColorEncoding::LabD50,
                OperationOpacity::new(0.5).expect("partial opacity"),
                false,
                [2.0, 3.0, 1.5, -4.0],
                0,
            ),
        ),
    ];
    for (label, snapshot) in cases {
        let canonical = CpuPixelpipeExecutor
            .execute(&snapshot)
            .unwrap_or_else(|error| panic!("{label} CPU result: {error}"));
        let full = service
            .execute(&snapshot)
            .unwrap_or_else(|error| panic!("{label} selected result: {error}"));
        let tiled = service
            .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
            .unwrap_or_else(|error| panic!("{label} tiled result: {error}"));
        assert_eq!(full.receipt().backend(), PixelpipeBackend::CpuCanonical);
        assert_eq!(
            tiled.receipt().backend(),
            PixelpipeBackend::CpuTiledFallback
        );
        assert_eq!(full.receipt().gpu_fallback(), None);
        assert_eq!(tiled.receipt().gpu_fallback(), None);
        assert_eq!(full.image(), canonical.image());
        assert_eq!(tiled.image(), canonical.image());
    }
}

#[tokio::test]
async fn wgpu_basicadj_matches_cpu_at_supported_source_boundaries_full_frame_and_tiled() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let service = PixelpipeExecutionService::with_gpu(runtime);

    for (label, input) in source_boundary_images() {
        let source_descriptor = input.descriptor();
        let snapshot = basicadj_snapshot(input);
        let canonical = CpuPixelpipeExecutor
            .execute(&snapshot)
            .expect("CPU basicadj result");
        let full = service
            .execute(&snapshot)
            .expect("full-frame GPU basicadj result");
        let tiled = service
            .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
            .expect("tiled GPU basicadj result");

        assert_eq!(
            full.receipt().backend(),
            PixelpipeBackend::WgpuBasic,
            "{label}: full-frame fallback {:?}",
            full.receipt().gpu_fallback()
        );
        assert_eq!(
            tiled.receipt().backend(),
            PixelpipeBackend::WgpuTiled,
            "{label}: tiled fallback {:?}",
            tiled.receipt().gpu_fallback()
        );
        assert_eq!(
            full.receipt().basicadj_plan_identity(),
            canonical.receipt().basicadj_plan_identity(),
            "{label}: full-frame BasicAdj plan"
        );
        assert_eq!(
            tiled.receipt().basicadj_plan_identity(),
            canonical.receipt().basicadj_plan_identity(),
            "{label}: tiled BasicAdj plan"
        );
        assert_eq!(
            full.receipt().basicadj_plan_identity(),
            tiled.receipt().basicadj_plan_identity(),
            "{label}: full and tiled receipts must reuse one qualified plan identity"
        );
        assert_eq!(full.receipt().snapshot_identity(), snapshot.identity());
        assert_eq!(tiled.receipt().snapshot_identity(), snapshot.identity());
        assert_eq!(full.receipt().dispatches(), 1, "{label}: full dispatches");
        assert_eq!(tiled.receipt().dispatches(), 4, "{label}: tiled dispatches");
        let tiling = tiled
            .receipt()
            .tiling()
            .unwrap_or_else(|| panic!("{label}: tiled BasicAdj receipt"));
        assert_eq!(tiling.tile_count(), 4, "{label}: tile count");
        assert_eq!(tiling.attempts(), 1, "{label}: attempts");
        assert_gpu_image_matches_cpu(
            label,
            full.image(),
            canonical.image(),
            source_descriptor,
            0.002,
        );
        assert_gpu_image_matches_cpu(
            label,
            tiled.image(),
            canonical.image(),
            source_descriptor,
            0.002,
        );
    }
}

#[tokio::test]
async fn wgpu_grain_matches_cpu_at_supported_source_boundaries_full_frame_and_tiled() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let service = PixelpipeExecutionService::with_gpu(runtime);

    for (label, input) in source_boundary_images() {
        let source_descriptor = input.descriptor();
        let snapshot =
            grain_snapshot_for_image_with_mode(input, 3.0, CpuPixelpipeOutputMode::FullExport);
        let canonical = CpuPixelpipeExecutor
            .execute(&snapshot)
            .expect("CPU grain result");
        let full = service
            .execute(&snapshot)
            .expect("full-frame GPU grain result");
        let tiled = service
            .execute_tiled(&snapshot, CpuTilePlan::new(2, 1).expect("tile plan"))
            .expect("tiled GPU grain result");

        assert_eq!(
            full.receipt().backend(),
            PixelpipeBackend::WgpuBasic,
            "{label}: full-frame fallback {:?}",
            full.receipt().gpu_fallback()
        );
        assert_eq!(
            tiled.receipt().backend(),
            PixelpipeBackend::WgpuTiled,
            "{label}: tiled fallback {:?}",
            tiled.receipt().gpu_fallback()
        );
        let tolerance = if source_descriptor.color_encoding() == RgbaF32ColorEncoding::LabD50 {
            0.05
        } else {
            0.004
        };
        assert_gpu_image_matches_cpu(
            label,
            full.image(),
            canonical.image(),
            source_descriptor,
            tolerance,
        );
        assert_gpu_image_matches_cpu(
            label,
            tiled.image(),
            canonical.image(),
            source_descriptor,
            tolerance,
        );
    }
}

fn basicadj_snapshot(image: RgbaF32Image) -> CpuPixelpipeSnapshot {
    let edit = Edit::from_parts(
        EditId::new(0x2000).expect("edit ID"),
        PhotoId::new(0x2001).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        vec![operation(
            0x2002,
            "rusttable.basicadj",
            &[
                ("black_point", 0.01),
                ("exposure", 0.35),
                ("contrast", 0.15),
                ("saturation", 0.1),
                ("vibrance", 0.05),
            ],
        )],
    )
    .expect("edit");
    CpuPixelpipeSnapshot::new(
        image,
        CompiledOperationGraph::compile(&edit).expect("graph"),
        CpuPixelpipeOutputMode::FullExport,
    )
}

fn source_boundary_images() -> Vec<(&'static str, RgbaF32Image)> {
    let dimensions = RasterDimensions::new(4, 2).expect("dimensions");
    let descriptor = |encoding| {
        RgbaF32Descriptor::with_source_representation(
            dimensions,
            encoding,
            RgbaF32SourceRepresentation::U16,
        )
        .with_source_orientation(Orientation::Rotate90)
    };
    let rgb_pixels = vec![
        RgbaF32Pixel::new(0.12, 0.25, 0.70, 0.15),
        RgbaF32Pixel::new(0.80, 0.15, 0.30, 0.35),
        RgbaF32Pixel::new(0.42, 0.55, 0.20, 0.55),
        RgbaF32Pixel::new(0.65, 0.45, 0.35, 0.75),
        RgbaF32Pixel::new(0.20, 0.70, 0.55, 0.25),
        RgbaF32Pixel::new(0.35, 0.30, 0.75, 0.45),
        RgbaF32Pixel::new(0.72, 0.62, 0.18, 0.65),
        RgbaF32Pixel::new(0.28, 0.38, 0.48, 0.85),
    ];
    let profile = ProfileId::from_content(
        b"pixelpipe GPU matrix source boundary",
        ProfileClass::Input,
        ProfileModel::Matrix,
        Pcs::XyzD50,
        ProfileParserVersion::new(1).expect("parser version"),
    )
    .expect("profile identity");
    let source_color = SourceColor::external(
        profile,
        Primaries::display_p3(),
        TransferFunction::Srgb,
        SourceColorEvidence::EmbeddedChromaticities,
    )
    .expect("matrix source color");
    let lab_pixels = vec![
        RgbaF32Pixel::new(30.0, -6.0, 4.0, 0.15),
        RgbaF32Pixel::new(38.0, 3.0, -5.0, 0.35),
        RgbaF32Pixel::new(46.0, 7.0, 2.0, 0.55),
        RgbaF32Pixel::new(54.0, -4.0, -6.0, 0.75),
        RgbaF32Pixel::new(62.0, 5.0, 6.0, 0.25),
        RgbaF32Pixel::new(70.0, -7.0, 3.0, 0.45),
        RgbaF32Pixel::new(78.0, 2.0, -4.0, 0.65),
        RgbaF32Pixel::new(86.0, 4.0, 5.0, 0.85),
    ];

    vec![
        (
            "bare linear sRGB",
            RgbaF32Image::new(
                descriptor(RgbaF32ColorEncoding::LinearSrgbD65),
                rgb_pixels.clone(),
            )
            .expect("linear sRGB image"),
        ),
        (
            "bare Display P3",
            RgbaF32Image::new(
                descriptor(RgbaF32ColorEncoding::DisplayP3D65),
                rgb_pixels.clone(),
            )
            .expect("Display P3 image"),
        ),
        (
            "external matrix source",
            RgbaF32Image::new(
                descriptor(RgbaF32ColorEncoding::External(profile)).with_source_color(source_color),
                rgb_pixels,
            )
            .expect("external matrix image"),
        ),
        (
            "Lab D50",
            RgbaF32Image::new(descriptor(RgbaF32ColorEncoding::LabD50), lab_pixels)
                .expect("Lab image"),
        ),
    ]
}

fn assert_gpu_image_matches_cpu(
    label: &str,
    actual: &RgbaF32Image,
    expected: &RgbaF32Image,
    source: RgbaF32Descriptor,
    tolerance: f32,
) {
    assert_eq!(
        actual.descriptor(),
        expected.descriptor(),
        "{label}: output descriptor"
    );
    assert_eq!(
        actual.descriptor().source_representation(),
        source.source_representation(),
        "{label}: source representation"
    );
    assert_eq!(
        actual.descriptor().source_orientation(),
        source.source_orientation(),
        "{label}: source orientation"
    );
    assert_eq!(
        actual.descriptor().source_color(),
        source.source_color(),
        "{label}: source color"
    );
    for (index, (actual, expected)) in actual.pixels().iter().zip(expected.pixels()).enumerate() {
        assert!(
            (actual.red() - expected.red()).abs() <= tolerance,
            "{label} pixel {index}: red {actual:?} != {expected:?}"
        );
        assert!(
            (actual.green() - expected.green()).abs() <= tolerance,
            "{label} pixel {index}: green {actual:?} != {expected:?}"
        );
        assert!(
            (actual.blue() - expected.blue()).abs() <= tolerance,
            "{label} pixel {index}: blue {actual:?} != {expected:?}"
        );
        assert_eq!(
            actual.alpha().to_bits(),
            expected.alpha().to_bits(),
            "{label} pixel {index}: alpha"
        );
    }
}
