use rusttable_core::{
    Edit, EditId, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterText, ParameterValue, PhotoId, Revision,
};
use rusttable_gpu::{GpuInitError, GpuRuntime, GpuRuntimeConfig};
use rusttable_pixelpipe::{
    CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, PixelpipeBackend, PixelpipeExecutionService,
    RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions};

fn vector(values: [f32; 7]) -> ParameterValue {
    let text = values
        .into_iter()
        .map(|value| value.to_string())
        .collect::<Vec<_>>()
        .join(",");
    ParameterValue::Text(ParameterText::new(format!("[{text}]")).expect("vector text"))
}

fn channelmixer_operation(id: u128, red: [f32; 7]) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(id).expect("operation ID"),
        OperationKey::new("rusttable.channelmixer").expect("operation key"),
        true,
        OperationOpacity::ONE,
        [
            (
                ParameterName::new("red").expect("parameter name"),
                vector(red),
            ),
            (
                ParameterName::new("green").expect("parameter name"),
                vector([0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            ),
            (
                ParameterName::new("blue").expect("parameter name"),
                vector([0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]),
            ),
            (
                ParameterName::new("algorithm_version").expect("parameter name"),
                ParameterValue::Integer(1),
            ),
        ],
    )
    .expect("Channel Mixer operation")
}

fn snapshot(operation: Operation) -> CpuPixelpipeSnapshot {
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let image = RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::SrgbD65),
        vec![RgbaF32Pixel::new(0.2, 0.4, 0.6, 0.25)],
    )
    .expect("image");
    let edit = Edit::from_parts(
        EditId::new(301).expect("edit ID"),
        PhotoId::new(302).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        [operation],
    )
    .expect("edit");
    CpuPixelpipeSnapshot::new(
        image,
        CompiledOperationGraph::compile(&edit).expect("graph"),
        CpuPixelpipeOutputMode::Preview,
    )
}

#[test]
fn snapshot_identity_includes_channelmixer_matrix_bits() {
    let base = snapshot(channelmixer_operation(
        303,
        [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
    ));
    let changed = snapshot(channelmixer_operation(
        303,
        [0.0, 0.0, 0.0, 0.999, 0.0, 0.0, 0.0],
    ));
    assert_ne!(base.identity(), changed.identity());
}

#[tokio::test]
async fn gpu_service_keeps_channelmixer_on_canonical_cpu_path() {
    let config = GpuRuntimeConfig {
        allow_cpu_fallback: false,
        ..GpuRuntimeConfig::default()
    };
    let runtime = match GpuRuntime::initialize(config).await {
        Ok(runtime) => runtime,
        Err(GpuInitError::NoAdapter) => return,
        Err(error) => panic!("GPU initialization failed: {error}"),
    };
    let result = PixelpipeExecutionService::with_gpu(runtime)
        .execute(&snapshot(channelmixer_operation(
            304,
            [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        )))
        .expect("CPU fallback result");
    assert_eq!(result.receipt().backend(), PixelpipeBackend::CpuCanonical);
}
