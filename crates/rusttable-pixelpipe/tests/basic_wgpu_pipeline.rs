use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{GpuRuntime, GpuRuntimeConfig};
use rusttable_pixelpipe::{
    CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, PixelpipeBackend,
    PixelpipeExecutionService, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
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
