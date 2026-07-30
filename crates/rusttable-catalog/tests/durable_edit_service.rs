mod support;

use rusttable_catalog::{
    DurableEditError, DurableEditOutcome, DurableEditService, EditRepositoryError,
};
use rusttable_core::{EditId, Revision};
use support::edit::{FakeEditRepository, edit, state_with_edit, state_with_photo};

#[test]
fn create_commits_once_and_advances_catalog_once() {
    let mut state = state_with_photo();
    let mut repository = FakeEditRepository::default();
    let value = edit(2, 1, 0);

    let outcome =
        DurableEditService::create(&mut state, Revision::from_u64(1), &value, &mut repository)
            .unwrap();

    assert_eq!(
        outcome,
        DurableEditOutcome::Created {
            edit: value.clone(),
            catalog_revision: Revision::from_u64(2)
        }
    );
    assert_eq!(state.edit(value.id()), Some(&value));
    assert_eq!(state.revision(), Revision::from_u64(2));
    assert_eq!(repository.edits.get(&value.id()), Some(&value));
    assert_eq!(repository.calls, ["commit_new"]);
}

#[test]
fn replacement_commits_once_and_advances_edit_and_catalog() {
    let mut state = state_with_edit(0);
    let current = state.edit(EditId::new(2).unwrap()).unwrap().clone();
    let replacement = edit(2, 1, 1);
    let mut repository = FakeEditRepository::default();
    repository.edits.insert(current.id(), current);

    let outcome = DurableEditService::replace(
        &mut state,
        Revision::from_u64(2),
        EditId::new(2).unwrap(),
        Revision::ZERO,
        &replacement,
        &mut repository,
    )
    .unwrap();

    assert_eq!(
        outcome,
        DurableEditOutcome::Replaced {
            edit: replacement.clone(),
            catalog_revision: Revision::from_u64(3)
        }
    );
    assert_eq!(state.edit(replacement.id()), Some(&replacement));
    assert_eq!(state.revision(), Revision::from_u64(3));
    assert_eq!(repository.edits.get(&replacement.id()), Some(&replacement));
    assert_eq!(repository.calls, ["commit_replacement"]);
}

#[test]
fn identical_create_retry_is_already_present_without_a_second_write() {
    let mut state = state_with_edit(0);
    let value = state.edit(EditId::new(2).unwrap()).unwrap().clone();
    let mut repository = FakeEditRepository::default();
    repository.edits.insert(value.id(), value.clone());

    let expected_catalog_revision = state.revision();
    let outcome = DurableEditService::create(
        &mut state,
        expected_catalog_revision,
        &value,
        &mut repository,
    )
    .unwrap();

    assert_eq!(
        outcome,
        DurableEditOutcome::AlreadyPresent {
            edit: value,
            catalog_revision: Revision::from_u64(2)
        }
    );
    assert!(repository.calls.is_empty());
}

#[test]
fn identical_replacement_retry_is_already_present_only_for_successor_revision() {
    let mut state = state_with_edit(0);
    let replacement = edit(2, 1, 1);
    state
        .apply(
            state.revision(),
            rusttable_catalog::CatalogCommand::ReplaceEdit {
                edit_id: replacement.id(),
                expected_edit_revision: Revision::ZERO,
                replacement: replacement.clone(),
            },
        )
        .unwrap();
    let mut repository = FakeEditRepository::default();
    repository
        .edits
        .insert(replacement.id(), replacement.clone());

    let expected_catalog_revision = state.revision();
    let outcome = DurableEditService::replace(
        &mut state,
        expected_catalog_revision,
        replacement.id(),
        Revision::ZERO,
        &replacement,
        &mut repository,
    )
    .unwrap();

    assert_eq!(
        outcome,
        DurableEditOutcome::AlreadyPresent {
            edit: replacement,
            catalog_revision: Revision::from_u64(3)
        }
    );
    assert!(repository.calls.is_empty());
}

#[test]
fn stale_catalog_revision_fails_before_repository_access() {
    let mut state = state_with_photo();
    let mut repository = FakeEditRepository::default();
    let error =
        DurableEditService::create(&mut state, Revision::ZERO, &edit(2, 1, 0), &mut repository)
            .unwrap_err();

    assert!(matches!(error, DurableEditError::Catalog(_)));
    assert!(repository.calls.is_empty());
}

#[test]
fn commit_failure_keeps_state_and_repository_unchanged() {
    let mut state = state_with_photo();
    let before = state.clone();
    let value = edit(2, 1, 0);
    let mut repository = FakeEditRepository {
        commit_error: Some(EditRepositoryError::CommitFailure),
        ..FakeEditRepository::default()
    };

    let error =
        DurableEditService::create(&mut state, Revision::from_u64(1), &value, &mut repository)
            .unwrap_err();

    assert!(matches!(
        error,
        DurableEditError::Repository(EditRepositoryError::CommitFailure)
    ));
    assert_eq!(state, before);
    assert!(repository.edits.is_empty());
}
