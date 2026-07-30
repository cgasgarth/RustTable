mod support;

use rusttable_catalog::{DurableEditError, DurableEditService, EditRepositoryError};
use rusttable_core::{EditId, Revision};
use support::edit::{FakeEditRepository, edit, state_with_edit, state_with_photo};

#[test]
fn persisted_only_and_state_only_edits_are_distinguishable() {
    let mut state = state_with_photo();
    let value = edit(2, 1, 0);
    let mut repository = FakeEditRepository::default();
    repository.edits.insert(value.id(), value.clone());
    let expected_catalog_revision = state.revision();
    let persisted_only = DurableEditService::create(
        &mut state,
        expected_catalog_revision,
        &value,
        &mut repository,
    )
    .unwrap_err();
    assert!(matches!(
        persisted_only,
        DurableEditError::PersistedOnly { .. }
    ));

    let mut state = state_with_edit(0);
    let mut repository = FakeEditRepository::default();
    let expected_catalog_revision = state.revision();
    let state_only = DurableEditService::create(
        &mut state,
        expected_catalog_revision,
        &value,
        &mut repository,
    )
    .unwrap_err();
    assert!(matches!(state_only, DurableEditError::StateOnly { .. }));
}

#[test]
fn persisted_and_state_mismatch_is_rejected_without_mutation() {
    let mut state = state_with_edit(0);
    let before = state.clone();
    let mut repository = FakeEditRepository::default();
    repository
        .edits
        .insert(EditId::new(2).unwrap(), edit(2, 1, 1));

    let expected_catalog_revision = state.revision();
    let error = DurableEditService::create(
        &mut state,
        expected_catalog_revision,
        &edit(2, 1, 1),
        &mut repository,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        DurableEditError::PersistedStateMismatch { .. }
    ));
    assert_eq!(state, before);
}

#[test]
fn requested_divergence_is_rejected_after_authority_consistency() {
    let mut state = state_with_edit(0);
    let value = state.edit(EditId::new(2).unwrap()).unwrap().clone();
    let mut repository = FakeEditRepository::default();
    repository.edits.insert(value.id(), value);

    let expected_catalog_revision = state.revision();
    let error = DurableEditService::create(
        &mut state,
        expected_catalog_revision,
        &edit(2, 1, 1),
        &mut repository,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        DurableEditError::RequestedEditMismatch { .. }
    ));
}

#[test]
fn replacement_lookup_and_revision_errors_are_typed_and_atomic() {
    let mut state = state_with_edit(0);
    let before = state.clone();
    let mut repository = FakeEditRepository::default();
    repository
        .edits
        .insert(EditId::new(2).unwrap(), edit(2, 1, 0));

    let expected_catalog_revision = state.revision();
    let revision_error = DurableEditService::replace(
        &mut state,
        expected_catalog_revision,
        EditId::new(2).unwrap(),
        Revision::from_u64(4),
        &edit(2, 1, 1),
        &mut repository,
    )
    .unwrap_err();
    assert!(matches!(
        revision_error,
        DurableEditError::Repository(EditRepositoryError::EditRevisionConflict { .. })
    ));
    assert_eq!(state, before);

    let mut missing_state = state_with_photo();
    let mut empty_repository = FakeEditRepository::default();
    let missing = DurableEditService::replace(
        &mut missing_state,
        Revision::from_u64(1),
        EditId::new(2).unwrap(),
        Revision::ZERO,
        &edit(2, 1, 1),
        &mut empty_repository,
    )
    .unwrap_err();
    assert!(matches!(missing, DurableEditError::UnknownEdit { .. }));
}

#[test]
fn repository_lookup_failure_is_nested_and_state_is_unchanged() {
    let mut state = state_with_photo();
    let before = state.clone();
    let mut repository = FakeEditRepository {
        lookup_error: Some(EditRepositoryError::Unavailable),
        ..FakeEditRepository::default()
    };

    let error = DurableEditService::create(
        &mut state,
        Revision::from_u64(1),
        &edit(2, 1, 0),
        &mut repository,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        DurableEditError::Repository(EditRepositoryError::Unavailable)
    ));
    assert!(std::error::Error::source(&error).is_some());
    assert_eq!(state, before);
}

#[test]
fn catalog_command_preflight_errors_are_nested_and_do_not_write() {
    let mut state = state_with_edit(0);
    let before = state.clone();
    let mut repository = FakeEditRepository::default();
    let current = state.edit(EditId::new(2).unwrap()).unwrap().clone();
    repository.edits.insert(current.id(), current);
    let invalid = edit(2, 1, 2);

    let expected_catalog_revision = state.revision();
    let error = DurableEditService::replace(
        &mut state,
        expected_catalog_revision,
        EditId::new(2).unwrap(),
        Revision::ZERO,
        &invalid,
        &mut repository,
    )
    .unwrap_err();

    assert!(matches!(error, DurableEditError::Catalog(_)));
    assert_eq!(state, before);
    assert!(repository.calls.is_empty());
}

#[test]
fn replacement_repository_failure_preserves_state() {
    let mut state = state_with_edit(0);
    let before = state.clone();
    let current = state.edit(EditId::new(2).unwrap()).unwrap().clone();
    let mut repository = FakeEditRepository {
        edits: [(current.id(), current)].into_iter().collect(),
        commit_error: Some(EditRepositoryError::CommitFailure),
        ..FakeEditRepository::default()
    };

    let expected_catalog_revision = state.revision();
    let error = DurableEditService::replace(
        &mut state,
        expected_catalog_revision,
        EditId::new(2).unwrap(),
        Revision::ZERO,
        &edit(2, 1, 1),
        &mut repository,
    )
    .unwrap_err();

    assert!(matches!(
        error,
        DurableEditError::Repository(EditRepositoryError::CommitFailure)
    ));
    assert_eq!(state, before);
}
