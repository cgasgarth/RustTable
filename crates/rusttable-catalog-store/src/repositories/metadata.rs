use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable};
use rusttable_catalog::{
    CatalogMetadataBatch, CatalogMetadataBatchEdit, CatalogMetadataBatchReceipt,
    CatalogMetadataDocument, CatalogMetadataError, CatalogMetadataIndexTerm, CatalogMetadataKey,
    CatalogMetadataRepository, CatalogMetadataValue,
};
use rusttable_core::{PhotoId, Revision};
use sha2::{Digest, Sha256};

use crate::schema;

const REVISION_KEY: &[u8] = b"catalog-metadata-revision";
type BeforeCommitHook = Arc<dyn Fn() -> Result<(), CatalogMetadataError> + Send + Sync>;

/// Atomic redb adapter for typed catalog metadata and its rebuildable index.
pub struct RedbCatalogMetadataRepository {
    database: Arc<Database>,
    before_commit: Option<BeforeCommitHook>,
}

impl RedbCatalogMetadataRepository {
    /// Opens a schema-checked catalog metadata repository.
    ///
    /// # Errors
    /// Returns a typed storage or corruption error.
    pub fn open(path: &Path) -> Result<Self, CatalogMetadataError> {
        let database = schema::open(path).map_err(|error| map_schema_error(&error))?;
        Ok(Self {
            database,
            before_commit: None,
        })
    }

    /// Opens a repository with a test-only failure seam immediately before commit.
    ///
    /// # Errors
    /// Returns a typed storage or corruption error.
    #[doc(hidden)]
    pub fn open_with_before_commit_hook<F>(
        path: &Path,
        hook: F,
    ) -> Result<Self, CatalogMetadataError>
    where
        F: Fn() -> Result<(), CatalogMetadataError> + Send + Sync + 'static,
    {
        let database = schema::open(path).map_err(|error| map_schema_error(&error))?;
        Ok(Self {
            database,
            before_commit: Some(Arc::new(hook)),
        })
    }

    fn decode_document(
        bytes: &[u8],
        expected_photo_id: PhotoId,
    ) -> Result<CatalogMetadataDocument, CatalogMetadataError> {
        let document: CatalogMetadataDocument =
            postcard::from_bytes(bytes).map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
        document.validate()?;
        if document.photo_id() != expected_photo_id {
            return Err(CatalogMetadataError::CorruptPersistedData);
        }
        Ok(document)
    }
}

impl CatalogMetadataRepository for RedbCatalogMetadataRepository {
    fn catalog_revision(&self) -> Result<Revision, CatalogMetadataError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| CatalogMetadataError::Unavailable)?;
        let table = transaction
            .open_table(schema::METADATA_REVISION_TABLE)
            .map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
        let value = table
            .get(REVISION_KEY)
            .map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
        value.map_or(Ok(Revision::ZERO), |value| decode_revision(value.value()))
    }

    fn get(
        &self,
        photo_id: PhotoId,
    ) -> Result<Option<CatalogMetadataDocument>, CatalogMetadataError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| CatalogMetadataError::Unavailable)?;
        let table = transaction
            .open_table(schema::METADATA_DOCUMENTS_TABLE)
            .map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
        let key = photo_id.get().to_be_bytes();
        let value = table
            .get(key.as_slice())
            .map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
        value
            .map(|value| Self::decode_document(value.value(), photo_id))
            .transpose()
    }

    fn apply_batch(
        &mut self,
        batch: &CatalogMetadataBatch,
    ) -> Result<CatalogMetadataBatchReceipt, CatalogMetadataError> {
        batch.validate()?;
        let mut edits = batch.photos.iter().collect::<Vec<_>>();
        edits.sort_by_key(|edit| edit.photo_id);
        if edits.is_empty() {
            return Ok(CatalogMetadataBatchReceipt {
                catalog_revision: self.catalog_revision()?,
                photo_revisions: BTreeMap::new(),
                state_sha256: Sha256::digest([]).into(),
            });
        }
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| CatalogMetadataError::Unavailable)?;
        let actual_revision = read_write_revision(&transaction)?;
        if actual_revision != batch.expected_catalog_revision {
            tracing::warn!(
                expected_revision = batch.expected_catalog_revision.get(),
                actual_revision = actual_revision.get(),
                photo_count = batch.photos.len(),
                "catalog metadata batch rejected"
            );
            return Err(CatalogMetadataError::RevisionConflict {
                expected: batch.expected_catalog_revision,
                actual: actual_revision,
            });
        }
        let next_revision = actual_revision
            .checked_increment()
            .map_err(|_| CatalogMetadataError::RevisionOverflow)?;
        let changes = prepare_changes(&transaction, &edits)?;
        persist_changes(&transaction, &changes, next_revision)?;
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        let receipt = batch_receipt(next_revision, &changes);
        transaction
            .commit()
            .map_err(|_| CatalogMetadataError::CommitFailure)?;
        Ok(receipt)
    }

    fn find(
        &self,
        key: &CatalogMetadataKey,
        value: &CatalogMetadataValue,
    ) -> Result<Vec<PhotoId>, CatalogMetadataError> {
        let (key_hash, value_hash) = CatalogMetadataIndexTerm::query_hashes(key, value);
        let mut prefix = Vec::with_capacity(64);
        prefix.extend_from_slice(&key_hash);
        prefix.extend_from_slice(&value_hash);
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| CatalogMetadataError::Unavailable)?;
        let index = transaction
            .open_table(schema::METADATA_INDEX_TABLE)
            .map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
        let mut photo_ids = Vec::new();
        for entry in index
            .iter()
            .map_err(|_| CatalogMetadataError::CorruptPersistedData)?
        {
            let (stored_key, _) = entry.map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
            let stored_key = stored_key.value();
            if stored_key.starts_with(&prefix) {
                photo_ids.push(decode_index_photo_id(stored_key)?);
            }
        }
        drop(index);
        drop(transaction);
        let mut verified = Vec::with_capacity(photo_ids.len());
        for photo_id in photo_ids {
            let document = self
                .get(photo_id)?
                .ok_or(CatalogMetadataError::CorruptPersistedData)?;
            if document.fields().get(key).is_some_and(|field| {
                field.selected().privacy().is_indexable()
                    && field.selected().values().as_slice().contains(value)
            }) {
                verified.push(photo_id);
            }
        }
        Ok(verified)
    }

    fn rebuild_indexes(&mut self) -> Result<usize, CatalogMetadataError> {
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| CatalogMetadataError::Unavailable)?;
        let terms = {
            let documents = transaction
                .open_table(schema::METADATA_DOCUMENTS_TABLE)
                .map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
            let mut terms = Vec::new();
            for entry in documents
                .iter()
                .map_err(|_| CatalogMetadataError::CorruptPersistedData)?
            {
                let (key, value) = entry.map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
                let photo_id = decode_photo_key(key.value())?;
                let document = Self::decode_document(value.value(), photo_id)?;
                terms.extend(document.index_terms());
            }
            terms
        };
        let mut index = transaction
            .open_table(schema::METADATA_INDEX_TABLE)
            .map_err(|_| CatalogMetadataError::Unavailable)?;
        let keys = index
            .iter()
            .map_err(|_| CatalogMetadataError::Unavailable)?
            .map(|entry| {
                entry
                    .map(|(key, _)| key.value().to_vec())
                    .map_err(|_| CatalogMetadataError::Unavailable)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for key in keys {
            index
                .remove(key.as_slice())
                .map_err(|_| CatalogMetadataError::Unavailable)?;
        }
        for term in &terms {
            index
                .insert(index_key(*term).as_slice(), &[][..])
                .map_err(|_| CatalogMetadataError::Unavailable)?;
        }
        drop(index);
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction
            .commit()
            .map_err(|_| CatalogMetadataError::CommitFailure)?;
        Ok(terms.len())
    }
}

fn prepare_changes(
    transaction: &redb::WriteTransaction,
    edits: &[&CatalogMetadataBatchEdit],
) -> Result<Vec<(CatalogMetadataDocument, CatalogMetadataDocument)>, CatalogMetadataError> {
    let photos = transaction
        .open_table(schema::PHOTO_INDEX_TABLE)
        .map_err(|_| CatalogMetadataError::Unavailable)?;
    let documents = transaction
        .open_table(schema::METADATA_DOCUMENTS_TABLE)
        .map_err(|_| CatalogMetadataError::Unavailable)?;
    let mut changes = Vec::with_capacity(edits.len());
    for edit in edits {
        let key = edit.photo_id.get().to_be_bytes();
        if photos
            .get(key.as_slice())
            .map_err(|_| CatalogMetadataError::Unavailable)?
            .is_none()
        {
            tracing::warn!(
                photo_id = %edit.photo_id,
                "catalog metadata batch references an unknown photo"
            );
            return Err(CatalogMetadataError::PhotoNotFound(edit.photo_id));
        }
        let prior = documents
            .get(key.as_slice())
            .map_err(|_| CatalogMetadataError::Unavailable)?
            .map(|value| {
                RedbCatalogMetadataRepository::decode_document(value.value(), edit.photo_id)
            })
            .transpose()?
            .unwrap_or_else(|| CatalogMetadataDocument::empty(edit.photo_id));
        let updated = prior
            .apply(edit.expected_revision, &edit.edits)
            .map_err(|error| {
                tracing::warn!(
                    photo_id = %edit.photo_id,
                    expected_revision = edit.expected_revision.get(),
                    error = %error,
                    "catalog metadata document edit rejected"
                );
                error
            })?;
        changes.push((prior, updated));
    }
    Ok(changes)
}

fn persist_changes(
    transaction: &redb::WriteTransaction,
    changes: &[(CatalogMetadataDocument, CatalogMetadataDocument)],
    next_revision: Revision,
) -> Result<(), CatalogMetadataError> {
    let mut index = transaction
        .open_table(schema::METADATA_INDEX_TABLE)
        .map_err(|_| CatalogMetadataError::Unavailable)?;
    for (prior, updated) in changes {
        for term in prior.index_terms() {
            index
                .remove(index_key(term).as_slice())
                .map_err(|_| CatalogMetadataError::Unavailable)?;
        }
        for term in updated.index_terms() {
            index
                .insert(index_key(term).as_slice(), &[][..])
                .map_err(|_| CatalogMetadataError::Unavailable)?;
        }
    }
    drop(index);
    let mut documents = transaction
        .open_table(schema::METADATA_DOCUMENTS_TABLE)
        .map_err(|_| CatalogMetadataError::Unavailable)?;
    for (_, document) in changes {
        let key = document.photo_id().get().to_be_bytes();
        let value = postcard::to_allocvec(document)
            .map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
        documents
            .insert(key.as_slice(), value.as_slice())
            .map_err(|_| CatalogMetadataError::Unavailable)?;
    }
    drop(documents);
    let mut revisions = transaction
        .open_table(schema::METADATA_REVISION_TABLE)
        .map_err(|_| CatalogMetadataError::Unavailable)?;
    revisions
        .insert(REVISION_KEY, next_revision.get().to_be_bytes().as_slice())
        .map_err(|_| CatalogMetadataError::Unavailable)?;
    Ok(())
}

fn read_write_revision(
    transaction: &redb::WriteTransaction,
) -> Result<Revision, CatalogMetadataError> {
    let table = transaction
        .open_table(schema::METADATA_REVISION_TABLE)
        .map_err(|_| CatalogMetadataError::Unavailable)?;
    let value = table
        .get(REVISION_KEY)
        .map_err(|_| CatalogMetadataError::Unavailable)?;
    value.map_or(Ok(Revision::ZERO), |value| decode_revision(value.value()))
}

fn decode_revision(bytes: &[u8]) -> Result<Revision, CatalogMetadataError> {
    let bytes: [u8; 8] = bytes
        .try_into()
        .map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
    Ok(Revision::from_u64(u64::from_be_bytes(bytes)))
}

fn decode_photo_key(bytes: &[u8]) -> Result<PhotoId, CatalogMetadataError> {
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| CatalogMetadataError::CorruptPersistedData)?;
    PhotoId::new(u128::from_be_bytes(bytes)).ok_or(CatalogMetadataError::CorruptPersistedData)
}

fn decode_index_photo_id(bytes: &[u8]) -> Result<PhotoId, CatalogMetadataError> {
    bytes
        .get(64..)
        .ok_or(CatalogMetadataError::CorruptPersistedData)
        .and_then(decode_photo_key)
}

fn index_key(term: CatalogMetadataIndexTerm) -> [u8; 80] {
    let mut key = [0_u8; 80];
    key[..32].copy_from_slice(&term.key_sha256);
    key[32..64].copy_from_slice(&term.value_sha256);
    key[64..].copy_from_slice(&term.photo_id.get().to_be_bytes());
    key
}

fn batch_receipt(
    revision: Revision,
    changes: &[(CatalogMetadataDocument, CatalogMetadataDocument)],
) -> CatalogMetadataBatchReceipt {
    let mut digest = Sha256::new();
    let mut photo_revisions = BTreeMap::new();
    for (_, document) in changes {
        digest.update(document.photo_id().get().to_be_bytes());
        digest.update(document.canonical_sha256());
        photo_revisions.insert(document.photo_id(), document.revision());
    }
    CatalogMetadataBatchReceipt {
        catalog_revision: revision,
        photo_revisions,
        state_sha256: digest.finalize().into(),
    }
}

fn map_schema_error(error: &rusttable_catalog::RepositoryError) -> CatalogMetadataError {
    match error {
        rusttable_catalog::RepositoryError::CorruptPersistedData => {
            CatalogMetadataError::CorruptPersistedData
        }
        _ => CatalogMetadataError::Unavailable,
    }
}
