use std::path::Path;

use redb::{Database, ReadableDatabase, ReadableTable};
use rusttable_catalog::{EditRepository, EditRepositoryError};
use rusttable_core::{Edit, EditId, Revision};

use crate::edit_codec;
use crate::schema::{self, EDITS_TABLE};

/// Durable edit persistence backed by the shared `RustTable` redb catalog file.
pub struct RedbEditRepository {
    database: Database,
}

impl RedbEditRepository {
    /// Opens the schema-versioned catalog file, including edit-table migration when needed.
    ///
    /// # Errors
    ///
    /// Returns a typed unavailable or corrupt-persisted-data error.
    pub fn open(path: &Path) -> Result<Self, EditRepositoryError> {
        Ok(Self {
            database: schema::open(path).map_err(|error| map_schema_error(&error))?,
        })
    }
}

impl EditRepository for RedbEditRepository {
    fn find_by_edit_id(&self, edit_id: EditId) -> Result<Option<Edit>, EditRepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| EditRepositoryError::Unavailable)?;
        let edits = transaction
            .open_table(EDITS_TABLE)
            .map_err(|_| EditRepositoryError::CorruptPersistedData)?;
        edits
            .get(edit_id.get().to_be_bytes().as_slice())
            .map_err(|_| EditRepositoryError::CorruptPersistedData)?
            .map(|value| {
                edit_codec::decode(value.value())
                    .map_err(|()| EditRepositoryError::CorruptPersistedData)
            })
            .transpose()
    }

    fn list(&self) -> Result<Vec<Edit>, EditRepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| EditRepositoryError::Unavailable)?;
        let edits = transaction
            .open_table(EDITS_TABLE)
            .map_err(|_| EditRepositoryError::CorruptPersistedData)?;
        edits
            .iter()
            .map_err(|_| EditRepositoryError::CorruptPersistedData)?
            .map(|entry| {
                let (_, value) = entry.map_err(|_| EditRepositoryError::CorruptPersistedData)?;
                edit_codec::decode(value.value())
                    .map_err(|()| EditRepositoryError::CorruptPersistedData)
            })
            .collect()
    }

    fn commit_new(&mut self, edit: &Edit) -> Result<(), EditRepositoryError> {
        let encoded =
            edit_codec::encode(edit).map_err(|()| EditRepositoryError::CorruptPersistedData)?;
        let key = edit.id().get().to_be_bytes();
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| EditRepositoryError::Unavailable)?;
        {
            let mut edits = transaction
                .open_table(EDITS_TABLE)
                .map_err(|_| EditRepositoryError::Unavailable)?;
            if edits
                .get(key.as_slice())
                .map_err(|_| EditRepositoryError::Unavailable)?
                .is_some()
            {
                return Err(EditRepositoryError::NewEditIdConflict { edit_id: edit.id() });
            }
            edits
                .insert(key.as_slice(), encoded.as_slice())
                .map_err(|_| EditRepositoryError::Unavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| EditRepositoryError::CommitFailure)
    }

    fn commit_replacement(
        &mut self,
        expected_edit_revision: Revision,
        edit: &Edit,
    ) -> Result<(), EditRepositoryError> {
        let encoded =
            edit_codec::encode(edit).map_err(|()| EditRepositoryError::CorruptPersistedData)?;
        let key = edit.id().get().to_be_bytes();
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| EditRepositoryError::Unavailable)?;
        {
            let mut edits = transaction
                .open_table(EDITS_TABLE)
                .map_err(|_| EditRepositoryError::Unavailable)?;
            let current_revision = {
                let current = edits
                    .get(key.as_slice())
                    .map_err(|_| EditRepositoryError::Unavailable)?
                    .ok_or(EditRepositoryError::UnknownEdit { edit_id: edit.id() })?;
                edit_codec::decode(current.value())
                    .map_err(|()| EditRepositoryError::CorruptPersistedData)?
                    .revision()
            };
            if current_revision != expected_edit_revision {
                return Err(EditRepositoryError::EditRevisionConflict {
                    edit_id: edit.id(),
                    expected: expected_edit_revision,
                    actual: current_revision,
                });
            }
            edits
                .insert(key.as_slice(), encoded.as_slice())
                .map_err(|_| EditRepositoryError::Unavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| EditRepositoryError::CommitFailure)
    }
}

fn map_schema_error(error: &rusttable_catalog::RepositoryError) -> EditRepositoryError {
    match error {
        rusttable_catalog::RepositoryError::Unavailable
        | rusttable_catalog::RepositoryError::CommitFailure => EditRepositoryError::Unavailable,
        rusttable_catalog::RepositoryError::CorruptPersistedData
        | rusttable_catalog::RepositoryError::SourceConflict { .. }
        | rusttable_catalog::RepositoryError::PhotoIdConflict { .. }
        | rusttable_catalog::RepositoryError::AssetIdConflict { .. } => {
            EditRepositoryError::CorruptPersistedData
        }
    }
}
