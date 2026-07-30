use std::error::Error;

use rusttable_core::{
    Edit, EditBuildError, EditId, EditRevisionError, Operation, OperationId, OperationKey, PhotoId,
    Revision,
};

fn operation(id: u128) -> Operation {
    Operation::new(
        OperationId::new(id).expect("test operation IDs are nonzero"),
        OperationKey::new("rusttable.exposure").expect("valid operation key"),
        id.is_multiple_of(2),
        [],
    )
    .expect("valid operation")
}

fn edit(operations: Vec<Operation>) -> Edit {
    Edit::new(
        EditId::new(1).expect("test edit ID is nonzero"),
        PhotoId::new(2).expect("test photo ID is nonzero"),
        Revision::from_u64(3),
        operations,
    )
    .expect("valid edit")
}

#[test]
fn new_and_reconstructed_edits_preserve_identity_revisions_and_order() {
    let first_operation = operation(1);
    let second_operation = operation(2);
    let first_id = first_operation.id();
    let second_id = second_operation.id();
    let value = edit(vec![first_operation, second_operation]);
    let reconstructed = Edit::from_parts(
        value.id(),
        value.photo_id(),
        value.base_photo_revision(),
        Revision::from_u64(4),
        value.operations().cloned().collect::<Vec<_>>(),
    )
    .expect("valid reconstruction");

    assert_eq!(value.revision(), Revision::ZERO);
    assert_eq!(value.base_photo_revision(), Revision::from_u64(3));
    assert_eq!(
        value.operations().map(Operation::id).collect::<Vec<_>>(),
        vec![first_id, second_id]
    );
    assert_eq!(reconstructed.revision(), Revision::from_u64(4));
    assert_eq!(reconstructed.operations().count(), 2);
}

#[test]
fn empty_edits_are_valid_and_operation_order_is_significant() {
    let empty = edit(Vec::new());
    let forward = edit(vec![operation(1), operation(2)]);
    let reverse = edit(vec![operation(2), operation(1)]);

    assert!(empty.operations().next().is_none());
    assert_ne!(forward, reverse);
    assert_eq!(
        forward.operations().map(Operation::id).collect::<Vec<_>>(),
        vec![OperationId::new(1).unwrap(), OperationId::new(2).unwrap()]
    );
    assert_eq!(
        reverse.operations().map(Operation::id).collect::<Vec<_>>(),
        vec![OperationId::new(2).unwrap(), OperationId::new(1).unwrap()]
    );
}

#[test]
fn duplicate_operation_ids_are_rejected() {
    let duplicate_id = OperationId::new(7).expect("nonzero");
    let error = Edit::new(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        vec![operation(7), operation(7)],
    )
    .expect_err("duplicate operation IDs are invalid");

    assert_eq!(
        error,
        EditBuildError::DuplicateOperationId {
            operation_id: duplicate_id
        }
    );
}

#[test]
fn revised_edits_advance_once_without_mutating_the_original() {
    let original = edit(vec![operation(1)]);
    let revised = original
        .revised(vec![operation(2)])
        .expect("revision succeeds");

    assert_eq!(original.revision(), Revision::ZERO);
    assert_eq!(revised.revision(), Revision::from_u64(1));
    assert_eq!(revised.id(), original.id());
    assert_eq!(revised.photo_id(), original.photo_id());
    assert_eq!(
        revised.base_photo_revision(),
        original.base_photo_revision()
    );
    assert_eq!(
        revised.operations().map(Operation::id).collect::<Vec<_>>(),
        vec![OperationId::new(2).unwrap()]
    );
    assert_eq!(
        original.operations().map(Operation::id).collect::<Vec<_>>(),
        vec![OperationId::new(1).unwrap()]
    );
}

#[test]
fn revision_overflow_precedes_replacement_validation_and_preserves_state() {
    let original = Edit::from_parts(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::from_u64(3),
        Revision::from_u64(u64::MAX),
        vec![operation(1)],
    )
    .expect("valid maximum edit");
    let before = original.clone();

    let error = original
        .revised(vec![operation(2), operation(2)])
        .expect_err("overflow is invalid");

    assert_eq!(error, EditRevisionError::RevisionOverflow);
    assert_eq!(original, before);
}

#[test]
fn invalid_replacement_preserves_nested_source_and_original() {
    let original = edit(vec![operation(1)]);
    let before = original.clone();
    let error = original
        .revised(vec![operation(2), operation(2)])
        .expect_err("duplicate replacement is invalid");

    let source = match &error {
        EditRevisionError::InvalidReplacementOperations { source } => source,
        EditRevisionError::RevisionOverflow => panic!("unexpected overflow"),
    };
    assert!(matches!(
        source,
        EditBuildError::DuplicateOperationId { .. }
    ));
    assert!(error.source().is_some());
    assert_eq!(original, before);
}
