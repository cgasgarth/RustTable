use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, WriteTransaction};
use rusttable_catalog::{
    CanonicalPayload, ContentBlobId, ContentBlobKind, HistoryInvariantReport, HistoryPage,
    HistoryPageRequest, HistoryRepository, HistoryRepositoryError, HistoryRevision,
    HistoryRevisionId, HistoryState, HistoryStateSnapshot, HistoryVersion, RepositoryError,
};
use rusttable_core::PhotoId;

use crate::schema::{
    self, HISTORY_BLOB_REFS_TABLE, HISTORY_BLOBS_TABLE, HISTORY_REVISIONS_TABLE,
    HISTORY_STATE_TABLE,
};

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
            database: schema::open(path).map_err(|error| map_schema_error(&error))?,
            photo_id,
        })
    }

    pub(crate) fn from_database(database: Arc<Database>, photo_id: PhotoId) -> Self {
        Self { database, photo_id }
    }
}

pub(crate) fn stage_history_commit(
    transaction: &WriteTransaction,
    photo_id: PhotoId,
    expected: HistoryVersion,
    state: &HistoryState,
) -> Result<(), HistoryRepositoryError> {
    if state.photo_id() != photo_id {
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
                .map(|bytes| (revision_key(photo_id, revision.id()), bytes))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|()| HistoryRepositoryError::CorruptPersistedData)?;
    let states = transaction
        .open_table(HISTORY_STATE_TABLE)
        .map_err(|_| HistoryRepositoryError::Unavailable)?;
    let key = photo_id.get().to_be_bytes();
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
    drop(states);
    let mut counts = std::collections::BTreeMap::<[u8; 43], u64>::new();
    let mut blobs = transaction
        .open_table(HISTORY_BLOBS_TABLE)
        .map_err(|_| HistoryRepositoryError::Unavailable)?;
    for revision in snapshot.revisions() {
        let payload = CanonicalPayload::from_history(revision.payload())
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
        for blob in [payload.edit(), payload.mask_blend(), payload.pipeline()] {
            let key = blob_key(blob.id());
            if let Some(existing) = blobs
                .get(key.as_slice())
                .map_err(|_| HistoryRepositoryError::Unavailable)?
            {
                if existing.value() != blob.bytes() {
                    return Err(HistoryRepositoryError::CorruptPersistedData);
                }
            } else {
                blobs
                    .insert(key.as_slice(), blob.bytes())
                    .map_err(|_| HistoryRepositoryError::Unavailable)?;
            }
            let count = counts.entry(key).or_default();
            *count = count
                .checked_add(1)
                .ok_or(HistoryRepositoryError::CorruptPersistedData)?;
        }
    }
    drop(blobs);
    let mut refs = transaction
        .open_table(HISTORY_BLOB_REFS_TABLE)
        .map_err(|_| HistoryRepositoryError::Unavailable)?;
    for (key, count) in counts {
        refs.insert(key.as_slice(), count.to_be_bytes().as_slice())
            .map_err(|_| HistoryRepositoryError::Unavailable)?;
    }
    drop(refs);
    let mut revisions = transaction
        .open_table(HISTORY_REVISIONS_TABLE)
        .map_err(|_| HistoryRepositoryError::Unavailable)?;
    for (key, bytes) in encoded_revisions {
        revisions
            .insert(key.as_slice(), bytes.as_slice())
            .map_err(|_| HistoryRepositoryError::Unavailable)?;
    }
    drop(revisions);
    let mut states = transaction
        .open_table(HISTORY_STATE_TABLE)
        .map_err(|_| HistoryRepositoryError::Unavailable)?;
    states
        .insert(key.as_slice(), metadata.as_slice())
        .map(|_| ())
        .map_err(|_| HistoryRepositoryError::Unavailable)
}

fn verify_blob_refs(
    actual: &std::collections::BTreeMap<[u8; 43], u64>,
    revisions: &[HistoryRevision],
) -> Result<(), HistoryRepositoryError> {
    let mut expected = std::collections::BTreeMap::<[u8; 43], u64>::new();
    for revision in revisions {
        let payload = CanonicalPayload::from_history(revision.payload())
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
        for blob in [payload.edit(), payload.mask_blend(), payload.pipeline()] {
            let count = expected.entry(blob_key(blob.id())).or_default();
            *count = count
                .checked_add(1)
                .ok_or(HistoryRepositoryError::CorruptPersistedData)?;
        }
    }
    if actual == &expected {
        Ok(())
    } else {
        Err(HistoryRepositoryError::CorruptPersistedData)
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
        let blobs_table = transaction
            .open_table(HISTORY_BLOBS_TABLE)
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
        let refs_table = transaction
            .open_table(HISTORY_BLOB_REFS_TABLE)
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
            let revision = crate::history_codec::decode_revision(value.value())
                .map_err(|()| HistoryRepositoryError::CorruptPersistedData)?;
            let payload = CanonicalPayload::from_history(revision.payload())
                .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
            for blob in [payload.edit(), payload.mask_blend(), payload.pipeline()] {
                let key = blob_key(blob.id());
                let stored = blobs_table
                    .get(key.as_slice())
                    .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?
                    .ok_or(HistoryRepositoryError::CorruptPersistedData)?;
                if stored.value() != blob.bytes() {
                    return Err(HistoryRepositoryError::CorruptPersistedData);
                }
            }
            revisions.push(revision);
        }
        let refcounts = refs_table
            .iter()
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?
            .map(|entry| {
                let (key, value) =
                    entry.map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
                let key = key
                    .value()
                    .try_into()
                    .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
                let count = u64::from_be_bytes(
                    value
                        .value()
                        .try_into()
                        .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?,
                );
                Ok::<_, HistoryRepositoryError>((key, count))
            })
            .collect::<Result<std::collections::BTreeMap<_, _>, _>>()?;
        verify_blob_refs(&refcounts, &revisions)?;
        revisions.sort_by_key(HistoryRevision::id);
        let snapshot = HistoryStateSnapshot::from_parts_with_journal(
            decoded.photo_id,
            decoded.version,
            decoded.commit_sequence,
            decoded.next_revision_id,
            decoded.next_branch_id,
            decoded.next_snapshot_id,
            decoded.active_branch,
            revisions,
            decoded.branches,
            decoded.snapshots,
            decoded.evidence,
            decoded.journal,
            decoded.provenance,
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
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| HistoryRepositoryError::Unavailable)?;
        stage_history_commit(&transaction, self.photo_id, expected, state)?;
        transaction
            .commit()
            .map_err(|_| HistoryRepositoryError::CommitFailure)
    }
}

impl RedbHistoryRepository {
    /// Returns the number of unique canonical payload blobs currently retained.
    ///
    /// # Errors
    ///
    /// Returns an availability or corruption error when the blob table cannot be read.
    pub fn blob_count(&self) -> Result<usize, HistoryRepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| HistoryRepositoryError::Unavailable)?;
        let blobs = transaction
            .open_table(HISTORY_BLOBS_TABLE)
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
        blobs
            .iter()
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?
            .try_fold(0_usize, |count, entry| {
                entry
                    .map_err(|_| HistoryRepositoryError::CorruptPersistedData)
                    .map(|_| count + 1)
            })
    }

    /// Lists immutable revisions in bounded ascending or descending pages.
    ///
    /// # Errors
    ///
    /// Returns an availability, corruption, or invalid-request error.
    pub fn page(
        &self,
        request: HistoryPageRequest,
    ) -> Result<Option<HistoryPage>, HistoryRepositoryError> {
        self.load()?
            .map(|state| state.page(request))
            .transpose()
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)
    }

    /// Reconstructs one immutable revision after validating all canonical blobs.
    ///
    /// # Errors
    ///
    /// Returns an availability, corruption, or invalid-revision error.
    pub fn reconstruct(
        &self,
        revision: HistoryRevisionId,
    ) -> Result<Option<HistoryRevision>, HistoryRepositoryError> {
        self.load()?
            .map(|state| state.reconstruct(revision))
            .transpose()
            .map_err(|_| HistoryRepositoryError::CorruptPersistedData)
    }

    /// Reopens and verifies graph, journal, blob, and reference-count invariants.
    ///
    /// # Errors
    ///
    /// Returns an availability or corruption error.
    pub fn verify_invariants(
        &self,
    ) -> Result<Option<HistoryInvariantReport>, HistoryRepositoryError> {
        let Some(state) = self.load()? else {
            return Ok(None);
        };
        Ok(Some(HistoryInvariantReport {
            photo_id: self.photo_id,
            revisions: state.revisions().count(),
            journal_entries: state.journal().count(),
            current_revision: state.current_pointer().revision(),
            unique_blobs: self.blob_count()?,
        }))
    }

    /// Deletes only blobs not referenced by any retained immutable revision.
    ///
    /// # Errors
    ///
    /// Returns an availability, corruption, or commit error without partial compaction.
    pub fn compact_unreachable(&mut self) -> Result<usize, HistoryRepositoryError> {
        let state = self
            .load()?
            .ok_or(HistoryRepositoryError::CorruptPersistedData)?;
        let reachable = state.revisions().try_fold(
            std::collections::BTreeSet::new(),
            |mut reachable, revision| {
                let payload = CanonicalPayload::from_history(revision.payload())
                    .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
                reachable.insert(payload.edit().id());
                reachable.insert(payload.mask_blend().id());
                reachable.insert(payload.pipeline().id());
                Ok::<_, HistoryRepositoryError>(reachable)
            },
        )?;
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| HistoryRepositoryError::Unavailable)?;
        let removed_keys = {
            let mut blobs = transaction
                .open_table(HISTORY_BLOBS_TABLE)
                .map_err(|_| HistoryRepositoryError::Unavailable)?;
            let unreachable_keys = blobs
                .iter()
                .map_err(|_| HistoryRepositoryError::Unavailable)?
                .map(|entry| {
                    let (key, _) =
                        entry.map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
                    let id = decode_blob_key(key.value())
                        .map_err(|()| HistoryRepositoryError::CorruptPersistedData)?;
                    Ok::<_, HistoryRepositoryError>(
                        (!reachable.contains(&id)).then(|| key.value().to_vec()),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();
            for key in &unreachable_keys {
                blobs
                    .remove(key.as_slice())
                    .map_err(|_| HistoryRepositoryError::Unavailable)?;
            }
            unreachable_keys.len()
        };
        transaction
            .commit()
            .map_err(|_| HistoryRepositoryError::CommitFailure)?;
        Ok(removed_keys)
    }

    /// Exports validated metadata and immutable revision records in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns a corruption error when any retained revision or canonical record is invalid.
    pub fn export_canonical(&self) -> Result<Vec<u8>, HistoryRepositoryError> {
        let state = self
            .load()?
            .ok_or(HistoryRepositoryError::CorruptPersistedData)?;
        let snapshot = state.persistence_snapshot();
        let mut output = crate::history_codec::encode_meta(&snapshot)
            .map_err(|()| HistoryRepositoryError::CorruptPersistedData)?;
        for revision in snapshot.revisions() {
            let bytes = crate::history_codec::encode_revision(revision)
                .map_err(|()| HistoryRepositoryError::CorruptPersistedData)?;
            let length = u64::try_from(bytes.len())
                .map_err(|_| HistoryRepositoryError::CorruptPersistedData)?;
            output.extend_from_slice(&length.to_be_bytes());
            output.extend_from_slice(&bytes);
        }
        Ok(output)
    }
}

fn revision_key(photo_id: PhotoId, revision: HistoryRevisionId) -> [u8; 24] {
    let mut key = [0; 24];
    key[..16].copy_from_slice(&photo_id.get().to_be_bytes());
    key[16..].copy_from_slice(&revision.get().to_be_bytes());
    key
}

fn blob_key(id: ContentBlobId) -> [u8; 43] {
    let mut key = [0; 43];
    key[0] = id.kind().tag();
    key[1..3].copy_from_slice(&id.schema().to_be_bytes());
    key[3..11].copy_from_slice(&id.length().to_be_bytes());
    key[11..].copy_from_slice(&id.digest());
    key
}

fn decode_blob_key(bytes: &[u8]) -> Result<ContentBlobId, ()> {
    if bytes.len() != 43 {
        return Err(());
    }
    let kind = match bytes[0] {
        1 => ContentBlobKind::Edit,
        2 => ContentBlobKind::MaskBlend,
        3 => ContentBlobKind::Pipeline,
        _ => return Err(()),
    };
    let schema = u16::from_be_bytes(bytes[1..3].try_into().map_err(|_| ())?);
    let length = u64::from_be_bytes(bytes[3..11].try_into().map_err(|_| ())?);
    let digest = bytes[11..].try_into().map_err(|_| ())?;
    Ok(ContentBlobId::from_parts(kind, schema, length, digest))
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
