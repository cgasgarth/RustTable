mod support;

use rusttable_catalog::{EditRepository, EditRepositoryError};
use rusttable_core::{EditId, Revision};
use support::edit::{FakeEditRepository, edit, state_with_edit};

#[test]
fn edit_repository_port_is_object_safe() {
    let repository: Box<dyn EditRepository> = Box::new(FakeEditRepository::default());
    assert_eq!(repository.list().unwrap(), Vec::new());
}

#[test]
fn list_returns_current_edits_in_ascending_id_order() {
    let mut repository = FakeEditRepository::default();
    repository
        .edits
        .insert(EditId::new(9).unwrap(), edit(9, 1, 0));
    repository
        .edits
        .insert(EditId::new(2).unwrap(), edit(2, 1, 0));

    assert_eq!(
        repository
            .list()
            .unwrap()
            .into_iter()
            .map(|value| value.id())
            .collect::<Vec<_>>(),
        [EditId::new(2).unwrap(), EditId::new(9).unwrap()]
    );
}

#[test]
fn new_commit_rechecks_id_absence() {
    let mut repository = FakeEditRepository::default();
    let value = edit(2, 1, 0);
    repository.edits.insert(value.id(), value.clone());

    assert_eq!(
        repository.commit_new(&value),
        Err(EditRepositoryError::NewEditIdConflict {
            edit_id: value.id()
        })
    );
}

#[test]
fn replacement_commit_rechecks_current_revision() {
    let mut repository = FakeEditRepository::default();
    repository
        .edits
        .insert(EditId::new(2).unwrap(), edit(2, 1, 1));

    assert_eq!(
        repository.commit_replacement(Revision::ZERO, &edit(2, 1, 2)),
        Err(EditRepositoryError::EditRevisionConflict {
            edit_id: EditId::new(2).unwrap(),
            expected: Revision::ZERO,
            actual: Revision::from_u64(1),
        })
    );
}

#[test]
fn support_state_has_the_expected_catalog_revision() {
    assert_eq!(state_with_edit(0).revision(), Revision::from_u64(2));
}
