use std::error::Error;

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_processing::{
    CompiledPipeline, OperationCompileError, PipelineCompileError, PipelineStepIndex,
};

fn operation(id: u128, enabled: bool) -> Operation {
    Operation::new(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new("rusttable.exposure").expect("valid operation key"),
        enabled,
        [(
            ParameterName::new("stops").expect("valid parameter name"),
            ParameterValue::Scalar(FiniteF64::new(1.0).expect("finite scalar")),
        )],
    )
    .expect("valid operation")
}

fn invalid_operation(id: u128) -> Operation {
    Operation::new(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::new("rusttable.unknown").expect("valid operation key"),
        true,
        [],
    )
    .expect("valid operation")
}

fn edit(operations: Vec<Operation>) -> Edit {
    Edit::from_parts(
        EditId::new(1).expect("nonzero edit ID"),
        PhotoId::new(2).expect("nonzero photo ID"),
        Revision::from_u64(3),
        Revision::from_u64(4),
        operations,
    )
    .expect("valid edit")
}

#[test]
fn compiles_empty_edit_with_provenance() {
    let source = edit(Vec::new());

    let pipeline = CompiledPipeline::compile(&source).expect("empty edit is valid");

    assert_eq!(pipeline.source_edit_id(), source.id());
    assert_eq!(pipeline.source_photo_id(), source.photo_id());
    assert_eq!(pipeline.base_photo_revision(), source.base_photo_revision());
    assert_eq!(pipeline.revision(), source.revision());
    assert_eq!(pipeline.steps().count(), 0);
    assert_eq!(pipeline.active_steps().count(), 0);
}

#[test]
fn preserves_authoring_order_and_indices() {
    let source = edit(vec![
        operation(1, true),
        operation(2, true),
        operation(3, true),
    ]);

    let pipeline = CompiledPipeline::compile(&source).expect("operations are valid");

    assert_eq!(step_ids(&pipeline), vec![1, 2, 3]);
    assert_eq!(step_indices(&pipeline), vec![0, 1, 2]);
    assert_eq!(
        step_ids(&pipeline),
        step_ids(&CompiledPipeline::compile(&source).unwrap())
    );
}

#[test]
fn different_authoring_order_produces_different_pipeline() {
    let forward = CompiledPipeline::compile(&edit(vec![operation(1, true), operation(2, true)]))
        .expect("forward edit is valid");
    let reverse = CompiledPipeline::compile(&edit(vec![operation(2, true), operation(1, true)]))
        .expect("reverse edit is valid");

    assert_ne!(forward, reverse);
    assert_eq!(step_ids(&forward), vec![1, 2]);
    assert_eq!(step_ids(&reverse), vec![2, 1]);
}

#[test]
fn retains_disabled_steps() {
    let source = edit(vec![operation(1, false), operation(2, true)]);

    let pipeline = CompiledPipeline::compile(&source).expect("operations are valid");

    assert_eq!(step_ids(&pipeline), vec![1, 2]);
    assert!(
        !pipeline
            .steps()
            .next()
            .expect("first step")
            .operation()
            .is_enabled()
    );
}

#[test]
fn active_steps_filter_without_renumbering() {
    let source = edit(vec![
        operation(1, true),
        operation(2, false),
        operation(3, true),
        operation(4, false),
        operation(5, true),
    ]);

    let pipeline = CompiledPipeline::compile(&source).expect("operations are valid");

    assert_eq!(
        pipeline
            .active_steps()
            .map(|step| step.index().get())
            .collect::<Vec<_>>(),
        vec![0, 2, 4]
    );
    assert_eq!(
        pipeline
            .active_steps()
            .map(|step| step.operation().operation_id().get())
            .collect::<Vec<_>>(),
        vec![1, 3, 5]
    );
}

#[test]
fn reports_exact_failing_step_atomically() {
    let source = edit(vec![
        operation(1, true),
        invalid_operation(2),
        operation(3, true),
    ]);

    let error = CompiledPipeline::compile(&source).expect_err("unknown key must fail");

    assert!(matches!(
        &error,
        PipelineCompileError::Operation {
            edit_id,
            step_index,
            operation_id,
            source: OperationCompileError::UnsupportedOperationKey { .. },
        } if *edit_id == source.id()
            && *step_index == PipelineStepIndex::new(1)
            && *operation_id == OperationId::new(2).unwrap()
    ));
    let nested = error
        .source()
        .and_then(|source| source.downcast_ref::<OperationCompileError>())
        .expect("pipeline error should preserve its nested source");
    assert!(matches!(
        nested,
        OperationCompileError::UnsupportedOperationKey { operation_id, .. }
            if *operation_id == OperationId::new(2).unwrap()
    ));
}

#[test]
fn equal_edits_compile_equally() {
    let first = edit(vec![operation(1, true), operation(2, false)]);
    let second = edit(vec![operation(1, true), operation(2, false)]);

    assert_eq!(
        CompiledPipeline::compile(&first),
        CompiledPipeline::compile(&second)
    );
}

#[test]
fn revisions_are_provenance_not_processing_inputs() {
    let first = Edit::from_parts(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::from_u64(3),
        Revision::from_u64(4),
        [operation(1, true)],
    )
    .unwrap();
    let second = Edit::from_parts(
        first.id(),
        first.photo_id(),
        Revision::from_u64(5),
        Revision::from_u64(6),
        [operation(1, true)],
    )
    .unwrap();

    let first_pipeline = CompiledPipeline::compile(&first).unwrap();
    let second_pipeline = CompiledPipeline::compile(&second).unwrap();

    assert_ne!(first_pipeline, second_pipeline);
    assert_eq!(
        first_pipeline.steps().next().unwrap().operation(),
        second_pipeline.steps().next().unwrap().operation()
    );
    assert_eq!(first_pipeline.base_photo_revision(), Revision::from_u64(3));
    assert_eq!(second_pipeline.revision(), Revision::from_u64(6));
}

fn step_ids(pipeline: &CompiledPipeline) -> Vec<u128> {
    pipeline
        .steps()
        .map(|step| step.operation().operation_id().get())
        .collect()
}

fn step_indices(pipeline: &CompiledPipeline) -> Vec<usize> {
    pipeline.steps().map(|step| step.index().get()).collect()
}
