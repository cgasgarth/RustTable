use std::collections::BTreeSet;

use redb::{ReadableDatabase, ReadableTable, WriteTransaction};
use rusttable_catalog::{
    DuplicateEvidence, DuplicateSearchResult, MAX_DUPLICATE_CANDIDATES, classify_duplicate,
};
use rusttable_core::PhotoId;

use super::{AtomicCatalogStoreError, RedbCatalogRepository};
use crate::schema;

impl RedbCatalogRepository {
    /// Finds bounded, deterministic duplicate classifications for review.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when an index or its evidence row is unavailable or corrupt.
    pub fn find_duplicates(
        &self,
        evidence: DuplicateEvidence,
    ) -> Result<DuplicateSearchResult, AtomicCatalogStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        let mut candidates = BTreeSet::new();
        let mut truncated = false;
        collect_index_candidates(
            &transaction,
            schema::DUPLICATE_SOURCE_INDEX_TABLE,
            &evidence.source().as_bytes(),
            48,
            &mut candidates,
            &mut truncated,
        )?;
        let mut exact_prefix = [0_u8; 40];
        exact_prefix[..32].copy_from_slice(&evidence.exact().sha256());
        exact_prefix[32..].copy_from_slice(&evidence.exact().byte_length().to_be_bytes());
        collect_index_candidates(
            &transaction,
            schema::DUPLICATE_EXACT_INDEX_TABLE,
            &exact_prefix,
            56,
            &mut candidates,
            &mut truncated,
        )?;
        if let Some(embedded) = evidence.embedded() {
            collect_index_candidates(
                &transaction,
                schema::DUPLICATE_EMBEDDED_INDEX_TABLE,
                &embedded.digest(),
                48,
                &mut candidates,
                &mut truncated,
            )?;
        }
        if let Some(visual) = evidence.visual() {
            for (index, chunk) in visual.index_chunks().into_iter().enumerate() {
                let mut prefix = [0_u8; 3];
                prefix[0] = u8::try_from(index).unwrap_or_default();
                prefix[1..].copy_from_slice(&chunk.to_be_bytes());
                collect_index_candidates(
                    &transaction,
                    schema::DUPLICATE_VISUAL_INDEX_TABLE,
                    &prefix,
                    19,
                    &mut candidates,
                    &mut truncated,
                )?;
            }
        }
        if candidates.len() > MAX_DUPLICATE_CANDIDATES {
            truncated = true;
            candidates = candidates
                .into_iter()
                .take(MAX_DUPLICATE_CANDIDATES)
                .collect();
        }
        let table = transaction
            .open_table(schema::DUPLICATE_EVIDENCE_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let mut matches = Vec::new();
        for photo_id in candidates {
            let persisted = table
                .get(photo_id.get().to_be_bytes().as_slice())
                .map_err(|_| AtomicCatalogStoreError::Corrupt)?
                .ok_or(AtomicCatalogStoreError::Corrupt)?;
            let persisted = crate::duplicate_codec::decode(persisted.value())
                .map_err(|()| AtomicCatalogStoreError::Corrupt)?;
            if persisted.photo_id() != photo_id {
                return Err(AtomicCatalogStoreError::Corrupt);
            }
            if let Some(duplicate) = classify_duplicate(evidence, persisted) {
                matches.push(duplicate);
            }
        }
        Ok(DuplicateSearchResult::from_candidates(matches, truncated))
    }
}

fn collect_index_candidates(
    transaction: &redb::ReadTransaction,
    definition: redb::TableDefinition<&[u8], &[u8]>,
    prefix: &[u8],
    key_length: usize,
    candidates: &mut BTreeSet<PhotoId>,
    truncated: &mut bool,
) -> Result<(), AtomicCatalogStoreError> {
    let mut lower = vec![0_u8; key_length];
    let mut upper = vec![u8::MAX; key_length];
    lower
        .get_mut(..prefix.len())
        .ok_or(AtomicCatalogStoreError::Corrupt)?
        .copy_from_slice(prefix);
    upper
        .get_mut(..prefix.len())
        .ok_or(AtomicCatalogStoreError::Corrupt)?
        .copy_from_slice(prefix);
    let table = transaction
        .open_table(definition)
        .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
    for (position, entry) in table
        .range(lower.as_slice()..=upper.as_slice())
        .map_err(|_| AtomicCatalogStoreError::Corrupt)?
        .enumerate()
    {
        if position == MAX_DUPLICATE_CANDIDATES {
            *truncated = true;
            break;
        }
        let (key, _) = entry.map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let photo_bytes = key
            .value()
            .get(key_length - 16..)
            .ok_or(AtomicCatalogStoreError::Corrupt)?
            .try_into()
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let photo_id = PhotoId::new(u128::from_be_bytes(photo_bytes))
            .ok_or(AtomicCatalogStoreError::Corrupt)?;
        candidates.insert(photo_id);
    }
    Ok(())
}

pub(super) fn stage_duplicate_evidence(
    transaction: &WriteTransaction,
    evidence: DuplicateEvidence,
    replace: bool,
) -> Result<(), AtomicCatalogStoreError> {
    let photo_id = evidence.photo_id().get().to_be_bytes();
    let old = transaction
        .open_table(schema::DUPLICATE_EVIDENCE_TABLE)
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?
        .get(photo_id.as_slice())
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?
        .map(|value| crate::duplicate_codec::decode(value.value()))
        .transpose()
        .map_err(|()| AtomicCatalogStoreError::Corrupt)?;
    match (replace, old) {
        (false, Some(_)) => return Err(AtomicCatalogStoreError::Conflict),
        (true, Some(old)) => remove_duplicate_indexes(transaction, old)?,
        (true | false, None) => {}
    }
    let encoded = crate::duplicate_codec::encode(evidence);
    transaction
        .open_table(schema::DUPLICATE_EVIDENCE_TABLE)
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?
        .insert(photo_id.as_slice(), encoded.as_slice())
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
    insert_duplicate_indexes(transaction, evidence)
}

fn insert_duplicate_indexes(
    transaction: &WriteTransaction,
    evidence: DuplicateEvidence,
) -> Result<(), AtomicCatalogStoreError> {
    insert_duplicate_index(
        transaction,
        schema::DUPLICATE_SOURCE_INDEX_TABLE,
        &crate::duplicate_codec::source_index_key(evidence),
    )?;
    insert_duplicate_index(
        transaction,
        schema::DUPLICATE_EXACT_INDEX_TABLE,
        &crate::duplicate_codec::exact_index_key(evidence),
    )?;
    if let Some(key) = crate::duplicate_codec::embedded_index_key(evidence) {
        insert_duplicate_index(transaction, schema::DUPLICATE_EMBEDDED_INDEX_TABLE, &key)?;
    }
    if let Some(keys) = crate::duplicate_codec::visual_index_keys(evidence) {
        for key in keys {
            insert_duplicate_index(transaction, schema::DUPLICATE_VISUAL_INDEX_TABLE, &key)?;
        }
    }
    Ok(())
}

fn remove_duplicate_indexes(
    transaction: &WriteTransaction,
    evidence: DuplicateEvidence,
) -> Result<(), AtomicCatalogStoreError> {
    remove_duplicate_index(
        transaction,
        schema::DUPLICATE_SOURCE_INDEX_TABLE,
        &crate::duplicate_codec::source_index_key(evidence),
    )?;
    remove_duplicate_index(
        transaction,
        schema::DUPLICATE_EXACT_INDEX_TABLE,
        &crate::duplicate_codec::exact_index_key(evidence),
    )?;
    if let Some(key) = crate::duplicate_codec::embedded_index_key(evidence) {
        remove_duplicate_index(transaction, schema::DUPLICATE_EMBEDDED_INDEX_TABLE, &key)?;
    }
    if let Some(keys) = crate::duplicate_codec::visual_index_keys(evidence) {
        for key in keys {
            remove_duplicate_index(transaction, schema::DUPLICATE_VISUAL_INDEX_TABLE, &key)?;
        }
    }
    Ok(())
}

fn insert_duplicate_index(
    transaction: &WriteTransaction,
    definition: redb::TableDefinition<&[u8], &[u8]>,
    key: &[u8],
) -> Result<(), AtomicCatalogStoreError> {
    transaction
        .open_table(definition)
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?
        .insert(key, &[][..])
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
    Ok(())
}

fn remove_duplicate_index(
    transaction: &WriteTransaction,
    definition: redb::TableDefinition<&[u8], &[u8]>,
    key: &[u8],
) -> Result<(), AtomicCatalogStoreError> {
    if transaction
        .open_table(definition)
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?
        .remove(key)
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?
        .is_none()
    {
        return Err(AtomicCatalogStoreError::Corrupt);
    }
    Ok(())
}
