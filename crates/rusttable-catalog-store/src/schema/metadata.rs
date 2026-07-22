use redb::{Database, TableDefinition};
use rusttable_catalog::RepositoryError;

use super::{CURRENT_SCHEMA_VERSION, SCHEMA_TABLE, VERSION_KEY, open_tag_tables};

pub(crate) const METADATA_DOCUMENTS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_metadata_documents");
pub(crate) const METADATA_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_metadata_index");
pub(crate) const METADATA_REVISION_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_metadata_revision");

pub(crate) fn migrate_metadata_and_tags_to_v12(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    open_metadata_tables(&transaction)?;
    open_tag_tables(&transaction)?;
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

pub(crate) fn open_metadata_tables(
    transaction: &redb::WriteTransaction,
) -> Result<(), RepositoryError> {
    transaction
        .open_table(METADATA_DOCUMENTS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(METADATA_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(METADATA_REVISION_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    Ok(())
}
