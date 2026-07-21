use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable};
use rusttable_catalog::{
    HistoryRepository, HistoryRepositoryError, HistoryRevision, HistoryRevisionId, HistoryState,
    HistoryStateSnapshot, HistoryVersion, RepositoryError,
};
use rusttable_core::PhotoId;

use crate::schema::{self, HISTORY_REVISIONS_TABLE, HISTORY_STATE_TABLE};

/// Transactional redb persistence for one photo's immutable edit-history graph.
pub struct RedbHistoryRepository {
    database: Arc<Database>,
    photo_id: PhotoId,
}

impl RedbHistoryRepository {
    /// Opens the shared schema-versioned catalog and selects one photo history.
    ///
    /// # Errors
    ///
    /// Returns a typed schema, availability, or corruption error.
    pub fn open(path: &Path, photo_id: PhotoId) -> Result<Self, HistoryRepositoryError> {
        Ok(Self {
            database: Arc::new(schema::open(path).map_err(|error| map_schema_error(&error))?),
            photo_id,
        })
    }
}

impl HistoryRepository for RedbHistoryRepository {
    fn load(&self) -> Result<Option<HistoryState>, HistoryRepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| HistoryRepositoryError::Unavailable)?;
        let states = transaction
            .open_table(HISTORY_STATE_TABLE)
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
        let key = self.photo_id.get().to_be_bytes();
        let Some(meta) = states
            .get(key.as_slice())
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?
        else {
            return Ok(None);
        };
        let decoded = crate::history_codec::decode_meta(meta.value())
            .map_err(|()| HistoryRepositoryError::CorruptPersistedData)?;
        if decoded.photo_id != self.photo_id {
            return Err(HistoryRepositoryError::CorruptPersistedData);
        }
        let revisions_table = transaction
            .open_table(HISTORY_REVISIONS_TABLE)
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
        let prefix = key;
        let mut revisions = Vec::new();
        for entry in revisions_table
            .iter()
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?
        {
            let (revision_key, value) =
                entry.map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
            let bytes = revision_key.value();
            if bytes.len() != 24 || bytes[..16] != prefix {
                continue;
            }
            revisions.push(
                crate::history_codec::decode_revision(value.value())
                    .map_err(|()| HistoryRepositoryError::CorruptPersistedData)?,
            );
        }
        revisions.sort_by_key(HistoryRevision::id);
        let snapshot = HistoryStateSnapshot::from_parts(
            decoded.photo_id,
            decoded.version,
            decoded.next_revision_id,
            decoded.next_branch_id,
            decoded.next_snapshot_id,
            decoded.active_branch,
            revisions,
            decoded.branches,
            decoded.snapshots,
            decoded.evidence,
        );
        HistoryState::restore(snapshot)
            .map(Some)
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)
    }

    fn commit(
        &mut self,
        expected: HistoryVersion,
        state: &HistoryState,
    ) -> Result<(), HistoryRepositoryError> {
        if state.photo_id() != self.photo_id {
            return Err(HistoryRepositoryError::CorruptPersistedData);
        }
        let snapshot = state.persistence_snapshot();
        let metadata = crate::history_codec::encode_meta(&snapshot)
            .map_err(|()| HistoryRepositoryError::CorruptPersistedData)?;
        let encoded_revisions = snapshot
            .revisions()
            .iter()
            .map(|revision| {
                crate::history_codec::encode_revision(revision)
                    .map(|bytes| (revision_key(self.photo_id, revision.id()), bytes))
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(|()| HistoryRepositoryError::CorruptPersistedData)?;

        let transaction = self
            .database
            .begin_write()
            .map_err(|_| HistoryRepositoryError::Unavailable)?;
        {
            let mut states = transaction
                .open_table(HISTORY_STATE_TABLE)
                .map_err(|_| HistoryRepositoryError::Unavailable)?;
            let key = self.photo_id.get().to_be_bytes();
            let actual = states
                .get(key.as_slice())
                .map_err(|_| HistoryRepositoryError::Unavailable)?
                .map(|value| {
                    crate::history_codec::decode_meta(value.value())
                        .map(|meta| meta.version)
                        .map_err(|()| HistoryRepositoryError::CorruptPersistedData)
                })
                .transpose()?
                .unwrap_or(HistoryVersion::ZERO);
            if actual != expected {
                return Err(HistoryRepositoryError::VersionConflict { expected, actual });
            }
            states
                .insert(key.as_slice(), metadata.as_slice())
                .map_err(|_| HistoryRepositoryError::Unavailable)?;
        }
        {
            let mut revisions_table = transaction
                .open_table(HISTORY_REVISIONS_TABLE)
                .map_err(|_| HistoryRepositoryError::Unavailable)?;
            let prefix = self.photo_id.get().to_be_bytes();
            let keep = encoded_revisions
                .iter()
                .map(|(key, _)| key.to_vec())
                .collect::<std::collections::BTreeSet<_>>();
            let stale_keys = revisions_table
                .iter()
                .map_err(|_| HistoryRepositoryError::Unavailable)?
                .filter_map(Result::ok)
                .filter_map(|(key, _)| {
                    let key = key.value();
                    (key.len() == 24 && key[..16] == prefix && !keep.contains(key))
                        .then(|| key.to_vec())
                })
                .collect::<Vec<_>>();
            for key in stale_keys {
                revisions_table
                    .remove(key.as_slice())
                    .map_err(|_| HistoryRepositoryError::Unavailable)?;
            }
            for (key, bytes) in encoded_revisions {
                revisions_table
                    .insert(key.as_slice(), bytes.as_slice())
                    .map_err(|_| HistoryRepositoryError::Unavailable)?;
            }
        }
        transaction
            .commit()
            .map_err(|_| HistoryRepositoryError::CommitFailure)
    }
}

fn revision_key(photo_id: PhotoId, revision: HistoryRevisionId) -> [u8; 24] {
    let mut key = [0; 24];
    key[..16].copy_from_slice(&photo_id.get().to_be_bytes());
    key[16..].copy_from_slice(&revision.get().to_be_bytes());
    key
}

fn map_schema_error(error: &RepositoryError) -> HistoryRepositoryError {
    match error {
        RepositoryError::Unavailable => HistoryRepositoryError::Unavailable,
        RepositoryError::CommitFailure => HistoryRepositoryError::CommitFailure,
        RepositoryError::CorruptPersistedData
        | RepositoryError::SourceConflict { .. }
        | RepositoryError::PhotoIdConflict { .. }
        | RepositoryError::AssetIdConflict { .. } => HistoryRepositoryError::CorruptPersistedData,
    }
}
