use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable};
use rusttable_catalog::{
    EditRepository, EditRepositoryError, HistoryCommand, HistoryCommitReceipt,
    HistoryOperationKind, HistoryOperationSummary, HistoryPayload, HistoryRepository, HistoryState,
};
use rusttable_core::{Edit, EditId, Revision};

use super::history::{RedbHistoryRepository, stage_history_commit};
use crate::codecs::edit as edit_codec;
use crate::schema::{self, EDITS_TABLE};

/// Durable edit persistence backed by the shared `RustTable` redb catalog file.
pub struct RedbEditRepository {
    database: Arc<Database>,
}

impl RedbEditRepository {
    /// Opens the schema-versioned catalog file, including edit-table migration when needed.
    ///
    /// # Errors
    ///
    /// Returns a typed unavailable or corrupt-persisted-data error.
    pub fn open(path: &Path) -> Result<Self, EditRepositoryError> {
        Ok(Self {
            database: Arc::new(schema::open(path).map_err(|error| map_schema_error(&error))?),
        })
    }

    pub(crate) const fn from_database(database: Arc<Database>) -> Self {
        Self { database }
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
        self.commit_new_with_receipt(edit).map(|_| ())
    }

    fn commit_replacement(
        &mut self,
        expected_edit_revision: Revision,
        edit: &Edit,
    ) -> Result<(), EditRepositoryError> {
        self.commit_replacement_with_receipt(expected_edit_revision, edit)
            .map(|_| ())
    }
}

impl RedbEditRepository {
    /// Commits a new current edit and exactly one canonical immutable revision.
    ///
    /// # Errors
    ///
    /// Returns a conflict, corruption, availability, or commit error.
    pub fn commit_new_with_receipt(
        &mut self,
        edit: &Edit,
    ) -> Result<HistoryCommitReceipt, EditRepositoryError> {
        let (state, expected_history, receipt) = self.prepare_history(edit)?;
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
        stage_history_commit(&transaction, edit.photo_id(), expected_history, &state)
            .map_err(|error| map_history_error(&error))?;
        transaction
            .commit()
            .map_err(|_| EditRepositoryError::CommitFailure)?;
        Ok(receipt)
    }

    /// Commits one optimistic current-edit replacement and one canonical revision.
    ///
    /// # Errors
    ///
    /// Returns a conflict, corruption, availability, or commit error.
    pub fn commit_replacement_with_receipt(
        &mut self,
        expected_edit_revision: Revision,
        edit: &Edit,
    ) -> Result<HistoryCommitReceipt, EditRepositoryError> {
        let (state, expected_history, receipt) = self.prepare_history(edit)?;
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
        stage_history_commit(&transaction, edit.photo_id(), expected_history, &state)
            .map_err(|error| map_history_error(&error))?;
        transaction
            .commit()
            .map_err(|_| EditRepositoryError::CommitFailure)?;
        Ok(receipt)
    }

    pub(crate) fn prepare_history(
        &self,
        edit: &Edit,
    ) -> Result<
        (
            HistoryState,
            rusttable_catalog::HistoryVersion,
            HistoryCommitReceipt,
        ),
        EditRepositoryError,
    > {
        let repository =
            RedbHistoryRepository::from_database(Arc::clone(&self.database), edit.photo_id());
        let mut state = repository
            .load()
            .map_err(|error| map_history_error(&error))?
            .unwrap_or_else(|| HistoryState::new(edit.photo_id()));
        let expected = state.version();
        if state
            .current_revision()
            .is_some_and(|revision| revision.payload().edit() == edit)
        {
            let revision = state
                .current_pointer()
                .revision()
                .ok_or(EditRepositoryError::CorruptPersistedData)?;
            let receipt = state
                .receipt(revision)
                .map_err(|_| EditRepositoryError::CorruptPersistedData)?;
            return Ok((state, expected, receipt));
        }
        let summary = HistoryOperationSummary::new(
            HistoryOperationKind::Parameter,
            None,
            None,
            "current edit",
        )
        .map_err(|_| EditRepositoryError::CorruptPersistedData)?;
        state
            .apply(
                expected,
                HistoryCommand::Append {
                    payload: HistoryPayload::new(edit.clone(), Vec::new(), Vec::new(), summary),
                },
            )
            .map_err(|_| EditRepositoryError::CorruptPersistedData)?;
        let revision = state
            .current_pointer()
            .revision()
            .ok_or(EditRepositoryError::CorruptPersistedData)?;
        let receipt = state
            .receipt(revision)
            .map_err(|_| EditRepositoryError::CorruptPersistedData)?;
        Ok((state, expected, receipt))
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

fn map_history_error(error: &rusttable_catalog::HistoryRepositoryError) -> EditRepositoryError {
    match error {
        rusttable_catalog::HistoryRepositoryError::VersionConflict { .. }
        | rusttable_catalog::HistoryRepositoryError::CommitFailure => {
            EditRepositoryError::CommitFailure
        }
        rusttable_catalog::HistoryRepositoryError::Unavailable => EditRepositoryError::Unavailable,
        rusttable_catalog::HistoryRepositoryError::CorruptPersistedData => {
            EditRepositoryError::CorruptPersistedData
        }
    }
}
