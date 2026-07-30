use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};

use super::*;

#[test]
fn reuses_single_output_slice_across_steps() {
    let operations = [
        operation(1, "rusttable.linear_offset", "value", 0.25),
        operation(2, "rusttable.exposure", "stops", 1.0),
    ];
    let edit = Edit::new(
        EditId::new(1).expect("nonzero edit ID"),
        PhotoId::new(2).expect("nonzero photo ID"),
        Revision::ZERO,
        operations,
    )
    .expect("valid edit");
    let pipeline = CompiledPipeline::compile(&edit).expect("valid pipeline");
    let mut pixels = vec![LinearRgb::new(
        FiniteF32::new(0.5).expect("finite"),
        FiniteF32::new(0.5).expect("finite"),
        FiniteF32::new(0.5).expect("finite"),
    )];
    let pointer = pixels.as_ptr();
    let capacity = pixels.capacity();

    for step in pipeline.active_steps() {
        apply_operation_with_plans(
            step.index(),
            step.operation(),
            &mut pixels,
            RasterDimensions::new(1, 1).expect("test dimensions"),
            0,
            None,
        )
        .expect("finite operation");
    }

    assert_eq!(pixels.as_ptr(), pointer);
    assert_eq!(pixels.capacity(), capacity);
}

fn operation(id: u128, key: &str, parameter: &str, value: f64) -> Operation {
    Operation::new(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new(key).expect("valid operation key"),
        true,
        [(
            ParameterName::new(parameter).expect("valid parameter name"),
            ParameterValue::Scalar(FiniteF64::new(value).expect("finite")),
        )],
    )
    .expect("valid operation")
}
