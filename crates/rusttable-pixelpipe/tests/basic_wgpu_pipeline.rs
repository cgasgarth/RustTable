use rusttable_color::{
    Pcs, Primaries, ProfileClass, ProfileId, ProfileModel, ProfileParserVersion, TransferFunction,
};
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{GpuInitError, GpuRuntime, GpuRuntimeConfig};
use rusttable_image::{Orientation, SourceColor, SourceColorEvidence};
use rusttable_masks::{
    GeometryAncestry, MaskGeometry, MaskGraphBuilder, MaskIdentity, MaskNode, MaskRaster, MaskRoi,
    MaskSource,
};
use rusttable_pixelpipe::{
    CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan,
    PixelpipeBackend, PixelpipeExecutionService, RgbaF32ColorEncoding, RgbaF32Descriptor,
    RgbaF32Image, RgbaF32Pixel, RgbaF32SourceRepresentation,
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
