use redb::{Database, ReadableTable, TableDefinition};

use rusttable_catalog::RepositoryError;

pub(crate) const VIRTUAL_COPIES_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_virtual_copies");
pub(crate) const VIRTUAL_COPY_STATE_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_virtual_copy_state");
pub(crate) const VIRTUAL_COPY_REVISION_KEY: &[u8] = b"virtual-copy-revision";

pub(super) fn open_tables(transaction: &redb::WriteTransaction) -> Result<(), RepositoryError> {
    transaction
        .open_table(VIRTUAL_COPIES_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(VIRTUAL_COPY_STATE_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    Ok(())
}

pub(super) fn ensure_tables(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    open_tables(&transaction)?;
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}

pub(super) fn migrate_to_v15(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    open_tables(&transaction)?;
    let mut schema = transaction
        .open_table(super::SCHEMA_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let version = schema
        .get(super::VERSION_KEY)
        .map_err(|_| RepositoryError::CorruptPersistedData)?
        .ok_or(RepositoryError::CorruptPersistedData)?;
    if version.value() != [14] {
        return Err(RepositoryError::CorruptPersistedData);
    }
    drop(version);
    schema
        .insert(super::VERSION_KEY, &[super::CURRENT_SCHEMA_VERSION][..])
        .map_err(|_| RepositoryError::Unavailable)?;
    drop(schema);
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}
