use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_pixelpipe::{
    CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot, CpuTilePlan,
    RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32Pixel,
};
use rusttable_processing::{CompiledOperationGraph, RasterDimensions};

fn graph() -> CompiledOperationGraph {
    let operation = Operation::new(
        OperationId::new(473).expect("operation ID"),
        OperationKey::new("rusttable.clahe").expect("operation key"),
        true,
        [
            (
                ParameterName::new("radius").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(2.0).unwrap()),
            ),
            (
                ParameterName::new("slope").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(2.0).unwrap()),
            ),
        ],
    )
    .expect("operation");
    let edit = Edit::from_parts(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        Revision::from_u64(1),
        [operation],
    )
    .unwrap();
    CompiledOperationGraph::compile(&edit).expect("graph")
}

fn image() -> RgbaF32Image {
    let dimensions = RasterDimensions::new(5, 3).unwrap();
    let pixels = (0..15)
        .map(|index| {
            let value = f32::from(u8::try_from(index).expect("focused test image fits u8")) / 20.0;
            RgbaF32Pixel::new(value, value + 0.1, value + 0.2, 0.2 + value)
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::SrgbD65),
        pixels,
    )
    .unwrap()
}

#[test]
fn full_and_tiled_cpu_pixelpipe_execution_share_the_frame_plan() {
    let snapshot = CpuPixelpipeSnapshot::new(image(), graph(), CpuPixelpipeOutputMode::FullExport);
    let executor = CpuPixelpipeExecutor;
    let full = executor.execute(&snapshot).expect("full CLAHE");
    let tiled = executor
        .execute_tiled(&snapshot, CpuTilePlan::new(2, 2).unwrap())
        .expect("full-frame CLAHE through tiled request");
    assert_eq!(full.image(), tiled.image());
    assert_eq!(full.receipt(), tiled.receipt());
    assert!(full.image().pixels().iter().all(|pixel| {
        pixel.red().is_finite()
            && pixel.green().is_finite()
            && pixel.blue().is_finite()
            && pixel.alpha().is_finite()
    }));
}
