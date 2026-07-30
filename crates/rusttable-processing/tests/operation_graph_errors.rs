use std::error::Error;

use rusttable_core::{Edit, EditId, Operation, OperationId, OperationKey, PhotoId, Revision};
use rusttable_processing::{
    CompiledOperationGraph, OperationCompileError, OperationGraphCompileError, PipelineStepIndex,
};

fn invalid_operation(id: u128) -> Operation {
    Operation::new(
        OperationId::new(id).unwrap(),
        OperationKey::new("rusttable.unsupported").unwrap(),
        true,
        [],
    )
    .unwrap()
}

fn valid_operation(id: u128) -> Operation {
    Operation::new(
        OperationId::new(id).unwrap(),
        OperationKey::new("rusttable.exposure").unwrap(),
        true,
        [(
            rusttable_core::ParameterName::new("stops").unwrap(),
            rusttable_core::ParameterValue::Scalar(rusttable_core::FiniteF64::new(1.0).unwrap()),
        )],
    )
    .unwrap()
}

fn edit(operations: Vec<Operation>) -> Edit {
    Edit::from_parts(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        Revision::ZERO,
        operations,
    )
    .unwrap()
}

#[test]
fn invalid_pipeline_returns_exact_nested_error_without_partial_graph() {
    let source = edit(vec![invalid_operation(7)]);
    let error = CompiledOperationGraph::compile(&source).unwrap_err();

    assert!(matches!(
        &error,
        OperationGraphCompileError::Pipeline(source_error)
            if matches!(
                source_error.as_ref(),
                rusttable_processing::PipelineCompileError::Operation {
                    edit_id,
                    step_index,
                    operation_id,
                    source: OperationCompileError::UnsupportedOperationKey { .. },
                } if *edit_id == source.id()
                    && *step_index == PipelineStepIndex::new(0)
                    && *operation_id == OperationId::new(7).unwrap()
            )
    ));
    assert!(error.source().is_some());
}

#[test]
fn later_invalid_operation_reports_its_authored_step() {
    let first = valid_operation(1);
    let error =
        CompiledOperationGraph::compile(&edit(vec![first, invalid_operation(8)])).unwrap_err();

    assert!(matches!(
        &error,
        OperationGraphCompileError::Pipeline(source_error)
            if matches!(
                source_error.as_ref(),
                rusttable_processing::PipelineCompileError::Operation {
                    step_index,
                    operation_id,
                    ..
                } if *step_index == PipelineStepIndex::new(1)
                    && *operation_id == OperationId::new(8).unwrap()
            )
    ));
}
