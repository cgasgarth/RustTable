use std::fmt;
use std::path::Path;

use rusttable_catalog::{EditRepository, EditRepositoryError};
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, EditId, PhotoId, Revision};

use super::{BasicEditDraft, BasicEditDraftReplacementError};

/// Commits one validated basic-edit draft after verifying its persisted identity.
///
/// The replacement contains exposure and RGB gain together, so callers cannot
/// persist an intermediate edit with only part of the draft applied.
///
/// # Errors
///
/// Returns a typed error when the persisted edit has changed, the draft cannot
/// produce a valid replacement, or the repository rejects the transaction.
pub fn commit_basic_edit(
    repository: &mut dyn EditRepository,
    draft: &BasicEditDraft,
) -> Result<Edit, BasicEditCommitError> {
    let persisted = repository
        .find_by_edit_id(draft.edit_id())
        .map_err(BasicEditCommitError::Repository)?
        .ok_or(BasicEditCommitError::MissingEdit {
            edit_id: draft.edit_id(),
        })?;
    verify_persisted_edit(&persisted, draft)?;
    let replacement = draft
        .replacement_edit()
        .map_err(BasicEditCommitError::Draft)?;
    repository
        .commit_replacement(draft.edit_revision(), &replacement)
        .map_err(BasicEditCommitError::Repository)?;
    Ok(replacement)
}

/// Opens the application catalog and atomically commits one basic-edit draft.
///
/// # Errors
///
/// Returns a typed error when the catalog cannot open or the atomic edit
/// replacement cannot be completed.
pub fn commit_basic_edit_at_path(
    catalog_path: &Path,
    draft: &BasicEditDraft,
) -> Result<Edit, BasicEditCommitError> {
    let mut repository =
        RedbCatalogRepository::open(catalog_path).map_err(BasicEditCommitError::Catalog)?;
    commit_basic_edit(&mut repository, draft)
}

fn verify_persisted_edit(
    persisted: &Edit,
    draft: &BasicEditDraft,
) -> Result<(), BasicEditCommitError> {
    if persisted.photo_id() != draft.photo_id() {
        return Err(BasicEditCommitError::PhotoMismatch {
            edit_id: draft.edit_id(),
            expected: draft.photo_id(),
            actual: persisted.photo_id(),
        });
    }
    if persisted.base_photo_revision() != draft.base_photo_revision() {
        return Err(BasicEditCommitError::BasePhotoRevisionMismatch {
            edit_id: draft.edit_id(),
            expected: draft.base_photo_revision(),
            actual: persisted.base_photo_revision(),
        });
    }
    if persisted.revision() != draft.edit_revision() {
        return Err(BasicEditCommitError::RevisionConflict {
            edit_id: draft.edit_id(),
            expected: draft.edit_revision(),
            actual: persisted.revision(),
        });
    }
    Ok(())
}

#[derive(Debug)]
pub enum BasicEditCommitError {
    Catalog(rusttable_catalog::RepositoryError),
    Repository(EditRepositoryError),
    Draft(BasicEditDraftReplacementError),
    MissingEdit {
        edit_id: EditId,
    },
    PhotoMismatch {
        edit_id: EditId,
        expected: PhotoId,
        actual: PhotoId,
    },
    BasePhotoRevisionMismatch {
        edit_id: EditId,
        expected: Revision,
        actual: Revision,
    },
    RevisionConflict {
        edit_id: EditId,
        expected: Revision,
        actual: Revision,
    },
}

impl fmt::Display for BasicEditCommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Catalog(error) => write!(formatter, "catalog unavailable: {error}"),
            Self::Repository(error) => write!(formatter, "edit repository failure: {error}"),
            Self::Draft(error) => write!(formatter, "invalid basic edit draft: {error}"),
            Self::MissingEdit { edit_id } => write!(formatter, "edit {edit_id} no longer exists"),
            Self::PhotoMismatch {
                edit_id,
                expected,
                actual,
            } => write!(
                formatter,
                "edit {edit_id} belongs to photo {actual}, not expected photo {expected}"
            ),
            Self::BasePhotoRevisionMismatch {
                edit_id,
                expected,
                actual,
            } => write!(
                formatter,
                "edit {edit_id} base photo revision changed from {expected} to {actual}"
            ),
            Self::RevisionConflict {
                edit_id,
                expected,
                actual,
            } => write!(
                formatter,
                "edit {edit_id} revision changed from {expected} to {actual}"
            ),
        }
    }
}

impl std::error::Error for BasicEditCommitError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Catalog(error) => Some(error),
            Self::Repository(error) => Some(error),
            Self::Draft(error) => Some(error),
            Self::MissingEdit { .. }
            | Self::PhotoMismatch { .. }
            | Self::BasePhotoRevisionMismatch { .. }
            | Self::RevisionConflict { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use rusttable_catalog::{EditRepository, EditRepositoryError};
    use rusttable_core::{
        Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity,
        ParameterName, ParameterValue, PhotoId, Revision,
    };

    use super::{BasicEditCommitError, BasicEditDraft, commit_basic_edit};

    #[derive(Debug)]
    struct Repository {
        edit: Edit,
        commits: usize,
    }

    impl EditRepository for Repository {
        fn find_by_edit_id(&self, edit_id: EditId) -> Result<Option<Edit>, EditRepositoryError> {
            Ok((edit_id == self.edit.id()).then(|| self.edit.clone()))
        }

        fn list(&self) -> Result<Vec<Edit>, EditRepositoryError> {
            Ok(vec![self.edit.clone()])
        }

        fn commit_new(&mut self, _edit: &Edit) -> Result<(), EditRepositoryError> {
            unreachable!("replacement test never creates an edit")
        }

        fn commit_replacement(
            &mut self,
            expected_edit_revision: Revision,
            edit: &Edit,
        ) -> Result<(), EditRepositoryError> {
            if self.edit.revision() != expected_edit_revision {
                return Err(EditRepositoryError::EditRevisionConflict {
                    edit_id: self.edit.id(),
                    expected: expected_edit_revision,
                    actual: self.edit.revision(),
                });
            }
            self.edit = edit.clone();
            self.commits += 1;
            Ok(())
        }
    }

    fn edit(revision: u64) -> Edit {
        let scalar = |value| ParameterValue::Scalar(FiniteF64::new(value).unwrap());
        Edit::from_parts(
            EditId::new(1).unwrap(),
            PhotoId::new(2).unwrap(),
            Revision::from_u64(3),
            Revision::from_u64(revision),
            [
                operation(10, "rusttable.exposure", [("stops", scalar(0.0))]),
                operation(
                    20,
                    "rusttable.rgb_gain",
                    [
                        ("red", scalar(1.0)),
                        ("green", scalar(1.0)),
                        ("blue", scalar(1.0)),
                    ],
                ),
            ],
        )
        .unwrap()
    }

    fn operation<const N: usize>(
        id: u128,
        key: &'static str,
        parameters: [(&'static str, ParameterValue); N],
    ) -> Operation {
        Operation::new_with_opacity(
            OperationId::new(id).unwrap(),
            OperationKey::new(key).unwrap(),
            true,
            OperationOpacity::ONE,
            parameters
                .into_iter()
                .map(|(name, value)| (ParameterName::new(name).unwrap(), value)),
        )
        .unwrap()
    }

    #[test]
    fn commits_all_basic_values_as_one_replacement() {
        let original = edit(4);
        let mut draft = BasicEditDraft::from_edit(&original).unwrap();
        draft.set_exposure_stops(1.25).unwrap();
        draft.set_rgb_red(0.4).unwrap();
        draft.set_rgb_green(1.5).unwrap();
        draft.set_rgb_blue(0.8).unwrap();
        let mut repository = Repository {
            edit: original,
            commits: 0,
        };

        let committed = commit_basic_edit(&mut repository, &draft).unwrap();

        assert_eq!(repository.commits, 1);
        assert_eq!(repository.edit, committed);
        assert_eq!(committed.revision(), Revision::from_u64(5));
        let reparsed = BasicEditDraft::from_edit(&committed).unwrap();
        assert_eq!(reparsed.exposure_stops().to_bits(), 1.25_f64.to_bits());
        assert_eq!(reparsed.rgb_red().to_bits(), 0.4_f64.to_bits());
        assert_eq!(reparsed.rgb_green().to_bits(), 1.5_f64.to_bits());
        assert_eq!(reparsed.rgb_blue().to_bits(), 0.8_f64.to_bits());
    }

    #[test]
    fn stale_draft_does_not_write_a_replacement() {
        let draft = BasicEditDraft::from_edit(&edit(4)).unwrap();
        let mut repository = Repository {
            edit: edit(5),
            commits: 0,
        };

        assert!(matches!(
            commit_basic_edit(&mut repository, &draft),
            Err(BasicEditCommitError::RevisionConflict {
                expected,
                actual,
                ..
            }) if expected == Revision::from_u64(4) && actual == Revision::from_u64(5)
        ));
        assert_eq!(repository.commits, 0);
        assert_eq!(repository.edit.revision(), Revision::from_u64(5));
    }
}
