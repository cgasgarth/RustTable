use std::fmt;

use rusttable_core::{Edit, EditId, Revision};

use crate::{CatalogCommand, CatalogError, CatalogState, EditRepository, EditRepositoryError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableEditOutcome {
    Created {
        edit: Edit,
        catalog_revision: Revision,
    },
    Replaced {
        edit: Edit,
        catalog_revision: Revision,
    },
    AlreadyPresent {
        edit: Edit,
        catalog_revision: Revision,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableEditError {
    Catalog(CatalogError),
    Repository(EditRepositoryError),
    PersistedOnly { edit_id: EditId },
    StateOnly { edit_id: EditId },
    PersistedStateMismatch { edit_id: EditId },
    RequestedEditMismatch { edit_id: EditId },
    UnknownEdit { edit_id: EditId },
}

impl fmt::Display for DurableEditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog(source) => write!(formatter, "durable edit catalog failure: {source}"),
            Self::Repository(source) => {
                write!(formatter, "durable edit repository failure: {source}")
            }
            Self::PersistedOnly { edit_id } => {
                write!(formatter, "edit {edit_id} exists only in the repository")
            }
            Self::StateOnly { edit_id } => write!(formatter, "edit {edit_id} exists only in state"),
            Self::PersistedStateMismatch { edit_id } => {
                write!(
                    formatter,
                    "edit {edit_id} differs between state and repository"
                )
            }
            Self::RequestedEditMismatch { edit_id } => {
                write!(
                    formatter,
                    "requested edit {edit_id} differs from authoritative state"
                )
            }
            Self::UnknownEdit { edit_id } => write!(formatter, "edit {edit_id} is unknown"),
        }
    }
}

impl std::error::Error for DurableEditError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(source) => Some(source),
            Self::Repository(source) => Some(source),
            _ => None,
        }
    }
}

pub struct DurableEditService;

impl DurableEditService {
    /// Creates one edit with failure-atomic catalog and repository coordination.
    ///
    /// # Errors
    ///
    /// Returns a typed catalog, repository, consistency, or retry error without
    /// mutating the caller's state before durable commit succeeds.
    pub fn create(
        state: &mut CatalogState,
        expected_catalog_revision: Revision,
        requested: &Edit,
        repository: &mut dyn EditRepository,
    ) -> Result<DurableEditOutcome, DurableEditError> {
        validate_catalog_revision(state, expected_catalog_revision)?;
        let persisted = repository
            .find_by_edit_id(requested.id())
            .map_err(DurableEditError::Repository)?;
        match (persisted, state.edit(requested.id())) {
            (None, None) => {
                let mut candidate = state.clone();
                let catalog_revision = candidate
                    .apply(
                        expected_catalog_revision,
                        CatalogCommand::CreateEdit(requested.clone()),
                    )
                    .map_err(DurableEditError::Catalog)?;
                repository
                    .commit_new(requested)
                    .map_err(DurableEditError::Repository)?;
                *state = candidate;
                Ok(DurableEditOutcome::Created {
                    edit: requested.clone(),
                    catalog_revision,
                })
            }
            (None, Some(_)) => Err(DurableEditError::StateOnly {
                edit_id: requested.id(),
            }),
            (Some(_), None) => Err(DurableEditError::PersistedOnly {
                edit_id: requested.id(),
            }),
            (Some(persisted), Some(state_edit)) => {
                if persisted != *state_edit {
                    return Err(DurableEditError::PersistedStateMismatch {
                        edit_id: requested.id(),
                    });
                }
                if persisted != *requested {
                    return Err(DurableEditError::RequestedEditMismatch {
                        edit_id: requested.id(),
                    });
                }
                Ok(DurableEditOutcome::AlreadyPresent {
                    edit: persisted,
                    catalog_revision: state.revision(),
                })
            }
        }
    }

    /// Replaces one edit with failure-atomic catalog and repository coordination.
    ///
    /// # Errors
    ///
    /// Returns a typed catalog, repository, consistency, or retry error without
    /// mutating the caller's state before durable commit succeeds.
    pub fn replace(
        state: &mut CatalogState,
        expected_catalog_revision: Revision,
        edit_id: EditId,
        expected_edit_revision: Revision,
        replacement: &Edit,
        repository: &mut dyn EditRepository,
    ) -> Result<DurableEditOutcome, DurableEditError> {
        validate_catalog_revision(state, expected_catalog_revision)?;
        let persisted = repository
            .find_by_edit_id(edit_id)
            .map_err(DurableEditError::Repository)?;
        let Some(persisted) = persisted else {
            return if state.edit(edit_id).is_some() {
                Err(DurableEditError::StateOnly { edit_id })
            } else {
                Err(DurableEditError::UnknownEdit { edit_id })
            };
        };
        let Some(state_edit) = state.edit(edit_id) else {
            return Err(DurableEditError::PersistedOnly { edit_id });
        };
        if persisted != *state_edit {
            return Err(DurableEditError::PersistedStateMismatch { edit_id });
        }
        if is_successful_retry(&persisted, expected_edit_revision, replacement) {
            return Ok(DurableEditOutcome::AlreadyPresent {
                edit: persisted,
                catalog_revision: state.revision(),
            });
        }
        if persisted.revision() != expected_edit_revision {
            return Err(DurableEditError::Repository(
                EditRepositoryError::EditRevisionConflict {
                    edit_id,
                    expected: expected_edit_revision,
                    actual: persisted.revision(),
                },
            ));
        }
        let mut candidate = state.clone();
        let catalog_revision = candidate
            .apply(
                expected_catalog_revision,
                CatalogCommand::ReplaceEdit {
                    edit_id,
                    expected_edit_revision,
                    replacement: replacement.clone(),
                },
            )
            .map_err(DurableEditError::Catalog)?;
        repository
            .commit_replacement(expected_edit_revision, replacement)
            .map_err(DurableEditError::Repository)?;
        *state = candidate;
        Ok(DurableEditOutcome::Replaced {
            edit: replacement.clone(),
            catalog_revision,
        })
    }
}

fn validate_catalog_revision(
    state: &CatalogState,
    expected: Revision,
) -> Result<(), DurableEditError> {
    if expected != state.revision() {
        return Err(DurableEditError::Catalog(
            CatalogError::CatalogRevisionConflict {
                expected,
                actual: state.revision(),
            },
        ));
    }
    Ok(())
}

fn is_successful_retry(
    persisted: &Edit,
    expected_edit_revision: Revision,
    replacement: &Edit,
) -> bool {
    expected_edit_revision
        .checked_increment()
        .is_ok_and(|successor| replacement.revision() == successor && replacement == persisted)
}
