mod support;

use rusttable_catalog::{EditRepository, EditRepositoryError, ImportRepository};
use rusttable_catalog_store::{RedbEditRepository, RedbImportRepository};
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};

fn edit(id: u128, photo_id: u128, revision: u64) -> Edit {
    Edit::from_parts(
        EditId::new(id).unwrap(),
        PhotoId::new(photo_id).unwrap(),
        Revision::ZERO,
        Revision::from_u64(revision),
        [Operation::new_with_opacity(
            OperationId::new(id + 100).unwrap(),
            OperationKey::new("rusttable.exposure").unwrap(),
            true,
            OperationOpacity::new(0.75).unwrap(),
            [(
                ParameterName::new("stops").unwrap(),
                ParameterValue::Scalar(FiniteF64::new(0.5).unwrap()),
            )],
        )
        .unwrap()],
    )
    .unwrap()
}

#[test]
fn edits_round_trip_reopen_and_share_the_import_catalog_file() {
    let path = support::temp_path("edit-round-trip");
    let record = support::record("fixture/photo.png", 10, 11, 12);
    let first = edit(2, 10, 0);
    let second = edit(9, 10, 0);

    {
        let mut imports = RedbImportRepository::open(&path).unwrap();
        imports.commit(&record).unwrap();
    }
    {
        let mut edits = RedbEditRepository::open(&path).unwrap();
        edits.commit_new(&second).unwrap();
        edits.commit_new(&first).unwrap();
        assert_eq!(
            edits.find_by_edit_id(first.id()).unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            edits
                .list()
                .unwrap()
                .iter()
                .map(Edit::id)
                .collect::<Vec<_>>(),
            [first.id(), second.id()]
        );
    }

    let imports = RedbImportRepository::open(&path).unwrap();
    assert_eq!(
        imports.find_by_photo_id(record.photo().id()).unwrap(),
        Some(record)
    );
    drop(imports);
    let edits = RedbEditRepository::open(&path).unwrap();
    assert_eq!(edits.find_by_edit_id(second.id()).unwrap(), Some(second));
    support::remove(&path);
}

#[test]
fn replacements_recheck_current_revision_in_the_write_transaction() {
    let path = support::temp_path("edit-replacement");
    let original = edit(2, 10, 0);
    let replacement = edit(2, 10, 1);
    let mut repository = RedbEditRepository::open(&path).unwrap();
    repository.commit_new(&original).unwrap();

    assert_eq!(
        repository.commit_replacement(Revision::from_u64(9), &replacement),
        Err(EditRepositoryError::EditRevisionConflict {
            edit_id: original.id(),
            expected: Revision::from_u64(9),
            actual: Revision::ZERO,
        })
    );
    repository
        .commit_replacement(Revision::ZERO, &replacement)
        .unwrap();
    assert_eq!(
        repository.find_by_edit_id(original.id()).unwrap(),
        Some(replacement)
    );
    support::remove(&path);
}
