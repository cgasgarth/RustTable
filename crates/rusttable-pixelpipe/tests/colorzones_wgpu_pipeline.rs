use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{GpuInitError, GpuRuntime, GpuRuntimeConfig};
use rusttable_pixelpipe::{
    CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan,
    PixelpipeBackend, PixelpipeExecutionService, RgbaF32ColorEncoding, RgbaF32Descriptor,
    RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::operations::colorzones::{ColorZonesChannel, ColorZonesPlan};
use rusttable_processing::{
    CompiledOperationGraph, ProcessingOperation, ProcessingOperationKind, RasterDimensions,
    builtin_registry,
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

fn scalar(value: f64) -> ParameterValue {
    ParameterValue::Scalar(FiniteF64::new(value).expect("finite Color Zones parameter"))
}

fn colorzones_operation(opacity: OperationOpacity) -> Operation {
    let operation_id = OperationId::new(0xc801).expect("Color Zones operation ID");
    let defaults = builtin_registry()
        .materialize_operation("rusttable.colorzones", operation_id)
        .expect("Color Zones defaults");
    let parameters = defaults
        .parameters()
        .map(|(name, value)| {
            let replacement = match name.as_str() {
                "channel" => Some(ParameterValue::Integer(0)),
                "mode" => Some(ParameterValue::Integer(1)),
                "curve_0_num_nodes" => Some(ParameterValue::Integer(4)),
                "curve_0_node_0_x" => Some(scalar(0.25)),
                "curve_0_node_0_y" | "curve_0_node_3_y" => Some(scalar(0.5)),
                "curve_0_node_1_x" => Some(scalar(0.49)),
                "curve_0_node_1_y" => Some(scalar(0.0)),
                "curve_0_node_2_x" => Some(scalar(0.4926)),
                "curve_0_node_2_y" => Some(scalar(1.0)),
                "curve_0_node_3_x" => Some(scalar(0.75)),
                _ => None,
            };
            (name.clone(), replacement.unwrap_or_else(|| value.clone()))
        })
        .collect::<Vec<_>>();
    Operation::new_with_opacity(
        operation_id,
        defaults.key().clone(),
        true,
        opacity,
        parameters,
    )
    .expect("checked Color Zones operation")
}

fn compiled_plan(operation: &Operation) -> ColorZonesPlan {
    let compiled = ProcessingOperation::compile(operation).expect("compiled Color Zones operation");
    let ProcessingOperationKind::ColorZones { plan } = compiled.kind() else {
        panic!("Color Zones operation compiled to the wrong kind");
    };
    plan.clone()
}

fn steep_lut_index(plan: &ColorZonesPlan) -> usize {
    let lut = plan.lut(ColorZonesChannel::Lightness);
    (1..lut.len() - 2)
        .max_by(|left, right| {
            let left_delta = (lut[*left + 1] - lut[*left]).abs();
            let right_delta = (lut[*right + 1] - lut[*right]).abs();
            left_delta.total_cmp(&right_delta)
        })
        .expect("Color Zones LUT has interior samples")
}

fn snapshot(operation: Operation, lightness: f32) -> CpuPixelpipeSnapshot {
    let dimensions = RasterDimensions::new(2, 1).expect("dimensions");
    let alpha = f32::from_bits(0x3eaa_aaab);
    let input = RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::LabD50),
        vec![
            RgbaF32Pixel::new(lightness, 24.0, -12.0, alpha),
            RgbaF32Pixel::new(lightness, -8.0, 18.0, f32::from_bits(0x3f2a_aaab)),
        ],
    )
    .expect("Lab input");
    let edit = Edit::from_parts(
        EditId::new(0xc802).expect("edit ID"),
        PhotoId::new(0xc803).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(4),
        [operation],
    )
    .expect("Color Zones edit");
    CpuPixelpipeSnapshot::new(
        input,
        CompiledOperationGraph::compile(&edit).expect("Color Zones graph"),
        CpuPixelpipeOutputMode::FullExport,
    )
}

#[tokio::test]
#[expect(
    clippy::suboptimal_flops,
    reason = "Preserve the native Color Zones LUT selection and opacity arithmetic order"
)]
async fn dedicated_colorzones_gpu_uses_nearest_lut_with_opacity_and_tiles_when_available() {
    let Some(runtime) = gpu_runtime().await else {
        return;
    };
    let opacity = OperationOpacity::new(0.5).expect("partial opacity");
    let operation = colorzones_operation(opacity);
    let plan = compiled_plan(&operation);
    let lut_index = steep_lut_index(&plan);
    let lut_index_f32 = f32::from(u16::try_from(lut_index).expect("LUT index fits u16"));
    let selection = (lut_index_f32 + 0.25) / 65_536.0;
    let lightness = selection * 100.0;
    let snapshot = snapshot(operation, lightness);
    let canonical = CpuPixelpipeExecutor
        .execute(&snapshot)
        .expect("canonical CPU Color Zones result");
    let service = PixelpipeExecutionService::with_gpu(runtime);

    let direct = service
        .execute(&snapshot)
        .expect("dedicated Color Zones GPU result");
    let tiled = service
        .execute_tiled(&snapshot, CpuTilePlan::new(1, 1).expect("one-pixel tiles"))
        .expect("tiled Color Zones GPU result");

    assert_eq!(direct.receipt().backend(), PixelpipeBackend::WgpuColorZones);
    assert_eq!(direct.receipt().dispatches(), 1);
    assert_eq!(direct.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(tiled.receipt().backend(), PixelpipeBackend::WgpuTiled);
    assert_eq!(tiled.receipt().dispatches(), 2);
    assert_eq!(tiled.receipt().snapshot_identity(), snapshot.identity());
    assert_eq!(
        tiled
            .receipt()
            .tiling()
            .expect("tiling receipt")
            .tile_count(),
        2
    );

    let nearest_candidate =
        lightness * 2.0_f32.powf(4.0 * (plan.lut(ColorZonesChannel::Lightness)[lut_index] - 0.5));
    let expected_lightness = lightness * 0.5 + nearest_candidate * 0.5;
    assert!(
        (direct.image().pixels()[0].red() - expected_lightness).abs() <= 0.000_05,
        "dedicated GPU must use the source OpenCL nearest LUT sample before opacity blending"
    );
    assert!(
        (direct.image().pixels()[0].red() - canonical.image().pixels()[0].red()).abs() > 0.000_01,
        "non-bin-aligned WGPU lookup must remain distinct from CPU interpolation"
    );
    for ((source, direct_pixel), tiled_pixel) in snapshot
        .input()
        .pixels()
        .iter()
        .zip(direct.image().pixels())
        .zip(tiled.image().pixels())
    {
        assert!((direct_pixel.red() - tiled_pixel.red()).abs() <= 0.000_05);
        assert!((direct_pixel.green() - tiled_pixel.green()).abs() <= 0.000_05);
        assert!((direct_pixel.blue() - tiled_pixel.blue()).abs() <= 0.000_05);
        assert_eq!(direct_pixel.alpha().to_bits(), source.alpha().to_bits());
        assert_eq!(tiled_pixel.alpha().to_bits(), source.alpha().to_bits());
    }
}

#[test]
fn colorzones_parameter_names_used_by_the_gpu_fixture_remain_canonical() {
    let operation = colorzones_operation(OperationOpacity::ONE);
    for name in [
        "channel",
        "mode",
        "curve_0_num_nodes",
        "curve_0_node_1_x",
        "curve_0_node_2_y",
    ] {
        assert!(
            operation
                .parameter(&ParameterName::new(name).expect("parameter name"))
                .is_some(),
            "missing canonical Color Zones parameter {name}"
        );
    }
}
