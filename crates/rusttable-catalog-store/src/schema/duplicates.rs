use std::collections::BTreeMap;

use redb::{Database, ReadableTable, TableDefinition};
use rusttable_catalog::RepositoryError;

use super::{
    CURRENT_SCHEMA_VERSION, PHOTO_GROUP_MEMBER_INDEX_TABLE, PHOTO_GROUPS_TABLE, RECORDS_TABLE,
    REFERENCE_PATH_INDEX_TABLE, SCHEMA_TABLE, VERSION_KEY,
};

pub(crate) const DUPLICATE_EVIDENCE_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_duplicate_evidence");
pub(crate) const DUPLICATE_SOURCE_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_duplicate_source_index");
pub(crate) const DUPLICATE_EXACT_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_duplicate_exact_index");
pub(crate) const DUPLICATE_EMBEDDED_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_duplicate_embedded_index");
pub(crate) const DUPLICATE_VISUAL_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_duplicate_visual_index");

pub(super) fn open_duplicate_tables(
    transaction: &redb::WriteTransaction,
) -> Result<(), RepositoryError> {
    transaction
        .open_table(DUPLICATE_EVIDENCE_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(DUPLICATE_SOURCE_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(DUPLICATE_EXACT_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(DUPLICATE_EMBEDDED_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(DUPLICATE_VISUAL_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    Ok(())
}

pub(super) fn migrate_duplicates_to_v13(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    open_duplicate_tables(&transaction)?;
    transaction
        .open_table(PHOTO_GROUPS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(PHOTO_GROUP_MEMBER_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    backfill_duplicate_evidence(&transaction)?;
    let mut schema = transaction
        .open_table(SCHEMA_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    schema
        .insert(VERSION_KEY, &[CURRENT_SCHEMA_VERSION][..])
        .map_err(|_| RepositoryError::Unavailable)?;
    drop(schema);
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}

pub(super) fn backfill_duplicate_evidence(
    transaction: &redb::WriteTransaction,
) -> Result<(), RepositoryError> {
    let path_identities = {
        let paths = transaction
            .open_table(REFERENCE_PATH_INDEX_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        paths
            .iter()
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .map(|entry| {
                let (identity, source) =
                    entry.map_err(|_| RepositoryError::CorruptPersistedData)?;
                let identity = identity
                    .value()
                    .try_into()
                    .map_err(|_| RepositoryError::CorruptPersistedData)?;
                Ok::<_, RepositoryError>((source.value().to_vec(), identity))
            })
            .collect::<Result<BTreeMap<Vec<u8>, [u8; 32]>, _>>()?
    };
    let evidence = {
        let records = transaction
            .open_table(RECORDS_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        records
            .iter()
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .filter_map(|entry| match entry {
                Ok((source, value)) => path_identities.get(source.value()).map(|identity| {
                    crate::codec::decode(value.value())
                        .map_err(|()| RepositoryError::CorruptPersistedData)
                        .map(|record| {
                            rusttable_catalog::DuplicateEvidence::from_record(
                                &record,
                                (*identity).into(),
                                None,
                            )
                        })
                }),
                Err(_) => Some(Err(RepositoryError::CorruptPersistedData)),
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    for item in evidence {
        stage_migrated_duplicate(transaction, item)?;
    }
    Ok(())
}

fn stage_migrated_duplicate(
    transaction: &redb::WriteTransaction,
    evidence: rusttable_catalog::DuplicateEvidence,
) -> Result<(), RepositoryError> {
    let encoded = crate::duplicate_codec::encode(evidence);
    let photo_id = evidence.photo_id().get().to_be_bytes();
    transaction
        .open_table(DUPLICATE_EVIDENCE_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?
        .insert(photo_id.as_slice(), encoded.as_slice())
        .map_err(|_| RepositoryError::Unavailable)?;
    insert_empty(
        transaction,
        DUPLICATE_SOURCE_INDEX_TABLE,
        &crate::duplicate_codec::source_index_key(evidence),
    )?;
    insert_empty(
        transaction,
        DUPLICATE_EXACT_INDEX_TABLE,
        &crate::duplicate_codec::exact_index_key(evidence),
    )?;
    if let Some(key) = crate::duplicate_codec::embedded_index_key(evidence) {
        insert_empty(transaction, DUPLICATE_EMBEDDED_INDEX_TABLE, &key)?;
    }
    if let Some(keys) = crate::duplicate_codec::visual_index_keys(evidence) {
        for key in keys {
            insert_empty(transaction, DUPLICATE_VISUAL_INDEX_TABLE, &key)?;
        }
    }
    Ok(())
}

fn insert_empty(
    transaction: &redb::WriteTransaction,
    definition: TableDefinition<&[u8], &[u8]>,
    key: &[u8],
) -> Result<(), RepositoryError> {
    transaction
        .open_table(definition)
        .map_err(|_| RepositoryError::Unavailable)?
        .insert(key, &[][..])
        .map_err(|_| RepositoryError::Unavailable)?;
    Ok(())
}
